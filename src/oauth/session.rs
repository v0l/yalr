//! Runtime OAuth session shared by OAuth-backed providers.
//!
//! Holds the current tokens in memory, transparently refreshes the access token
//! shortly before it expires, and persists refreshed tokens back to the database.

use std::sync::Arc;

use tokio::sync::RwLock;

use crate::db::{Database, OAuthCredentials};

use super::{now_ms, refresh_tokens, OAuthKind, OAuthTokens};

/// Refresh the token when it has this many milliseconds (or fewer) of life left.
const REFRESH_SKEW_MS: i64 = 60_000;

#[derive(Debug, Clone)]
struct TokenState {
    access_token: String,
    refresh_token: String,
    expires_at: i64,
    account_id: Option<String>,
}

/// A self-refreshing OAuth session bound to a single provider row.
pub struct OAuthSession {
    kind: OAuthKind,
    provider_id: i64,
    db: Arc<Database>,
    http: reqwest::Client,
    state: RwLock<TokenState>,
}

impl OAuthSession {
    pub fn new(
        kind: OAuthKind,
        provider_id: i64,
        db: Arc<Database>,
        access_token: String,
        refresh_token: String,
        expires_at: i64,
        account_id: Option<String>,
    ) -> Self {
        Self {
            kind,
            provider_id,
            db,
            http: reqwest::Client::new(),
            state: RwLock::new(TokenState {
                access_token,
                refresh_token,
                expires_at,
                account_id,
            }),
        }
    }

    /// The ChatGPT account id, if any (OpenAI only).
    pub async fn account_id(&self) -> Option<String> {
        self.state.read().await.account_id.clone()
    }

    /// Return a valid access token, refreshing it first if it is expired or close to it.
    pub async fn access_token(&self) -> Result<String, super::OAuthError> {
        {
            let st = self.state.read().await;
            if st.expires_at - now_ms() > REFRESH_SKEW_MS {
                return Ok(st.access_token.clone());
            }
        }
        self.refresh().await
    }

    /// Force a token refresh and persist the result.
    pub async fn refresh(&self) -> Result<String, super::OAuthError> {
        // Take the write lock for the whole refresh so concurrent callers coalesce.
        let mut st = self.state.write().await;
        // Another task may have refreshed while we waited for the lock.
        if st.expires_at - now_ms() > REFRESH_SKEW_MS {
            return Ok(st.access_token.clone());
        }
        let refresh_token = st.refresh_token.clone();
        let tokens: OAuthTokens = refresh_tokens(&self.http, self.kind, &refresh_token).await?;
        // OpenAI rotates account id only via id_token; keep the existing one if absent.
        let account_id = tokens.account_id.clone().or_else(|| st.account_id.clone());

        st.access_token = tokens.access_token.clone();
        st.refresh_token = tokens.refresh_token.clone();
        st.expires_at = tokens.expires_at;
        st.account_id = account_id.clone();

        let creds = OAuthCredentials {
            access_token: tokens.access_token.clone(),
            refresh_token: tokens.refresh_token,
            expires_at: tokens.expires_at,
            account_id,
        };
        if let Err(e) = self.db.update_provider_oauth(self.provider_id, &creds).await {
            tracing::warn!(error = %e, provider_id = self.provider_id, "Failed to persist refreshed OAuth tokens");
        }
        Ok(tokens.access_token)
    }
}
