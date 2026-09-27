use std::fs;
use std::path::{Path, PathBuf};

use crate::api::{ApiError, AuthRefreshClient, RefreshedTokens};
use crate::formatters::parse_iso_timestamp;
use crate::models::AuthFileSnapshot;

pub const AUTH_REFRESH_INTERVAL_SECS: f64 = 8.0 * 24.0 * 3600.0;

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|p| !p.as_os_str().is_empty())
}

pub fn default_auth_file_path() -> PathBuf {
    home_dir().map(|h| h.join(".codex/auth.json")).unwrap_or_else(|| PathBuf::from("auth.json"))
}

pub fn default_accounts_dir() -> PathBuf {
    let (dir, _) = crate::storage::default_paths();
    dir.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from(".")).join("accounts")
}

fn safe_key(email: &str) -> String {
    let mut s: String = email
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._@+-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    s = s.trim_matches(|c| c == '.' || c == '_' || c == '-').to_string();
    if s.is_empty() {
        s = "unknown".into();
    }
    s
}

fn copy_atomic(source: &Path, target: &Path) -> std::io::Result<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = target.with_extension(format!(
        "{}.tmp",
        std::process::id()
    ));
    fs::copy(source, &tmp)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
    }
    fs::rename(&tmp, target)?;
    Ok(())
}

fn current_refresh_timestamp() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

/// Abstraction over token refresh so tests can fake it.
/// Mirrors AuthRefreshClient.refresh_tokens.
pub trait TokenRefresher {
    fn refresh_tokens(&self, refresh_token: &str) -> Result<RefreshedTokens, ApiError>;
}

impl TokenRefresher for AuthRefreshClient {
    fn refresh_tokens(&self, refresh_token: &str) -> Result<RefreshedTokens, ApiError> {
        self.refresh_tokens(refresh_token)
    }
}

fn snapshot_needs_refresh(snapshot: &AuthFileSnapshot, now: f64) -> bool {
    let tokens = match snapshot.tokens.as_ref() {
        Some(t) => t,
        None => return false,
    };
    match tokens.refresh_token.as_deref() {
        Some(rt) if !rt.is_empty() => {}
        _ => return false,
    }
    match parse_iso_timestamp(snapshot.last_refresh.as_deref()) {
        None => true,
        Some(ts) => now - ts >= AUTH_REFRESH_INTERVAL_SECS,
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
        Self { auth_file_path, accounts_dir }
    }

    pub fn with_defaults() -> Self {
        Self::new(default_auth_file_path(), default_accounts_dir())
    }

    pub fn auth_file_exists(&self) -> bool {
        self.auth_file_path.is_file()
    }

    pub fn load_snapshot_from_path(&self, path: &Path) -> Result<AuthFileSnapshot, String> {
        let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let v: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| e.to_string())?;
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

    pub fn backup_path_for_email(&self, email: &str) -> PathBuf {
        self.accounts_dir.join(format!("auth-{}.json", safe_key(email)))
    }

    pub fn backup_exists(&self, email: &str) -> bool {
        self.backup_path_for_email(email).is_file()
    }

    pub fn list_backup_emails(&self) -> Vec<String> {
        let mut out = Vec::new();
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
                    if let Some(email) = s.email.filter(|e| !e.is_empty()) {
                        out.push(email);
                        continue;
                    }
                    let account_id =
                        s.tokens.and_then(|t| t.account_id).filter(|a| !a.is_empty());
                    if account_id.is_some() {
                        out.push(name["auth-".len()..name.len() - ".json".len()].to_string());
                    }
                }
                None => continue,
            }
        }
        out.sort();
        out.dedup();
        out
    }

    pub fn backup_auth_from_path(&self, email: &str, source: &Path) -> Result<PathBuf, String> {
        let target = self.backup_path_for_email(email);
        copy_atomic(source, &target).map_err(|e| e.to_string())?;
        Ok(target)
    }

    pub fn backup_current_auth(&self, email: &str) -> Result<PathBuf, String> {
        if !self.auth_file_exists() {
            return Err(format!("{} not found", self.auth_file_path.display()));
        }
        let path = self.auth_file_path.clone();
        self.backup_auth_from_path(email, &path)
    }

    pub fn remove_backup(&self, email: &str) -> bool {
        if email.is_empty() {
            return false;
        }
        fs::remove_file(self.backup_path_for_email(email)).is_ok()
    }

    pub fn remove_backup_if_access_token_matches(
        &self,
        email: &str,
        access_token: &str,
    ) -> bool {
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
        let target = self.backup_path_for_email(email);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(snapshot).map_err(|e| e.to_string())?;
        let tmp = target.with_extension(format!("{}.tmp", std::process::id()));
        fs::write(&tmp, text).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
        }
        fs::rename(&tmp, &target).map_err(|e| e.to_string())?;
        Ok(target)
    }

    pub fn export_backups(&self, emails: &[String]) -> HashMapEmailSnapshots {
        let mut wanted: Vec<String> = emails.to_vec();
        wanted.extend(self.list_backup_emails());
        wanted.sort();
        wanted.dedup();
        let mut out = HashMapEmailSnapshots::new();
        for email in wanted {
            if !self.backup_exists(&email) {
                continue;
            }
            if let Ok(snap) = self.load_backup_snapshot(&email) {
                out.insert(email, snap);
            }
        }
        out
    }

    pub fn import_backups(
        &self,
        backups: &HashMapEmailSnapshots,
    ) -> usize {
        let mut count = 0;
        for (email, snapshot) in backups {
            if email.is_empty() {
                continue;
            }
            let has_token = snapshot
                .tokens
                .as_ref()
                .and_then(|t| t.access_token.as_ref())
                .map(|t| !t.is_empty())
                .unwrap_or(false);
            if !has_token {
                continue;
            }
            if self.write_backup_snapshot(email, snapshot).is_ok() {
                count += 1;
            }
        }
        count
    }

    fn refresh_snapshot_tokens<R: TokenRefresher>(
        &self,
        snapshot: &AuthFileSnapshot,
        refresher: &R,
    ) -> Result<AuthFileSnapshot, String> {
        let tokens = snapshot.tokens.as_ref().ok_or_else(|| "auth backup has no tokens object".to_string())?;
        let rt = tokens
            .refresh_token
            .as_deref()
            .filter(|r| !r.is_empty())
            .ok_or_else(|| "auth backup has no refresh_token".to_string())?;
        let refreshed = refresher.refresh_tokens(rt).map_err(|e| e.to_string())?;
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

    pub fn refresh_backup_if_due<R: TokenRefresher>(
        &self,
        email: &str,
        refresher: &R,
        force: bool,
        now: f64,
    ) -> Result<AuthFileSnapshot, String> {
        let snapshot = self.load_backup_snapshot(email)?;
        if !force && !snapshot_needs_refresh(&snapshot, now) {
            return Ok(snapshot);
        }
        let next = self.refresh_snapshot_tokens(&snapshot, refresher)?;
        self.write_backup_snapshot(email, &next)?;
        Ok(next)
    }

    pub fn switch_to_account_backup(
        &self,
        target_email: &str,
        current_email: Option<&str>,
    ) -> Result<Option<PathBuf>, String> {
        if !self.backup_exists(target_email) {
            return Err(format!("no backup for {target_email}"));
        }
        let current_backup = if let Some(current) = current_email.filter(|e| !e.is_empty()) {
            if self.auth_file_exists() {
                Some(self.backup_current_auth(current)?)
            } else {
                None
            }
        } else {
            None
        };
        let source = self.backup_path_for_email(target_email);
        let target = self.auth_file_path.clone();
        copy_atomic(&source, &target).map_err(|e| e.to_string())?;
        Ok(current_backup)
    }

    pub fn create_login_codex_home(&self) -> Result<PathBuf, String> {
        fs::create_dir_all(&self.accounts_dir).map_err(|e| e.to_string())?;
        let dir = self.accounts_dir.join(format!(
            "login-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
        }
        Ok(dir)
    }

    pub fn remove_login_codex_home(&self, home: Option<&Path>) {
        let home = match home {
            Some(h) => h,
            None => return,
        };
        let Ok(canonical_accounts) = self.accounts_dir.canonicalize() else { return };
        let Ok(canonical_target) = home.canonicalize() else { return };
        if !canonical_target.starts_with(&canonical_accounts) {
            return;
        }
        if home.file_name().and_then(|n| n.to_str()).map(|n| n.starts_with("login-")).unwrap_or(false) {
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
        fn refresh_tokens(&self, _rt: &str) -> Result<RefreshedTokens, ApiError> {
            Ok(self.tokens.clone())
        }
    }

    fn temp_service(tag: &str) -> (AuthFileService, PathBuf) {
        let dir = std::env::temp_dir().join(format!("codex-auth-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let svc = AuthFileService::new(
            dir.join(".codex/auth.json"),
            dir.join("accounts"),
        );
        (svc, dir)
    }

    fn fixture_snapshot(email: &str, last_refresh: &str) -> AuthFileSnapshot {
        serde_json::from_value(serde_json::json!({
            "auth_mode": "chatgpt",
            "email": email,
            "tokens": {
                "id_token": "id", "access_token": format!("access-{email}"),
                "refresh_token": "rt", "account_id": "uuid"
            },
            "last_refresh": last_refresh
        }))
        .unwrap()
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
        write_json(&svc.auth_file_path, &serde_json::json!({
            "tokens": {"access_token": "active-a", "refresh_token": "r"},
            "last_refresh": "2026-04-24T15:17:00.949966Z"}));
        svc.backup_current_auth("a@x.y").unwrap();
        assert!(svc.backup_exists("a@x.y"));

        write_json(&svc.auth_file_path, &serde_json::json!({
            "tokens": {"access_token": "active-b", "refresh_token": "r"},
            "last_refresh": "2026-04-24T15:17:00.949966Z"}));
        svc.backup_current_auth("b@x.y").unwrap();

        svc.switch_to_account_backup("a@x.y", Some("b@x.y")).unwrap();
        assert_eq!(svc.load_access_token().as_deref(), Some("active-a"));
        // Current account was backed up before switching.
        assert_eq!(svc.load_backup_access_token("b@x.y").as_deref(), Some("active-b"));
        assert!(svc.switch_to_account_backup("nope@x.y", None).is_err());
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

    #[test]
    fn refresh_due_and_force() {
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
        let out = svc.refresh_backup_if_due("u@x.y", &fake, false, now).unwrap();
        assert_eq!(out.tokens.unwrap().access_token.as_deref(), Some("fresh"));

        // Fresh snapshot is not due again.
        let out2 = svc.refresh_backup_if_due("u@x.y", &fake, false, now + 60.0).unwrap();
        assert_eq!(
            out2.tokens.unwrap().access_token.as_deref(),
            Some("fresh")
        );
        // Force refreshes regardless.
        let out3 = svc.refresh_backup_if_due("u@x.y", &fake, true, now + 60.0).unwrap();
        assert!(out3.last_refresh.is_some());
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
        with_bad.insert("bad@x.y".into(), serde_json::from_value(serde_json::json!({
            "tokens": {"refresh_token": "only"}
        })).unwrap());
        assert_eq!(svc2.import_backups(&with_bad), 1);
        assert!(svc2.backup_exists("u@x.y"));
        assert!(!svc2.backup_exists("bad@x.y"));
    }

    #[test]
    fn login_home_lifecycle_and_path_traversal_guard() {
        let (svc, _dir) = temp_service("login");
        let home = svc.create_login_codex_home().unwrap();
        assert!(home.is_dir());
        assert_eq!(
            svc.active_auth_path_for_home(&home),
            home.join("auth.json")
        );
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
    fn safe_key_sanitizes() {
        let (svc, _dir) = temp_service("key");
        let p = svc.backup_path_for_email("a/b@c.com");
        assert!(p.to_string_lossy().contains("auth-a_b@c.com.json"));
    }
}
