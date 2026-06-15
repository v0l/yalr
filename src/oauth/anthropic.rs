//! Anthropic (Claude Pro/Max) OAuth flow.
//!
//! Authorization Code + PKCE against `claude.ai`, token exchange at the
//! Anthropic console. Uses the well-known public Claude Code client id.

use serde::{Deserialize, Serialize};

use super::{now_ms, OAuthError, OAuthTokens};

pub const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
pub const AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
pub const TOKEN_URL: &str = "https://console.anthropic.com/v1/oauth/token";
/// Manual/copy-paste redirect that displays the code (and state) to the user.
pub const REDIRECT_URI: &str = "https://console.anthropic.com/oauth/code/callback";
pub const SCOPES: &str = "org:create_api_key user:profile user:inference";

/// Build the authorize URL. `code=true` selects the copy/paste-friendly page.
pub fn build_authorize_url(challenge: &str, state: &str) -> String {
    let q = [
        ("code", "true"),
        ("client_id", CLIENT_ID),
        ("response_type", "code"),
        ("redirect_uri", REDIRECT_URI),
        ("scope", SCOPES),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
    ];
    let query = q
        .iter()
        .map(|(k, v)| format!("{}={}", k, urlencode(v)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{}?{}", AUTHORIZE_URL, query)
}

#[derive(Debug, Serialize)]
struct ExchangeRequest<'a> {
    grant_type: &'a str,
    code: &'a str,
    state: &'a str,
    client_id: &'a str,
    redirect_uri: &'a str,
    code_verifier: &'a str,
}

#[derive(Debug, Serialize)]
struct RefreshRequest<'a> {
    grant_type: &'a str,
    refresh_token: &'a str,
    client_id: &'a str,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

/// Exchange the authorization code for tokens.
/// The Anthropic console may hand back `CODE#STATE`; the caller passes the parsed
/// `state` (falling back to the PKCE verifier if none was returned).
pub async fn exchange_code(
    client: &reqwest::Client,
    code: &str,
    verifier: &str,
    state: &str,
) -> Result<OAuthTokens, OAuthError> {
    let body = ExchangeRequest {
        grant_type: "authorization_code",
        code,
        state,
        client_id: CLIENT_ID,
        redirect_uri: REDIRECT_URI,
        code_verifier: verifier,
    };
    let resp = client.post(TOKEN_URL).json(&body).send().await?;
    parse_token_response(resp, None).await
}

/// Refresh tokens using a refresh token.
pub async fn refresh(
    client: &reqwest::Client,
    refresh_token: &str,
) -> Result<OAuthTokens, OAuthError> {
    let body = RefreshRequest {
        grant_type: "refresh_token",
        refresh_token,
        client_id: CLIENT_ID,
    };
    let resp = client.post(TOKEN_URL).json(&body).send().await?;
    parse_token_response(resp, Some(refresh_token.to_string())).await
}

async fn parse_token_response(
    resp: reqwest::Response,
    fallback_refresh: Option<String>,
) -> Result<OAuthTokens, OAuthError> {
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(OAuthError::TokenEndpoint {
            status: status.as_u16(),
            body,
        });
    }
    let parsed: TokenResponse = resp
        .json()
        .await
        .map_err(|e| OAuthError::InvalidResponse(e.to_string()))?;
    let refresh_token = parsed
        .refresh_token
        .or(fallback_refresh)
        .ok_or_else(|| OAuthError::InvalidResponse("missing refresh_token".to_string()))?;
    let expires_at = now_ms() + parsed.expires_in.unwrap_or(3600) * 1000;
    Ok(OAuthTokens {
        access_token: parsed.access_token,
        refresh_token,
        expires_at,
        account_id: None,
    })
}

/// Percent-encode a query component (RFC 3986 unreserved chars kept).
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_url_contains_required_params() {
        let url = build_authorize_url("CHAL", "STATE");
        assert!(url.starts_with(AUTHORIZE_URL));
        assert!(url.contains("code=true"));
        assert!(url.contains(&format!("client_id={}", CLIENT_ID)));
        assert!(url.contains("code_challenge=CHAL"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=STATE"));
        assert!(url.contains("scope=org%3Acreate_api_key"));
    }

    #[test]
    fn urlencode_handles_special_chars() {
        assert_eq!(urlencode("a:b c"), "a%3Ab%20c");
        assert_eq!(urlencode("safe-_.~"), "safe-_.~");
    }
}
