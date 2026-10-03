//! Native ChatGPT OAuth login (streamlined desktop-auth flow, no CLI).
//!
//! Mirrors the upstream `openai/codex` browser login (`codex-rs/login`):
//! PKCE S256, loopback callback on 127.0.0.1:1455, form-encoded code
//! exchange at `{issuer}/oauth/token`, tokens persisted to `auth.json`.
//! The sign-in URL is wrapped exactly like the official Codex Desktop app:
//! `https://chatgpt.com/codex/desktop-auth?authorize_url=…&codex_streamlined_login=true`.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use base64::Engine;
use rand::RngCore;
use serde::Deserialize;
use sha2::{Digest, Sha256};

pub const OAUTH_ISSUER: &str = "https://auth.openai.com";
pub const CALLBACK_PORT: u16 = 1455;
pub const FALLBACK_PORT: u16 = 1457;
const SCOPE: &str = "openid profile email offline_access api.connectors.read api.connectors.invoke";
const DESKTOP_AUTH_BASE: &str = "https://chatgpt.com/codex/desktop-auth";

pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

pub fn generate_pkce() -> Pkce {
    let mut bytes = [0u8; 64];
    rand::rng().fill_bytes(&mut bytes);
    let verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    let digest = Sha256::digest(verifier.as_bytes());
    Pkce {
        verifier,
        challenge: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest),
    }
}

pub fn generate_state() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub fn redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}/auth/callback")
}

fn append_param(out: &mut String, first: &mut bool, key: &str, value: &str) {
    use std::fmt::Write as _;
    let _ = write!(
        out,
        "{}{}={}",
        if *first { '?' } else { '&' },
        key,
        url::form_urlencoded::byte_serialize(value.as_bytes()).collect::<String>()
    );
    *first = false;
}

/// Inner OAuth authorize URL, same shape the CLI builds.
pub fn authorize_url(client_id: &str, port: u16, pkce: &Pkce, state: &str) -> String {
    let mut out = format!("{OAUTH_ISSUER}/oauth/authorize");
    let mut first = true;
    append_param(&mut out, &mut first, "response_type", "code");
    append_param(&mut out, &mut first, "client_id", client_id);
    append_param(&mut out, &mut first, "redirect_uri", &redirect_uri(port));
    append_param(&mut out, &mut first, "code_challenge", &pkce.challenge);
    append_param(&mut out, &mut first, "code_challenge_method", "S256");
    append_param(&mut out, &mut first, "state", state);
    append_param(&mut out, &mut first, "scope", SCOPE);
    append_param(&mut out, &mut first, "id_token_add_organizations", "true");
    append_param(&mut out, &mut first, "codex_cli_simplified_flow", "true");
    append_param(&mut out, &mut first, "originator", "codex_cli_rs");
    out
}

/// Official streamlined wrapper the Codex Desktop app shows users.
pub fn desktop_auth_url(authorize_url: &str) -> String {
    let mut out = DESKTOP_AUTH_BASE.to_string();
    let mut first = true;
    append_param(&mut out, &mut first, "authorize_url", authorize_url);
    append_param(&mut out, &mut first, "codex_streamlined_login", "true");
    out
}

/// Validate the callback query and return the authorization code.
pub fn parse_callback_query(query: &str, expected_state: &str) -> Result<String, String> {
    let mut code: Option<String> = None;
    let mut state: Option<String> = None;
    let mut error: Option<String> = None;
    for (k, v) in url::form_urlencoded::parse(query.as_bytes()) {
        match k.as_ref() {
            "code" => code = Some(v.into_owned()),
            "state" => state = Some(v.into_owned()),
            "error" => error = Some(v.into_owned()),
            _ => {}
        }
    }
    if state.as_deref() != Some(expected_state) {
        return Err("Sign-in response did not match this login attempt.".into());
    }
    if let Some(e) = error.filter(|e| !e.is_empty()) {
        return Err(format!("Sign-in failed: {e}"));
    }
    code.filter(|c| !c.is_empty())
        .ok_or_else(|| "Sign-in completed without an authorization code.".into())
}

#[derive(Debug, Deserialize)]
pub struct ExchangedTokens {
    pub id_token: String,
    pub access_token: String,
    pub refresh_token: String,
}

/// Form-encoded authorization-code exchange, per upstream `OAuthClient`.
pub async fn exchange_code(
    client: &reqwest::Client,
    issuer: &str,
    client_id: &str,
    code: &str,
    redirect_uri: &str,
    verifier: &str,
) -> Result<ExchangedTokens, String> {
    let endpoint = format!("{}/oauth/token", issuer.trim_end_matches('/'));
    let body = [
        ("grant_type", "authorization_code"),
        ("client_id", client_id),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("code_verifier", verifier),
    ];
    let resp = client
        .post(&endpoint)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(
            url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(body)
                .finish(),
        )
        .send()
        .await
        .map_err(|e| format!("Token exchange failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Token exchange failed (HTTP {}).", resp.status()));
    }
    resp.json::<ExchangedTokens>()
        .await
        .map_err(|e| format!("Token exchange returned an unreadable response: {e}"))
}

/// `chatgpt_account_id` claim from the ID token payload (signature is
/// verified server-side on use; here it only files the account id).
pub fn account_id_from_id_token(id_token: &str) -> Option<String> {
    let payload = id_token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.get("chatgpt_account_id")?.as_str().map(String::from)
}

const SUCCESS_HTML: &str = "<html><body style=\"font-family:sans-serif;padding:2em\">\
    <h2>Signed in to Codex</h2>\
    <p>You can close this window and return to Codex Account Monitor.</p>\
    </body></html>";

/// One-shot loopback callback server. The background thread serves a single
/// `/auth/callback` request (or error), records the outcome, then exits.
/// `shutdown` cancels the wait from the login watchdog/cancel path.
pub struct CallbackOutcome {
    pub shutdown: Arc<AtomicBool>,
    pub code: Arc<Mutex<Option<Result<String, String>>>>,
}

pub fn start_callback_server(port: u16, state: String) -> Result<(u16, CallbackOutcome), String> {
    let listener = TcpListener::bind(format!("127.0.0.1:{port}"))
        .map_err(|e| format!("Login callback port {port} is unavailable: {e}"))?;
    let actual = listener
        .local_addr()
        .map(|a| a.port())
        .map_err(|e| format!("Could not read login callback port: {e}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("Could not start login listener: {e}"))?;
    let outcome = CallbackOutcome {
        shutdown: Arc::new(AtomicBool::new(false)),
        code: Arc::new(Mutex::new(None)),
    };
    let shutdown = outcome.shutdown.clone();
    let slot = outcome.code.clone();
    thread::spawn(move || {
        for _ in 0..6000 {
            if shutdown.load(Ordering::SeqCst) {
                return;
            }
            match listener.accept() {
                Ok((stream, _)) => {
                    let result = handle_callback_connection(stream, &state);
                    *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
                    return;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(100));
                }
                Err(_) => {
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
    });
    Ok((actual, outcome))
}

fn handle_callback_connection(
    mut stream: std::net::TcpStream,
    expected_state: &str,
) -> Result<String, String> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut raw = Vec::new();
    loop {
        let mut chunk = [0u8; 1024];
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                raw.extend_from_slice(&chunk[..n]);
                if raw.windows(4).any(|w| w == b"\r\n\r\n") || raw.len() >= 8192 {
                    break;
                }
            }
        }
    }
    let head = String::from_utf8_lossy(&raw);
    let request_line = head.lines().next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let target = parts.nth(1).unwrap_or("/");
    let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
    let result = if target.starts_with("/auth/callback") {
        parse_callback_query(query, expected_state)
    } else {
        Err("Unknown login request.".into())
    };
    let (status, page) = match &result {
        Ok(_) => ("200 OK", SUCCESS_HTML.to_string()),
        Err(message) => (
            "400 Bad Request",
            format!("<html><body><h2>Sign-in failed</h2><p>{message}</p></body></html>"),
        ),
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
        page.len()
    );
    let _ = stream.flush();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_verifier() {
        let pkce = generate_pkce();
        assert!((43..=128).contains(&pkce.verifier.len()));
        let digest = Sha256::digest(pkce.verifier.as_bytes());
        let expect = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
        assert_eq!(pkce.challenge, expect);
        assert!(!generate_state().is_empty());
    }

    #[test]
    fn desktop_wrapper_round_trips() {
        let pkce = generate_pkce();
        let inner = authorize_url("test-client", 1455, &pkce, "state-123");
        assert!(inner.contains("code_challenge_method=S256"));
        assert!(inner.contains("127.0.0.1%3A1455") || inner.contains("127.0.0.1:1455"));
        let wrapped = desktop_auth_url(&inner);
        assert!(wrapped.starts_with("https://chatgpt.com/codex/desktop-auth?"));
        assert!(wrapped.contains("codex_streamlined_login=true"));
        let parsed = url::Url::parse(&wrapped).unwrap();
        let params: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(params.get("authorize_url").map(String::as_str), Some(inner.as_str()));
    }

    #[test]
    fn callback_validation() {
        assert_eq!(
            parse_callback_query("code=abc&state=s1", "s1").as_deref(),
            Ok("abc")
        );
        assert!(parse_callback_query("code=abc&state=other", "s1").is_err());
        assert!(parse_callback_query("state=s1", "s1").is_err());
        assert!(parse_callback_query("error=access_denied&state=s1", "s1").is_err());
    }

    #[test]
    fn account_id_from_fake_jwt() {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"chatgpt_account_id":"acct-1"}"#);
        let token = format!("header.{payload}.sig");
        assert_eq!(account_id_from_id_token(&token).as_deref(), Some("acct-1"));
        assert_eq!(account_id_from_id_token("not-a-jwt"), None);
    }

    #[tokio::test]
    async fn exchange_against_mock_server() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let body =
            r#"{"id_token":"id","access_token":"access","refresh_token":"refresh"}"#.to_string();
        let handle = tokio::task::spawn_blocking(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = stream.read(&mut buf);
            let head = String::from_utf8_lossy(&buf).to_string();
            assert!(head.contains("grant_type=authorization_code"));
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(resp.as_bytes()).unwrap();
            head.contains("code_verifier=verifier-123").to_string()
        });
        let client = reqwest::Client::builder().build().unwrap();
        let tokens = exchange_code(
            &client,
            &format!("http://{addr}"),
            "test-client",
            "code-123",
            "http://127.0.0.1:1455/auth/callback",
            "verifier-123",
        )
        .await
        .unwrap();
        assert_eq!(tokens.access_token, "access");
        assert_eq!(tokens.refresh_token, "refresh");
        assert_eq!(handle.await.unwrap(), "true");
    }
}
