use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::api::{ApiError, AuthRefreshClient, UsageApiClient};
use crate::auth::{AuthFileService, HashMapEmailSnapshots};
use crate::models::{ResetCreditsPayload, UsageMap, UsageResponse};
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

fn augmented_path_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for extra in [
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/opt/homebrew/bin"),
            home.join(".local/bin"),
            home.join(".bun/bin"),
            home.join(".volta/bin"),
            home.join(".npm-global/bin"),
            home.join("bin"),
        ] {
            if !dirs.contains(&extra) {
                dirs.push(extra);
            }
        }
    }
    dirs
}

fn sidecar_triple() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-gnu"),
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        ("windows", "aarch64") => Some("aarch64-pc-windows-msvc"),
        _ => None,
    }
}

/// Path of the `codex` CLI shipped inside the app bundle (externalBin
/// sidecar, staged by scripts/fetch-codex-sidecar.mjs). In dev mode the
/// bundle does not exist, so the staged source dir is checked instead.
pub fn bundled_codex_binary(app: &AppHandle) -> Option<PathBuf> {
    let triple = sidecar_triple()?;
    let exe = if cfg!(windows) { ".exe" } else { "" };
    let file = format!("codex-{triple}{exe}");
    let mut candidates = Vec::new();
    if let Ok(dir) = app.path().resource_dir() {
        candidates.push(dir.join("binaries").join(&file));
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries").join(&file));
    candidates.into_iter().find(|p| p.is_file())
}

/// Locate the `codex` CLI: bundled sidecar first, then the
/// CODEX_MONITOR_CODEX_BIN override, then PATH plus well-known install
/// roots (GUI apps on macOS do not inherit the shell PATH).
/// Mirrors _find_codex_binary, with the PyInstaller-bundle branch replaced
/// by the Tauri externalBin sidecar.
pub fn find_codex_binary() -> Option<PathBuf> {
    if let Ok(env_bin) = std::env::var("CODEX_MONITOR_CODEX_BIN") {
        let p = PathBuf::from(&env_bin);
        if p.is_file() {
            return Some(p);
        }
    }
    let names: &[&str] = if cfg!(windows) { &["codex.exe", "codex.cmd"] } else { &["codex"] };
    for dir in augmented_path_dirs() {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(md) = fs::metadata(&candidate) {
                        if md.permissions().mode() & 0o111 == 0 {
                            let _ = fs::set_permissions(&candidate, fs::Permissions::from_mode(0o755));
                        }
                    }
                }
                return Some(candidate);
            }
        }
    }
    None
}

fn extract_url(line: &str) -> Option<String> {
    let start = line.find("https://").or_else(|| line.find("http://"))?;
    let end = line[start..]
        .find(|c: char| c.is_whitespace())
        .map(|i| start + i)
        .unwrap_or(line.len());
    let url = line[start..end].trim_end_matches(['.', ',', ')', ';', ']']);
    (!url.is_empty()).then(|| url.to_string())
}

fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for ch in chars.by_ref() {
                    if ch.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        if c == '\r' {
            continue;
        }
        out.push(c);
    }
    out
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
    let codex_bin = bundled_codex_binary(&app).or_else(find_codex_binary).ok_or_else(|| {
        "Could not find the `codex` CLI. Install it (npm i -g @openai/codex) or set CODEX_MONITOR_CODEX_BIN.".to_string()
    })?;
    let home = {
        let st = state_lock(&state);
        st.auth.create_login_codex_home()?
    };
    let mut cmd = if cfg!(windows) && codex_bin.extension().map(|e| e == "cmd").unwrap_or(false) {
        let mut c = Command::new("cmd");
        c.args(["/c", &codex_bin.to_string_lossy(), "login"]);
        c
    } else {
        let mut c = Command::new(&codex_bin);
        c.arg("login");
        c
    };
    cmd.env("CODEX_HOME", &home).stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
    let mut child = cmd.spawn().map_err(|e| format!("Failed to start login: {e}"))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let cancel = Arc::new(AtomicBool::new(false));
    *state.login.lock().unwrap() =
        Some(LoginSession { home: home.clone(), cancel: cancel.clone() });

    let app_out = app.clone();
    // The CLI mirrors everything to both stdout and stderr; skip repeats so
    // each line is shown (and each URL opened) once.
    let last_line: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let mut streams: Vec<Box<dyn Read + Send>> = Vec::new();
    if let Some(s) = stdout {
        streams.push(Box::new(s));
    }
    if let Some(s) = stderr {
        streams.push(Box::new(s));
    }
    for stream in streams {
        let app_c = app_out.clone();
        let cancel_c = cancel.clone();
        let last_c = last_line.clone();
        thread::spawn(move || {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                let clean = strip_ansi(&line);
                if clean.trim().is_empty() {
                    continue;
                }
                let duplicate = last_c
                    .lock()
                    .map(|mut seen| seen.replace(clean.clone()) == Some(clean.clone()))
                    .unwrap_or(false);
                if duplicate {
                    continue;
                }
                let _ = app_c.emit("codex-login-output", clean.clone());
                if let Some(url) = extract_url(&clean) {
                    let _ = app_c.emit("codex-login-url", url);
                }
                if cancel_c.load(Ordering::SeqCst) {
                    break;
                }
            }
        });
    }

    let state_c = app.clone();
    thread::spawn(move || {
        let state_c: State<AppState> = state_c.state();
        let exit_ok = loop {
            if cancel.load(Ordering::SeqCst) {
                let _ = child.kill();
                let _ = child.wait();
                finish_login(&app_out, &state_c, &home, LoginEnd::Cancelled);
                return;
            }
            match child.try_wait() {
                Ok(Some(status)) => break status.success(),
                Ok(None) => thread::sleep(std::time::Duration::from_millis(200)),
                Err(_) => break false,
            }
        };
        state_c.login.lock().unwrap().take();
        if exit_ok {
            finish_login(&app_out, &state_c, &home, LoginEnd::Exited);
        } else {
            finish_login(&app_out, &state_c, &home, LoginEnd::Failed);
        }
    });
    append_log(&state, "Started Codex login.");
    Ok("Login started. Complete the flow in the dialog.".into())
}

enum LoginEnd {
    Exited,
    Failed,
    Cancelled,
}

fn finish_login(app: &AppHandle, state: &State<AppState>, home: &Path, end: LoginEnd) {
    let api = UsageApiClient::new();
    let done = |ok: bool, message: String| {
        let st = state_lock(state);
        st.auth.remove_login_codex_home(Some(home));
        drop(st);
        append_log(state, &message);
        let _ = app.emit("codex-login-done", serde_json::json!({"ok": ok, "message": message}));
    };
    match end {
        LoginEnd::Cancelled => {
            done(false, "Login cancelled.".into());
            return;
        }
        LoginEnd::Failed => {
            done(false, "Login process exited without success.".into());
            return;
        }
        LoginEnd::Exited => {}
    }
    let isolated_path = {
        let st = state_lock(state);
        st.auth.active_auth_path_for_home(home)
    };
    let snapshot = {
        let st = state_lock(state);
        st.auth.load_snapshot_from_path(&isolated_path)
    };
    let jwt = match snapshot {
        Ok(s) => s.tokens.and_then(|t| t.access_token).filter(|t| !t.is_empty()),
        Err(e) => {
            done(false, format!("Login produced an unreadable auth file: {e}"));
            return;
        }
    };
    let Some(jwt) = jwt else {
        done(false, "Login produced no access token.".into());
        return;
    };
    let response: UsageResponse = match tauri::async_runtime::block_on(api.fetch_usage(&jwt)) {
        Ok(r) => r,
        Err(e) => {
            done(false, format!("Login succeeded but quota fetch failed: {e}"));
            return;
        }
    };
    let mut st = state_lock(state);
    let email = match st.apply_usage_response(&response, jwt.clone(), now_secs(), false) {
        Some(e) => e,
        None => {
            drop(st);
            done(false, "Login produced no usable account.".into());
            return;
        }
    };
    if let Some(current) = st.current_email.clone() {
        if current != email {
            let _ = st.auth.backup_current_auth(&current);
        }
    }
    let source = st.auth.active_auth_path_for_home(home);
    if let Err(e) = st.auth.activate_auth_from_path(&source) {
        drop(st);
        done(false, format!("Could not activate the new account: {e}"));
        return;
    }
    st.set_current(email.clone(), jwt);
    if let Some(account_id) = response.account_id.clone() {
        let jwt = st.latest_jwt_for(Some(&email)).unwrap_or_default();
        if let Ok(credits) = tauri::async_runtime::block_on(api.fetch_reset_credits(&jwt, &account_id)) {
            st.apply_reset_credits(&email, credits);
        }
    }
    drop(st);
    done(true, format!("Signed in as {email}."));
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
        let quit = Command::new("osascript")
            .args(["-e", "tell application \"Codex\" to quit"])
            .output();
        std::thread::sleep(std::time::Duration::from_secs(1));
        let open = Command::new("open").args(["-a", "Codex"]).status();
        match (quit, open) {
            (_, Ok(s)) if s.success() => Ok("Restarted the Codex app.".into()),
            _ => Err("Could not restart the Codex app.".into()),
        }
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
    fn login_url_extraction() {
        assert_eq!(extract_url("no url here"), None);
        assert_eq!(
            extract_url("navigate to https://auth.openai.com/oauth/authorize?a=1 to continue.").as_deref(),
            Some("https://auth.openai.com/oauth/authorize?a=1")
        );
        assert_eq!(
            extract_url("http://localhost:1455/callback").as_deref(),
            Some("http://localhost:1455/callback")
        );
    }

    #[test]
    fn sidecar_triple_matches_external_bin_naming() {
        let triple = sidecar_triple().expect("test host must map to a sidecar triple");
        assert!(triple.starts_with(std::env::consts::ARCH));
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
