//! OpenAI (ChatGPT Plus/Pro via Codex) OAuth flow.
//!
//! Authorization Code + PKCE against `auth.openai.com`. The ChatGPT account id
//! is embedded in the access token JWT and must be sent as the
//! `chatgpt-account-id` header on API requests.

use base64::Engine;
use serde::{Deserialize, Serialize};

use super::{now_ms, OAuthError, OAuthTokens};

pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
pub const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
pub const SCOPE: &str = "openid profile email offline_access";
/// JWT claim namespace holding ChatGPT auth info.
const JWT_CLAIM_PATH: &str = "https://api.openai.com/auth";

pub fn build_authorize_url(challenge: &str, state: &str) -> String {
    let q = [
        ("response_type", "code"),
        ("client_id", CLIENT_ID),
        ("redirect_uri", REDIRECT_URI),
        ("scope", SCOPE),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
        ("originator", "codex_cli_rs"),
    ];
    let query = q
        .iter()
        .map(|(k, v)| format!("{}={}", k, urlencode(v)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{}?{}", AUTHORIZE_URL, query)
}

#[derive(Debug, Serialize)]
struct ExchangeForm<'a> {
    grant_type: &'a str,
    client_id: &'a str,
    code: &'a str,
    code_verifier: &'a str,
    redirect_uri: &'a str,
}

#[derive(Debug, Serialize)]
struct RefreshForm<'a> {
    grant_type: &'a str,
    client_id: &'a str,
    refresh_token: &'a str,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    id_token: Option<String>,
}

pub async fn exchange_code(
    client: &reqwest::Client,
    code: &str,
    verifier: &str,
) -> Result<OAuthTokens, OAuthError> {
    let form = ExchangeForm {
        grant_type: "authorization_code",
        client_id: CLIENT_ID,
        code,
        code_verifier: verifier,
        redirect_uri: REDIRECT_URI,
    };
    let resp = client.post(TOKEN_URL).form(&form).send().await?;
    parse_token_response(resp, None).await
}

pub async fn refresh(
    client: &reqwest::Client,
    refresh_token: &str,
) -> Result<OAuthTokens, OAuthError> {
    let form = RefreshForm {
        grant_type: "refresh_token",
        client_id: CLIENT_ID,
        refresh_token,
    };
    let resp = client.post(TOKEN_URL).form(&form).send().await?;
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
    // Prefer account id from the id_token, fall back to the access token.
    let account_id = parsed
        .id_token
        .as_deref()
        .and_then(extract_account_id)
        .or_else(|| extract_account_id(&parsed.access_token));
    Ok(OAuthTokens {
        access_token: parsed.access_token,
        refresh_token,
        expires_at,
        account_id,
    })
}

/// Extract `chatgpt_account_id` from a JWT's `https://api.openai.com/auth` claim.
pub fn extract_account_id(jwt: &str) -> Option<String> {
    let payload = jwt.split('.').nth(1)?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(payload))
        .ok()?;
    let value: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    value
        .get(JWT_CLAIM_PATH)
        .and_then(|a| a.get("chatgpt_account_id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

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
    fn authorize_url_has_codex_params() {
        let url = build_authorize_url("CH", "ST");
        assert!(url.contains("codex_cli_simplified_flow=true"));
        assert!(url.contains("id_token_add_organizations=true"));
        assert!(url.contains("originator=codex_cli_rs"));
        assert!(url.contains("scope=openid%20profile%20email%20offline_access"));
    }

    #[test]
    fn extract_account_id_from_jwt() {
        // header.payload.signature where payload encodes the claim.
        let claim = serde_json::json!({
            "https://api.openai.com/auth": { "chatgpt_account_id": "acct_123" }
        });
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&claim).unwrap());
        let jwt = format!("h.{}.s", payload);
        assert_eq!(extract_account_id(&jwt).as_deref(), Some("acct_123"));
    }

    #[test]
    fn extract_account_id_missing() {
        let claim = serde_json::json!({ "sub": "x" });
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&claim).unwrap());
        let jwt = format!("h.{}.s", payload);
        assert_eq!(extract_account_id(&jwt), None);
    }
}
