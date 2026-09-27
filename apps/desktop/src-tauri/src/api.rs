use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::models::{ResetCreditsPayload, UsageResponse};

pub const USAGE_API_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
pub const RESET_CREDITS_API_URL: &str =
    "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits";
pub const AUTH_REFRESH_URL: &str = "https://auth.openai.com/oauth/token";
pub const AUTH_REFRESH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

fn browser_headers() -> HashMap<String, String> {
    [
        ("Accept", "application/json, text/plain, */*"),
        ("Accept-Language", "en-US,en;q=0.9"),
        ("Origin", "https://chatgpt.com"),
        ("Referer", "https://chatgpt.com/"),
        ("Sec-CH-UA", "\"Google Chrome\";v=\"135\", \"Chromium\";v=\"135\", \"Not.A/Brand\";v=\"8\""),
        ("Sec-CH-UA-Mobile", "?0"),
        ("Sec-CH-UA-Platform", "\"macOS\""),
        ("Sec-Fetch-Dest", "empty"),
        ("Sec-Fetch-Mode", "cors"),
        ("Sec-Fetch-Site", "same-origin"),
        (
            "User-Agent",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.0.0 Safari/537.36",
        ),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

/// Typed API failure. Display strings are human-readable on purpose:
/// the UI shows them directly, never a traceback (see carry-over #29).
#[derive(Debug, Clone, PartialEq)]
pub enum ApiError {
    Network(String),
    Unauthorized,
    Status(u16, String),
    Parse(String),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network(msg) => write!(f, "Network error: {msg}. Check your connection and try again."),
            Self::Unauthorized => write!(f, "Session expired (401). Sign in again."),
            Self::Status(code, _) => write!(f, "Request failed (HTTP {code}). Try again shortly."),
            Self::Parse(msg) => write!(f, "Unexpected response: {msg}."),
        }
    }
}

impl From<reqwest::Error> for ApiError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_timeout() {
            return Self::Network("timed out after 15s".into());
        }
        if e.is_connect() {
            return Self::Network("could not reach the server".into());
        }
        if let Some(status) = e.status() {
            if status.as_u16() == 401 {
                return Self::Unauthorized;
            }
            return Self::Status(status.as_u16(), String::new());
        }
        Self::Network(e.to_string())
    }
}

fn client() -> Result<reqwest::blocking::Client, ApiError> {
    reqwest::blocking::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| ApiError::Network(e.to_string()))
}

/// Mirrors UsageApiClient in codex_monitor_app/api.py.
pub struct UsageApiClient {
    pub usage_url: String,
    pub reset_credits_url: String,
}

impl UsageApiClient {
    pub fn new() -> Self {
        Self {
            usage_url: USAGE_API_URL.into(),
            reset_credits_url: RESET_CREDITS_API_URL.into(),
        }
    }

    pub fn fetch_usage(&self, jwt: &str) -> Result<UsageResponse, ApiError> {
        let mut req = client()?.get(&self.usage_url);
        for (k, v) in browser_headers() {
            req = req.header(k, v);
        }
        let resp = req.bearer_auth(jwt).send()?;
        let status = resp.status();
        if status.as_u16() == 401 {
            return Err(ApiError::Unauthorized);
        }
        if !status.is_success() {
            return Err(ApiError::Status(status.as_u16(), String::new()));
        }
        resp.json::<UsageResponse>().map_err(|e| ApiError::Parse(e.to_string()))
    }

    pub fn fetch_reset_credits(
        &self,
        jwt: &str,
        account_id: &str,
    ) -> Result<ResetCreditsPayload, ApiError> {
        let mut req = client()?.get(&self.reset_credits_url);
        for (k, v) in browser_headers() {
            req = req.header(k, v);
        }
        let resp = req.bearer_auth(jwt).header("ChatGPT-Account-ID", account_id).send()?;
        let status = resp.status();
        if status.as_u16() == 401 {
            return Err(ApiError::Unauthorized);
        }
        if !status.is_success() {
            return Err(ApiError::Status(status.as_u16(), String::new()));
        }
        resp.json::<ResetCreditsPayload>().map_err(|e| ApiError::Parse(e.to_string()))
    }
}

impl Default for UsageApiClient {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RefreshedTokens {
    pub id_token: Option<String>,
    pub access_token: String,
    pub refresh_token: Option<String>,
}

/// Mirrors AuthRefreshClient in codex_monitor_app/api.py.
pub struct AuthRefreshClient {
    pub refresh_url: String,
    pub client_id: String,
}

impl AuthRefreshClient {
    pub fn new() -> Self {
        Self { refresh_url: AUTH_REFRESH_URL.into(), client_id: AUTH_REFRESH_CLIENT_ID.into() }
    }

    pub fn refresh_tokens(&self, refresh_token: &str) -> Result<RefreshedTokens, ApiError> {
        let body = serde_json::json!({
            "client_id": self.client_id,
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
        });
        let mut req = client()?.post(&self.refresh_url);
        for (k, v) in browser_headers() {
            req = req.header(k, v);
        }
        let resp = req.header("Accept", "application/json").json(&body).send()?;
        let status = resp.status();
        if status.as_u16() == 401 {
            return Err(ApiError::Unauthorized);
        }
        if !status.is_success() {
            return Err(ApiError::Status(status.as_u16(), String::new()));
        }
        let v: serde_json::Value =
            resp.json().map_err(|e| ApiError::Parse(e.to_string()))?;
        let access = v
            .get("access_token")
            .and_then(|t| t.as_str())
            .filter(|t| !t.is_empty())
            .ok_or_else(|| ApiError::Parse("token refresh response has no access_token".into()))?;
        Ok(RefreshedTokens {
            id_token: v.get("id_token").and_then(|t| t.as_str()).map(String::from),
            access_token: access.to_string(),
            refresh_token: v.get("refresh_token").and_then(|t| t.as_str()).map(String::from),
        })
    }
}

impl Default for AuthRefreshClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    struct Mock {
        addr: std::net::SocketAddr,
        seen_auth: mpsc::Receiver<String>,
        _handle: thread::JoinHandle<()>,
    }

    fn serve_once(status: u16, body: &str) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        let body = body.to_string();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = vec![0u8; 8192];
            let n = stream.read(&mut buf).unwrap();
            let head = String::from_utf8_lossy(&buf[..n]).to_string();
            let auth = head
                .lines()
                .find(|l| l.to_lowercase().starts_with("authorization:"))
                .unwrap_or("")
                .to_string();
            let _ = tx.send(auth);
            let reason = match status {
                200 => "OK",
                401 => "Unauthorized",
                500 => "Internal Server Error",
                _ => "OK",
            };
            let resp = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(resp.as_bytes()).unwrap();
        });
        Mock { addr, seen_auth: rx, _handle: handle }
    }

    fn client_for(mock: &Mock) -> UsageApiClient {
        let base = format!("http://{}", mock.addr);
        UsageApiClient { usage_url: base.clone(), reset_credits_url: base }
    }

    #[test]
    fn fetch_usage_ok_and_bearer_header() {
        let mock = serve_once(200, r#"{"email":"a@b.c","rate_limit":{"primary_window":{"used_percent":10,"reset_at":5}}}"#);
        let c = client_for(&mock);
        let r = c.fetch_usage("jwt-123").unwrap();
        assert_eq!(r.email.as_deref(), Some("a@b.c"));
        assert_eq!(
            mock.seen_auth.recv().unwrap().to_lowercase(),
            "authorization: bearer jwt-123"
        );
    }

    #[test]
    fn fetch_usage_401_maps_to_unauthorized() {
        let mock = serve_once(401, "{}");
        let c = client_for(&mock);
        assert_eq!(c.fetch_usage("bad"), Err(ApiError::Unauthorized));
    }

    #[test]
    fn fetch_usage_500_and_garbage() {
        let mock = serve_once(500, "{}");
        assert!(matches!(client_for(&mock).fetch_usage("x"), Err(ApiError::Status(500, _))));
        let mock2 = serve_once(200, "not-json{{{");
        assert!(matches!(client_for(&mock2).fetch_usage("x"), Err(ApiError::Parse(_))));
    }

    #[test]
    fn reset_credits_ok() {
        let mock = serve_once(200, r#"{"available_count":1,"credits":[{"status":"available","expires_at":"2026-05-01T00:00:00Z"}]}"#);
        let c = client_for(&mock);
        let r = c.fetch_reset_credits("jwt", "acct").unwrap();
        assert_eq!(r.available_count, Some(1));
    }

    #[test]
    fn refresh_ok_and_missing_access_token() {
        let mock = serve_once(200, r#"{"access_token":"new-a","refresh_token":"new-r"}"#);
        let c = AuthRefreshClient {
            refresh_url: format!("http://{}", mock.addr),
            client_id: "test".into(),
        };
        let t = c.refresh_tokens("old-r").unwrap();
        assert_eq!(t.access_token, "new-a");
        assert_eq!(t.refresh_token.as_deref(), Some("new-r"));

        let mock2 = serve_once(200, r#"{"nope":true}"#);
        let c2 = AuthRefreshClient {
            refresh_url: format!("http://{}", mock2.addr),
            client_id: "test".into(),
        };
        assert!(matches!(c2.refresh_tokens("x"), Err(ApiError::Parse(_))));
    }

    #[test]
    fn error_messages_are_human_readable() {
        assert!(ApiError::Unauthorized.to_string().contains("401"));
        assert!(!ApiError::Network("x".into()).to_string().contains("reqwest"));
        assert!(!format!("{:?}", ApiError::Status(500, String::new())).contains("reqwest"));
    }
}
