use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::models::{AccountUsage, RateLimitWindow, ResetCreditsPayload, UsageMap};

/// Valid auto-fetch labels. Mirrors AUTO_FETCH_OPTIONS in config.py.
pub const AUTO_FETCH_OPTIONS: &[&str] = &["None", "15 Mins", "1 Hr", "3 Hrs", "12 Hrs", "24 Hrs"];

/// v2 envelope version. Bumped only on breaking on-disk changes.
pub const STORAGE_SCHEMA_VERSION: u32 = 2;

/// v2 file names. Deliberately DIFFERENT from the Python app's
/// `usage.json` / `usage.meta.json` so old and new installs can never
/// clobber each other (ID-level separation). Old files are only ever read
/// by the one-time migrator in `legacy.rs` — never written.
pub const STORAGE_FILE_NAME: &str = "usage.v2.json";
pub const META_FILE_NAME: &str = "usage.v2.meta.json";

pub type Meta = Map<String, Value>;

fn default_app_data_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = dirs_home() {
            return home.join("Library/Application Support/CodexMonitor");
        }
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("CodexMonitor");
        }
    }
    if let Some(home) = dirs_home() {
        return home.join(".local/share/CodexMonitor");
    }
    PathBuf::from(".")
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

/// (storage_path, meta_path) used by the shipped app.
/// Mirrors LOCAL_STORAGE_FILE / LOCAL_STORAGE_META_FILE in config.py,
//  namespaced to v2 file names (see STORAGE_FILE_NAME).
pub fn default_paths() -> (PathBuf, PathBuf) {
    let dir = default_app_data_dir();
    (dir.join(STORAGE_FILE_NAME), dir.join(META_FILE_NAME))
}

fn is_number(v: &Value) -> bool {
    matches!(v, Value::Number(_))
}

const ACCOUNT_FIELDS: &[&str] = &[
    "reset_ts",
    "used_percent",
    "primary_window",
    "secondary_window",
    "short_window",
    "weekly_window",
    "auto_fetch",
    "last_fetched",
    "jwt",
    "archived",
    "resets",
];

pub(crate) fn looks_like_account_payload(value: &Value) -> bool {
    match value {
        Value::Object(map) => ACCOUNT_FIELDS.iter().any(|f| map.contains_key(*f)),
        _ => false,
    }
}

fn sanitize_window(value: &Value) -> Option<RateLimitWindow> {
    let map = value.as_object()?;
    let mut w = RateLimitWindow::default();
    let mut any = false;
    for (field, slot) in [
        ("used_percent", &mut w.used_percent),
        ("limit_window_seconds", &mut w.limit_window_seconds),
        ("reset_after_seconds", &mut w.reset_after_seconds),
        ("reset_at", &mut w.reset_at),
    ] {
        if let Some(v) = map.get(field).filter(|v| is_number(v)).and_then(Value::as_f64) {
            *slot = Some(v);
            any = true;
        }
    }
    any.then_some(w)
}

/// Strip secrets/legacy keys, keep typed data. Mirrors _sanitize_account_data.
/// NOTE: unlike Python (which keeps any "resets" dict), an unparseable
/// resets payload is dropped instead of persisted.
pub(crate) fn sanitize_account(value: &Value) -> Option<AccountUsage> {
    let map = value.as_object()?;
    let mut acc = AccountUsage::default();
    for (field, slot) in [
        ("reset_ts", &mut acc.reset_ts),
        ("used_percent", &mut acc.used_percent),
        ("last_fetched", &mut acc.last_fetched),
    ] {
        if let Some(v) = map.get(field).filter(|v| is_number(v)).and_then(Value::as_f64) {
            *slot = Some(v);
        }
    }
    for (field, slot) in [
        ("primary_window", &mut acc.primary_window),
        ("secondary_window", &mut acc.secondary_window),
        ("short_window", &mut acc.short_window),
        ("weekly_window", &mut acc.weekly_window),
    ] {
        if let Some(raw) = map.get(field) {
            if let Some(w) = sanitize_window(raw) {
                *slot = Some(w);
            }
        }
    }
    if map.get("archived") == Some(&Value::Bool(true)) {
        acc.archived = Some(true);
    }
    if let Some(resets) = map.get("resets").filter(|v| v.is_object()) {
        if let Ok(payload) = serde_json::from_value::<ResetCreditsPayload>(resets.clone()) {
            acc.resets = Some(payload);
        }
    }
    Some(acc)
}

pub(crate) fn sanitize_auto_fetch(value: &Value) -> &str {
    match value.as_str() {
        Some(s) if AUTO_FETCH_OPTIONS.contains(&s) => s,
        _ => "None",
    }
}

/// Mirrors UsageStorage._sanitize_meta.
pub(crate) fn sanitize_meta(meta: &Map<String, Value>) -> Meta {
    let mut clean = Meta::new();
    if let Some(Value::String(email)) = meta.get("current_account_email") {
        if !email.is_empty() {
            clean.insert("current_account_email".into(), Value::String(email.clone()));
        }
    }
    if let Some(v) = meta.get("auto_fetch") {
        let s = sanitize_auto_fetch(v);
        if AUTO_FETCH_OPTIONS.contains(&s) {
            clean.insert("auto_fetch".into(), Value::String(s.into()));
        }
    }
    if let Some(v @ Value::String(_)) | Some(v @ Value::Null) = meta.get("sort_column") {
        clean.insert("sort_column".into(), v.clone());
    }
    for key in ["sort_asc", "show_archived", "show_5h_columns", "logs_expanded"] {
        if let Some(v) = meta.get(key) {
            clean.insert(key.into(), Value::Bool(bool_of(v)));
        }
    }
    clean
}

fn bool_of(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Null => false,
        Value::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

fn merge_window(
    existing: Option<RateLimitWindow>,
    new: &Option<RateLimitWindow>,
) -> Option<RateLimitWindow> {
    match (existing, new) {
        (_, Some(n)) => Some(n.clone()),
        (e, None) => e,
    }
}

/// Merge two account payloads, preferring the newer last_fetched for usage
/// fields. Mirrors _merge_account_payloads (jwt/auto_fetch never stored).
fn merge_account(existing: Option<&AccountUsage>, new_data: &AccountUsage) -> AccountUsage {
    let mut merged = existing.cloned().unwrap_or_default();
    let new_ts = new_data.last_fetched.unwrap_or(0.0);
    let old_ts = merged.last_fetched.unwrap_or(0.0);
    if new_ts >= old_ts {
        if new_data.reset_ts.is_some() {
            merged.reset_ts = new_data.reset_ts;
        }
        if new_data.used_percent.is_some() {
            merged.used_percent = new_data.used_percent;
        }
        merged.primary_window = merge_window(merged.primary_window, &new_data.primary_window);
        merged.secondary_window =
            merge_window(merged.secondary_window, &new_data.secondary_window);
        merged.short_window = merge_window(merged.short_window, &new_data.short_window);
        merged.weekly_window = merge_window(merged.weekly_window, &new_data.weekly_window);
        if new_data.last_fetched.is_some() {
            merged.last_fetched = new_data.last_fetched;
        }
    }
    if new_data.archived.is_some() {
        merged.archived = new_data.archived;
    }
    if new_data.resets.is_some() {
        merged.resets = new_data.resets.clone();
    }
    merged
}

fn read_json(path: &Path) -> Option<Value> {
    fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok())
}
fn write_json(path: &Path, value: &Value) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    fs::write(path, serde_json::to_string(value).unwrap())
}

#[cfg(test)]
pub(crate) fn temp_test_paths(tag: &str) -> (PathBuf, PathBuf) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("codex-v2-{tag}-{}-{n}", std::process::id()));
    (dir.join(STORAGE_FILE_NAME), dir.join(META_FILE_NAME))
}

/// Pure-v2 store. Mirrors UsageStorage in codex_monitor_app/storage.py
/// MINUS all legacy-shape repair (that lives in `legacy.rs`).
/// Invariant: this module never reads or writes the Python app's
/// `usage.json` / `~/.codex_usage_store.json` paths.
pub struct UsageStorage {
    pub storage_path: PathBuf,
    pub meta_path: PathBuf,
    pub meta: Meta,
}

impl UsageStorage {
    pub fn new(storage_path: PathBuf, meta_path: PathBuf) -> Self {
        Self { storage_path, meta_path, meta: Meta::new() }
    }

    pub fn with_defaults() -> Self {
        let (s, m) = default_paths();
        Self::new(s, m)
    }

    fn load_meta_file(&self) -> Meta {
        read_json(&self.meta_path)
            .and_then(|v| v.as_object().cloned())
            .map(|m| sanitize_meta(&m))
            .unwrap_or_default()
    }

    fn save_meta_file(&self) -> std::io::Result<()> {
        write_json(&self.meta_path, &Value::Object(sanitize_meta(&self.meta)))
    }

    /// Load the v2 envelope. Unknown shapes yield empty (never crash,
    /// never repair-in-place — repair is the migrator's job).
    pub fn load(&mut self) -> UsageMap {
        let mut data: UsageMap = HashMap::new();
        self.meta = self.load_meta_file();
        if let Some(Value::Object(root)) =
            read_json(&self.storage_path).filter(|v| v.is_object())
        {
            if let Some(Value::Object(accounts)) = root.get("accounts") {
                for (email, value) in accounts {
                    if !looks_like_account_payload(value) {
                        continue;
                    }
                    if let Some(acc) = sanitize_account(value) {
                        data.insert(email.clone(), acc);
                    }
                }
            }
        }
        data
    }

    /// Save the v2 envelope `{schema_version, accounts}` + meta file.
    pub fn save(&mut self, data: &UsageMap) -> std::io::Result<()> {
        let mut accounts = Map::new();
        for (email, acc) in data {
            let clean = sanitize_account(&serde_json::to_value(acc).unwrap()).unwrap_or_default();
            accounts.insert(email.clone(), serde_json::to_value(clean).unwrap());
        }
        write_json(
            &self.storage_path,
            &serde_json::json!({
                "schema_version": STORAGE_SCHEMA_VERSION,
                "accounts": accounts,
            }),
        )?;
        self.save_meta_file()
    }

    pub fn get_meta_value(&self, key: &str) -> Option<&Value> {
        self.meta.get(key)
    }

    /// None / "" removes the key. Mirrors set_meta_value.
    pub fn set_meta_value(&mut self, key: &str, value: Option<Value>) {
        match value {
            None => {
                self.meta.remove(key);
            }
            Some(Value::String(s)) if s.is_empty() => {
                self.meta.remove(key);
            }
            Some(v) => {
                self.meta.insert(key.to_string(), v);
            }
        }
    }

    /// User-facing export (schema_version 2 envelope + config).
    pub fn export_data(&self, data: &UsageMap) -> Value {
        let mut accounts = Map::new();
        for (email, acc) in data {
            let clean = sanitize_account(&serde_json::to_value(acc).unwrap()).unwrap_or_default();
            accounts.insert(email.clone(), serde_json::to_value(clean).unwrap());
        }
        serde_json::json!({
            "schema_version": STORAGE_SCHEMA_VERSION,
            "accounts": accounts,
            "config": sanitize_meta(&self.meta),
        })
    }

    /// User-facing import: merge accounts + config, error on bad payloads.
    /// Per-account legacy keys (jwt/auto_fetch) are stripped by sanitize.
    pub fn import_data(&mut self, payload: &Value) -> Result<UsageMap, String> {
        let root = payload
            .as_object()
            .ok_or_else(|| "Import file root must be a JSON object.".to_string())?;

        let raw_accounts: HashMap<String, &Value> = match root.get("accounts") {
            Some(Value::Object(accounts)) => {
                accounts.iter().map(|(k, v)| (k.clone(), v)).collect()
            }
            _ => root
                .iter()
                .filter(|(k, _)| *k != "__meta__" && *k != "config" && *k != "schema_version")
                .map(|(k, v)| (k.clone(), v))
                .collect(),
        };

        let mut imported: UsageMap = HashMap::new();
        for (email, value) in &raw_accounts {
            if email.is_empty() || !looks_like_account_payload(value) {
                continue;
            }
            if let Some(acc) = sanitize_account(value) {
                imported.insert(email.clone(), acc);
            }
        }
        if imported.is_empty() {
            return Err("Import file does not contain account data.".to_string());
        }

        let mut merged = self.load();
        for (email, acc) in &imported {
            let next = merge_account(merged.get(email), acc);
            merged.insert(email.clone(), next);
        }

        let raw_config = root
            .get("config")
            .or_else(|| root.get("__meta__"))
            .and_then(|v| v.as_object());
        if let Some(config) = raw_config {
            for (k, v) in sanitize_meta(config) {
                self.meta.insert(k, v);
            }
        }
        if let Some(Value::String(current)) = self.meta.get("current_account_email").cloned() {
            if !merged.contains_key(&current) {
                self.meta.remove("current_account_email");
            }
        }

        self.save(&merged).map_err(|e| e.to_string())?;
        Ok(merged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    pub(crate) fn temp_paths(tag: &str) -> (PathBuf, PathBuf) {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("codex-v2-{tag}-{}-{n}", std::process::id()));
        (dir.join(STORAGE_FILE_NAME), dir.join(META_FILE_NAME))
    }

    fn sample_account(used: f64, fetched: f64) -> AccountUsage {
        AccountUsage {
            reset_ts: Some(200.0),
            used_percent: Some(used),
            last_fetched: Some(fetched),
            ..Default::default()
        }
    }

    #[test]
    fn save_load_round_trip_with_meta() {
        let (s, m) = temp_paths("roundtrip");
        let mut st = UsageStorage::new(s.clone(), m.clone());
        let mut data = UsageMap::new();
        data.insert("a@example.com".into(), sample_account(50.0, 20.0));
        st.set_meta_value("current_account_email", Some(Value::String("a@example.com".into())));
        st.set_meta_value("auto_fetch", Some(Value::String("12 Hrs".into())));
        st.save(&data).unwrap();

        let mut st2 = UsageStorage::new(s, m);
        let loaded = st2.load();
        assert_eq!(loaded["a@example.com"].used_percent, Some(50.0));
        assert_eq!(
            st2.get_meta_value("current_account_email"),
            Some(&Value::String("a@example.com".into()))
        );
        assert_eq!(st2.get_meta_value("auto_fetch"), Some(&Value::String("12 Hrs".into())));
        // Envelope carries the version stamp.
        let raw: Value =
            serde_json::from_str(&fs::read_to_string(&st2.storage_path).unwrap()).unwrap();
        assert_eq!(raw["schema_version"], Value::from(STORAGE_SCHEMA_VERSION));
    }

    #[test]
    fn empty_or_corrupt_files_yield_empty_without_crash() {
        let (s, m) = temp_paths("empty");
        let mut st = UsageStorage::new(s, m);
        assert!(st.load().is_empty());
        let (s2, m2) = temp_paths("corrupt");
        fs::create_dir_all(s2.parent().unwrap()).unwrap();
        fs::write(&s2, "{not json").unwrap();
        fs::write(&m2, "[1,2").unwrap();
        let mut st2 = UsageStorage::new(s2, m2);
        assert!(st2.load().is_empty());
    }

    #[test]
    fn junk_entries_skipped_secrets_stripped() {
        let (s, m) = temp_paths("junk");
        let mut st = UsageStorage::new(s.clone(), m);
        let dirty = serde_json::json!({
            "schema_version": 2,
            "accounts": {
                "ok@example.com": {"reset_ts": 1, "used_percent": 10, "last_fetched": 5},
                "junk@example.com": {"foo": "bar"},
                "leak@example.com": {"reset_ts": 1, "jwt": "secret", "auto_fetch": "1 Hr"}
            }
        });
        fs::create_dir_all(s.parent().unwrap()).unwrap();
        fs::write(&s, serde_json::to_string(&dirty).unwrap()).unwrap();
        let data = st.load();
        assert!(data.contains_key("ok@example.com"));
        assert!(!data.contains_key("junk@example.com"));
        assert!(data.contains_key("leak@example.com"));
        st.save(&data).unwrap();
        let raw: Value = serde_json::from_str(&fs::read_to_string(&s).unwrap()).unwrap();
        let leak = raw["accounts"]["leak@example.com"].as_object().unwrap();
        assert!(!leak.contains_key("jwt"));
        assert!(!leak.contains_key("auto_fetch"));
    }

    #[test]
    fn export_import_merge_newer_wins() {
        let (s, m) = temp_paths("merge");
        let mut st = UsageStorage::new(s, m);
        let mut data = UsageMap::new();
        data.insert("a@example.com".into(), sample_account(10.0, 100.0));
        st.save(&data).unwrap();

        let old = serde_json::json!({"accounts": {"a@example.com": {
            "reset_ts": 2, "used_percent": 99, "last_fetched": 50}}});
        let merged = st.import_data(&old).unwrap();
        assert_eq!(merged["a@example.com"].used_percent, Some(10.0));

        let fresh = serde_json::json!({"accounts": {
            "a@example.com": {"reset_ts": 3, "used_percent": 70, "last_fetched": 200},
            "b@example.com": {"reset_ts": 3, "used_percent": 5, "last_fetched": 200}}});
        let merged2 = st.import_data(&fresh).unwrap();
        assert_eq!(merged2["a@example.com"].used_percent, Some(70.0));
        assert!(merged2.contains_key("b@example.com"));

        assert!(st.import_data(&serde_json::json!([1, 2])).is_err());
        assert!(st.import_data(&serde_json::json!({"config": {}})).is_err());
    }

    #[test]
    fn meta_set_remove_and_invalid_auto_fetch_dropped() {
        let (s, m) = temp_paths("meta");
        let mut st = UsageStorage::new(s, m);
        st.set_meta_value("k", Some(Value::String("v".into())));
        assert_eq!(st.get_meta_value("k"), Some(&Value::String("v".into())));
        st.set_meta_value("k", Some(Value::String("".into())));
        assert_eq!(st.get_meta_value("k"), None);
        st.set_meta_value("auto_fetch", Some(Value::String("never".into())));
        st.save(&UsageMap::new()).unwrap();
        let mut st2 = UsageStorage::new(st.storage_path.clone(), st.meta_path.clone());
        st2.load();
        // Invalid labels normalize to "None" on save, same as Python.
        assert_eq!(st2.get_meta_value("auto_fetch"), Some(&Value::String("None".into())));
    }
}
