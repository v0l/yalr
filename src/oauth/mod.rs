//! OAuth support for subscription-based providers (Claude Pro/Max, ChatGPT Plus/Pro).
//!
//! These providers authenticate with short-lived OAuth access tokens (plus a
//! refresh token) issued against the user's consumer subscription, instead of a
//! static API key.
//!
//! ## Flow
//! Both Anthropic and OpenAI use well-known *public* OAuth clients whose allowed
//! redirect URIs are fixed (Anthropic redirects to a console page that displays
//! the code; OpenAI redirects to `http://localhost:1455`). A self-hosted router
//! cannot register its own redirect URI with those clients, so the practical web
//! flow is:
//!
//! 1. `start()` — generate PKCE + state, return the authorize URL.
//! 2. User opens the URL, authorizes, and copies back the resulting code
//!    (or the full redirect URL, from which we parse `code`/`state`).
//! 3. `exchange()` — swap the code for tokens using the stored PKCE verifier.
//! 4. Tokens are persisted and refreshed automatically by the provider.

pub mod anthropic;
pub mod openai;
pub mod session;

pub use session::OAuthSession;

use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::db::ProviderType;

/// Which OAuth subscription flow to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthKind {
    /// Claude Pro/Max via claude.ai OAuth.
    Anthropic,
    /// ChatGPT Plus/Pro via Codex OAuth.
    OpenAi,
}

impl OAuthKind {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "anthropic" | "anthropic-oauth" | "claude" => Some(OAuthKind::Anthropic),
            "openai" | "openai-oauth" | "chatgpt" | "codex" => Some(OAuthKind::OpenAi),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            OAuthKind::Anthropic => "anthropic-oauth",
            OAuthKind::OpenAi => "openai-oauth",
        }
    }

    pub fn provider_type(&self) -> ProviderType {
        match self {
            OAuthKind::Anthropic => ProviderType::AnthropicOauth,
            OAuthKind::OpenAi => ProviderType::OpenAiOauth,
        }
    }

    /// Default upstream base URL to store for a freshly connected provider.
    pub fn default_base_url(&self) -> &'static str {
        match self {
            OAuthKind::Anthropic => "https://api.anthropic.com",
            OAuthKind::OpenAi => "https://chatgpt.com/backend-api/codex",
        }
    }

    pub fn try_from_provider_type(pt: ProviderType) -> Option<Self> {
        match pt {
            ProviderType::AnthropicOauth => Some(OAuthKind::Anthropic),
            ProviderType::OpenAiOauth => Some(OAuthKind::OpenAi),
            _ => None,
        }
    }
}

/// PKCE code verifier + challenge pair.
#[derive(Debug, Clone)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

/// Base64url (no padding) encode.
pub fn base64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Generate a PKCE verifier (32 random bytes) and S256 challenge.
pub fn generate_pkce() -> Pkce {
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    let verifier = base64url(&buf);
    let digest = Sha256::digest(verifier.as_bytes());
    let challenge = base64url(&digest);
    Pkce { verifier, challenge }
}

/// Generate a random anti-CSRF state value (hex).
pub fn generate_state() -> String {
    let mut buf = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut buf);
    buf.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Tokens returned by a token-endpoint call.
#[derive(Debug, Clone)]
pub struct OAuthTokens {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix epoch milliseconds when the access token expires.
    pub expires_at: i64,
    /// ChatGPT account id (OpenAI only).
    pub account_id: Option<String>,
}

/// A login attempt awaiting code submission. Stored server-side keyed by `state`.
#[derive(Debug, Clone)]
pub struct PendingLogin {
    pub kind: OAuthKind,
    pub verifier: String,
    pub state: String,
    /// Desired provider name/slug supplied by the admin.
    pub name: String,
    pub slug: String,
    pub created_at: std::time::Instant,
}

/// What the admin UI needs to begin an OAuth login.
#[derive(Debug, Clone)]
pub struct AuthorizeInfo {
    pub authorize_url: String,
    pub state: String,
}

/// Server-side store of in-flight logins, keyed by `state`.
pub type PendingStore =
    std::sync::Arc<tokio::sync::Mutex<std::collections::HashMap<String, PendingLogin>>>;

/// Maximum age of a pending login before it is discarded.
pub const PENDING_TTL: std::time::Duration = std::time::Duration::from_secs(600);

/// Errors from the OAuth subsystem.
#[derive(Debug, thiserror::Error)]
pub enum OAuthError {
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("token endpoint returned HTTP {status}: {body}")]
    TokenEndpoint { status: u16, body: String },
    #[error("invalid token response: {0}")]
    InvalidResponse(String),
    #[error("unknown or expired login state")]
    UnknownState,
}

/// Parse an authorization code from raw user input which may be:
/// - a bare code
/// - `CODE#STATE`
/// - a full redirect URL containing `?code=...&state=...`
pub fn parse_code_input(input: &str) -> (String, Option<String>) {
    let value = input.trim();
    if let Ok(url) = url_parse(value) {
        let code = url.0;
        let state = url.1;
        if code.is_some() {
            return (code.unwrap_or_default(), state);
        }
    }
    if let Some((code, state)) = value.split_once('#') {
        return (code.to_string(), Some(state.to_string()));
    }
    (value.to_string(), None)
}

/// Minimal query-param extraction for a possible URL. Returns (code, state).
fn url_parse(value: &str) -> Result<(Option<String>, Option<String>), ()> {
    if !value.starts_with("http://") && !value.starts_with("https://") {
        return Err(());
    }
    let query = value.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut code = None;
    let mut state = None;
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            let decoded = urldecode(v);
            match k {
                "code" => code = Some(decoded),
                "state" => state = Some(decoded),
                _ => {}
            }
        }
    }
    Ok((code, state))
}

/// Tiny percent-decoder (sufficient for OAuth codes/state).
fn urldecode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                if let (Some(h), Some(l)) = (hi, lo) {
                    out.push((h * 16 + l) as u8);
                    i += 3;
                    continue;
                }
                out.push(bytes[i]);
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Build the authorize URL + state for the given flow.
pub fn build_authorize(kind: OAuthKind, pkce: &Pkce, state: &str) -> String {
    match kind {
        OAuthKind::Anthropic => anthropic::build_authorize_url(&pkce.challenge, state),
        OAuthKind::OpenAi => openai::build_authorize_url(&pkce.challenge, state),
    }
}

/// Exchange an authorization code for tokens.
pub async fn exchange_code(
    client: &reqwest::Client,
    kind: OAuthKind,
    code: &str,
    verifier: &str,
    state: &str,
) -> Result<OAuthTokens, OAuthError> {
    match kind {
        OAuthKind::Anthropic => anthropic::exchange_code(client, code, verifier, state).await,
        OAuthKind::OpenAi => openai::exchange_code(client, code, verifier).await,
    }
}

/// Refresh an access token.
pub async fn refresh_tokens(
    client: &reqwest::Client,
    kind: OAuthKind,
    refresh_token: &str,
) -> Result<OAuthTokens, OAuthError> {
    match kind {
        OAuthKind::Anthropic => anthropic::refresh(client, refresh_token).await,
        OAuthKind::OpenAi => openai::refresh(client, refresh_token).await,
    }
}

/// Current time in unix epoch milliseconds.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_is_deterministic_challenge() {
        let p = generate_pkce();
        // Re-derive challenge from verifier.
        let digest = Sha256::digest(p.verifier.as_bytes());
        assert_eq!(p.challenge, base64url(&digest));
        // base64url has no padding chars.
        assert!(!p.challenge.contains('='));
        assert!(!p.verifier.contains('+'));
    }

    #[test]
    fn state_is_hex_32_chars() {
        let s = generate_state();
        assert_eq!(s.len(), 32);
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn parse_bare_code() {
        let (code, state) = parse_code_input("abc123");
        assert_eq!(code, "abc123");
        assert!(state.is_none());
    }

    #[test]
    fn parse_code_hash_state() {
        let (code, state) = parse_code_input("abc123#xyz");
        assert_eq!(code, "abc123");
        assert_eq!(state.as_deref(), Some("xyz"));
    }

    #[test]
    fn parse_full_redirect_url() {
        let (code, state) =
            parse_code_input("http://localhost:1455/auth/callback?code=AAA&state=BBB");
        assert_eq!(code, "AAA");
        assert_eq!(state.as_deref(), Some("BBB"));
    }

    #[test]
    fn parse_url_encoded() {
        let (code, _) = parse_code_input("https://x.test/cb?code=a%2Bb&state=s");
        assert_eq!(code, "a+b");
    }

    #[test]
    fn kind_roundtrip() {
        assert_eq!(OAuthKind::from_str("claude"), Some(OAuthKind::Anthropic));
        assert_eq!(OAuthKind::from_str("chatgpt"), Some(OAuthKind::OpenAi));
        assert_eq!(
            OAuthKind::Anthropic.provider_type(),
            ProviderType::AnthropicOauth
        );
    }
}
