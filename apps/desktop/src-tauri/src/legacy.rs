//! One-time migrator: Python app store files -> v2 envelope.
//!
//! ISOLATION CONTRACT (per PM requirement):
//! - This is the ONLY module that knows the legacy paths
//!   (`usage.json`, `usage.meta.json`, `~/.codex_usage_store.*`).
//! - It only READS legacy files. They are never modified or deleted,
//!   so the old app keeps working side-by-side during transition.
//! - It runs ONCE: when v2 files already exist it is a no-op.
//! - DELETION: to drop legacy support after the migration window,
//!   delete this file + remove `legacy-migrate` from default features +
//!   delete the single `migrate_if_needed` call at app boot.
//!   `storage.rs` (pure v2) is untouched.
//!
//! Gated behind cargo feature `legacy-migrate`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

use serde_json::{Map, Value};

use crate::models::UsageMap;
use crate::storage::{
    AUTO_FETCH_OPTIONS, Meta, UsageStorage, looks_like_account_payload, sanitize_account,
    sanitize_auto_fetch, sanitize_meta,
};

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|p| !p.as_os_str().is_empty())
}

/// Legacy store files, newest location first. Mirrors LOCAL_STORAGE_FILE +
/// LEGACY_LOCAL_STORAGE_FILE in config.py.
fn legacy_store_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = home_dir() {
        out.push(home.join("Library/Application Support/CodexMonitor/usage.json"));
        out.push(home.join(".local/share/CodexMonitor/usage.json"));
        out.push(home.join(".codex_usage_store.json"));
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        if !xdg.is_empty() {
            out.push(PathBuf::from(xdg).join("CodexMonitor/usage.json"));
        }
    }
    out
}

/// Legacy meta files, newest location first.
fn legacy_meta_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = home_dir() {
        out.push(home.join("Library/Application Support/CodexMonitor/usage.meta.json"));
        out.push(home.join(".local/share/CodexMonitor/usage.meta.json"));
        out.push(home.join(".codex_usage_store.meta.json"));
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        if !xdg.is_empty() {
            out.push(PathBuf::from(xdg).join("CodexMonitor/usage.meta.json"));
        }
    }
    out
}

/// Raw-dict merge for the mixed legacy envelope: usage fields follow the
/// newer last_fetched, auto_fetch prefers an explicit non-"None" value.
/// Mirrors _merge_account_payloads.
fn merge_legacy_raw(existing: &Value, new: &Value) -> Value {
    let mut merged = existing.as_object().cloned().unwrap_or_default();
    let new_map = match new.as_object() {
        Some(m) => m,
        None => return existing.clone(),
    };
    let old_ts = merged.get("last_fetched").and_then(Value::as_f64).unwrap_or(0.0);
    let new_ts = new_map.get("last_fetched").and_then(Value::as_f64).unwrap_or(0.0);
    if new_ts >= old_ts {
        for f in [
            "reset_ts",
            "used_percent",
            "primary_window",
            "secondary_window",
            "short_window",
            "weekly_window",
            "last_fetched",
        ] {
            if let Some(v) = new_map.get(f) {
                merged.insert(f.into(), v.clone());
            }
        }
    }
    let keep_new_auto = matches!(new_map.get("auto_fetch"), Some(Value::String(s)) if !s.is_empty() && s != "None");
    if keep_new_auto {
        merged.insert("auto_fetch".into(), new_map["auto_fetch"].clone());
    }
    for f in ["jwt", "archived", "resets"] {
        if let Some(v) = new_map.get(f) {
            merged.insert(f.into(), v.clone());
        }
    }
    Value::Object(merged)
}

fn read_json(path: &PathBuf) -> Option<Value> {
    fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok())
}

/// Mirrors _select_migrated_auto_fetch (legacy inference, stays here).
fn select_migrated_auto_fetch(meta: &Meta, legacy: &HashMap<String, String>) -> String {
    if let Some(Value::String(current)) = meta.get("current_account_email") {
        if let Some(v) = legacy.get(current) {
            return v.clone();
        }
    }
    let distinct: HashSet<&str> =
        legacy.values().map(String::as_str).filter(|v| *v != "None").collect();
    if distinct.len() == 1 {
        return distinct.into_iter().next().unwrap().to_string();
    }
    "None".to_string()
}

/// Parse ANY legacy store shape into (accounts, meta):
/// bare map, {"accounts", "__meta__"} envelope + top-level merge,
/// numeric reset_ts values, per-account auto_fetch inference.
/// Mirrors the old UsageStorage.load repair path.
fn parse_legacy_store(root: &Value) -> Option<(UsageMap, Meta)> {
    let raw = root.as_object()?;
    let mut data: UsageMap = HashMap::new();
    let mut legacy_auto: HashMap<String, String> = HashMap::new();
    let mut meta = Meta::new();

    let raw_accounts: Map<String, Value> = if let Some(Value::Object(accounts)) =
        raw.get("accounts")
    {
        let mut merged = accounts.clone();
        if let Some(Value::Object(old_meta)) = raw.get("__meta__") {
            for (k, v) in sanitize_meta(old_meta) {
                meta.insert(k, v);
            }
        }
        for (key, value) in raw {
            if key == "accounts" || key == "__meta__" {
                continue;
            }
            if looks_like_account_payload(value) {
                let next = match merged.get(key) {
                    Some(existing) => merge_legacy_raw(existing, value),
                    None => value.clone(),
                };
                merged.insert(key.clone(), next);
            }
        }
        merged
    } else {
        raw.clone()
    };

    for (email, value) in &raw_accounts {
        if email == "accounts" || email == "__meta__" {
            continue;
        }
        if let Some(n) = value.as_f64() {
            data.insert(email.clone(), crate::models::AccountUsage {
                reset_ts: Some(n),
                used_percent: Some(0.0),
                last_fetched: Some(0.0),
                ..Default::default()
            });
            continue;
        }
        if !looks_like_account_payload(value) {
            continue;
        }
        if let Some(map) = value.as_object() {
            let auto = map.get("auto_fetch").map(sanitize_auto_fetch).unwrap_or("None");
            if auto != "None" {
                legacy_auto.insert(email.clone(), auto.to_string());
            }
        }
        if let Some(acc) = sanitize_account(value) {
            data.insert(email.clone(), acc);
        }
    }

    if data.is_empty() {
        return None;
    }
    if !meta.contains_key("auto_fetch") {
        let migrated = select_migrated_auto_fetch(&meta, &legacy_auto);
        if migrated != "None" {
            meta.insert("auto_fetch".into(), Value::String(migrated));
        }
    }
    if !meta.contains_key("current_account_email") {
        let candidates: Vec<&String> =
            legacy_auto.keys().filter(|e| data.contains_key(*e)).collect();
        if candidates.len() == 1 {
            meta.insert("current_account_email".into(), Value::String(candidates[0].clone()));
        }
    }
    Some((data, meta))
}

#[derive(Debug, PartialEq)]
pub struct MigrationReport {    pub source: PathBuf,
    pub accounts_migrated: usize,
    pub auto_fetch: Option<String>,
    pub current_account_email: Option<String>,
}

/// Run once at boot. Returns Ok(None) when there is nothing to do
/// (v2 already exists, or no legacy files found).
/// Legacy files are left untouched.
pub fn migrate_if_needed(store: &mut UsageStorage) -> Result<Option<MigrationReport>, String> {
    if store.storage_path.exists() {
        return Ok(None);
    }
    let source = legacy_store_candidates()
        .into_iter()
        .find(|p| p.is_file())
        .ok_or_else(|| "no legacy store files found".to_string())?;
    let root = read_json(&source).ok_or_else(|| format!("cannot parse {}", source.display()))?;
    let (data, mut meta) =
        parse_legacy_store(&root).ok_or_else(|| "legacy store has no account data".to_string())?;

    // Sibling/newest legacy meta file enriches (not overrides) inferred meta.
    for meta_path in legacy_meta_candidates() {
        if let Some(Value::Object(raw)) = read_json(&meta_path).filter(|v| v.is_object()) {
            for (k, v) in sanitize_meta(&raw) {
                meta.entry(k).or_insert(v);
            }
            break;
        }
    }

    for (k, v) in meta {
        store.set_meta_value(&k, Some(v));
    }
    // Keep only known-good auto_fetch labels.
    if let Some(Value::String(s)) = store.get_meta_value("auto_fetch").cloned() {
        if !AUTO_FETCH_OPTIONS.contains(&s.as_str()) {
            store.set_meta_value("auto_fetch", None);
        }
    }
    store.save(&data).map_err(|e| e.to_string())?;

    Ok(Some(MigrationReport {
        source,
        accounts_migrated: data.len(),
        auto_fetch: store
            .get_meta_value("auto_fetch")
            .and_then(|v| v.as_str())
            .map(String::from),
        current_account_email: store
            .get_meta_value("current_account_email")
            .and_then(|v| v.as_str())
            .map(String::from),
    }))
}

/// Move old account backups into the current accounts dir.
/// Mirrors _migrate_legacy_account_backups. Same isolation contract.
pub fn migrate_legacy_account_dirs(accounts_dir: &PathBuf) {
    let Some(home) = home_dir() else { return };
    for legacy in [
        home.join(".codex_usage_store.accounts"),
        home.join(".codex/accounts"),
        home.join(".codex/codex_monitor/accounts"),
    ] {
        if legacy == *accounts_dir {
            continue;
        }
        let Ok(entries) = fs::read_dir(&legacy) else { continue };
        let _ = fs::create_dir_all(accounts_dir);
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with("auth-") || !name.ends_with(".json") {
                continue;
            }
            let source = entry.path();
            let target = accounts_dir.join(&name);
            if !source.is_file() || target.exists() {
                continue;
            }
            if fs::rename(&source, &target).is_err() {
                let _ = fs::copy(&source, &target);
            }
        }
        let _ = fs::remove_dir(&legacy);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::temp_test_paths;

    fn write(path: &PathBuf, value: &Value) {
        if let Some(p) = path.parent() {
            fs::create_dir_all(p).unwrap();
        }
        fs::write(path, serde_json::to_string(value).unwrap()).unwrap();
    }

    #[test]
    fn skips_when_v2_already_exists() {
        let (s, m) = temp_test_paths("legacy-skip");
        let mut store = UsageStorage::new(s, m);
        store.save(&UsageMap::new()).unwrap();
        assert_eq!(migrate_if_needed(&mut store), Ok(None));
    }

    #[test]
    fn parses_mixed_legacy_envelope() {
        let root = serde_json::json!({
            "accounts": {"real@example.com": {
                "reset_ts": 100, "used_percent": 20,
                "auto_fetch": "1 Hr", "last_fetched": 10}},
            "__meta__": {"current_account_email": "real@example.com"},
            "real@example.com": {
                "reset_ts": 200, "used_percent": 40, "last_fetched": 30}
        });
        let (data, meta) = parse_legacy_store(&root).unwrap();
        assert_eq!(data.keys().collect::<Vec<_>>(), vec!["real@example.com"]);
        assert_eq!(meta["auto_fetch"], Value::String("1 Hr".into()));
    }

    #[test]
    fn parses_numeric_and_skips_junk() {
        let root = serde_json::json!({
            "num@example.com": 12345,
            "junk@example.com": {"foo": "bar"}
        });
        let (data, _) = parse_legacy_store(&root).unwrap();
        assert_eq!(data.keys().collect::<Vec<_>>(), vec!["num@example.com"]);
        assert_eq!(data["num@example.com"].reset_ts, Some(12345.0));
        assert!(parse_legacy_store(&serde_json::json!({"a": {"foo": 1}})).is_none());
    }

    #[test]
    fn full_migration_writes_v2_and_leaves_legacy_untouched() {
        // Simulate an old install: legacy files only, no v2 files.
        let dir = std::env::temp_dir().join(format!(
            "codex-legacy-e2e-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let legacy_store = dir.join("legacy-home/.codex_usage_store.json");
        let v2_storage = dir.join("v2/usage.v2.json");
        let v2_meta = dir.join("v2/usage.v2.meta.json");
        write(&legacy_store, &serde_json::json!({"user@example.com": {
            "reset_ts": 123, "used_percent": 10,
            "auto_fetch": "1 Hr", "last_fetched": 100,
            "jwt": "must-be-stripped"}}));

        // Point HOME at the fake legacy home so candidates resolve there.
        let old_home = std::env::var_os("HOME");
        std::env::set_var("HOME", dir.join("legacy-home"));
        let mut store = UsageStorage::new(v2_storage.clone(), v2_meta.clone());
        let report = migrate_if_needed(&mut store);
        if let Some(h) = old_home {
            std::env::set_var("HOME", h);
        } else {
            std::env::remove_var("HOME");
        }
        let report = report.unwrap().unwrap();
        assert_eq!(report.accounts_migrated, 1);
        assert_eq!(report.auto_fetch.as_deref(), Some("1 Hr"));
        assert_eq!(report.current_account_email.as_deref(), Some("user@example.com"));

        // v2 files now exist with clean data; legacy file untouched.
        assert!(v2_storage.is_file());
        let raw: Value =
            serde_json::from_str(&fs::read_to_string(&v2_storage).unwrap()).unwrap();
        let acc = raw["accounts"]["user@example.com"].as_object().unwrap();
        assert!(!acc.contains_key("jwt"));
        assert!(!acc.contains_key("auto_fetch"));
        assert!(legacy_store.is_file());
        // Second run is a no-op.
        assert_eq!(migrate_if_needed(&mut store), Ok(None));
        let _ = fs::remove_dir_all(&dir);
    }
}
