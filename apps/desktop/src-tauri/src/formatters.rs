use chrono::{Local, NaiveDateTime, TimeZone, Utc};

use crate::models::{ResetCredit, ResetCreditsPayload};

/// "2026-04-24 15:17 (2d 3h 5m)" in local time, "-" when unknown.
/// Mirrors `format_reset_display` in codex_monitor_app/formatters.py.
pub fn format_reset_display(reset_ts: Option<f64>, now_ts: f64) -> String {
    let reset_ts = match reset_ts {
        Some(ts) if ts != 0.0 => ts,
        _ => return "-".to_string(),
    };
    let stamp = format_local(reset_ts);
    let countdown = if now_ts >= reset_ts {
        "0m".to_string()
    } else {
        countdown_parts((reset_ts - now_ts) as i64)
    };
    format!("{stamp} ({countdown})")
}

/// Remaining quota as "60%". Whole numbers print without decimals.
/// NOTE: Python prints 60.0 for float input ("60.0%"); v2 normalizes to
/// "60%". Final display rule is pinned by issue #27.
pub fn format_quota_left(used_percent: f64) -> String {
    let left = 100.0 - used_percent;
    if left.fract() == 0.0 && left.is_finite() {
        format!("{}%", left as i64)
    } else {
        format!("{left}%")
    }
}

/// Parse ISO-8601 like Python's datetime.fromisoformat (plus "Z").
/// Naive timestamps are assumed UTC, matching services.py behaviour.
pub fn parse_iso_timestamp(value: Option<&str>) -> Option<f64> {
    let raw = value?;
    if raw.is_empty() {
        return None;
    }
    let normalized = match raw.strip_suffix('Z') {
        Some(stripped) => format!("{stripped}+00:00"),
        None => raw.to_string(),
    };
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&normalized) {
        return Some(dt.timestamp_millis() as f64 / 1000.0);
    }
    for fmt in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
    ] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(&normalized, fmt) {
            let utc: chrono::DateTime<Utc> =
                Utc.from_local_datetime(&naive).single()?;
            return Some(utc.timestamp_millis() as f64 / 1000.0);
        }
    }
    None
}

fn countdown_parts(diff_secs: i64) -> String {
    let days = diff_secs / 86400;
    let hours = (diff_secs % 86400) / 3600;
    let minutes = (diff_secs % 3600) / 60;
    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{days}d"));
    }
    if hours > 0 {
        parts.push(format!("{hours}h"));
    }
    parts.push(format!("{minutes}m"));
    parts.join(" ")
}

/// "expired" / "1d 2h 3m". Mirrors `_countdown_text`.
pub fn countdown_text(seconds_remaining: i64) -> String {
    if seconds_remaining <= 0 {
        return "expired".to_string();
    }
    countdown_parts(seconds_remaining)
}

fn format_local(ts: f64) -> String {
    let secs = ts as i64;
    if let Some(dt) = Local.timestamp_opt(secs, 0).single() {
        return dt.format("%Y-%m-%d %H:%M").to_string();
    }
    Utc.timestamp_opt(secs, 0)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "-".to_string())
}

/// Local "granted at" stamp or "-". Mirrors `format_reset_granted_at`.
pub fn format_reset_granted_at(granted_at: Option<&str>) -> String {
    match parse_iso_timestamp(granted_at) {
        Some(ts) => format_local(ts),
        None => "-".to_string(),
    }
}

/// "2026-05-01 12:00 (3d 2h 10m)". Mirrors `format_reset_credit_expires`.
pub fn format_reset_credit_expires(expires_at: Option<&str>, now_ts: f64) -> String {
    let expires_ts = match parse_iso_timestamp(expires_at) {
        Some(ts) => ts,
        None => return "-".to_string(),
    };
    let stamp = format_local(expires_ts);
    let countdown = countdown_text((expires_ts - now_ts) as i64);
    format!("{stamp} ({countdown})")
}

/// "3d 2h 10m" / "expired" / "-". Mirrors `format_reset_time_remaining`.
pub fn format_reset_time_remaining(expires_at: Option<&str>, now_ts: f64) -> String {
    let expires_ts = match parse_iso_timestamp(expires_at) {
        Some(ts) => ts,
        None => return "-".to_string(),
    };
    countdown_text((expires_ts - now_ts) as i64)
}

/// Earliest-expiring available credit. Mirrors `soonest_expiring_credit`.
pub fn soonest_expiring_credit(
    payload: Option<&ResetCreditsPayload>,
) -> Option<ResetCredit> {
    let payload = payload?;
    let credits = payload.credits.as_ref()?;
    let mut available: Vec<&ResetCredit> = credits
        .iter()
        .filter(|c| c.status.as_deref() == Some("available"))
        .collect();
    if available.is_empty() {
        return None;
    }
    let mut parsed: Vec<(&ResetCredit, f64)> = available
        .iter()
        .filter_map(|c| parse_iso_timestamp(c.expires_at.as_deref()).map(|ts| (*c, ts)))
        .collect();
    if parsed.is_empty() {
        return Some((*available.remove(0)).clone());
    }
    parsed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    Some(parsed[0].0.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AuthFileSnapshot, UsageResponse};

    fn credit(status: &str, expires_at: &str) -> ResetCredit {
        ResetCredit {
            status: Some(status.to_string()),
            expires_at: Some(expires_at.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn reset_display_missing_is_dash() {
        assert_eq!(format_reset_display(None, 1_000.0), "-");
        assert_eq!(format_reset_display(Some(0.0), 1_000.0), "-");
    }

    #[test]
    fn reset_display_past_is_zero_minutes() {
        let s = format_reset_display(Some(1_000.0), 2_000.0);
        assert!(s.ends_with("(0m)"), "got {s}");
    }

    #[test]
    fn reset_display_countdown_parts() {
        // 1d 1h 1m 1s -> "1d 1h 1m" (seconds truncated, like Python int())
        let now = 1_000_000.0;
        let s = format_reset_display(Some(now + 90_061.0), now);
        assert!(s.ends_with("(1d 1h 1m)"), "got {s}");
        let s2 = format_reset_display(Some(now + 3_661.0), now);
        assert!(s2.ends_with("(1h 1m)"), "got {s2}");
    }

    #[test]
    fn quota_left_formatting() {
        assert_eq!(format_quota_left(40.0), "60%");
        assert_eq!(format_quota_left(25.5), "74.5%");
        assert_eq!(format_quota_left(0.0), "100%");
        // Carry-over old #5 (59% vs 62%): integer-normalized remaining, so
        // float formatting can never drift the on-screen number again.
        assert_eq!(format_quota_left(41.0), "59%");
    }

    #[test]
    fn iso_parse_zulu_and_naive_and_invalid() {
        let zulu = parse_iso_timestamp(Some("2026-04-24T15:17:00.949966Z"));
        assert!(zulu.is_some());
        assert!(zulu.unwrap() > 1_700_000_000.0);
        let naive = parse_iso_timestamp(Some("2026-04-24T15:17:00"));
        assert!(naive.is_some());
        assert!((zulu.unwrap() - naive.unwrap()).abs() < 1.0);
        assert_eq!(parse_iso_timestamp(None), None);
        assert_eq!(parse_iso_timestamp(Some("")), None);
        assert_eq!(parse_iso_timestamp(Some("not-a-date")), None);
    }

    #[test]
    fn countdown_text_edges() {
        assert_eq!(countdown_text(-5), "expired");
        assert_eq!(countdown_text(0), "expired");
        assert_eq!(countdown_text(90), "1m");
        assert_eq!(countdown_text(3_661), "1h 1m");
        assert_eq!(countdown_text(90_061), "1d 1h 1m");
    }

    #[test]
    fn granted_at_invalid_is_dash() {
        assert_eq!(format_reset_granted_at(None), "-");
        assert_eq!(format_reset_granted_at(Some("junk")), "-");
        assert_ne!(format_reset_granted_at(Some("2026-04-24T15:17:00Z")), "-");
    }

    #[test]
    fn expires_helpers() {
        let now = parse_iso_timestamp(Some("2026-04-24T15:17:00Z")).unwrap();
        let s = format_reset_credit_expires(Some("2026-04-25T15:17:00Z"), now);
        assert!(s.contains("23h 59m") || s.contains("1d"), "got {s}");
        assert_eq!(format_reset_time_remaining(Some("2026-04-24T15:17:00Z"), now), "expired");
        assert_eq!(format_reset_time_remaining(None, now), "-");
        assert_eq!(format_reset_credit_expires(Some("junk"), now), "-");
    }

    #[test]
    fn soonest_credit_picks_earliest_available() {
        let payload = ResetCreditsPayload {
            credits: Some(vec![
                credit("available", "2026-05-02T00:00:00Z"),
                credit("redeemed", "2026-04-20T00:00:00Z"),
                credit("available", "2026-04-26T00:00:00Z"),
            ]),
            ..Default::default()
        };
        let soonest = soonest_expiring_credit(Some(&payload)).unwrap();
        assert_eq!(soonest.expires_at.as_deref(), Some("2026-04-26T00:00:00Z"));
        assert!(soonest_expiring_credit(None).is_none());
        let empty = ResetCreditsPayload { credits: Some(vec![]), ..Default::default() };
        assert!(soonest_expiring_credit(Some(&empty)).is_none());
        let unparseable = ResetCreditsPayload {
            credits: Some(vec![credit("available", "junk")]),
            ..Default::default()
        };
        assert!(soonest_expiring_credit(Some(&unparseable)).is_some());
    }

    #[test]
    fn models_round_trip_old_fixture() {
        // Shape of a real auth.json backup (README example).
        let raw = serde_json::json!({
            "auth_mode": "chatgpt",
            "OPENAI_API_KEY": null,
            "tokens": {
                "id_token": "eyJ.x",
                "access_token": "eyJ.y",
                "refresh_token": "rt_...",
                "account_id": "uuid"
            },
            "last_refresh": "2026-04-24T15:17:00.949966Z"
        });
        let snap: AuthFileSnapshot = serde_json::from_value(raw).unwrap();
        assert_eq!(snap.tokens.unwrap().account_id.as_deref(), Some("uuid"));
        // Unknown API fields are ignored.
        let usage: UsageResponse = serde_json::from_value(serde_json::json!({
            "email": "a@b.c",
            "plan_type": "plus",
            "promo": {"x": 1},
            "rate_limit": {"primary_window": {"used_percent": 10, "reset_at": 5}}
        }))
        .unwrap();
        assert_eq!(usage.email.as_deref(), Some("a@b.c"));
    }
}
