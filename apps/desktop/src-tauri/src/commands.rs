use std::fs;
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::api::{ApiError, AuthRefreshClient, UsageApiClient};
use crate::auth::{jwt_email, snapshot_email, AuthFileService, HashMapEmailSnapshots};
use crate::models::{AuthFileSnapshot, AuthTokens, ResetCreditsPayload, UsageMap, UsageResponse};
use crate::state::{now_secs, usage_response_email, MonitorState, AUTO_FETCH_OPTIONS};
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
    pub show_menu_bar: bool,
    pub notifications_enabled: bool,
    pub auto_check_updates: bool,
    pub locale: String,
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedAccount {
    pub email: String,
    pub message: String,
    pub replaced: bool,
}

#[derive(Serialize)]
pub struct SwitchResult {
    pub email: String,
    pub already_active: bool,
    pub codex_was_running: bool,
    pub codex_reopened: bool,
    pub warning: Option<String>,
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
    if let Ok(mut f) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
    {
        let _ = f.write_all(line.as_bytes());
    }
}

/// Steps of a close -> write -> reopen sequence, surfaced to the switch
/// dialog so it can show live progress instead of one opaque spinner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LifecycleStage {
    Closing,
    Switching,
    Reopening,
    Reopened,
}

impl LifecycleStage {
    fn as_str(self) -> &'static str {
        match self {
            LifecycleStage::Closing => "closing",
            LifecycleStage::Switching => "switching",
            LifecycleStage::Reopening => "reopening",
            LifecycleStage::Reopened => "reopened",
        }
    }
}

fn stage_from_log(line: &str) -> Option<LifecycleStage> {
    if line.contains("closing Codex") || line.contains("requesting native Codex termination") {
        Some(LifecycleStage::Closing)
    } else if line.contains("writing the selected account") {
        Some(LifecycleStage::Switching)
    } else if line.contains("launching resolved Codex bundle") {
        Some(LifecycleStage::Reopening)
    } else if line.contains("launch verified") {
        Some(LifecycleStage::Reopened)
    } else {
        None
    }
}

fn emit_stage(app: &AppHandle, line: &str) {
    if let Some(stage) = stage_from_log(line) {
        let _ = app.emit("switch-stage", stage.as_str());
    }
}

fn append_log(state: &AppState, message: &str) {
    if message.is_empty() {
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
    if let Ok(mut f) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&state.log_path)
    {
        let _ = f.write_all(line.as_bytes());
    }
}

fn state_lock<'a>(state: &'a State<AppState>) -> std::sync::MutexGuard<'a, MonitorState> {
    state.inner.lock().unwrap()
}

/// Current menu-bar preference. A poisoned lock is treated as "off" so a
/// window close can never hide the app behind a missing tray item.
pub fn menu_bar_enabled(app: &AppHandle) -> bool {
    app.state::<AppState>()
        .inner
        .lock()
        .map(|st| st.show_menu_bar)
        .unwrap_or(false)
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
    let account_id = response
        .account_id
        .clone()
        .or_else(|| fallback_account_id.map(String::from));
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

/// State half of a quota fetch. Mirrors _bg_fetch_single: validate identity ->
/// apply usage -> preserve the live session -> store reset credits. The lock
/// is held only here.
fn apply_fetched(
    st: &mut MonitorState,
    data: FetchedData,
    jwt: &str,
    expected_email: Option<&str>,
    activate: bool,
    now: f64,
) -> Result<String, String> {
    // Resolve and validate the response identity before touching usage or the
    // current account. A mismatched fetch must leave state exactly as it was.
    let email = usage_response_email(&data.response)
        .ok_or_else(|| "Usage response had no usable account data.".to_string())?;
    if let Some(expected) = expected_email.filter(|e| !e.is_empty()) {
        if !expected.eq_ignore_ascii_case(&email) {
            return Err(format!(
                "Account changed during fetch ({expected} -> {email})."
            ));
        }
    }
    if let Some(token_email) = jwt_email(jwt) {
        if !token_email.eq_ignore_ascii_case(&email) {
            return Err(format!(
                "The fetched quota belongs to {email}, but the token belongs to {token_email}. Nothing was saved."
            ));
        }
    }
    let email = st
        .apply_usage_response(&data.response, jwt.to_string(), now, activate)
        .ok_or_else(|| "Usage response had no usable account data.".to_string())?;
    if activate {
        // Preserve the live session under the identity inside the file, never
        // under the response email (they can disagree while a switch settles).
        let _ = st.auth.backup_live_auth();
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
    AuthOutcome::LoggedOut {
        message: message.into(),
    }
}

fn add_warning(slot: &mut Option<String>, message: String) {
    match slot {
        Some(existing) => {
            existing.push(' ');
            existing.push_str(&message);
        }
        None => *slot = Some(message),
    }
}

#[tauri::command]
pub fn get_snapshot(app: AppHandle, state: State<AppState>) -> Snapshot {
    let snapshot = {
        let st = state_lock(&state);
        let mut accounts = st.usage.clone();
        // Saved sign-ins without usage (fresh imports, offline fetches) still
        // belong in the list so they can be fetched or switched to.
        for email in st.auth.list_backup_emails() {
            if accounts.keys().any(|key| key.eq_ignore_ascii_case(&email)) {
                continue;
            }
            accounts.insert(email, Default::default());
        }
        Snapshot {
            accounts,
            current_email: st.current_email.clone(),
            auto_fetch: st.auto_fetch.clone(),
            auto_fetch_options: AUTO_FETCH_OPTIONS.iter().map(|s| s.to_string()).collect(),
            sort_column: st.sort_column.clone(),
            sort_asc: st.sort_asc,
            show_archived: st.show_archived,
            logs_expanded: st.logs_expanded,
            show_menu_bar: st.show_menu_bar,
            notifications_enabled: st.notifications_enabled,
            auto_check_updates: st.auto_check_updates,
            locale: st.locale.clone(),
            auth_file_exists: st.auth.auth_file_exists(),
            backup_emails: st.auth.list_backup_emails(),
            app_version: app.package_info().version.to_string(),
        }
    };
    crate::tray::update_title(&app, &snapshot);
    snapshot
}

#[tauri::command]
pub async fn manual_fetch(state: State<'_, AppState>) -> Result<FetchResult, String> {
    // Fetching can refresh the saved session, so it must not overlap a switch
    // writing the same file from the other window.
    let _guard = crate::auth::AUTH_OPERATION_LOCK.lock().await;
    let api = UsageApiClient::new();
    let now = now_secs();
    let (jwt, expected, fallback_account) = {
        let st = state_lock(&state);
        let live = st.auth.load_snapshot().ok();
        let live_token = live
            .as_ref()
            .and_then(|snapshot| snapshot.tokens.as_ref())
            .and_then(|tokens| tokens.access_token.clone())
            .filter(|token| !token.is_empty());
        let jwt = match live_token {
            Some(token) => token,
            None if live.is_some() => {
                return Err("The active Codex session has no access token. Sign in again.".into())
            }
            None => st
                .latest_jwt_for(None)
                .ok_or_else(|| "No signed-in account.".to_string())?,
        };
        let expected = live
            .as_ref()
            .and_then(snapshot_email)
            .or_else(|| st.current_email.clone())
            .or_else(|| jwt_email(&jwt));
        let fallback = live
            .as_ref()
            .and_then(|snapshot| snapshot.tokens.as_ref())
            .and_then(|tokens| tokens.account_id.clone());
        (jwt, expected, fallback)
    };
    let data = fetch_remote(&api, &jwt, fallback_account.as_deref()).await?;
    let mut st = state_lock(&state);
    let email = apply_fetched(&mut st, data, &jwt, expected.as_deref(), true, now)?;
    drop(st);
    let message = format!("Fetched quota for {email}.");
    append_log(&state, &message);
    Ok(FetchResult { email, message })
}

#[tauri::command]
pub async fn fetch_backup(
    state: State<'_, AppState>,
    email: String,
) -> Result<FetchResult, String> {
    let _guard = crate::auth::AUTH_OPERATION_LOCK.lock().await;
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
        let snap = st.auth.load_verified_backup(&email).map_err(|e| {
            format!(
                "Saved sign-in for {email} is unusable ({e}). Remove the account or sign in again."
            )
        })?;
        let tokens = snap.tokens.clone().ok_or_else(|| {
            format!("No saved tokens for {email}. Remove the account if it is gone.")
        })?;
        let jwt = tokens.access_token.clone().unwrap_or_default();
        if jwt.is_empty() {
            return Err(format!(
                "No saved access token for {email}. Remove the account if it is gone."
            ));
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
                AuthFileService::new(st.auth.auth_file_path.clone(), st.auth.accounts_dir.clone())
            };
            let refreshed = auth_svc
                .refresh_backup_if_due(&email, &refresher, true, now)
                .await?;
            if let Some(actual) = snapshot_email(&refreshed) {
                if !actual.eq_ignore_ascii_case(&email) {
                    return Err(format!(
                        "The refreshed session belongs to {actual}, not {email}. Nothing was saved."
                    ));
                }
            }
            let jwt = refreshed
                .tokens
                .and_then(|t| t.access_token)
                .unwrap_or_default();
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

/// Add an account from an existing Codex `auth.json` without activating it.
/// The file is filed under the identity it carries; the live session and the
/// Codex process are left untouched.
#[tauri::command]
pub async fn import_auth_json(
    state: State<'_, AppState>,
    path: String,
) -> Result<ImportedAccount, String> {
    let source = PathBuf::from(path.trim());
    if path.trim().is_empty() {
        return Err("Choose an auth.json file to import.".into());
    }
    if !source.is_file() {
        return Err(format!("No file at {}.", source.display()));
    }
    let _guard = crate::auth::AUTH_OPERATION_LOCK.lock().await;
    let snapshot = {
        let st = state_lock(&state);
        st.auth
            .load_snapshot_from_path(&source)
            .map_err(|e| format!("That file is not a readable Codex auth.json ({e})."))?
    };
    store_imported_snapshot(&state, &snapshot)
}

/// Import auth.json contents pasted into the Add account dialog. The text is
/// never logged: it carries refresh tokens.
#[tauri::command]
pub async fn import_auth_json_text(
    state: State<'_, AppState>,
    text: String,
) -> Result<ImportedAccount, String> {
    if text.trim().is_empty() {
        return Err("Paste the contents of an auth.json file to import.".into());
    }
    let _guard = crate::auth::AUTH_OPERATION_LOCK.lock().await;
    let snapshot = AuthFileService::parse_snapshot(&text)
        .map_err(|e| format!("That pasted text is not a readable Codex auth.json ({e})."))?;
    store_imported_snapshot(&state, &snapshot)
}

fn store_imported_snapshot(
    state: &State<'_, AppState>,
    snapshot: &crate::models::AuthFileSnapshot,
) -> Result<ImportedAccount, String> {
    let (email, replaced) = {
        let st = state_lock(state);
        let email = crate::auth::login_email(snapshot).ok_or_else(|| {
            "That auth.json has no ChatGPT sign-in. Open Codex with it once, then import it again."
                .to_string()
        })?;
        let replaced = st.auth.backup_exists(&email);
        st.auth.import_backup_snapshot(snapshot)?;
        (email, replaced)
    };
    let message = if replaced {
        format!("Refreshed the saved sign-in for {email}.")
    } else {
        format!("Added {email}.")
    };
    append_log(state, &message);
    Ok(ImportedAccount {
        email,
        message,
        replaced,
    })
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
    struct ReadyAuth {
        token: String,
        marker: String,
        fallback_account: Option<String>,
        last_refresh: Option<String>,
        live_email: Option<String>,
    }
    // Phase 1: gather file state under a single short lock, then drop the
    // guard before any network I/O. Never nest state_lock (std Mutex is
    // non-reentrant) and never hold the guard across fetch_remote.
    let ready: Option<ReadyAuth> = {
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
                return Ok(AuthOutcome::ParseError {
                    message: format!("Could not read auth file: {e}"),
                })
            }
        };
        let token = snapshot
            .tokens
            .clone()
            .and_then(|t| t.access_token)
            .unwrap_or_default();
        if token.is_empty() {
            return Ok(AuthOutcome::MissingToken);
        }
        let marker = snapshot.last_refresh.clone().unwrap_or_default();
        let last_refresh = snapshot.last_refresh.clone();
        let live_email = snapshot_email(&snapshot);
        let fallback_account = snapshot.tokens.and_then(|t| t.account_id);
        Some(ReadyAuth {
            token,
            marker,
            fallback_account,
            last_refresh,
            live_email,
        })
    };
    let ReadyAuth {
        token,
        marker,
        fallback_account,
        last_refresh,
        live_email,
    } = match ready {
        Some(v) => v,
        None => return Ok(AuthOutcome::NoChange),
    };
    // Phase 2: compute need + update markers under a second short lock.
    let need = {
        let mut st = state_lock(&state);
        let refresh_changed = !marker.is_empty() && Some(marker.clone()) != st.last_refresh_marker;
        let token_changed = Some(token.clone()) != st.last_access_token;
        let identity_changed = live_email
            .as_deref()
            .map(|email| {
                st.current_email
                    .as_deref()
                    .map(|current| !current.eq_ignore_ascii_case(email))
                    .unwrap_or(true)
            })
            .unwrap_or(false);
        if identity_changed {
            if let Some(email) = live_email.clone() {
                st.set_current(email, token.clone());
            }
        }
        let auth_path = st.auth.auth_file_path.clone();
        st.last_signature = file_signature(&auth_path);
        st.last_refresh_marker = last_refresh;
        st.last_access_token = Some(token.clone());
        st.latest_jwt = Some(token.clone());
        if refresh_changed {
            Need::Refresh
        } else if token_changed || identity_changed {
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
                match apply_fetched(&mut st, data, &token, live_email.as_deref(), true, now) {
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
                match apply_fetched(&mut st, data, &token, live_email.as_deref(), true, now) {
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

/// One-step safe switch. When Codex is running this closes it, writes the
/// selected session, and reopens the same bundle. A quota fetch failure never
/// turns an applied switch into an error.
#[tauri::command]
pub async fn switch_account(
    app: AppHandle,
    state: State<'_, AppState>,
    email: String,
) -> Result<SwitchResult, String> {
    let _auth_guard = crate::auth::AUTH_OPERATION_LOCK.lock().await;
    let current = {
        let st = state_lock(&state);
        if !st.auth.backup_exists(&email) {
            return Err(format!(
                "No saved sign-in for {email}. Sign in to it again before switching."
            ));
        }
        st.auth
            .load_snapshot()
            .ok()
            .and_then(|snapshot| snapshot_email(&snapshot))
            .or_else(|| st.current_email.clone())
    };

    let already_active = {
        let st = state_lock(&state);
        st.auth
            .target_already_active(&email)
            .map_err(|e| format!("Could not read the live Codex session: {e}"))?
    };

    if already_active {
        let (jwt, fallback) = {
            let mut st = state_lock(&state);
            let jwt = st
                .auth
                .load_access_token()
                .ok_or_else(|| "The active session has no access token.".to_string())?;
            let fallback = st
                .auth
                .load_snapshot()
                .ok()
                .and_then(|s| s.tokens)
                .and_then(|t| t.account_id);
            st.set_current(email.clone(), jwt.clone());
            (jwt, fallback)
        };
        drop(_auth_guard);
        let mut warning = None;
        refresh_quota_after_switch(&state, &email, &jwt, fallback.as_deref(), &mut warning).await;
        let message = format!("Already using {email}.");
        append_log(&state, &message);
        return Ok(SwitchResult {
            email,
            already_active: true,
            codex_was_running: false,
            codex_reopened: false,
            warning,
            message,
        });
    }

    // Chat refresh tokens are single-use. Refresh a stale saved session
    // before it is copied over the live file; when that fails the credentials
    // cannot be trusted, so stop before Codex is touched instead of launching
    // it on a dead session.
    let auth_svc = {
        let st = state_lock(&state);
        AuthFileService::new(st.auth.auth_file_path.clone(), st.auth.accounts_dir.clone())
    };
    let mut warning = None;
    if let Err(error) = auth_svc
        .refresh_backup_if_due(&email, &AuthRefreshClient::new(), false, now_secs())
        .await
    {
        let message = format!(
            "Could not prepare the saved session for {email}: {error} Codex was left untouched. Sign in to that account again, then switch."
        );
        append_log(&state, &message);
        return Err(message);
    }

    append_log(
        &state,
        &match current.as_deref() {
            Some(current) if current != email => {
                format!("Switch requested: {current} -> {email}.")
            }
            _ => format!("Switch requested: {email}."),
        },
    );

    let app_worker = app.clone();
    let app_log = app.clone();
    let email_worker = email.clone();
    let current_worker = current.clone();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        let state = app_worker.state::<AppState>();
        crate::restart::switch_with_lifecycle(
            || {
                let st = state_lock(&state);
                st.auth
                    .switch_to_account_checked(&email_worker, current_worker.as_deref())
                    .map(|_| ())
            },
            |line| {
                append_log(&state, line);
                emit_stage(&app_log, line);
            },
        )
    })
    .await
    .map_err(|error| format!("Switch worker failed: {error}"))
    .and_then(|result| {
        if result.is_err() {
            let _ = app.emit("switch-stage", "failed");
        }
        result
    })?;
    let _ = app.emit("switch-stage", "done");

    let (jwt, fallback, active_email) = {
        let mut st = state_lock(&state);
        let live = st.auth.load_snapshot().ok();
        let live_email = live.as_ref().and_then(snapshot_email);
        if let Some(live_email) = live_email.as_deref() {
            if !live_email.eq_ignore_ascii_case(&email) {
                add_warning(
                    &mut warning,
                    format!(
                        "Codex reopened with {live_email} instead of {email}. Switch again or re-add that account."
                    ),
                );
            }
            // Codex may refresh tokens the moment it starts. File the live
            // session back under its own identity so a later switch never
            // restores an already-rotated refresh token.
            let _ = st.auth.backup_live_auth();
        }
        let jwt = st
            .auth
            .load_access_token()
            .ok_or_else(|| "The switched session has no access token.".to_string())?;
        let fallback = live
            .and_then(|snapshot| snapshot.tokens)
            .and_then(|tokens| tokens.account_id);
        let active = live_email.unwrap_or_else(|| email.clone());
        st.set_current(active.clone(), jwt.clone());
        (jwt, fallback, active)
    };
    drop(_auth_guard);

    refresh_quota_after_switch(
        &state,
        &active_email,
        &jwt,
        fallback.as_deref(),
        &mut warning,
    )
    .await;

    let message = if outcome.codex_was_running && outcome.codex_reopened {
        format!("Switched to {email}. Codex reopened with the new session.")
    } else if outcome.codex_was_running {
        format!("Switched to {email}. Codex did not reopen — open it manually.")
    } else {
        format!("Switched to {email}.")
    };
    if let Some(warning) = &warning {
        append_log(&state, &format!("Switch warning: {warning}"));
    }
    append_log(&state, &message);
    let _ = app.emit("switch-completed", &message);

    Ok(SwitchResult {
        email,
        already_active: false,
        codex_was_running: outcome.codex_was_running,
        codex_reopened: outcome.codex_reopened,
        warning,
        message,
    })
}

async fn refresh_quota_after_switch(
    state: &State<'_, AppState>,
    email: &str,
    jwt: &str,
    fallback_account_id: Option<&str>,
    warning: &mut Option<String>,
) {
    let api = UsageApiClient::new();
    match fetch_remote(&api, jwt, fallback_account_id).await {
        Ok(data) => {
            let mut st = state_lock(state);
            if let Err(error) = apply_fetched(&mut st, data, jwt, Some(email), false, now_secs()) {
                add_warning(warning, format!("Quota refresh failed: {error}"));
            }
        }
        Err(error) => add_warning(warning, format!("Quota refresh failed: {error}")),
    }
}

#[tauri::command]
pub fn codex_status() -> Result<crate::restart::CodexStatus, String> {
    crate::restart::status()
}

#[tauri::command]
pub async fn remove_account(state: State<'_, AppState>, email: String) -> Result<String, String> {
    let _guard = crate::auth::AUTH_OPERATION_LOCK.lock().await;
    let mut st = state_lock(&state);
    let key = st.canonical_key(&email).unwrap_or(email);
    if !st.remove_account(&key) {
        return Err(format!("Unknown account {key}."));
    }
    st.auth.remove_backup(&key);
    let message = format!("Removed {key}.");
    drop(st);
    append_log(&state, &message);
    Ok(message)
}

#[tauri::command]
pub fn set_archived(
    state: State<AppState>,
    email: String,
    archived: bool,
) -> Result<String, String> {
    let mut st = state_lock(&state);
    if !st.set_archived(&email, archived) {
        return Err(format!("Unknown account {email}."));
    }
    let message = if archived {
        format!("Archived {email}.")
    } else {
        format!("Unarchived {email}.")
    };
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
pub fn save_show_menu_bar(
    app: AppHandle,
    state: State<AppState>,
    show: bool,
) -> Result<String, String> {
    crate::tray::set_visible(&app, show)?;
    state_lock(&state).save_show_menu_bar(show);
    let message = if show {
        "Menu bar item shown."
    } else {
        "Menu bar item hidden. Closing the window now quits Codex Monitor."
    };
    append_log(&state, message);
    Ok(message.into())
}

#[tauri::command]
pub fn save_notifications_enabled(state: State<AppState>, enabled: bool) -> Result<(), String> {
    state_lock(&state).save_notifications_enabled(enabled);
    Ok(())
}

#[tauri::command]
pub fn save_auto_check_updates(state: State<AppState>, enabled: bool) -> Result<(), String> {
    state_lock(&state).save_auto_check_updates(enabled);
    Ok(())
}

#[tauri::command]
pub fn save_locale(app: AppHandle, state: State<AppState>, locale: String) -> Result<(), String> {
    let locale = crate::state::sanitize_locale(&locale);
    state_lock(&state).save_locale(&locale);
    // The preference is already saved; a hidden menu bar or a failed relabel
    // must not turn a successful language change into a settings error.
    if let Err(error) = crate::tray::set_locale(&app, crate::state::resolved_locale(&locale)) {
        eprintln!("tray locale update failed: {error}");
    }
    Ok(())
}

#[tauri::command]
pub fn get_resets(state: State<AppState>, email: String) -> Option<ResetCreditsPayload> {
    state_lock(&state)
        .usage
        .get(&email)
        .and_then(|a| a.resets.clone())
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

/// Append one line to the activity log from the frontend (update checks,
/// installs, and other UI-side events the backend never sees). Clipped to
/// 500 chars so a buggy caller cannot flood the file.
#[tauri::command]
pub fn log_message(state: State<AppState>, message: String) -> Result<(), String> {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    const MAX_LEN: usize = 500;
    let clipped: String = trimmed.chars().take(MAX_LEN).collect();
    append_log(&state, &clipped);
    Ok(())
}

#[tauri::command]
pub async fn export_data(state: State<'_, AppState>, path: String) -> Result<String, String> {
    let _guard = crate::auth::AUTH_OPERATION_LOCK.lock().await;
    let combined = {
        let st = state_lock(&state);
        let payload = st.storage.export_data(&st.usage);
        let backups: HashMapEmailSnapshots = st.auth.export_backups(&[]);
        serde_json::json!({
            "schema_version": 2,
            "accounts": payload["accounts"],
            "config": payload["config"],
            "backups": backups,
        })
    };
    let text = serde_json::to_vec_pretty(&combined)
        .map_err(|e| format!("Could not serialize export: {e}"))?;
    crate::fsutil::write_atomic(std::path::Path::new(&path), &text, 0o600)
        .map_err(|e| format!("Could not write export: {e}"))?;
    Ok(format!("Exported to {path}."))
}

#[tauri::command]
pub async fn import_data(state: State<'_, AppState>, path: String) -> Result<String, String> {
    let _guard = crate::auth::AUTH_OPERATION_LOCK.lock().await;
    let text = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let payload: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("Import file is not valid JSON: {e}"))?;
    let mut st = state_lock(&state);
    let merged = st.storage.import_data(&payload)?;
    if let Some(backups) = payload.get("backups").and_then(|v| v.as_object()) {
        let typed: HashMapEmailSnapshots = backups
            .iter()
            .filter_map(|(k, v)| {
                serde_json::from_value(v.clone())
                    .ok()
                    .map(|s| (k.clone(), s))
            })
            .collect();
        st.auth.import_backups(&typed);
    }
    // Re-read merged state (import_data already saved through storage).
    st.usage = merged;
    st.current_email = st
        .auth
        .load_snapshot()
        .ok()
        .and_then(|snapshot| snapshot_email(&snapshot))
        .and_then(|email| st.canonical_key(&email))
        .or_else(|| {
            st.storage
                .get_meta_value("current_account_email")
                .and_then(|v| v.as_str().map(String::from))
                .filter(|e| st.usage.contains_key(e))
        });
    st.session_tokens.clear();
    st.latest_jwt = None;
    let message = format!("Imported {} account(s).", st.usage.len());
    drop(st);
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
        let live = st.auth.load_snapshot().ok();
        let live_email = live.as_ref().and_then(snapshot_email);
        let live_token = live
            .as_ref()
            .and_then(|snapshot| snapshot.tokens.as_ref())
            .and_then(|tokens| tokens.access_token.clone())
            .filter(|token| !token.is_empty());
        if let (Some(live_email), Some(live_token)) = (live_email.clone(), live_token.clone()) {
            let differs = st
                .current_email
                .as_deref()
                .map(|current| !current.eq_ignore_ascii_case(&live_email))
                .unwrap_or(true);
            if differs {
                st.set_current(live_email, live_token);
            }
        }
        let Some(current) = st.due_auto_fetch_email(now) else {
            return Ok(None);
        };
        let jwt = if live_email
            .as_deref()
            .map(|email| email.eq_ignore_ascii_case(&current))
            .unwrap_or(false)
        {
            live_token
        } else {
            st.latest_jwt_for(Some(&current))
        };
        let Some(jwt) = jwt else { return Ok(None) };
        let fallback = live
            .as_ref()
            .and_then(|snapshot| snapshot.tokens.as_ref())
            .and_then(|tokens| tokens.account_id.clone());
        (jwt, current, fallback)
    };
    match fetch_remote(&api, &jwt, fallback_account.as_deref()).await {
        Ok(data) => {
            let mut st = state_lock(&state);
            match apply_fetched(&mut st, data, &jwt, Some(current.as_str()), true, now) {
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
    let (port, outcome) = match crate::oauth_login::start_callback_server(
        crate::oauth_login::CALLBACK_PORT,
        oauth_state.clone(),
    ) {
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
    // Activation rewrites the live auth file; never overlap a switch.
    let _auth_guard = tauri::async_runtime::block_on(crate::auth::AUTH_OPERATION_LOCK.lock());
    let api = UsageApiClient::new();
    let fail = |message: String| {
        if let Some(sess) = state.login.lock().unwrap().take() {
            let st = state_lock(state);
            st.auth.remove_login_codex_home(Some(&sess.home));
        }
        append_log(state, &message);
        let _ = app.emit(
            "codex-login-done",
            serde_json::json!({"ok": false, "message": message}),
        );
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
    let access_token = tokens.access_token.clone();
    let account_id = crate::oauth_login::account_id_from_id_token(&tokens.id_token);
    let mut snapshot = AuthFileSnapshot {
        auth_mode: Some("chatgpt".into()),
        openai_api_key: None,
        tokens: Some(AuthTokens {
            id_token: Some(tokens.id_token),
            access_token: Some(tokens.access_token),
            refresh_token: Some(tokens.refresh_token),
            account_id,
            extra: tokens.extra.clone(),
        }),
        last_refresh: Some(
            chrono::Utc::now()
                .format("%Y-%m-%dT%H:%M:%S%.6fZ")
                .to_string(),
        ),
        email: None,
        extra: Default::default(),
    };
    let claimed_email = snapshot_email(&snapshot).or_else(|| jwt_email(&access_token));
    let response: UsageResponse =
        match tauri::async_runtime::block_on(api.fetch_usage(&access_token)) {
            Ok(r) => r,
            Err(e) => {
                fail(format!("Login succeeded but quota fetch failed: {e}"));
                return;
            }
        };
    let response_email = match usage_response_email(&response) {
        Some(email) => email,
        None => {
            fail("Login produced no usable account.".into());
            return;
        }
    };
    if let Some(claimed) = claimed_email.as_deref() {
        if !claimed.eq_ignore_ascii_case(&response_email) {
            fail(format!(
                "Sign-in identity changed ({claimed} -> {response_email}). Nothing was activated."
            ));
            return;
        }
    }
    snapshot.email = Some(response_email.clone());

    let auth_svc = {
        let st = state_lock(state);
        AuthFileService::new(st.auth.auth_file_path.clone(), st.auth.accounts_dir.clone())
    };
    let source = auth_svc.active_auth_path_for_home(&home);
    if let Err(e) = auth_svc.write_snapshot_to_path(&source, &snapshot) {
        fail(format!("Login failed: {e}"));
        return;
    }
    // Adding an account is not a switch: the live Codex sign-in and the
    // running Codex process stay exactly as they are. The new session is
    // filed under its own identity so the user can activate it later.
    let live_backup_error = {
        let st = state_lock(state);
        if st.auth.auth_file_exists() {
            st.auth.backup_live_auth().err()
        } else {
            None
        }
    };
    if let Some(e) = live_backup_error {
        append_log(
            state,
            &format!("Previous Codex session could not be backed up ({e}); continuing."),
        );
    }
    let added = {
        let st = state_lock(state);
        st.auth.import_backup_snapshot(&snapshot)
    };
    let email = match added {
        Ok(email) => email,
        Err(e) => {
            fail(format!("Could not save the new account: {e}"));
            return;
        }
    };

    let message = {
        let mut st = state_lock(state);
        if st
            .apply_usage_response(&response, access_token.clone(), now_secs(), false)
            .is_none()
        {
            drop(st);
            fail("Login produced no usable account.".into());
            return;
        }
        if let Some(account_id) = response.account_id.clone() {
            if let Ok(credits) =
                tauri::async_runtime::block_on(api.fetch_reset_credits(&access_token, &account_id))
            {
                st.apply_reset_credits(&email, credits);
            }
        }
        format!("Added {email}. Switch to it from the list when you're ready.")
    };
    if state.login.lock().unwrap().take().is_some() {
        let st = state_lock(state);
        st.auth.remove_login_codex_home(Some(&home));
    }
    append_log(state, &message);
    let _ = app.emit(
        "codex-login-done",
        serde_json::json!({"ok": true, "message": message}),
    );
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
pub async fn restart_codex(app: AppHandle) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        // Lifecycle polling must not block Tauri's UI/event thread.
        tauri::async_runtime::spawn_blocking(move || {
            let state = app.state::<AppState>();
            append_log(&state, "Restart requested for Codex app.");
            match crate::restart::run(|message| {
                append_log(&state, message);
                emit_stage(&app, message);
            }) {
                Ok(()) => {
                    let _ = app.emit("switch-stage", "done");
                    Ok("Restarted the Codex app.".into())
                }
                Err(error) => {
                    let _ = app.emit("switch-stage", "failed");
                    append_log(&state, &format!("Restart failed: {error}"));
                    Err(error)
                }
            }
        })
        .await
        .map_err(|error| format!("Restart worker failed: {error}"))?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err("Restarting the Codex app is only supported on macOS.".into())
    }
}

#[tauri::command]
pub fn tray_show_main(app: AppHandle) {
    crate::tray::show_main_window(&app);
}

#[tauri::command]
pub fn tray_hide(app: AppHandle) {
    crate::tray::hide(&app);
}

#[tauri::command]
pub fn app_quit(app: AppHandle) {
    crate::tray::quit(&app);
}

/// Migrate legacy store files once, then seed state. Called from setup.
pub fn build_monitor_state() -> (MonitorState, PathBuf) {
    let storage = UsageStorage::with_defaults();
    let auth = AuthFileService::with_defaults();
    let log_path = storage
        .storage_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("activity.log");
    boot_line(&log_path, "CodexMonitor started.");
    let _ = crate::fsutil::ensure_private_dir(&auth.accounts_dir);
    let repaired = auth.reconcile_backups();
    if repaired > 0 {
        boot_line(
            &log_path,
            &format!("Repaired {repaired} saved sign-in(s) stored under the wrong account."),
        );
    }
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
        codex_status,
        remove_account,
        set_archived,
        save_auto_fetch,
        save_sort,
        save_show_archived,
        save_logs_expanded,
        save_show_menu_bar,
        save_notifications_enabled,
        save_auto_check_updates,
        save_locale,
        get_resets,
        get_logs,
        clear_logs,
        log_message,
        export_data,
        import_data,
        import_auth_json,
        import_auth_json_text,
        logout,
        login_start,
        login_cancel,
        open_login_url,
        restart_codex,
        tray_show_main,
        tray_hide,
        app_quit,
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
            show_menu_bar: st.show_menu_bar,
            notifications_enabled: st.notifications_enabled,
            auto_check_updates: st.auto_check_updates,
            locale: st.locale.clone(),
            auth_file_exists: st.auth.auth_file_exists(),
            backup_emails: st.auth.list_backup_emails(),
            app_version: "test".into(),
        };
        let text = serde_json::to_string(&snap).expect("snapshot must serialize");
        assert!(text.contains("accounts"));
    }

    #[test]
    fn lifecycle_stages_follow_the_switch_log() {
        assert_eq!(
            stage_from_log("Switch: closing Codex before writing the new session."),
            Some(LifecycleStage::Closing)
        );
        assert_eq!(
            stage_from_log("Switch: Codex closed; writing the selected account."),
            Some(LifecycleStage::Switching)
        );
        assert_eq!(
            stage_from_log("Restart: launching resolved Codex bundle (attempt 1/3)."),
            Some(LifecycleStage::Reopening)
        );
        assert_eq!(
            stage_from_log("Restart: Codex main process is running; launch verified."),
            Some(LifecycleStage::Reopened)
        );
        assert_eq!(
            stage_from_log("Restart: resolved bundle /Applications/ChatGPT.app."),
            None
        );
        assert_eq!(LifecycleStage::Closing.as_str(), "closing");
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

    fn test_state(tag: &str) -> MonitorState {
        let dir = std::env::temp_dir().join(format!("codex-commands-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        MonitorState::load(
            UsageStorage::new(dir.join("u.v2.json"), dir.join("u.v2.meta.json")),
            AuthFileService::new(dir.join("auth.json"), dir.join("accounts")),
        )
    }

    fn usage(email: &str) -> UsageResponse {
        serde_json::from_value(serde_json::json!({
            "email": email,
            "rate_limit": {
                "primary_window": {
                    "used_percent": 10.0,
                    "limit_window_seconds": 604800.0,
                    "reset_at": 1_778_000_000.0
                }
            }
        }))
        .unwrap()
    }

    fn jwt_for(email: &str) -> String {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
        let payload = URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&serde_json::json!({"email": email})).unwrap());
        format!("header.{payload}.signature")
    }

    #[test]
    fn mismatched_fetch_does_not_mutate_usage_or_current() {
        let mut st = test_state("mismatch");
        st.apply_usage_response(&usage("a@x.y"), "jwt-a".into(), 1.0, true);
        let before = st.usage.clone();
        let error = apply_fetched(
            &mut st,
            FetchedData {
                response: usage("b@x.y"),
                credits: None,
            },
            "jwt-a",
            Some("a@x.y"),
            true,
            2.0,
        )
        .unwrap_err();
        assert!(error.contains("Account changed during fetch"));
        assert_eq!(
            st.usage, before,
            "a rejected fetch must not add or update a row"
        );
        assert_eq!(st.current_email.as_deref(), Some("a@x.y"));
        assert_eq!(st.latest_jwt_for(None).as_deref(), Some("jwt-a"));
    }

    #[test]
    fn fetch_response_must_match_the_token_identity() {
        let mut st = test_state("token-mismatch");
        st.apply_usage_response(&usage("a@x.y"), "jwt-a".into(), 1.0, true);
        let before = st.usage.clone();
        let error = apply_fetched(
            &mut st,
            FetchedData {
                response: usage("a@x.y"),
                credits: None,
            },
            &jwt_for("b@x.y"),
            None,
            true,
            2.0,
        )
        .unwrap_err();
        assert!(error.contains("token belongs to b@x.y"));
        assert_eq!(st.usage, before);
        assert_eq!(st.current_email.as_deref(), Some("a@x.y"));
    }
}
