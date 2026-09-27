use serde::{Deserialize, Serialize};

/// One rate-limit window from the wham/usage API.
/// Mirrors `RateLimitWindow` in codex_monitor_app/models.py.
/// All fields optional: the Python side drops missing/bool values.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RateLimitWindow {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_window_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_after_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_at: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ResetCredit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub granted_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redeem_started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redeemed_at: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ResetCreditsPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_earned_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credits: Option<Vec<ResetCredit>>,
}

/// Per-account stored usage. Mirrors `AccountUsage` in models.py.
/// Note: `jwt` / `auto_fetch` are never persisted (sanitized on save).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AccountUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_ts: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_window: Option<RateLimitWindow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary_window: Option<RateLimitWindow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_window: Option<RateLimitWindow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weekly_window: Option<RateLimitWindow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_fetched: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets: Option<ResetCreditsPayload>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AuthTokens {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
}

/// Snapshot of ~/.codex/auth.json. Mirrors `AuthFileSnapshot`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AuthFileSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "OPENAI_API_KEY")]
    pub openai_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<AuthTokens>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_refresh: Option<String>,
    /// Extra: backups carry the account email; active file usually doesn't.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RateLimitPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_reached: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_window: Option<RateLimitWindow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary_window: Option<RateLimitWindow>,
}

/// Raw wham/usage response. Extra API fields are ignored on deserialize.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct UsageResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit: Option<RateLimitPayload>,
}

pub type UsageMap = std::collections::HashMap<String, AccountUsage>;
