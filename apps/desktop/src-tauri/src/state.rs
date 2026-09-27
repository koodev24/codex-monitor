use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::auth::AuthFileService;
use crate::models::{RateLimitWindow, UsageMap, UsageResponse};
use crate::storage::{Meta, UsageStorage};

pub const AUTO_FETCH_OPTIONS: &[&str] =
    &["None", "15 Mins", "1 Hr", "3 Hrs", "12 Hrs", "24 Hrs"];

fn auto_fetch_seconds(label: &str) -> u64 {
    match label {
        "15 Mins" => 15 * 60,
        "1 Hr" => 3600,
        "3 Hrs" => 3 * 3600,
        "12 Hrs" => 12 * 3600,
        "24 Hrs" => 24 * 3600,
        _ => 0,
    }
}

pub fn now_secs() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn sanitize_window(value: &serde_json::Value) -> RateLimitWindow {
    let mut w = RateLimitWindow::default();
    if let Some(map) = value.as_object() {
        for (field, slot) in [
            ("used_percent", &mut w.used_percent),
            ("limit_window_seconds", &mut w.limit_window_seconds),
            ("reset_after_seconds", &mut w.reset_after_seconds),
            ("reset_at", &mut w.reset_at),
        ] {
            if let Some(n) = map.get(field).and_then(Value::as_f64) {
                *slot = Some(n);
            }
        }
    }
    w
}

fn window_seconds(w: &RateLimitWindow) -> f64 {
    w.limit_window_seconds.unwrap_or(0.0)
}

/// Classify primary/secondary into (short, weekly). Mirrors
/// MonitorStateService._classify_rate_limit_windows.
pub fn classify_windows(
    primary: RateLimitWindow,
    secondary: Option<RateLimitWindow>,
) -> (RateLimitWindow, RateLimitWindow) {
    let mut windows = vec![primary];
    if let Some(s) = secondary {
        if s.reset_at.is_some() {
            windows.push(s);
        }
    }
    if windows.is_empty() || windows.iter().all(|w| w.reset_at.is_none()) {
        return (RateLimitWindow::default(), RateLimitWindow::default());
    }
    let weekly = windows
        .iter()
        .find(|w| window_seconds(w) >= 6.0 * 24.0 * 3600.0)
        .cloned()
        .or_else(|| {
            windows.iter().max_by(|a, b| {
                window_seconds(a).partial_cmp(&window_seconds(b)).unwrap_or(std::cmp::Ordering::Equal)
            }).cloned()
        })
        .unwrap_or_default();
    let short = windows
        .into_iter()
        .find(|w| {
            w.reset_at != weekly.reset_at || w.limit_window_seconds != weekly.limit_window_seconds
        })
        .unwrap_or_default();
    (short, weekly)
}

/// In-memory monitor state. Mirrors MonitorStateService; persistence
/// goes through UsageStorage (pure v2).
pub struct MonitorState {
    pub storage: UsageStorage,
    pub auth: AuthFileService,
    pub usage: UsageMap,
    pub session_tokens: HashMap<String, String>,
    pub current_email: Option<String>,
    pub latest_jwt: Option<String>,
    pub auto_fetch: String,
    pub sort_column: Option<String>,
    pub sort_asc: bool,
    pub show_archived: bool,
    pub logs_expanded: bool,
    pub last_signature: Option<(u64, i128)>,
    pub last_access_token: Option<String>,
    pub last_refresh_marker: Option<String>,
}

impl MonitorState {
    pub fn load(mut storage: UsageStorage, auth: AuthFileService) -> Self {
        let usage = storage.load();
        let meta_snapshot = storage.meta.clone();
        let get = |k: &str| meta_snapshot.get(k).cloned();
        let current = get("current_account_email")
            .and_then(|v| v.as_str().map(String::from))
            .filter(|e| usage.contains_key(e))
            .or_else(|| {
                if usage.len() == 1 {
                    usage.keys().next().cloned()
                } else {
                    None
                }
            });
        let auto = get("auto_fetch")
            .and_then(|v| v.as_str().map(String::from))
            .filter(|v| AUTO_FETCH_OPTIONS.contains(&v.as_str()))
            .unwrap_or_else(|| "None".into());
        Self {
            storage,
            auth,
            usage,
            session_tokens: HashMap::new(),
            current_email: current,
            latest_jwt: None,
            auto_fetch: auto,
            sort_column: get("sort_column").and_then(|v| v.as_str().map(String::from)),
            sort_asc: get("sort_asc").map(|v| v != Value::Bool(false)).unwrap_or(true),
            show_archived: get("show_archived")
                .map(|v| v == Value::Bool(true))
                .unwrap_or(false),
            logs_expanded: get("logs_expanded")
                .map(|v| v == Value::Bool(true))
                .unwrap_or(false),
            last_signature: None,
            last_access_token: None,
            last_refresh_marker: None,
        }
    }

    fn persist(&mut self) {
        let _ = self.storage.save(&self.usage);
    }

    pub fn save_auto_fetch(&mut self, value: &str) -> bool {
        let Some(current) = self.current_email.clone() else { return false };
        if !self.usage.contains_key(&current) {
            return false;
        }
        let v = if AUTO_FETCH_OPTIONS.contains(&value) { value } else { "None" };
        self.auto_fetch = v.into();
        self.storage.set_meta_value("auto_fetch", Some(Value::String(v.into())));
        self.persist();
        true
    }

    pub fn save_sort(&mut self, column: Option<String>, asc: bool) {
        self.sort_column = column.clone();
        self.sort_asc = asc;
        self.storage.set_meta_value("sort_column", column.map(Value::String));
        self.storage
            .set_meta_value("sort_asc", Some(Value::Bool(asc)));
        self.persist();
    }

    pub fn save_show_archived(&mut self, show: bool) {
        self.show_archived = show;
        self.storage.set_meta_value("show_archived", Some(Value::Bool(show)));
        self.persist();
    }

    pub fn save_logs_expanded(&mut self, expanded: bool) {
        self.logs_expanded = expanded;
        self.storage
            .set_meta_value("logs_expanded", Some(Value::Bool(expanded)));
        self.persist();
    }

    pub fn set_current(&mut self, email: String, jwt: String) {
        self.current_email = Some(email.clone());
        self.session_tokens = HashMap::from([(email.clone(), jwt.clone())]);
        self.latest_jwt = Some(jwt);
        self.storage.set_meta_value(
            "current_account_email",
            Some(Value::String(email)),
        );
        self.persist();
    }

    pub fn remember_jwt(&mut self, email: &str, jwt: String) {
        if !email.is_empty() {
            self.session_tokens.insert(email.into(), jwt.clone());
        }
        if Some(email) == self.current_email.as_deref() {
            self.latest_jwt = Some(jwt);
        }
    }

    pub fn clear_session(&mut self) -> bool {
        let had = self.current_email.is_some()
            || !self.session_tokens.is_empty()
            || self.latest_jwt.is_some();
        self.session_tokens.clear();
        self.current_email = None;
        self.latest_jwt = None;
        self.storage.set_meta_value("current_account_email", None);
        self.persist();
        had
    }

    pub fn remove_account(&mut self, email: &str) -> bool {
        if !self.usage.contains_key(email) {
            return false;
        }
        self.usage.remove(email);
        self.session_tokens.remove(email);
        if self.current_email.as_deref() == Some(email) {
            self.current_email = None;
            self.latest_jwt = None;
            self.storage.set_meta_value("current_account_email", None);
        }
        self.persist();
        true
    }

    pub fn set_archived(&mut self, email: &str, archived: bool) -> bool {
        let Some(acc) = self.usage.get_mut(email) else { return false };
        acc.archived = if archived { Some(true) } else { None };
        if !archived {
            acc.archived = None;
        }
        self.persist();
        true
    }

    /// Apply a usage response. Mirrors apply_usage_response.
    /// Returns the account email, or None when the payload is unusable.
    pub fn apply_usage_response(
        &mut self,
        response: &UsageResponse,
        jwt: String,
        now: f64,
        activate: bool,
    ) -> Option<String> {
        let email = response.email.clone().filter(|e| !e.is_empty())?;
        let rate = response.rate_limit.as_ref()?;
        let primary = rate.primary_window.clone().map(|v| sanitize_window(&serde_json::to_value(v).unwrap())).unwrap_or_default();
        let secondary = rate
            .secondary_window
            .clone()
            .map(|v| sanitize_window(&serde_json::to_value(v).unwrap()));
        if primary.reset_at.is_none()
            && secondary.as_ref().and_then(|w| w.reset_at).is_none()
        {
            return None;
        }
        let (short, weekly) = classify_windows(primary.clone(), secondary.clone());
        let reset_at = weekly.reset_at.or(primary.reset_at).unwrap_or(0.0);
        let used = weekly.used_percent.or(primary.used_percent).unwrap_or(0.0);

        let entry = self.usage.entry(email.clone()).or_default();
        entry.reset_ts = Some(reset_at);
        entry.used_percent = Some(used);
        entry.last_fetched = Some(now);
        if primary.reset_at.is_some() {
            entry.primary_window = Some(primary.clone());
        }
        if let Some(s) = secondary {
            if s.reset_at.is_some() {
                entry.secondary_window = Some(s);
            }
        }
        if short.reset_at.is_some() {
            entry.short_window = Some(short);
        }
        if weekly.reset_at.is_some() {
            entry.weekly_window = Some(weekly);
        }
        if activate {
            self.set_current(email.clone(), jwt);
        } else {
            self.session_tokens.insert(email.clone(), jwt);
            self.persist();
        }
        Some(email)
    }

    pub fn apply_reset_credits(&mut self, email: &str, payload: crate::models::ResetCreditsPayload) {
        if let Some(acc) = self.usage.get_mut(email) {
            acc.resets = Some(payload);
            self.persist();
        }
    }

    pub fn due_auto_fetch_jwt(&mut self, now: f64) -> Option<String> {
        let current = self.current_email.clone()?;
        let interval = auto_fetch_seconds(&self.auto_fetch);
        if interval == 0 {
            return None;
        }
        let data = self.usage.get_mut(&current)?;
        if now - data.last_fetched.unwrap_or(0.0) < interval as f64 {
            return None;
        }
        data.last_fetched = Some(now);
        self.latest_jwt.clone().or_else(|| self.session_tokens.get(&current).cloned())
    }

    pub fn latest_jwt_for(&self, email: Option<&str>) -> Option<String> {
        if let Some(jwt) = self.latest_jwt.clone() {
            return Some(jwt);
        }
        let key = email
            .map(String::from)
            .or_else(|| self.current_email.clone())?;
        self.session_tokens.get(&key).cloned()
    }

    pub fn meta(&self) -> &Meta {
        &self.storage.meta
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(email: &str, primary_secs: f64, secondary_secs: Option<f64>) -> UsageResponse {
        let win = |secs: f64, used: f64| {
            serde_json::from_value(serde_json::json!({
                "used_percent": used,
                "limit_window_seconds": secs,
                "reset_after_seconds": secs,
                "reset_at": 1_778_000_000.0
            }))
            .unwrap()
        };
        UsageResponse {
            email: Some(email.into()),
            rate_limit: Some(crate::models::RateLimitPayload {
                primary_window: Some(win(primary_secs, 25.0)),
                secondary_window: secondary_secs.map(|s| win(s, 40.0)),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn blank_state() -> MonitorState {
        let dir = std::env::temp_dir().join(format!("codex-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let storage = UsageStorage::new(dir.join("u.v2.json"), dir.join("u.v2.meta.json"));
        let auth = AuthFileService::new(dir.join("auth.json"), dir.join("accounts"));
        MonitorState::load(storage, auth)
    }

    #[test]
    fn classifies_short_and_weekly() {
        let mut st = blank_state();
        let email = st.apply_usage_response(&response("u@x.y", 16_000.0, Some(604_800.0)), "jwt".into(), 100.0, true);
        assert_eq!(email.as_deref(), Some("u@x.y"));
        let acc = &st.usage["u@x.y"];
        assert_eq!(acc.short_window.as_ref().unwrap().limit_window_seconds, Some(16_000.0));
        assert_eq!(acc.weekly_window.as_ref().unwrap().limit_window_seconds, Some(604_800.0));
        assert_eq!(st.current_email.as_deref(), Some("u@x.y"));
    }

    #[test]
    fn rejects_useless_payloads() {
        let mut st = blank_state();
        assert_eq!(st.apply_usage_response(&UsageResponse::default(), "j".into(), 0.0, true), None);
        let no_windows = UsageResponse {
            email: Some("u@x.y".into()),
            rate_limit: Some(crate::models::RateLimitPayload::default()),
            ..Default::default()
        };
        assert_eq!(st.apply_usage_response(&no_windows, "j".into(), 0.0, true), None);
    }

    #[test]
    fn logout_clears_everything() {
        let mut st = blank_state();
        st.apply_usage_response(&response("u@x.y", 604_800.0, None), "j".into(), 0.0, true);
        assert!(st.clear_session());
        assert_eq!(st.current_email, None);
        assert_eq!(st.latest_jwt_for(None), None);
        assert_eq!(st.storage.get_meta_value("current_account_email"), None);
        assert!(!st.clear_session());
    }

    #[test]
    fn archive_remove_and_auto_fetch_due() {
        let mut st = blank_state();
        st.apply_usage_response(&response("u@x.y", 604_800.0, None), "j".into(), 0.0, true);
        assert!(st.set_archived("u@x.y", true));
        assert_eq!(st.usage["u@x.y"].archived, Some(true));
        assert!(st.set_archived("u@x.y", false));
        assert_eq!(st.usage["u@x.y"].archived, None);
        assert!(!st.set_archived("nope@x.y", true));

        st.auto_fetch = "1 Hr".into();
        assert_eq!(st.due_auto_fetch_jwt(100.0), None);
        assert_eq!(st.due_auto_fetch_jwt(5000.0).as_deref(), Some("j"));
        assert!(st.remove_account("u@x.y"));
        assert!(!st.remove_account("u@x.y"));
    }
}
