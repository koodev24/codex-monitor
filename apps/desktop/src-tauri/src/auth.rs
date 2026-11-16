use std::fs;
use std::path::{Path, PathBuf};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

use crate::api::{ApiError, AuthRefreshClient, RefreshedTokens};
use crate::formatters::parse_iso_timestamp;
use crate::fsutil;
use crate::models::AuthFileSnapshot;

pub const AUTH_REFRESH_INTERVAL_SECS: f64 = 8.0 * 24.0 * 3600.0;

/// Refresh an access token slightly before it expires so Codex never starts on
/// a token that dies mid-handshake.
pub const ACCESS_TOKEN_SKEW_SECS: f64 = 5.0 * 60.0;

/// Serializes every operation that can rewrite the live auth file. Switching
/// from the main window, the menu-bar popover, or a login must never overlap.
pub static AUTH_OPERATION_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwitchAuthOutcome {
    AlreadyActive,
    Switched,
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

pub fn default_auth_file_path() -> PathBuf {
    home_dir()
        .map(|h| h.join(".codex/auth.json"))
        .unwrap_or_else(|| PathBuf::from("auth.json"))
}

pub fn default_accounts_dir() -> PathBuf {
    let (dir, _) = crate::storage::default_paths();
    dir.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("accounts")
}

fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

fn safe_key(email: &str) -> String {
    let mut s: String = normalize_email(email)
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._@+-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    s = s
        .trim_matches(|c| c == '.' || c == '_' || c == '-')
        .to_string();
    if s.is_empty() {
        s = "unknown".into();
    }
    s
}

fn copy_atomic(source: &Path, target: &Path) -> std::io::Result<()> {
    fsutil::copy_atomic(source, target, 0o600)
}

fn current_refresh_timestamp() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.6fZ")
        .to_string()
}

fn token_claims(token: &str) -> Option<serde_json::Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn id_token_claims(snapshot: &AuthFileSnapshot) -> Option<serde_json::Value> {
    token_claims(snapshot.tokens.as_ref()?.id_token.as_deref()?)
}

fn claims_email(claims: &serde_json::Value) -> Option<String> {
    claims
        .get("email")
        .and_then(|email| email.as_str())
        .map(str::trim)
        .filter(|email| !email.is_empty())
        .map(str::to_string)
}

/// Identity evidence may live in the stored `email` field or inside the
/// ChatGPT ID token. The signed token claim wins: the top-level field is an
/// app annotation that can be stale, missing, or copied from another account.
pub fn snapshot_email(snapshot: &AuthFileSnapshot) -> Option<String> {
    id_token_claims(snapshot)
        .as_ref()
        .and_then(claims_email)
        .or_else(|| {
            snapshot
                .email
                .clone()
                .map(|email| email.trim().to_string())
                .filter(|email| !email.is_empty())
        })
}

/// Email claim from an access token, used to verify that a usage response
/// belongs to the token that fetched it.
pub fn jwt_email(token: &str) -> Option<String> {
    token_claims(token).as_ref().and_then(claims_email)
}

/// Login identity for files that may predate the stored `email` field: the ID
/// token wins, then the stored annotation, then the access token claim.
pub fn login_email(snapshot: &AuthFileSnapshot) -> Option<String> {
    snapshot_email(snapshot).or_else(|| {
        snapshot
            .tokens
            .as_ref()
            .and_then(|tokens| tokens.access_token.as_deref())
            .and_then(jwt_email)
    })
}

fn claims_account_id(claims: &serde_json::Value) -> Option<String> {
    claims
        .get("https://api.openai.com/auth")
        .and_then(|auth| auth.get("chatgpt_account_id"))
        .and_then(|id| id.as_str())
        .or_else(|| claims.get("chatgpt_account_id").and_then(|id| id.as_str()))
        .filter(|id| !id.trim().is_empty())
        .map(str::to_string)
}

fn snapshot_account_id(snapshot: &AuthFileSnapshot) -> Option<String> {
    if let Some(id) = snapshot
        .tokens
        .as_ref()
        .and_then(|tokens| tokens.account_id.clone())
        .filter(|id| !id.trim().is_empty())
    {
        return Some(id);
    }
    let tokens = snapshot.tokens.as_ref()?;
    tokens
        .id_token
        .as_deref()
        .and_then(token_claims)
        .as_ref()
        .and_then(claims_account_id)
        .or_else(|| {
            tokens
                .access_token
                .as_deref()
                .and_then(token_claims)
                .as_ref()
                .and_then(claims_account_id)
        })
}

/// Codex records the workspace in `tokens.account_id`. Saved sessions that
/// predate that field still carry the same id in their JWT claims; fill it in
/// so the CLI and the desktop app can resolve the account the same way.
fn with_account_id(mut snapshot: AuthFileSnapshot) -> AuthFileSnapshot {
    let stored = snapshot
        .tokens
        .as_ref()
        .and_then(|tokens| tokens.account_id.as_deref())
        .is_some_and(|id| !id.trim().is_empty());
    if !stored {
        if let Some(id) = snapshot_account_id(&snapshot) {
            if let Some(tokens) = snapshot.tokens.as_mut() {
                tokens.account_id = Some(id);
            }
        }
    }
    snapshot
}

/// A workspace ID alone can be shared, so email is checked too. Codex treats
/// an account as already active only when the workspace id and the ChatGPT
/// user both match; when a file has no id, its exact ID token is the only
/// proof that it is the same session.
pub fn snapshots_same_account(a: &AuthFileSnapshot, b: &AuthFileSnapshot) -> bool {
    let same_id_token = || {
        let raw = |snapshot: &AuthFileSnapshot| {
            snapshot
                .tokens
                .as_ref()
                .and_then(|tokens| tokens.id_token.clone())
                .filter(|token| !token.is_empty())
        };
        match (raw(a), raw(b)) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        }
    };
    let ids_match = match (snapshot_account_id(a), snapshot_account_id(b)) {
        (Some(a), Some(b)) => a == b,
        _ => same_id_token(),
    };
    if !ids_match {
        return false;
    }
    match (snapshot_email(a), snapshot_email(b)) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(&b),
        _ => same_id_token(),
    }
}

/// Abstraction over token refresh so tests can fake it.
/// Mirrors AuthRefreshClient.refresh_tokens.
pub trait TokenRefresher {
    fn refresh_tokens(
        &self,
        refresh_token: &str,
    ) -> impl std::future::Future<Output = Result<RefreshedTokens, ApiError>> + Send;
}

impl TokenRefresher for AuthRefreshClient {
    fn refresh_tokens(
        &self,
        refresh_token: &str,
    ) -> impl std::future::Future<Output = Result<RefreshedTokens, ApiError>> + Send {
        self.refresh_tokens(refresh_token)
    }
}

fn parse_jwt_exp(token: &str) -> Option<f64> {
    token_claims(token)
        .as_ref()
        .and_then(|claims| claims.get("exp"))?
        .as_f64()
}

/// Refresh decisions follow the token Codex actually uses. Access-token
/// expiry is authoritative; the stored `last_refresh` timestamp is only a
/// fallback for opaque tokens. Refresh tokens are single-use, so a session
/// that is still valid must never be rotated just because it is old.
fn snapshot_needs_refresh(snapshot: &AuthFileSnapshot, now: f64) -> bool {
    let tokens = match snapshot.tokens.as_ref() {
        Some(t) => t,
        None => return false,
    };
    match tokens.refresh_token.as_deref() {
        Some(rt) if !rt.is_empty() => {}
        _ => return false,
    }
    if let Some(exp) = tokens.access_token.as_deref().and_then(parse_jwt_exp) {
        return exp <= now + ACCESS_TOKEN_SKEW_SECS;
    }
    match parse_iso_timestamp(snapshot.last_refresh.as_deref()) {
        Some(ts) => now - ts >= AUTH_REFRESH_INTERVAL_SECS,
        // Nothing proves the session is stale; never rotate on a guess.
        None => false,
    }
}

/// Mirrors AuthFileService in codex_monitor_app/services.py.
/// All paths injectable for tests; the shipped app uses defaults.
pub struct AuthFileService {
    pub auth_file_path: PathBuf,
    pub accounts_dir: PathBuf,
}

impl AuthFileService {
    pub fn new(auth_file_path: PathBuf, accounts_dir: PathBuf) -> Self {
        Self {
            auth_file_path,
            accounts_dir,
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(default_auth_file_path(), default_accounts_dir())
    }

    pub fn auth_file_exists(&self) -> bool {
        self.auth_file_path.is_file()
    }

    pub fn load_snapshot_from_path(&self, path: &Path) -> Result<AuthFileSnapshot, String> {
        let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
        Self::parse_snapshot(&text)
    }

    /// Parse an auth.json payload that arrived as text (pasted or read from a
    /// file) with the same shape checks in both cases.
    pub fn parse_snapshot(text: &str) -> Result<AuthFileSnapshot, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if !v.is_object() {
            return Err("auth.json root must be an object".into());
        }
        serde_json::from_value(v).map_err(|e| e.to_string())
    }

    pub fn load_snapshot(&self) -> Result<AuthFileSnapshot, String> {
        self.load_snapshot_from_path(&self.auth_file_path)
    }

    pub fn load_access_token(&self) -> Option<String> {
        self.load_snapshot()
            .ok()
            .and_then(|s| s.tokens)
            .and_then(|t| t.access_token)
    }

    /// Resolve an existing backup path case-insensitively so accounts created
    /// before keys were normalized (or by hand) are still found.
    fn resolved_backup_path(&self, email: &str) -> Option<PathBuf> {
        let wanted = format!("auth-{}.json", safe_key(email));
        let entries = fs::read_dir(&self.accounts_dir).ok()?;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.eq_ignore_ascii_case(&wanted) && entry.path().is_file() {
                return Some(entry.path());
            }
        }
        None
    }

    pub fn backup_path_for_email(&self, email: &str) -> PathBuf {
        self.resolved_backup_path(email).unwrap_or_else(|| {
            self.accounts_dir
                .join(format!("auth-{}.json", safe_key(email)))
        })
    }

    pub fn backup_exists(&self, email: &str) -> bool {
        self.backup_path_for_email(email).is_file()
    }

    /// True when the backup key and the identity inside the file agree.
    /// Older installs can contain a file saved under the wrong email; those
    /// are repaired at startup by `reconcile_backups`.
    pub fn backup_identity_matches_key(&self, snapshot: &AuthFileSnapshot, key: &str) -> bool {
        match snapshot_email(snapshot) {
            Some(actual) => actual.eq_ignore_ascii_case(key),
            None => true,
        }
    }

    pub fn list_backup_emails(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let entries = fs::read_dir(&self.accounts_dir).ok();
        if entries.is_none() {
            return out;
        }
        for entry in entries.unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with("auth-") || !name.ends_with(".json") {
                continue;
            }
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let snap: Option<AuthFileSnapshot> = fs::read_to_string(&path)
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok());
            match snap {
                Some(s) => {
                    if let Some(email) = snapshot_email(&s) {
                        out.push(email);
                        continue;
                    }
                    let account_id = s
                        .tokens
                        .and_then(|t| t.account_id)
                        .filter(|a| !a.is_empty());
                    if account_id.is_some() {
                        out.push(name["auth-".len()..name.len() - ".json".len()].to_string());
                    }
                }
                None => continue,
            }
        }
        out.sort_by_key(|email| email.to_lowercase());
        out.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        out
    }

    /// Save an external Codex `auth.json` as an account backup without
    /// touching the live session. Mirrors what a fresh sign-in would store so
    /// imported accounts behave like any other saved account afterwards.
    pub fn import_backup_snapshot(&self, snapshot: &AuthFileSnapshot) -> Result<String, String> {
        let tokens = snapshot
            .tokens
            .as_ref()
            .filter(|tokens| {
                [
                    tokens.access_token.as_deref(),
                    tokens.refresh_token.as_deref(),
                ]
                .iter()
                .flatten()
                .any(|token| !token.trim().is_empty())
            })
            .ok_or_else(|| {
                "That auth.json has no ChatGPT tokens. Only ChatGPT sign-ins can be imported."
                    .to_string()
            })?;
        if let Some(token_email) = tokens.access_token.as_deref().and_then(jwt_email) {
            if let Some(file_email) = snapshot_email(snapshot) {
                if !token_email.eq_ignore_ascii_case(&file_email) {
                    return Err(format!(
                        "The file is labelled {file_email} but its token belongs to {token_email}. Nothing was imported."
                    ));
                }
            }
        }
        let email = login_email(snapshot).ok_or_else(|| {
            "That auth.json has no readable ChatGPT identity. Sign in to it in Codex once, then import it again."
                .to_string()
        })?;
        self.write_backup_snapshot(&email, snapshot)?;
        Ok(email)
    }

    pub fn backup_current_auth(&self, email: &str) -> Result<PathBuf, String> {
        if !self.auth_file_exists() {
            return Err(format!("{} not found", self.auth_file_path.display()));
        }
        let live = self.load_snapshot()?;
        self.write_backup_snapshot(email, &live)
    }

    /// Preserve the live session under the identity inside the file, never
    /// under an email supplied by stale state. Returns the saved key.
    pub fn backup_live_auth(&self) -> Result<Option<String>, String> {
        if !self.auth_file_exists() {
            return Ok(None);
        }
        let live = self.load_snapshot()?;
        let Some(email) = snapshot_email(&live) else {
            return Ok(None);
        };
        self.write_backup_snapshot(&email, &live)?;
        Ok(Some(email))
    }

    pub fn remove_backup(&self, email: &str) -> bool {
        if email.is_empty() {
            return false;
        }
        fs::remove_file(self.backup_path_for_email(email)).is_ok()
    }

    pub fn remove_backup_if_access_token_matches(&self, email: &str, access_token: &str) -> bool {
        if email.is_empty() || access_token.is_empty() {
            return false;
        }
        match self.load_backup_access_token(email) {
            Some(t) if t == access_token => self.remove_backup(email),
            _ => false,
        }
    }

    pub fn load_backup_snapshot(&self, email: &str) -> Result<AuthFileSnapshot, String> {
        let path = self.backup_path_for_email(email);
        self.load_snapshot_from_path(&path)
    }

    pub fn load_backup_access_token(&self, email: &str) -> Option<String> {
        self.load_backup_snapshot(email)
            .ok()
            .and_then(|s| s.tokens)
            .and_then(|t| t.access_token)
    }

    fn write_backup_snapshot(
        &self,
        email: &str,
        snapshot: &AuthFileSnapshot,
    ) -> Result<PathBuf, String> {
        if email.trim().is_empty() {
            return Err("Refusing to save a backup without an account email.".into());
        }
        let snapshot = with_account_id(snapshot.clone());
        if let Some(actual) = snapshot_email(&snapshot) {
            if !actual.eq_ignore_ascii_case(email) {
                return Err(format!(
                    "Refusing to save the session for {actual} under {email}."
                ));
            }
        }
        let target = self.backup_path_for_email(email);
        fsutil::ensure_private_dir(&self.accounts_dir).map_err(|e| e.to_string())?;
        let text = serde_json::to_string_pretty(&snapshot).map_err(|e| e.to_string())?;
        fsutil::write_atomic(&target, text.as_bytes(), 0o600).map_err(|e| e.to_string())?;
        Ok(target)
    }

    /// Load a backup only when its contents belong to the key it was saved
    /// under. A mismatch means an older build mislabeled the file.
    pub fn load_verified_backup(&self, email: &str) -> Result<AuthFileSnapshot, String> {
        let snapshot = self.load_backup_snapshot(email)?;
        if let Some(actual) = snapshot_email(&snapshot) {
            if !actual.eq_ignore_ascii_case(email) {
                return Err(format!(
                    "The saved sign-in for {email} actually belongs to {actual}. Remove it and sign in again."
                ));
            }
        }
        Ok(snapshot)
    }

    /// Rewrite backup files that were previously saved under the wrong email.
    /// The correctly-named copy is written first; the old file is removed only
    /// after that succeeds, so a crash cannot lose the session.
    pub fn reconcile_backups(&self) -> usize {
        let Ok(entries) = fs::read_dir(&self.accounts_dir) else {
            return 0;
        };
        let mut repaired = 0;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with("auth-") || !name.ends_with(".json") {
                continue;
            }
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Ok(snapshot) = self.load_snapshot_from_path(&path) else {
                continue;
            };
            let Some(actual) = snapshot_email(&snapshot) else {
                continue;
            };
            let key = &name["auth-".len()..name.len() - ".json".len()];
            if actual.eq_ignore_ascii_case(key) || safe_key(&actual) == key.to_lowercase() {
                continue;
            }
            let target = self.backup_path_for_email(&actual);
            if fsutil::copy_atomic(&path, &target, 0o600).is_ok() && target != path {
                let _ = fs::remove_file(&path);
                repaired += 1;
            }
        }
        repaired
    }

    pub fn export_backups(&self, emails: &[String]) -> HashMapEmailSnapshots {
        let mut wanted: Vec<String> = emails.to_vec();
        wanted.extend(self.list_backup_emails());
        wanted.sort_by_key(|email| email.to_lowercase());
        wanted.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        let mut out = HashMapEmailSnapshots::new();
        for email in wanted {
            if !self.backup_exists(&email) {
                continue;
            }
            if let Ok(snap) = self.load_verified_backup(&email) {
                // Export under the identity inside the file, never under a
                // stale map key that could mislabel a future import.
                let key = snapshot_email(&snap).unwrap_or(email);
                out.insert(key, snap);
            }
        }
        out
    }

    pub fn import_backups(&self, backups: &HashMapEmailSnapshots) -> usize {
        let mut count = 0;
        for (key, snapshot) in backups {
            let has_token = snapshot
                .tokens
                .as_ref()
                .and_then(|t| t.access_token.as_ref())
                .map(|t| !t.is_empty())
                .unwrap_or(false);
            if !has_token {
                continue;
            }
            // Trust the identity inside the credential, not the JSON key.
            let email = snapshot_email(snapshot)
                .or_else(|| key.contains('@').then(|| key.trim().to_string()))
                .filter(|email| !email.is_empty());
            let Some(email) = email else { continue };
            if self.write_backup_snapshot(&email, snapshot).is_ok() {
                count += 1;
            }
        }
        count
    }

    async fn refresh_snapshot_tokens<R: TokenRefresher>(
        &self,
        snapshot: &AuthFileSnapshot,
        refresher: &R,
    ) -> Result<AuthFileSnapshot, String> {
        let tokens = snapshot
            .tokens
            .as_ref()
            .ok_or_else(|| "auth backup has no tokens object".to_string())?;
        let rt = tokens
            .refresh_token
            .as_deref()
            .filter(|r| !r.is_empty())
            .ok_or_else(|| "auth backup has no refresh_token".to_string())?;
        let refreshed = refresher
            .refresh_tokens(rt)
            .await
            .map_err(|e| e.to_string())?;
        if refreshed.access_token.is_empty() {
            return Err("token refresh response has no access_token".into());
        }
        let mut next = snapshot.clone();
        let mut next_tokens = tokens.clone();
        if let Some(id) = refreshed.id_token.filter(|s| !s.is_empty()) {
            next_tokens.id_token = Some(id);
        }
        next_tokens.access_token = Some(refreshed.access_token);
        if let Some(r) = refreshed.refresh_token.filter(|s| !s.is_empty()) {
            next_tokens.refresh_token = Some(r);
        }
        next.tokens = Some(next_tokens);
        next.last_refresh = Some(current_refresh_timestamp());
        Ok(next)
    }

    pub async fn refresh_backup_if_due<R: TokenRefresher>(
        &self,
        email: &str,
        refresher: &R,
        force: bool,
        now: f64,
    ) -> Result<AuthFileSnapshot, String> {
        let snapshot = self.load_verified_backup(email)?;
        if !force && !snapshot_needs_refresh(&snapshot, now) {
            return Ok(snapshot);
        }
        let next = self.refresh_snapshot_tokens(&snapshot, refresher).await?;
        self.write_backup_snapshot(email, &next)?;
        Ok(next)
    }

    /// True when the live auth already belongs to `target_email`. Also syncs
    /// rotated Codex tokens into the backup so a no-op click never stale-ifies
    /// a future switch back.
    pub fn target_already_active(&self, target_email: &str) -> Result<bool, String> {
        if !self.auth_file_exists() || !self.backup_exists(target_email) {
            return Ok(false);
        }
        if fsutil::has_multiple_links(&self.auth_file_path) {
            return Err(format!(
                "{} is hard-linked. Remove the extra link so Codex and Codex Monitor cannot read different copies of the session.",
                self.auth_file_path.display()
            ));
        }
        let target = self.load_verified_backup(target_email)?;
        let live = self.load_snapshot()?;
        if snapshots_same_account(&live, &target) {
            if let Some(identity) = snapshot_email(&live) {
                self.write_backup_snapshot(&identity, &live)?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Switch using the live file as the source of truth:
    /// - no-op when the live session already belongs to the target;
    /// - preserve Codex-rotated tokens instead of restoring a stale backup;
    /// - never file a live session under the wrong email;
    /// - verify the file after writing it.
    pub fn switch_to_account_checked(
        &self,
        target_email: &str,
        current_email: Option<&str>,
    ) -> Result<SwitchAuthOutcome, String> {
        if !self.backup_exists(target_email) {
            return Err(format!(
                "No saved sign-in for {target_email}. Sign in to it again before switching."
            ));
        }
        if fsutil::has_multiple_links(&self.auth_file_path) {
            return Err(format!(
                "{} is hard-linked. Remove the extra link so Codex and Codex Monitor cannot read different copies of the session.",
                self.auth_file_path.display()
            ));
        }
        let target = self.load_verified_backup(target_email)?;
        let has_session = target
            .tokens
            .as_ref()
            .and_then(|tokens| tokens.access_token.as_deref())
            .is_some_and(|token| !token.trim().is_empty());
        if !has_session {
            return Err(format!(
                "The saved sign-in for {target_email} has no access token. Sign in to it again before switching."
            ));
        }

        if self.auth_file_exists() {
            let live = self
                .load_snapshot()
                .map_err(|e| format!("Cannot read the live Codex session: {e}"))?;
            if snapshots_same_account(&live, &target) {
                // Codex rotates single-use refresh tokens; sync the live file
                // back to the backup so a later switch back is not stale.
                if let Some(identity) = snapshot_email(&live) {
                    self.write_backup_snapshot(&identity, &live)?;
                }
                return Ok(SwitchAuthOutcome::AlreadyActive);
            }
            // Preserve the outgoing session under the identity inside the
            // file. Only fall back to the caller's current email when the
            // file carries no identity at all.
            match snapshot_email(&live) {
                Some(identity) => {
                    self.write_backup_snapshot(&identity, &live)?;
                }
                None => {
                    if let Some(current) = current_email.filter(|email| !email.is_empty()) {
                        let _ = self.write_backup_snapshot(current, &live);
                    }
                }
            }
        }

        // Write the selected session with a fresh `last_refresh` stamp. Codex
        // uses that stamp to decide whether it must refresh on startup; a
        // stale value makes it rotate the single-use refresh token during the
        // handoff, which is exactly when a session can be lost.
        let mut live = with_account_id(target.clone());
        live.email = None;
        live.last_refresh = Some(current_refresh_timestamp());
        let text = serde_json::to_string_pretty(&live)
            .map_err(|e| format!("Could not serialize the selected session: {e}"))?;
        fsutil::write_atomic(&self.auth_file_path, text.as_bytes(), 0o600)
            .map_err(|e| format!("Could not write the Codex session: {e}"))?;

        let applied = self
            .load_snapshot()
            .map_err(|e| format!("auth.json did not read back after the switch: {e}"))?;
        if !snapshots_same_account(&applied, &target) {
            return Err(
                "The switched file did not match the selected account. Nothing else was changed."
                    .into(),
            );
        }
        Ok(SwitchAuthOutcome::Switched)
    }

    pub fn create_login_codex_home(&self) -> Result<PathBuf, String> {
        fsutil::ensure_private_dir(&self.accounts_dir).map_err(|e| e.to_string())?;
        let dir = self.accounts_dir.join(format!(
            "login-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        ));
        fsutil::ensure_private_dir(&dir).map_err(|e| e.to_string())?;
        Ok(dir)
    }

    pub fn remove_login_codex_home(&self, home: Option<&Path>) {
        let home = match home {
            Some(h) => h,
            None => return,
        };
        let Ok(canonical_accounts) = self.accounts_dir.canonicalize() else {
            return;
        };
        let Ok(canonical_target) = home.canonicalize() else {
            return;
        };
        if !canonical_target.starts_with(&canonical_accounts) {
            return;
        }
        if home
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with("login-"))
            .unwrap_or(false)
        {
            let _ = fs::remove_dir_all(home);
        }
    }

    pub fn active_auth_path_for_home(&self, codex_home: &Path) -> PathBuf {
        codex_home.join("auth.json")
    }

    pub fn activate_auth_from_path(&self, source: &Path) -> Result<PathBuf, String> {
        let target = self.auth_file_path.clone();
        copy_atomic(source, &target).map_err(|e| e.to_string())?;
        Ok(target)
    }

    /// Write a snapshot atomically at an explicit path, used for the isolated
    /// login home before the credentials are promoted to the live file.
    pub fn write_snapshot_to_path(
        &self,
        path: &Path,
        snapshot: &AuthFileSnapshot,
    ) -> Result<(), String> {
        let text = serde_json::to_string_pretty(snapshot).map_err(|e| e.to_string())?;
        fsutil::write_atomic(path, text.as_bytes(), 0o600).map_err(|e| e.to_string())
    }

    /// Activate freshly exchanged credentials and verify the live file really
    /// contains the account we just signed in as.
    pub fn activate_auth_from_path_checked(
        &self,
        source: &Path,
        expected: &AuthFileSnapshot,
    ) -> Result<PathBuf, String> {
        let target = self.activate_auth_from_path(source)?;
        let applied = self
            .load_snapshot()
            .map_err(|e| format!("auth.json did not read back after activation: {e}"))?;
        if !snapshots_same_account(&applied, expected) {
            return Err(
                "The activated file did not match the account that just signed in. Nothing else was changed."
                    .into(),
            );
        }
        Ok(target)
    }
}

pub type HashMapEmailSnapshots = std::collections::HashMap<String, AuthFileSnapshot>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::RefreshedTokens;

    struct FakeRefresher {
        tokens: RefreshedTokens,
    }

    impl TokenRefresher for FakeRefresher {
        fn refresh_tokens(
            &self,
            _rt: &str,
        ) -> impl std::future::Future<Output = Result<RefreshedTokens, ApiError>> + Send {
            let tokens = self.tokens.clone();
            async move { Ok(tokens) }
        }
    }

    fn temp_service(tag: &str) -> (AuthFileService, PathBuf) {
        let dir = std::env::temp_dir().join(format!("codex-auth-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let svc = AuthFileService::new(dir.join(".codex/auth.json"), dir.join("accounts"));
        (svc, dir)
    }

    fn fixture_snapshot(email: &str, last_refresh: &str) -> AuthFileSnapshot {
        serde_json::from_value(serde_json::json!({
            "auth_mode": "chatgpt",
            "email": email,
            "tokens": {
                "id_token": jwt_for(email), "access_token": format!("access-{email}"),
                "refresh_token": "rt", "account_id": "uuid"
            },
            "last_refresh": last_refresh
        }))
        .unwrap()
    }

    fn jwt_for(email: &str) -> String {
        let payload = URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&serde_json::json!({"email": email})).unwrap());
        format!("header.{payload}.signature")
    }

    fn jwt_with_exp(exp: f64) -> String {
        let payload =
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&serde_json::json!({"exp": exp})).unwrap());
        format!("header.{payload}.signature")
    }

    fn write_json(path: &Path, v: &serde_json::Value) {
        if let Some(p) = path.parent() {
            fs::create_dir_all(p).unwrap();
        }
        fs::write(path, serde_json::to_string(v).unwrap()).unwrap();
    }

    #[test]
    fn backup_switch_round_trip() {
        let (svc, _dir) = temp_service("roundtrip");
        write_json(
            &svc.auth_file_path,
            &serde_json::to_value(fixture_snapshot("a@x.y", "2026-04-24T15:17:00.949966Z"))
                .unwrap(),
        );
        svc.backup_current_auth("a@x.y").unwrap();
        assert!(svc.backup_exists("a@x.y"));

        write_json(
            &svc.auth_file_path,
            &serde_json::to_value(fixture_snapshot("b@x.y", "2026-04-24T15:17:00.949966Z"))
                .unwrap(),
        );
        svc.backup_current_auth("b@x.y").unwrap();

        let outcome = svc
            .switch_to_account_checked("a@x.y", Some("b@x.y"))
            .unwrap();
        assert_eq!(outcome, SwitchAuthOutcome::Switched);
        assert_eq!(svc.load_access_token().as_deref(), Some("access-a@x.y"));
        // Current account was backed up before switching.
        assert_eq!(
            svc.load_backup_access_token("b@x.y").as_deref(),
            Some("access-b@x.y")
        );
        assert!(svc.switch_to_account_checked("nope@x.y", None).is_err());
    }

    #[test]
    fn switching_to_the_live_account_is_a_no_op_that_keeps_rotated_tokens() {
        let (svc, _dir) = temp_service("noop");
        let live = fixture_snapshot("a@x.y", "2026-04-24T15:17:00.949966Z");
        svc.write_backup_snapshot("a@x.y", &live).unwrap();
        write_json(
            &svc.auth_file_path,
            &serde_json::json!({
            "email": "a@x.y",
            "tokens": {"access_token": "rotated-live", "refresh_token": "rotated-r", "account_id": "uuid"},
            "last_refresh": "2026-04-25T15:17:00.949966Z"}),
        );

        let outcome = svc
            .switch_to_account_checked("a@x.y", Some("a@x.y"))
            .unwrap();
        assert_eq!(outcome, SwitchAuthOutcome::AlreadyActive);
        assert_eq!(svc.load_access_token().as_deref(), Some("rotated-live"));
        assert_eq!(
            svc.load_backup_access_token("a@x.y").as_deref(),
            Some("rotated-live"),
            "the rotated session must replace the stale backup"
        );
    }

    #[test]
    fn switching_back_keeps_tokens_that_codex_rotated_while_active() {
        // A -> B -> A. Codex refreshes B's single-use refresh token while B is
        // active; switching away must preserve the rotated session so switching
        // back (and the usage meters that depend on it) still works.
        let (svc, _dir) = temp_service("rotate-away");
        let a = fixture_snapshot("a@x.y", "2026-04-24T15:17:00.949966Z");
        let b = fixture_snapshot("b@x.y", "2026-04-24T15:17:00.949966Z");
        svc.write_backup_snapshot("a@x.y", &a).unwrap();
        svc.write_backup_snapshot("b@x.y", &b).unwrap();
        write_json(&svc.auth_file_path, &serde_json::to_value(&a).unwrap());

        svc.switch_to_account_checked("b@x.y", Some("a@x.y"))
            .unwrap();
        assert_eq!(svc.load_access_token().as_deref(), Some("access-b@x.y"));

        // Codex starts, refreshes B, and rewrites the live file.
        write_json(
            &svc.auth_file_path,
            &serde_json::json!({
                "auth_mode": "chatgpt",
                "tokens": {
                    "id_token": jwt_for("b@x.y"),
                    "access_token": "rotated-access-b",
                    "refresh_token": "rotated-refresh-b",
                    "account_id": "uuid"
                },
                "last_refresh": "2026-04-25T15:17:00.949966Z"
            }),
        );

        svc.switch_to_account_checked("a@x.y", Some("b@x.y"))
            .unwrap();
        let saved_b: AuthFileSnapshot = svc.load_backup_snapshot("b@x.y").unwrap();
        let saved_tokens = saved_b.tokens.unwrap();
        assert_eq!(
            saved_tokens.refresh_token.as_deref(),
            Some("rotated-refresh-b"),
            "the rotated refresh token must survive the switch away from B"
        );
        assert_eq!(svc.load_access_token().as_deref(), Some("access-a@x.y"));

        svc.switch_to_account_checked("b@x.y", Some("a@x.y"))
            .unwrap();
        let live = svc.load_snapshot().unwrap();
        assert_eq!(
            live.tokens.as_ref().unwrap().refresh_token.as_deref(),
            Some("rotated-refresh-b")
        );
        assert_eq!(snapshot_email(&live).as_deref(), Some("b@x.y"));

        // A -> B -> A again must not restore any already-used token.
        svc.switch_to_account_checked("a@x.y", Some("b@x.y"))
            .unwrap();
        assert_eq!(svc.load_access_token().as_deref(), Some("access-a@x.y"));
        assert_eq!(
            svc.load_backup_access_token("b@x.y").as_deref(),
            Some("rotated-access-b")
        );
    }

    #[test]
    fn pasted_auth_json_parses_like_a_file() {
        let snapshot = fixture_snapshot("a@x.y", "2026-04-24T15:17:00.949966Z");
        let text = serde_json::to_string(&snapshot).unwrap();
        let parsed = AuthFileService::parse_snapshot(&text).unwrap();
        assert_eq!(login_email(&parsed).as_deref(), Some("a@x.y"));
        assert!(AuthFileService::parse_snapshot("[]").is_err());
        assert!(AuthFileService::parse_snapshot("not json").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn switch_writes_through_a_symlinked_auth_file() {
        use std::os::unix::fs::symlink;

        let (svc, _dir) = temp_service("symlink");
        let a = fixture_snapshot("a@x.y", "2026-04-24T15:17:00.949966Z");
        let b = fixture_snapshot("b@x.y", "2026-04-24T15:17:00.949966Z");
        svc.write_backup_snapshot("a@x.y", &a).unwrap();
        svc.write_backup_snapshot("b@x.y", &b).unwrap();

        let real = svc.auth_file_path.with_file_name("real-auth.json");
        fs::create_dir_all(real.parent().unwrap()).unwrap();
        fs::write(&real, serde_json::to_string(&a).unwrap()).unwrap();
        symlink(&real, &svc.auth_file_path).unwrap();

        svc.switch_to_account_checked("b@x.y", Some("a@x.y"))
            .unwrap();

        assert!(
            fs::symlink_metadata(&svc.auth_file_path)
                .unwrap()
                .file_type()
                .is_symlink(),
            "the switch must not replace a symlinked auth.json"
        );
        let written: AuthFileSnapshot =
            serde_json::from_str(&fs::read_to_string(&real).unwrap()).unwrap();
        assert_eq!(
            written.tokens.unwrap().access_token.as_deref(),
            Some("access-b@x.y")
        );
    }

    #[cfg(unix)]
    #[test]
    fn switch_refuses_a_hard_linked_auth_file_without_touching_it() {
        let (svc, _dir) = temp_service("hardlink");
        let a = fixture_snapshot("a@x.y", "2026-04-24T15:17:00.949966Z");
        let b = fixture_snapshot("b@x.y", "2026-04-24T15:17:00.949966Z");
        svc.write_backup_snapshot("a@x.y", &a).unwrap();
        svc.write_backup_snapshot("b@x.y", &b).unwrap();
        fs::create_dir_all(svc.auth_file_path.parent().unwrap()).unwrap();
        fs::write(&svc.auth_file_path, serde_json::to_string(&a).unwrap()).unwrap();
        let other = svc.auth_file_path.with_file_name("other-auth.json");
        fs::hard_link(&svc.auth_file_path, &other).unwrap();

        let error = svc
            .switch_to_account_checked("b@x.y", Some("a@x.y"))
            .unwrap_err();
        assert!(error.contains("hard-linked"), "{error}");
        assert_eq!(
            svc.load_access_token().as_deref(),
            Some("access-a@x.y"),
            "a refused switch must leave the live session untouched"
        );
        let linked: AuthFileSnapshot =
            serde_json::from_str(&fs::read_to_string(&other).unwrap()).unwrap();
        assert_eq!(
            linked.tokens.unwrap().access_token.as_deref(),
            Some("access-a@x.y")
        );
    }

    #[test]
    fn switching_writes_the_target_and_preserves_the_live_session() {
        let (svc, _dir) = temp_service("checked");
        let target = fixture_snapshot("b@x.y", "2026-04-24T15:17:00.949966Z");
        svc.write_backup_snapshot("b@x.y", &target).unwrap();
        let current = fixture_snapshot("a@x.y", "2026-04-24T15:17:00.949966Z");
        svc.write_backup_snapshot("a@x.y", &current).unwrap();
        write_json(
            &svc.auth_file_path,
            &serde_json::json!({
            "email": "a@x.y",
            "tokens": {"access_token": "active-a", "refresh_token": "r", "account_id": "uuid"},
            "last_refresh": "2026-04-24T15:17:00.949966Z"}),
        );

        let outcome = svc
            .switch_to_account_checked("b@x.y", Some("a@x.y"))
            .unwrap();
        assert_eq!(outcome, SwitchAuthOutcome::Switched);
        assert_eq!(svc.load_access_token().as_deref(), Some("access-b@x.y"));
        assert_eq!(
            svc.load_backup_access_token("a@x.y").as_deref(),
            Some("active-a")
        );
    }

    #[test]
    fn live_session_is_never_saved_under_an_email_that_does_not_own_it() {
        let (svc, _dir) = temp_service("mismatch");
        let target = fixture_snapshot("b@x.y", "2026-04-24T15:17:00.949966Z");
        svc.write_backup_snapshot("b@x.y", &target).unwrap();
        let stated_current = fixture_snapshot("a@x.y", "2026-04-24T15:17:00.949966Z");
        svc.write_backup_snapshot("a@x.y", &stated_current).unwrap();
        write_json(
            &svc.auth_file_path,
            &serde_json::json!({
            "email": "someone-else@x.y",
            "tokens": {"access_token": "foreign", "refresh_token": "r", "account_id": "other"},
            "last_refresh": "2026-04-24T15:17:00.949966Z"}),
        );

        svc.switch_to_account_checked("b@x.y", Some("a@x.y"))
            .unwrap();
        assert_eq!(
            svc.load_backup_access_token("a@x.y").as_deref(),
            Some("access-a@x.y"),
            "a foreign live session must not overwrite a@x.y's backup"
        );
        assert_eq!(
            svc.load_backup_access_token("someone-else@x.y").as_deref(),
            Some("foreign"),
            "the outgoing session must still be preserved under its real identity"
        );
    }

    #[test]
    fn id_token_claim_outranks_a_stale_stored_email() {
        let snap: AuthFileSnapshot = serde_json::from_value(serde_json::json!({
            "email": "stale@x.y",
            "tokens": {"id_token": jwt_for("real@x.y"), "access_token": "a"}
        }))
        .unwrap();
        assert_eq!(snapshot_email(&snap).as_deref(), Some("real@x.y"));
        assert_eq!(jwt_email(&jwt_for("real@x.y")).as_deref(), Some("real@x.y"));
        assert_eq!(jwt_email("not-a-jwt"), None);
    }

    #[test]
    fn backup_writes_reject_a_foreign_snapshot() {
        let (svc, _dir) = temp_service("reject-write");
        let foreign = fixture_snapshot("b@x.y", "2026-04-24T15:17:00Z");
        assert!(svc.write_backup_snapshot("a@x.y", &foreign).is_err());
        assert!(!svc.backup_exists("a@x.y"));
    }

    #[test]
    fn missing_current_backup_still_preserves_a_foreign_live_session() {
        let (svc, _dir) = temp_service("missing-current");
        svc.write_backup_snapshot("b@x.y", &fixture_snapshot("b@x.y", "2026-04-24T15:17:00Z"))
            .unwrap();
        write_json(
            &svc.auth_file_path,
            &serde_json::to_value(fixture_snapshot("c@x.y", "2026-04-24T15:17:00Z")).unwrap(),
        );

        // State still believes a@x.y is active and has no backup for it.
        let outcome = svc
            .switch_to_account_checked("b@x.y", Some("a@x.y"))
            .unwrap();
        assert_eq!(outcome, SwitchAuthOutcome::Switched);
        assert_eq!(
            svc.load_backup_access_token("c@x.y").as_deref(),
            Some("access-c@x.y"),
            "the live session must be filed under c@x.y, not a@x.y"
        );
        assert!(!svc.backup_exists("a@x.y"));
    }

    #[test]
    fn switching_from_a_mislabeled_backup_is_refused() {
        let (svc, _dir) = temp_service("mislabeled");
        // Simulate an older install that saved b@x.y's session as a@x.y.
        let wrong = fixture_snapshot("b@x.y", "2026-04-24T15:17:00Z");
        write_json(
            &svc.backup_path_for_email("a@x.y"),
            &serde_json::to_value(&wrong).unwrap(),
        );
        let err = svc.switch_to_account_checked("a@x.y", None).unwrap_err();
        assert!(
            err.contains("actually belongs to b@x.y"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn reconcile_backups_moves_a_mislabeled_file() {
        let (svc, _dir) = temp_service("reconcile");
        let wrong = fixture_snapshot("b@x.y", "2026-04-24T15:17:00Z");
        write_json(
            &svc.backup_path_for_email("a@x.y"),
            &serde_json::to_value(&wrong).unwrap(),
        );

        assert_eq!(svc.reconcile_backups(), 1);
        assert!(!svc.backup_exists("a@x.y"));
        assert_eq!(
            svc.load_backup_access_token("b@x.y").as_deref(),
            Some("access-b@x.y")
        );
        // Running it again is a no-op.
        assert_eq!(svc.reconcile_backups(), 0);
    }

    #[test]
    fn list_backup_emails_uses_the_id_token_identity() {
        let (svc, _dir) = temp_service("list-claims");
        let snap: AuthFileSnapshot = serde_json::from_value(serde_json::json!({
            "email": "stale@x.y",
            "tokens": {"id_token": jwt_for("real@x.y"), "access_token": "a", "account_id": "id"}
        }))
        .unwrap();
        write_json(
            &svc.backup_path_for_email("stale@x.y"),
            &serde_json::to_value(snap).unwrap(),
        );
        assert_eq!(svc.list_backup_emails(), vec!["real@x.y"]);
    }

    #[test]
    fn import_uses_the_identity_inside_the_credential() {
        let (svc, _dir) = temp_service("import-identity");
        let mut map = HashMapEmailSnapshots::new();
        map.insert(
            "wrong@x.y".into(),
            fixture_snapshot("real@x.y", "2026-04-24T15:17:00Z"),
        );
        assert_eq!(svc.import_backups(&map), 1);
        assert!(svc.backup_exists("real@x.y"));
        assert!(!svc.backup_exists("wrong@x.y"));
    }

    #[test]
    fn unknown_auth_fields_survive_struct_round_trip() {
        let (svc, _dir) = temp_service("passthrough");
        write_json(
            &svc.auth_file_path,
            &serde_json::json!({
            "auth_mode": "chatgpt",
            "future_top": {"nested": [1, 2]},
            "tokens": {
                "access_token": "a", "refresh_token": "r",
                "future_token_field": "keep-me"
            },
            "last_refresh": "2026-04-24T15:17:00.949966Z"}),
        );
        let snap = svc.load_snapshot().unwrap();
        let path = svc.write_backup_snapshot("a@x.y", &snap).unwrap();
        let raw: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(raw["future_top"]["nested"], serde_json::json!([1, 2]));
        assert_eq!(
            raw["tokens"]["future_token_field"],
            serde_json::json!("keep-me")
        );
        assert_eq!(raw["tokens"]["access_token"], serde_json::json!("a"));
    }

    #[test]
    fn list_backup_emails_prefers_email_then_filename() {
        let (svc, _dir) = temp_service("list");
        svc.write_backup_snapshot("a@x.y", &fixture_snapshot("a@x.y", "2026-04-24T15:17:00Z"))
            .unwrap();
        // No email field, but account_id present -> filename-derived key.
        svc.write_backup_snapshot(
            "b@x.y",
            &serde_json::from_value(serde_json::json!({
                "tokens": {"access_token": "t", "account_id": "uuid-b"}
            }))
            .unwrap(),
        )
        .unwrap();
        // Neither email nor account_id -> skipped.
        svc.write_backup_snapshot(
            "c@x.y",
            &serde_json::from_value(serde_json::json!({
                "tokens": {"access_token": "t"}
            }))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(svc.list_backup_emails(), vec!["a@x.y", "b@x.y"]);
    }

    #[tokio::test]
    async fn refresh_due_and_force() {
        let (svc, _dir) = temp_service("refresh");
        let old = fixture_snapshot("u@x.y", "2020-01-01T00:00:00Z");
        svc.write_backup_snapshot("u@x.y", &old).unwrap();
        let fake = FakeRefresher {
            tokens: RefreshedTokens {
                id_token: None,
                access_token: "fresh".into(),
                refresh_token: Some("fresh-r".into()),
            },
        };
        let now = 1_800_000_000.0;
        let out = svc
            .refresh_backup_if_due("u@x.y", &fake, false, now)
            .await
            .unwrap();
        assert_eq!(out.tokens.unwrap().access_token.as_deref(), Some("fresh"));

        // Fresh snapshot is not due again.
        let out2 = svc
            .refresh_backup_if_due("u@x.y", &fake, false, now + 60.0)
            .await
            .unwrap();
        assert_eq!(out2.tokens.unwrap().access_token.as_deref(), Some("fresh"));
        // Force refreshes regardless.
        let out3 = svc
            .refresh_backup_if_due("u@x.y", &fake, true, now + 60.0)
            .await
            .unwrap();
        assert!(out3.last_refresh.is_some());
    }

    #[tokio::test]
    async fn refresh_is_driven_by_access_token_expiry() {
        let (svc, _dir) = temp_service("freshness");
        let now = 1_800_000_000.0;
        let mut snap = fixture_snapshot("a@x.y", "2020-01-01T00:00:00Z");
        snap.tokens.as_mut().unwrap().access_token = Some(jwt_with_exp(now + 3_600.0));
        assert!(
            !snapshot_needs_refresh(&snap, now),
            "a valid access token must not be rotated just because the stamp is old"
        );

        snap.tokens.as_mut().unwrap().access_token = Some(jwt_with_exp(now + 60.0));
        assert!(
            snapshot_needs_refresh(&snap, now),
            "a token expiring inside the skew must be refreshed"
        );

        snap.tokens.as_mut().unwrap().access_token = Some("opaque".into());
        assert!(
            snapshot_needs_refresh(&snap, now),
            "opaque tokens fall back to the stored stamp"
        );

        let fresh = fixture_snapshot("a@x.y", "2020-01-01T00:00:00Z");
        svc.write_backup_snapshot("a@x.y", &fresh).unwrap();
        let fake = FakeRefresher {
            tokens: RefreshedTokens {
                id_token: None,
                access_token: jwt_with_exp(now + 86_400.0),
                refresh_token: Some("rotated".into()),
            },
        };
        svc.refresh_backup_if_due("a@x.y", &fake, false, now)
            .await
            .unwrap();
        let again = svc
            .refresh_backup_if_due("a@x.y", &fake, false, now + 60.0)
            .await
            .unwrap();
        assert_eq!(
            again.tokens.as_ref().unwrap().refresh_token.as_deref(),
            Some("rotated"),
            "the refreshed session must not be rotated a second time"
        );
    }

    #[test]
    fn switch_stamps_a_fresh_handoff_and_keeps_unknown_fields() {
        let (svc, _dir) = temp_service("handoff");
        let target: AuthFileSnapshot = serde_json::from_value(serde_json::json!({
            "auth_mode": "chatgpt",
            "email": "b@x.y",
            "tokens": {
                "id_token": jwt_for("b@x.y"),
                "access_token": "access-b",
                "refresh_token": "rt-b",
                "account_id": "uuid-b",
                "scope": "openid profile email",
                "token_type": "Bearer"
            },
            "last_refresh": "2026-01-01T00:00:00Z",
            "future_top": {"keep": true}
        }))
        .unwrap();
        svc.write_backup_snapshot("b@x.y", &target).unwrap();

        let outcome = svc.switch_to_account_checked("b@x.y", None).unwrap();
        assert_eq!(outcome, SwitchAuthOutcome::Switched);

        let live = svc.load_snapshot().unwrap();
        let stamp = parse_iso_timestamp(live.last_refresh.as_deref()).unwrap();
        assert!(
            (chrono::Utc::now().timestamp() as f64 - stamp).abs() < 60.0,
            "the live handoff must look fresh to Codex"
        );
        assert!(
            live.email.is_none(),
            "our storage annotation must not leak into auth.json"
        );
        assert_eq!(
            live.extra.get("future_top"),
            Some(&serde_json::json!({"keep": true})),
            "unknown fields written by Codex must survive the switch"
        );
        let tokens = live.tokens.unwrap();
        assert_eq!(tokens.access_token.as_deref(), Some("access-b"));
        assert_eq!(
            tokens.extra.get("scope"),
            Some(&serde_json::json!("openid profile email"))
        );
        assert_eq!(
            svc.load_backup_snapshot("b@x.y")
                .unwrap()
                .last_refresh
                .as_deref(),
            Some("2026-01-01T00:00:00Z"),
            "the stored session keeps its real refresh timestamp"
        );
    }

    #[test]
    fn switch_fills_in_the_workspace_id_from_the_token_claims() {
        let (svc, _dir) = temp_service("account-id");
        let claims = serde_json::json!({
            "email": "c@x.y",
            "https://api.openai.com/auth": {"chatgpt_account_id": "workspace-123"}
        });
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap());
        let target: AuthFileSnapshot = serde_json::from_value(serde_json::json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "id_token": format!("header.{payload}.signature"),
                "access_token": "access-c",
                "refresh_token": "rt-c"
            },
            "last_refresh": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        svc.write_backup_snapshot("c@x.y", &target).unwrap();

        let stored = svc.load_backup_snapshot("c@x.y").unwrap();
        assert_eq!(
            stored.tokens.as_ref().unwrap().account_id.as_deref(),
            Some("workspace-123"),
            "saved sessions must carry the workspace id Codex expects"
        );

        svc.switch_to_account_checked("c@x.y", None).unwrap();
        let live = svc.load_snapshot().unwrap();
        assert_eq!(
            live.tokens.unwrap().account_id.as_deref(),
            Some("workspace-123")
        );
    }

    #[test]
    fn account_identity_needs_both_workspace_and_email() {
        let mut a = fixture_snapshot("a@x.y", "2026-01-01T00:00:00Z");
        let same = a.clone();
        assert!(snapshots_same_account(&a, &same));

        let mut other_workspace = a.clone();
        other_workspace.tokens.as_mut().unwrap().account_id = Some("other".into());
        assert!(
            !snapshots_same_account(&a, &other_workspace),
            "a different workspace is a different quota"
        );

        let mut other_user = a.clone();
        other_user.email = Some("b@x.y".into());
        other_user.tokens.as_mut().unwrap().id_token = Some(jwt_for("b@x.y"));
        assert!(!snapshots_same_account(&a, &other_user));

        a.tokens.as_mut().unwrap().account_id = None;
        a.tokens.as_mut().unwrap().id_token = Some(jwt_for("a@x.y"));
        let mut raw = a.clone();
        raw.email = None;
        assert!(
            snapshots_same_account(&a, &raw),
            "same ID token is the same session"
        );
        raw.tokens.as_mut().unwrap().id_token = Some(jwt_for("b@x.y"));
        raw.email = Some("a@x.y".into());
        assert!(!snapshots_same_account(&a, &raw));
    }

    #[tokio::test]
    async fn refresh_never_files_a_different_account_under_the_old_key() {
        let (svc, _dir) = temp_service("refresh-identity");
        let old = fixture_snapshot("u@x.y", "2020-01-01T00:00:00Z");
        svc.write_backup_snapshot("u@x.y", &old).unwrap();
        let fake = FakeRefresher {
            tokens: RefreshedTokens {
                id_token: Some(jwt_for("intruder@x.y")),
                access_token: "new".into(),
                refresh_token: Some("new-r".into()),
            },
        };
        let error = svc
            .refresh_backup_if_due("u@x.y", &fake, true, 1_800_000_000.0)
            .await
            .unwrap_err();
        assert!(error.contains("Refusing to save the session for intruder@x.y"));
        assert_eq!(
            svc.load_backup_access_token("u@x.y").as_deref(),
            Some("access-u@x.y"),
            "a failed identity check must leave the original backup untouched"
        );
    }

    #[test]
    fn remove_backup_token_match() {
        let (svc, _dir) = temp_service("remove");
        let snap = fixture_snapshot("u@x.y", "2026-04-24T15:17:00Z");
        svc.write_backup_snapshot("u@x.y", &snap).unwrap();
        assert!(!svc.remove_backup_if_access_token_matches("u@x.y", "wrong"));
        assert!(svc.backup_exists("u@x.y"));
        assert!(svc.remove_backup_if_access_token_matches("u@x.y", "access-u@x.y"));
        assert!(!svc.backup_exists("u@x.y"));
        assert!(!svc.remove_backup(""));
    }

    #[test]
    fn export_import_backups() {
        let (svc, _dir) = temp_service("impexp");
        let snap = fixture_snapshot("u@x.y", "2026-04-24T15:17:00Z");
        svc.write_backup_snapshot("u@x.y", &snap).unwrap();
        let exported = svc.export_backups(&["missing@x.y".to_string()]);
        assert!(exported.contains_key("u@x.y"));

        let (svc2, _d2) = temp_service("impexp2");
        let mut with_bad = exported.clone();
        with_bad.insert(
            "bad@x.y".into(),
            serde_json::from_value(serde_json::json!({
                "tokens": {"refresh_token": "only"}
            }))
            .unwrap(),
        );
        assert_eq!(svc2.import_backups(&with_bad), 1);
        assert!(svc2.backup_exists("u@x.y"));
        assert!(!svc2.backup_exists("bad@x.y"));
    }

    #[test]
    fn login_home_lifecycle_and_path_traversal_guard() {
        let (svc, _dir) = temp_service("login");
        let home = svc.create_login_codex_home().unwrap();
        assert!(home.is_dir());
        assert_eq!(svc.active_auth_path_for_home(&home), home.join("auth.json"));
        svc.remove_login_codex_home(Some(home.as_path()));
        assert!(!home.exists());
        // Must not delete outside accounts dir.
        let outside = std::env::temp_dir().join(format!("codex-outside-{}", std::process::id()));
        fs::create_dir_all(&outside).unwrap();
        svc.remove_login_codex_home(Some(outside.as_path()));
        assert!(outside.is_dir());
        let _ = fs::remove_dir_all(&outside);
    }

    #[test]
    fn import_files_under_the_login_identity_and_rejects_mismatch() {
        let (svc, _dir) = temp_service("import");
        let snap: AuthFileSnapshot = serde_json::from_value(serde_json::json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "id_token": "id",
                "access_token": jwt_for("imported@x.y"),
                "refresh_token": "rt"
            }
        }))
        .unwrap();
        assert_eq!(svc.import_backup_snapshot(&snap).unwrap(), "imported@x.y");
        assert!(svc.backup_exists("imported@x.y"));

        let mismatch: AuthFileSnapshot = serde_json::from_value(serde_json::json!({
            "auth_mode": "chatgpt",
            "email": "claimed@x.y",
            "tokens": {
                "id_token": jwt_for("claimed@x.y"),
                "access_token": jwt_for("someone-else@x.y"),
                "refresh_token": "rt"
            }
        }))
        .unwrap();
        let err = svc.import_backup_snapshot(&mismatch).unwrap_err();
        assert!(err.contains("someone-else@x.y"), "{err}");
        assert!(!svc.backup_exists("claimed@x.y"));
    }

    #[test]
    fn import_rejects_a_file_without_chatgpt_tokens() {
        let (svc, _dir) = temp_service("import-empty");
        let snap: AuthFileSnapshot = serde_json::from_value(serde_json::json!({
            "auth_mode": "apikey",
            "email": "none@x.y",
            "tokens": {}
        }))
        .unwrap();
        assert!(svc.import_backup_snapshot(&snap).is_err());
    }

    #[test]
    fn safe_key_sanitizes() {
        let (svc, _dir) = temp_service("key");
        let p = svc.backup_path_for_email("a/b@c.com");
        assert!(p.to_string_lossy().contains("auth-a_b@c.com.json"));
    }
}
