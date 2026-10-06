use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::api::{ApiError, AuthRefreshClient, UsageApiClient};
use crate::auth::{AuthFileService, HashMapEmailSnapshots};
use crate::models::{AuthFileSnapshot, AuthTokens, ResetCreditsPayload, UsageMap, UsageResponse};
use crate::state::{AUTO_FETCH_OPTIONS, MonitorState, now_secs};
use crate::storage::UsageStorage;
use crate::watcher::file_signature;

pub struct AppState {
    pub inner: Mutex<MonitorState>,
    pub log_path: PathBuf,
    pub login: Mutex<Option<LoginSession>>,
}

pub struct LoginSession {
    pub home: PathBuf,
    pub cancel: Arc<AtomicBool>,
    pub outcome: crate::oauth_login::CallbackOutcome,
    pub verifier: String,
    pub port: u16,
}

#[derive(Serialize)]
pub struct Snapshot {
    pub accounts: UsageMap,
    pub current_email: Option<String>,
    pub auto_fetch: String,
    pub auto_fetch_options: Vec<String>,
    pub sort_column: Option<String>,
    pub sort_asc: bool,
    pub show_archived: bool,
    pub logs_expanded: bool,
    pub auth_file_exists: bool,
    pub backup_emails: Vec<String>,
    pub app_version: String,
}

#[derive(Serialize)]
#[serde(tag = "kind")]
pub enum AuthOutcome {
    NoChange,
    Fetched { email: String, message: String },
    AuthRefreshed { message: String },
    LoggedOut { message: String },
    MissingToken,
    ParseError { message: String },
    NoFile,
}

#[derive(Serialize)]
pub struct FetchResult {
    pub email: String,
    pub message: String,
}

fn boot_line(log_path: &Path, message: &str) {
    if let Some(parent) = log_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    use std::fmt::Write as _;
    let mut line = String::new();
    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let _ = writeln!(line, "[{ts}] {message}");
    use std::io::Write as _;
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(log_path) {
        let _ = f.write_all(line.as_bytes());
    }
}

fn append_log(state: &AppState, message: &str) {    if message.is_empty() {
        return;
    }
    if let Some(parent) = state.log_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    use std::fmt::Write as _;
    let mut line = String::new();
    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let _ = writeln!(line, "[{ts}] {message}");
    use std::io::Write as _;
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&state.log_path) {
        let _ = f.write_all(line.as_bytes());
    }
}

fn state_lock<'a>(state: &'a State<AppState>) -> std::sync::MutexGuard<'a, MonitorState> {
    state.inner.lock().unwrap()
}

/// Network half of a quota fetch: usage + reset credits. Holds NO locks —
/// callers must gather inputs first and apply afterwards, otherwise every
/// other command queues behind the mutex and the UI freezes.
struct FetchedData {
    response: UsageResponse,
    credits: Option<ResetCreditsPayload>,
}

async fn fetch_remote(
    api: &UsageApiClient,
    jwt: &str,
    fallback_account_id: Option<&str>,
) -> Result<FetchedData, String> {
    let response: UsageResponse = api.fetch_usage(jwt).await.map_err(|e| e.to_string())?;
    let account_id = response.account_id.clone().or_else(|| fallback_account_id.map(String::from));
    let mut credits = None;
    if let Some(account_id) = account_id {
        match api.fetch_reset_credits(jwt, &account_id).await {
            Ok(c) => credits = Some(c),
            Err(ApiError::Unauthorized) => {}
            Err(e) => {
                return Err(format!("Quota fetched, but reset credits failed: {e}"));
            }
        }
    }
    Ok(FetchedData { response, credits })
}

/// State half of a quota fetch. Mirrors _bg_fetch_single: apply usage ->
/// backup current auth -> store reset credits. Lock held only here.
fn apply_fetched(
    st: &mut MonitorState,
    data: FetchedData,
    jwt: &str,
    expected_email: Option<&str>,
    activate: bool,
    now: f64,
) -> Result<String, String> {
    let email = st
        .apply_usage_response(&data.response, jwt.to_string(), now, activate)
        .ok_or_else(|| "Usage response had no usable account data.".to_string())?;
    if let Some(expected) = expected_email.filter(|e| !e.is_empty()) {
        if expected != email {
            return Err(format!("Account changed during fetch ({expected} -> {email})."));
        }
    }
    if activate {
        let _ = st.auth.backup_current_auth(&email);
    }
    if let Some(credits) = data.credits {
        st.apply_reset_credits(&email, credits);
    }
    Ok(email)
}

fn finalize_logout(st: &mut MonitorState, message: &str) -> AuthOutcome {
    st.clear_session();
    st.last_access_token = None;
    st.last_refresh_marker = None;
    st.last_signature = None;
    AuthOutcome::LoggedOut { message: message.into() }
}

#[tauri::command]
pub fn get_snapshot(app: AppHandle, state: State<AppState>) -> Snapshot {
    let st = state_lock(&state);
    Snapshot {
        accounts: st.usage.clone(),
        current_email: st.current_email.clone(),
        auto_fetch: st.auto_fetch.clone(),
        auto_fetch_options: AUTO_FETCH_OPTIONS.iter().map(|s| s.to_string()).collect(),
        sort_column: st.sort_column.clone(),
        sort_asc: st.sort_asc,
        show_archived: st.show_archived,
        logs_expanded: st.logs_expanded,
        auth_file_exists: st.auth.auth_file_exists(),
        backup_emails: st.auth.list_backup_emails(),
        app_version: app.package_info().version.to_string(),
    }
}

#[tauri::command]
pub async fn manual_fetch(state: State<'_, AppState>) -> Result<FetchResult, String> {
    let api = UsageApiClient::new();
    let now = now_secs();
    let (jwt, current, fallback_account) = {
        let st = state_lock(&state);
        let jwt = st.latest_jwt_for(None).ok_or_else(|| "No signed-in account.".to_string())?;
        let fallback = st
            .auth
            .load_snapshot()
            .ok()
            .and_then(|s| s.tokens)
            .and_then(|t| t.account_id);
        (jwt, st.current_email.clone(), fallback)
    };
    let data = fetch_remote(&api, &jwt, fallback_account.as_deref()).await?;
    let mut st = state_lock(&state);
    let email = apply_fetched(&mut st, data, &jwt, current.as_deref(), true, now)?;
    drop(st);
    let message = format!("Fetched quota for {email}.");
    append_log(&state, &message);
    Ok(FetchResult { email, message })
}

#[tauri::command]
pub async fn fetch_backup(state: State<'_, AppState>, email: String) -> Result<FetchResult, String> {
    let api = UsageApiClient::new();
    let refresher = AuthRefreshClient::new();
    let now = now_secs();
    let (jwt, fallback_account) = {
        let st = state_lock(&state);
        if !st.auth.backup_exists(&email) {
            return Err(format!(
                "NO_BACKUP {email}: No saved sign-in for {email} (backup file missing). The quota row is kept, but it cannot be fetched without signing in again."
            ));
        }
        let snap = st.auth.load_backup_snapshot(&email).map_err(|e| {
            format!("Saved sign-in for {email} is unreadable ({e}). Remove the account if it is gone.")
        })?;
        let tokens = snap.tokens.clone().ok_or_else(|| {
            format!("No saved tokens for {email}. Remove the account if it is gone.")
        })?;
        let jwt = tokens.access_token.clone().unwrap_or_default();
        if jwt.is_empty() {
            return Err(format!("No saved access token for {email}. Remove the account if it is gone."));
        }
        (jwt, tokens.account_id.clone())
    };
    match fetch_remote(&api, &jwt, fallback_account.as_deref()).await {
        Ok(data) => {
            let mut st = state_lock(&state);
            let email = apply_fetched(&mut st, data, &jwt, Some(&email), false, now)?;
            drop(st);
            let message = format!("Fetched quota for {email}.");
            append_log(&state, &message);
            Ok(FetchResult { email, message })
        }
        Err(e) if is_unauthorized(&e) => {
            // Refresh via a path-cloned service so the state lock is not
            // held across .await (std MutexGuard is not Send).
            let auth_svc = {
                let st = state_lock(&state);
                AuthFileService::new(
                    st.auth.auth_file_path.clone(),
                    st.auth.accounts_dir.clone(),
                )
            };
            let refreshed = auth_svc.refresh_backup_if_due(&email, &refresher, true, now).await?;
            let jwt = refreshed.tokens.and_then(|t| t.access_token).unwrap_or_default();
            if jwt.is_empty() {
                return Err("Token refresh produced no access token.".into());
            }
            let data = fetch_remote(&api, &jwt, None).await?;
            let mut st = state_lock(&state);
            let email = apply_fetched(&mut st, data, &jwt, Some(&email), false, now_secs())?;
            st.remember_jwt(&email, jwt);
            drop(st);
            let message = format!("Fetched quota for {email} (token refreshed).");
            append_log(&state, &message);
            Ok(FetchResult { email, message })
        }
        Err(e) => Err(e),
    }
}

fn is_unauthorized(message: &str) -> bool {
    message.contains("401") || message.contains("expired")
}

/// Read the active auth file and reconcile. Mirrors process_auth_file.
/// The frontend calls this on watcher events + a 5s poll (replacing the
/// tkinter after() loop); retry backoff for MissingToken lives there too.
#[tauri::command]
pub async fn process_auth_file(state: State<'_, AppState>) -> Result<AuthOutcome, String> {
    let api = UsageApiClient::new();
    let now = now_secs();
    enum Need {
        Idle,
        Refresh,
        Changed,
    }
    // Phase 1: gather file state under a single short lock, then drop the
    // guard before any network I/O. Never nest state_lock (std Mutex is
    // non-reentrant) and never hold the guard across fetch_remote.
    let ready: Option<(String, String, Option<String>, Option<String>)> = {
        let st = state_lock(&state);
        if !st.auth.auth_file_exists() {
            drop(st);
            let msg = "Auth file removed. Signed out.".to_string();
            append_log(&state, &msg);
            let mut st = state_lock(&state);
            return Ok(finalize_logout(&mut st, &msg));
        }
        let snapshot = match st.auth.load_snapshot() {
            Ok(s) => s,
            Err(e) => {
                return Ok(AuthOutcome::ParseError { message: format!("Could not read auth file: {e}") })
            }
        };
        let token = snapshot.tokens.clone().and_then(|t| t.access_token).unwrap_or_default();
        if token.is_empty() {
            return Ok(AuthOutcome::MissingToken);
        }
        let marker = snapshot.last_refresh.clone().unwrap_or_default();
        let last_refresh = snapshot.last_refresh.clone();
        let fallback_account = snapshot.tokens.and_then(|t| t.account_id);
        Some((token, marker, fallback_account, last_refresh))
    };
    let (token, marker, fallback_account, last_refresh) = match ready {
        Some(v) => v,
        None => return Ok(AuthOutcome::NoChange),
    };
    // Phase 2: compute need + update markers under a second short lock.
    let need = {
        let mut st = state_lock(&state);
        let refresh_changed = !marker.is_empty() && Some(marker.clone()) != st.last_refresh_marker;
        let token_changed = Some(token.clone()) != st.last_access_token;
        let auth_path = st.auth.auth_file_path.clone();
        st.last_signature = file_signature(&auth_path);
        st.last_refresh_marker = last_refresh;
        st.last_access_token = Some(token.clone());
        st.latest_jwt = Some(token.clone());
        if refresh_changed {
            Need::Refresh
        } else if token_changed {
            Need::Changed
        } else {
            Need::Idle
        }
    };
    // NOTE: original logic required a stored marker before treating a lone
    // token change as a refresh; markers are now always stored above, so any
    // change fetches. Same observable behaviour without holding the lock.

    let outcome = match need {
        Need::Idle => AuthOutcome::NoChange,
        Need::Refresh => match fetch_remote(&api, &token, fallback_account.as_deref()).await {
            Ok(data) => {
                let mut st = state_lock(&state);
                match apply_fetched(&mut st, data, &token, None, true, now) {
                    Ok(email) => {
                        drop(st);
                        let message = format!("Detected Codex auth refresh; fetched {email}.");
                        append_log(&state, &message);
                        AuthOutcome::AuthRefreshed { message }
                    }
                    Err(e) => AuthOutcome::ParseError { message: e },
                }
            }
            Err(e) => AuthOutcome::ParseError { message: e },
        },
        Need::Changed => match fetch_remote(&api, &token, fallback_account.as_deref()).await {
            Ok(data) => {
                let mut st = state_lock(&state);
                match apply_fetched(&mut st, data, &token, None, true, now) {
                    Ok(email) => {
                        drop(st);
                        let message = format!("Fetched quota for {email}.");
                        append_log(&state, &message);
                        AuthOutcome::Fetched { email, message }
                    }
                    Err(e) => AuthOutcome::ParseError { message: e },
                }
            }
            Err(e) => {
                if is_unauthorized(&e) {
                    let mut st = state_lock(&state);
                    let msg = "Session expired. Signed out.".to_string();
                    append_log(&state, &msg);
                    finalize_logout(&mut st, &msg)
                } else {
                    AuthOutcome::ParseError { message: e }
                }
            }
        },
    };
    Ok(outcome)
}

#[tauri::command]
pub async fn switch_account(state: State<'_, AppState>, email: String) -> Result<String, String> {
    let current = state_lock(&state).current_email.clone();
    {
        let st = state_lock(&state);
        st.auth.switch_to_account_backup(&email, current.as_deref())?;
    }
    // Fetch fresh quota for the newly activated account.
    let (jwt, fallback_account) = {
        let st = state_lock(&state);
        let jwt = st
            .auth
            .load_access_token()
            .ok_or_else(|| "Activated account has no access token.".to_string())?;
        let fallback = st
            .auth
            .load_snapshot()
            .ok()
            .and_then(|s| s.tokens)
            .and_then(|t| t.account_id);
        (jwt, fallback)
    };
    let api = UsageApiClient::new();
    let data = fetch_remote(&api, &jwt, fallback_account.as_deref()).await?;
    let mut st = state_lock(&state);
    let fetched = apply_fetched(&mut st, data, &jwt, Some(&email), true, now_secs())?;
    drop(st);
    let message = format!("Switched to {fetched}.");
    append_log(&state, &message);
    Ok(message)
}

#[tauri::command]
pub fn remove_account(state: State<AppState>, email: String) -> Result<String, String> {
    let mut st = state_lock(&state);
    if !st.remove_account(&email) {
        return Err(format!("Unknown account {email}."));
    }
    st.auth.remove_backup(&email);
    let message = format!("Removed {email}.");
    append_log(&state, &message);
    Ok(message)
}

#[tauri::command]
pub fn set_archived(state: State<AppState>, email: String, archived: bool) -> Result<String, String> {
    let mut st = state_lock(&state);
    if !st.set_archived(&email, archived) {
        return Err(format!("Unknown account {email}."));
    }
    let message =
        if archived { format!("Archived {email}.") } else { format!("Unarchived {email}.") };
    append_log(&state, &message);
    Ok(message)
}

#[tauri::command]
pub fn save_auto_fetch(state: State<AppState>, value: String) -> Result<String, String> {
    let mut st = state_lock(&state);
    if !st.save_auto_fetch(&value) {
        return Err("No active account to configure.".into());
    }
    let message = format!("Auto-fetch set to {}.", st.auto_fetch);
    append_log(&state, &message);
    Ok(message)
}

#[tauri::command]
pub fn save_sort(state: State<AppState>, column: Option<String>, asc: bool) -> Result<(), String> {
    state_lock(&state).save_sort(column, asc);
    Ok(())
}

#[tauri::command]
pub fn save_show_archived(state: State<AppState>, show: bool) -> Result<(), String> {
    state_lock(&state).save_show_archived(show);
    Ok(())
}

#[tauri::command]
pub fn save_logs_expanded(state: State<AppState>, expanded: bool) -> Result<(), String> {
    state_lock(&state).save_logs_expanded(expanded);
    Ok(())
}

#[tauri::command]
pub fn get_resets(state: State<AppState>, email: String) -> Option<ResetCreditsPayload> {
    state_lock(&state).usage.get(&email).and_then(|a| a.resets.clone())
}

#[tauri::command]
pub fn get_logs(state: State<AppState>) -> Vec<String> {
    fs::read_to_string(&state.log_path)
        .unwrap_or_default()
        .lines()
        .rev()
        .take(500)
        .map(String::from)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

#[tauri::command]
pub fn clear_logs(state: State<AppState>) -> Result<(), String> {
    fs::write(&state.log_path, "").map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn export_data(state: State<AppState>, path: String) -> Result<String, String> {
    let st = state_lock(&state);
    let payload = st.storage.export_data(&st.usage);
    let backups: HashMapEmailSnapshots = st.auth.export_backups(&[]);
    let combined = serde_json::json!({
        "schema_version": 2,
        "accounts": payload["accounts"],
        "config": payload["config"],
        "backups": backups,
    });
    fs::write(&path, serde_json::to_string_pretty(&combined).unwrap()).map_err(|e| e.to_string())?;
    Ok(format!("Exported to {path}."))
}

#[tauri::command]
pub fn import_data(state: State<AppState>, path: String) -> Result<String, String> {
    let text = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let payload: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("Import file is not valid JSON: {e}"))?;
    let mut st = state_lock(&state);
    let merged = st.storage.import_data(&payload)?;
    if let Some(backups) = payload.get("backups").and_then(|v| v.as_object()) {
        let typed: HashMapEmailSnapshots = backups
            .iter()
            .filter_map(|(k, v)| serde_json::from_value(v.clone()).ok().map(|s| (k.clone(), s)))
            .collect();
        st.auth.import_backups(&typed);
    }
    // Re-read merged state (import_data already saved through storage).
    st.usage = merged;
    st.current_email = st
        .storage
        .get_meta_value("current_account_email")
        .and_then(|v| v.as_str().map(String::from))
        .filter(|e| st.usage.contains_key(e));
    st.session_tokens.clear();
    st.latest_jwt = None;
    let message = format!("Imported {} account(s).", st.usage.len());
    append_log(&state, &message);
    Ok(message)
}

#[tauri::command]
pub fn logout(state: State<AppState>) -> Result<String, String> {
    let mut st = state_lock(&state);
    st.clear_session();
    let message = "Signed out.".to_string();
    append_log(&state, &message);
    Ok(message)
}

#[tauri::command]
pub async fn check_auto_fetch(state: State<'_, AppState>) -> Result<Option<FetchResult>, String> {
    let api = UsageApiClient::new();
    let now = now_secs();
    let (jwt, current, fallback_account) = {
        let mut st = state_lock(&state);
        let Some(jwt) = st.due_auto_fetch_jwt(now) else { return Ok(None) };
        let fallback = st
            .auth
            .load_snapshot()
            .ok()
            .and_then(|s| s.tokens)
            .and_then(|t| t.account_id);
        (jwt, st.current_email.clone(), fallback)
    };
    match fetch_remote(&api, &jwt, fallback_account.as_deref()).await {
        Ok(data) => {
            let mut st = state_lock(&state);
            match apply_fetched(&mut st, data, &jwt, current.as_deref(), true, now) {
                Ok(email) => {
                    drop(st);
                    let message = format!("Auto-fetched quota for {email}.");
                    append_log(&state, &message);
                    Ok(Some(FetchResult { email, message }))
                }
                Err(e) => {
                    append_log(&state, &format!("Auto-fetch failed: {e}"));
                    Err(e)
                }
            }
        }
        Err(e) => {
            append_log(&state, &format!("Auto-fetch failed: {e}"));
            Err(e)
        }
    }
}

#[tauri::command]
pub fn login_start(app: AppHandle, state: State<AppState>) -> Result<String, String> {
    if state.login.lock().unwrap().is_some() {
        return Err("A login is already in progress.".into());
    }
    let home = {
        let st = state_lock(&state);
        st.auth.create_login_codex_home()?
    };
    // Native streamlined OAuth: no CLI, no scraping. The sign-in URL below
    // matches the official Codex Desktop format; the loopback server catches
    // the callback and the tokens are exchanged in-process.
    let pkce = crate::oauth_login::generate_pkce();
    let oauth_state = crate::oauth_login::generate_state();
    let (port, outcome) =
        match crate::oauth_login::start_callback_server(crate::oauth_login::CALLBACK_PORT, oauth_state.clone()) {
            Ok(v) => v,
            Err(_) => crate::oauth_login::start_callback_server(
                crate::oauth_login::FALLBACK_PORT,
                oauth_state.clone(),
            )?,
        };
    let inner = crate::oauth_login::authorize_url(
        crate::api::AUTH_REFRESH_CLIENT_ID,
        port,
        &pkce,
        &oauth_state,
    );
    let url = crate::oauth_login::desktop_auth_url(&inner);
    let cancel = Arc::new(AtomicBool::new(false));
    *state.login.lock().unwrap() = Some(LoginSession {
        home: home.clone(),
        cancel: cancel.clone(),
        outcome,
        verifier: pkce.verifier,
        port,
    });

    let _ = app.emit(
        "codex-login-output",
        "Sign-in started. Complete it in the browser, then return here.".to_string(),
    );
    let _ = app.emit("codex-login-url", url);
    let app_out = app.clone();
    let state_c = app.clone();
    thread::spawn(move || {
        let state_c: State<AppState> = state_c.state();
        loop {
            if cancel.load(Ordering::SeqCst) {
                if let Some(sess) = state_c.login.lock().unwrap().take() {
                    sess.outcome.shutdown.store(true, Ordering::SeqCst);
                    let st = state_lock(&state_c);
                    st.auth.remove_login_codex_home(Some(&sess.home));
                }
                append_log(&state_c, "Login cancelled.");
                let _ = app_out.emit(
                    "codex-login-done",
                    serde_json::json!({"ok": false, "message": "Login cancelled."}),
                );
                return;
            }
            let arrival = state_c.login.lock().unwrap().as_ref().and_then(|s| {
                s.outcome
                    .code
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take()
            });
            match arrival {
                None => thread::sleep(std::time::Duration::from_millis(200)),
                Some(Err(message)) => {
                    if let Some(sess) = state_c.login.lock().unwrap().take() {
                        let st = state_lock(&state_c);
                        st.auth.remove_login_codex_home(Some(&sess.home));
                    }
                    append_log(&state_c, &message);
                    let _ = app_out.emit(
                        "codex-login-done",
                        serde_json::json!({"ok": false, "message": message}),
                    );
                    return;
                }
                Some(Ok(code)) => {
                    finish_native_login(&app_out, &state_c, code);
                    return;
                }
            }
        }
    });
    append_log(&state, "Started Codex login.");
    Ok("Login started. Complete the flow in the dialog.".into())
}

fn finish_native_login(app: &AppHandle, state: &State<AppState>, code: String) {
    let api = UsageApiClient::new();
    let fail = |message: String| {
        if let Some(sess) = state.login.lock().unwrap().take() {
            let st = state_lock(state);
            st.auth.remove_login_codex_home(Some(&sess.home));
        }
        append_log(state, &message);
        let _ = app.emit("codex-login-done", serde_json::json!({"ok": false, "message": message}));
    };
    let (home, verifier, port) = match state.login.lock().unwrap().as_ref() {
        Some(s) => (s.home.clone(), s.verifier.clone(), s.port),
        None => {
            fail("Login session ended before sign-in completed.".into());
            return;
        }
    };
    let redirect = crate::oauth_login::redirect_uri(port);
    let http = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            fail(format!("Login failed: {e}"));
            return;
        }
    };
    let tokens = match tauri::async_runtime::block_on(crate::oauth_login::exchange_code(
        &http,
        crate::oauth_login::OAUTH_ISSUER,
        crate::api::AUTH_REFRESH_CLIENT_ID,
        &code,
        &redirect,
        &verifier,
    )) {
        Ok(t) => t,
        Err(e) => {
            fail(e);
            return;
        }
    };
    let jwt = tokens.access_token.clone();
    let account_id = crate::oauth_login::account_id_from_id_token(&tokens.id_token);
    let snapshot = AuthFileSnapshot {
        auth_mode: Some("chatgpt".into()),
        openai_api_key: None,
        tokens: Some(AuthTokens {
            id_token: Some(tokens.id_token),
            access_token: Some(tokens.access_token),
            refresh_token: Some(tokens.refresh_token),
            account_id,
        }),
        last_refresh: Some(chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()),
        email: None,
    };
    {
        let st = state_lock(state);
        let path = st.auth.active_auth_path_for_home(&home);
        let text = match serde_json::to_string_pretty(&snapshot) {
            Ok(t) => t,
            Err(e) => {
                drop(st);
                fail(format!("Login failed: {e}"));
                return;
            }
        };
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Err(e) = fs::write(&path, text) {
            drop(st);
            fail(format!("Login failed: {e}"));
            return;
        }
    }
    let response: UsageResponse = match tauri::async_runtime::block_on(api.fetch_usage(&jwt)) {
        Ok(r) => r,
        Err(e) => {
            fail(format!("Login succeeded but quota fetch failed: {e}"));
            return;
        }
    };
    let message = {
        let mut st = state_lock(state);
        let email = match st.apply_usage_response(&response, jwt.clone(), now_secs(), false) {
            Some(e) => e,
            None => {
                drop(st);
                fail("Login produced no usable account.".into());
                return;
            }
        };
        if let Some(current) = st.current_email.clone() {
            if current != email {
                let _ = st.auth.backup_current_auth(&current);
            }
        }
        let source = st.auth.active_auth_path_for_home(&home);
        if let Err(e) = st.auth.activate_auth_from_path(&source) {
            drop(st);
            fail(format!("Could not activate the new account: {e}"));
            return;
        }
        st.set_current(email.clone(), jwt);
        if let Some(account_id) = response.account_id.clone() {
            let jwt = st.latest_jwt_for(Some(&email)).unwrap_or_default();
            if let Ok(credits) =
                tauri::async_runtime::block_on(api.fetch_reset_credits(&jwt, &account_id))
            {
                st.apply_reset_credits(&email, credits);
            }
        }
        format!("Signed in as {email}.")
    };
    if state.login.lock().unwrap().take().is_some() {
        let st = state_lock(state);
        st.auth.remove_login_codex_home(Some(&home));
    }
    append_log(state, &message);
    let _ = app.emit("codex-login-done", serde_json::json!({"ok": true, "message": message}));
}

#[tauri::command]
pub fn login_cancel(app: AppHandle, state: State<AppState>) -> Result<String, String> {
    let guard = state.login.lock().unwrap();
    if guard.is_none() {
        return Err("No login in progress.".into());
    }
    guard.as_ref().unwrap().cancel.store(true, Ordering::SeqCst);
    drop(guard);
    append_log(&state, "Login cancel requested.");
    let _ = app.emit("codex-login-output", "Cancelling…".to_string());
    Ok("Cancelling login…".into())
}

fn is_http_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// Open the Codex login URL in a private window when possible so users with
/// several ChatGPT accounts can pick which one to sign in with. Best-effort:
/// falls through the known macOS browsers and reports failure so the
/// frontend can open the URL in the default browser instead.
#[tauri::command]
pub fn open_login_url(url: String) -> Result<String, String> {
    let url = url.trim().to_string();
    if !is_http_url(&url) {
        return Err("Login URL is not a valid http(s) address.".into());
    }
    #[cfg(target_os = "macos")]
    {
        const CANDIDATES: &[(&str, &[&str])] = &[
            ("Google Chrome", &["--incognito"]),
            ("Brave Browser", &["--incognito"]),
            ("Microsoft Edge", &["--inprivate"]),
            ("Arc", &["--incognito"]),
            ("Firefox", &["-private-window"]),
        ];
        for (app, flags) in CANDIDATES {
            let mut cmd = Command::new("open");
            cmd.arg("-a").arg(app).arg("--args").args(*flags).arg(&url);
            if cmd.status().map(|s| s.success()).unwrap_or(false) {
                return Ok(format!("Opened login page in {app} private window."));
            }
        }
        Err("No private-window browser found.".into())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = url;
        Err("Private-window open is only supported on macOS.".into())
    }
}

#[tauri::command]
pub fn restart_codex() -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        // Ask Codex to quit gracefully. Failure here is non-fatal: the app
        // may already be closed, in which case we just launch it below.
        let quit = Command::new("osascript")
            .args(["-e", "tell application \"Codex\" to quit"])
            .output();
        if let Err(e) = &quit {
            return Err(format!("Could not ask Codex to quit: {e}"));
        }
        // Wait until the old process is actually gone before relaunching.
        // `open -a` while the previous instance is still terminating fails
        // with LSOpenURLsWithCompletionHandler error -600 (procNotFound):
        // the app ends up closed and never reopened.
        let mut gone = false;
        for _ in 0..40 {
            let alive = Command::new("pgrep")
                .args(["-x", "Codex"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(true);
            if !alive {
                gone = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        if !gone {
            return Err(
                "Codex is still closing (close it manually, then reopen it).".into(),
            );
        }
        // Grace period so LaunchServices releases the old instance.
        std::thread::sleep(std::time::Duration::from_millis(500));
        for attempt in 0..2 {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(1500));
            }
            match Command::new("open").args(["-a", "Codex"]).output() {
                Ok(o) if o.status.success() => return Ok("Restarted the Codex app.".into()),
                Ok(o) if attempt == 1 => {
                    let detail = String::from_utf8_lossy(&o.stderr).trim().to_string();
                    if detail.is_empty() {
                        return Err("Could not reopen Codex.".into());
                    }
                    return Err(format!("Could not reopen Codex: {detail}"));
                }
                _ => {}
            }
        }
        Err("Could not reopen Codex.".into())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("Restarting the Codex app is only supported on macOS.".into())
    }
}

/// Migrate legacy store files once, then seed state. Called from setup.
pub fn build_monitor_state() -> (MonitorState, PathBuf) {    let storage = UsageStorage::with_defaults();
    let auth = AuthFileService::with_defaults();
    let log_path = storage
        .storage_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("activity.log");
    boot_line(&log_path, "CodexMonitor started.");
    let mut storage = storage;
    #[cfg(feature = "legacy-migrate")]
    {
        crate::legacy::migrate_legacy_account_dirs(&auth.accounts_dir);
        match crate::legacy::migrate_if_needed(&mut storage) {
            Ok(Some(report)) => boot_line(
                &log_path,
                &format!(
                    "Migrated {} account(s) from previous install ({}).",
                    report.accounts_migrated,
                    report.source.display()
                ),
            ),
            Ok(None) => {}
            Err(e) => boot_line(&log_path, &format!("Migration skipped: {e}")),
        }
    }
    // Legacy activity log moves next to the v2 store (mirrors _migrate_legacy_log_file).
    if !log_path.exists() {
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            let legacy = home.join(".codex_usage_store.log");
            if legacy.is_file() {
                let _ = fs::copy(&legacy, &log_path);
            }
        }
    }
    let state = MonitorState::load(storage, auth);
    (state, log_path)
}

pub fn all_commands() -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool {
    tauri::generate_handler![
        get_snapshot,
        manual_fetch,
        fetch_backup,
        check_auto_fetch,
        process_auth_file,
        switch_account,
        remove_account,
        set_archived,
        save_auto_fetch,
        save_sort,
        save_show_archived,
        save_logs_expanded,
        get_resets,
        get_logs,
        clear_logs,
        export_data,
        import_data,
        logout,
        login_start,
        login_cancel,
        open_login_url,
        restart_codex,
    ]
}

#[cfg(test)]
mod real_store_tests {
    use super::*;

    #[test]
    fn snapshot_serializes_against_real_default_store() {
        let storage = UsageStorage::with_defaults();
        let auth = AuthFileService::with_defaults();
        let st = MonitorState::load(storage, auth);
        let snap = Snapshot {
            accounts: st.usage.clone(),
            current_email: st.current_email.clone(),
            auto_fetch: st.auto_fetch.clone(),
            auto_fetch_options: vec![],
            sort_column: st.sort_column.clone(),
            sort_asc: st.sort_asc,
            show_archived: st.show_archived,
            logs_expanded: st.logs_expanded,
            auth_file_exists: st.auth.auth_file_exists(),
            backup_emails: st.auth.list_backup_emails(),
            app_version: "test".into(),
        };
        let text = serde_json::to_string(&snap).expect("snapshot must serialize");
        assert!(text.contains("accounts"));
    }

    #[test]
    fn login_url_validation() {
        assert!(is_http_url("https://auth.openai.com/authorize?x=1"));
        assert!(is_http_url("http://localhost:1455/callback"));
        assert!(!is_http_url("javascript:alert(1)"));
        assert!(!is_http_url("file:///etc/passwd"));
        assert!(!is_http_url(""));
        assert!(open_login_url("javascript:alert(1)".into()).is_err());
    }
}
