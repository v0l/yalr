use axum::http::StatusCode;

use crate::auth::admin::UserExtractor;
use crate::db::{Provider, ProviderType, RoutingConfig, User};
use crate::state::AppState;

pub type TenantError = (StatusCode, String);

pub struct Scope {
    pub user: User,
}

impl Scope {
    pub fn is_admin(&self) -> bool {
        self.user.is_admin
    }

    pub fn owner(&self) -> Option<i64> {
        (!self.is_admin()).then_some(self.user.id)
    }

    pub fn sees(&self, owner_user_id: Option<i64>) -> bool {
        self.is_admin() || owner_user_id == Some(self.user.id)
    }

    pub fn handle(&self) -> String {
        let raw = self.user.username.as_deref().or(self.user.external_id.as_deref()).unwrap_or("user");
        let clean: String = raw
            .to_ascii_lowercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .take(24)
            .collect();
        if clean.is_empty() { format!("u{}", self.user.id) } else { clean }
    }

    pub fn provider_slug(&self, slug: &str) -> Result<String, TenantError> {
        let clean = clean_name(slug).ok_or(bad_request("slug is required"))?;
        Ok(if self.is_admin() { clean } else { format!("u{}-{clean}", self.user.id) })
    }

    pub fn config_name(&self, name: &str) -> Result<String, TenantError> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(bad_request("name is required"));
        }
        if self.is_admin() {
            return Ok(trimmed.to_string());
        }
        let bare = trimmed.strip_prefix(&format!("{}/", self.handle())).unwrap_or(trimmed);
        let clean = clean_name(bare).ok_or(bad_request("name is required"))?;
        Ok(format!("{}/{clean}", self.handle()))
    }

    pub fn provider_type(&self, raw: &str) -> Result<ProviderType, TenantError> {
        let kind = ProviderType::from_str(raw).unwrap_or(ProviderType::OpenAi);
        if !self.is_admin() && kind.is_oauth() {
            return Err(bad_request("OAuth providers cannot be user owned"));
        }
        Ok(kind)
    }

    pub async fn check_url(&self, state: &AppState, base_url: &str) -> Result<(), TenantError> {
        if self.is_admin() {
            return Ok(());
        }
        state.config.router.check_user_url(base_url.trim()).await.map_err(|e| bad_request(&e.to_string()))
    }

    pub async fn provider(&self, state: &AppState, slug: &str) -> Result<Provider, TenantError> {
        state
            .db
            .get_provider_by_slug(slug)
            .await
            .map_err(internal)?
            .filter(|p| self.sees(p.owner_user_id))
            .ok_or_else(|| not_found("Provider not found"))
    }

    pub async fn provider_by_id(&self, state: &AppState, id: i64) -> Result<Provider, TenantError> {
        state
            .db
            .get_provider_by_id(id)
            .await
            .map_err(internal)?
            .filter(|p| self.sees(p.owner_user_id))
            .ok_or_else(|| not_found("Provider not found"))
    }

    pub async fn providers(&self, state: &AppState) -> Result<Vec<Provider>, TenantError> {
        match self.owner() {
            None => state.db.list_providers().await,
            Some(owner) => state.db.list_providers_owned_by(owner).await,
        }
        .map_err(internal)
    }

    pub async fn config(&self, state: &AppState, id: i64) -> Result<RoutingConfig, TenantError> {
        state
            .db
            .get_routing_config(id)
            .await
            .map_err(internal)?
            .filter(|c| self.sees(c.owner_user_id))
            .ok_or_else(|| not_found("Routing config not found"))
    }

    pub async fn configs(&self, state: &AppState) -> Result<Vec<RoutingConfig>, TenantError> {
        match self.owner() {
            None => state.db.list_routing_configs().await,
            Some(owner) => state.db.list_routing_configs_owned_by(owner).await,
        }
        .map_err(internal)
    }

    pub async fn provider_names(&self, state: &AppState) -> Result<Option<Vec<String>>, TenantError> {
        if self.is_admin() {
            return Ok(None);
        }
        Ok(Some(self.providers(state).await?.into_iter().map(|p| p.name).collect()))
    }
}

impl From<UserExtractor> for Scope {
    fn from(UserExtractor(auth): UserExtractor) -> Self {
        Self { user: auth.user }
    }
}

fn clean_name(raw: &str) -> Option<String> {
    let clean: String = raw
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
        .collect();
    (!clean.is_empty()).then_some(clean)
}

pub fn internal(e: impl ToString) -> TenantError {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

pub fn not_found(message: &str) -> TenantError {
    (StatusCode::NOT_FOUND, message.to_string())
}

pub fn bad_request(message: &str) -> TenantError {
    (StatusCode::BAD_REQUEST, message.to_string())
}

pub fn forbidden(message: &str) -> TenantError {
    (StatusCode::FORBIDDEN, message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(id: i64, name: &str, is_admin: bool) -> Scope {
        Scope {
            user: User {
                id,
                username: Some(name.into()),
                password_hash: None,
                external_id: None,
                user_type: crate::db::UserType::Internal,
                is_admin,
                created_at: Default::default(),
                updated_at: Default::default(),
                display_name: None,
                picture: None,
                nip05: None,
                profile_fetched_at: None,
            },
        }
    }

    #[test]
    fn tenant_names_are_namespaced_and_admin_names_are_not() {
        let alice = scope(7, "Alice", false);
        let admin = scope(1, "root", true);
        assert_eq!(alice.provider_slug("My GPU").unwrap(), "u7-mygpu");
        assert_eq!(admin.provider_slug("my-gpu").unwrap(), "my-gpu");
        assert_eq!(alice.config_name("fast").unwrap(), "alice/fast");
        assert_eq!(alice.config_name("alice/fast").unwrap(), "alice/fast");
        assert_eq!(alice.config_name("bob/fast").unwrap(), "alice/bobfast");
        assert_eq!(admin.config_name("code-high").unwrap(), "code-high");
        assert!(alice.config_name("  ").is_err());
    }

    #[test]
    fn only_admins_see_rows_they_do_not_own() {
        let alice = scope(7, "alice", false);
        let admin = scope(1, "root", true);
        assert!(alice.sees(Some(7)));
        assert!(!alice.sees(Some(8)));
        assert!(!alice.sees(None));
        assert!(admin.sees(None));
        assert!(admin.sees(Some(8)));
    }
}
