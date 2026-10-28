use super::{
    ApiKey, Database, ModelAccess, NewApiKey, NewUser, NewUserModelPermission, Page, UpdateUserModelPermission, User,
    UserType,
};
use sqlx::QueryBuilder;
use std::collections::HashMap;

pub struct UserChanges<'a> {
    pub username: Option<&'a str>,
    pub password_hash: Option<&'a str>,
    pub is_admin: Option<bool>,
}

impl Database {
    pub async fn create_user(&self, user: NewUser<'_>) -> Result<User, sqlx::Error> {
        let id = sqlx::query(
            "INSERT INTO users (username, password_hash, external_id, user_type, is_admin) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(user.username)
        .bind(user.password_hash)
        .bind(user.external_id)
        .bind(user.user_type as u16)
        .bind(user.is_admin)
        .execute(&self.pool)
        .await?
        .last_insert_id() as i64;
        self.get_user_by_id(id).await?.ok_or(sqlx::Error::RowNotFound)
    }

    pub async fn get_user_by_id(&self, id: i64) -> Result<Option<User>, sqlx::Error> {
        sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = ?").bind(id).fetch_optional(&self.pool).await
    }

    pub async fn get_user_by_username(&self, username: &str) -> Result<Option<User>, sqlx::Error> {
        sqlx::query_as::<_, User>("SELECT * FROM users WHERE username = ?")
            .bind(username)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn get_user_by_external_id(
        &self,
        external_id: &str,
        user_type: UserType,
    ) -> Result<Option<User>, sqlx::Error> {
        sqlx::query_as::<_, User>("SELECT * FROM users WHERE external_id = ? AND user_type = ?")
            .bind(external_id)
            .bind(user_type as u16)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn get_or_create_user_by_nostr_pubkey(&self, pubkey: &str) -> Result<User, sqlx::Error> {
        if let Some(user) = self.get_user_by_external_id(pubkey, UserType::Nostr).await? {
            return Ok(user);
        }
        let username = format!("nostr_{}", &pubkey[..16]);
        sqlx::query(
            "INSERT IGNORE INTO users (username, password_hash, external_id, user_type, is_admin) \
             VALUES (?, NULL, ?, ?, FALSE)",
        )
        .bind(&username)
        .bind(pubkey)
        .bind(UserType::Nostr as u16)
        .execute(&self.pool)
        .await?;
        self.get_user_by_external_id(pubkey, UserType::Nostr).await?.ok_or(sqlx::Error::RowNotFound)
    }

    pub async fn user_exists(&self, username: &str) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE username = ?)")
            .bind(username)
            .fetch_one(&self.pool)
            .await
    }

    pub async fn has_users(&self) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users)").fetch_one(&self.pool).await
    }

    pub async fn list_users(&self, page: Page<'_>) -> Result<Vec<User>, sqlx::Error> {
        let mut query = QueryBuilder::new("SELECT * FROM users WHERE 1 = 1");
        if let Some(before) = page.before {
            query.push(" AND id < ").push_bind(before);
        }
        if let Some(search) = page.search {
            let like = format!("%{search}%");
            query.push(" AND (username LIKE ").push_bind(like.clone()).push(" OR external_id LIKE ").push_bind(like).push(")");
        }
        query.push(" ORDER BY id DESC LIMIT ").push_bind(page.limit);
        query.build_query_as::<User>().fetch_all(&self.pool).await
    }

    pub async fn update_user(&self, id: i64, changes: UserChanges<'_>) -> Result<User, sqlx::Error> {
        let mut query = QueryBuilder::new("UPDATE users SET updated_at = CURRENT_TIMESTAMP(3)");
        if let Some(username) = changes.username {
            query.push(", username = ").push_bind(username);
        }
        if let Some(hash) = changes.password_hash {
            query.push(", password_hash = ").push_bind(hash);
        }
        if let Some(is_admin) = changes.is_admin {
            query.push(", is_admin = ").push_bind(is_admin);
        }
        query.push(" WHERE id = ").push_bind(id);
        query.build().execute(&self.pool).await?;
        self.invalidate_auth_cache();
        self.get_user_by_id(id).await?.ok_or(sqlx::Error::RowNotFound)
    }

    pub async fn delete_user(&self, id: i64) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM users WHERE id = ?").bind(id).execute(&self.pool).await?;
        self.invalidate_auth_cache();
        Ok(result.rows_affected() > 0)
    }

    pub async fn balances_for_users(&self, ids: &[i64]) -> Result<HashMap<i64, i64>, sqlx::Error> {
        if ids.is_empty() {
            return Ok(Default::default());
        }
        let mut query = QueryBuilder::new("SELECT user_id, balance_msat FROM user_balances WHERE user_id IN (");
        let mut separated = query.separated(", ");
        for id in ids {
            separated.push_bind(*id);
        }
        query.push(")");
        let rows: Vec<(i64, i64)> = query.build_query_as().fetch_all(&self.pool).await?;
        Ok(rows.into_iter().collect())
    }

    pub async fn create_api_key(&self, key: NewApiKey<'_>) -> Result<ApiKey, sqlx::Error> {
        let id = sqlx::query(
            "INSERT INTO api_keys (key_hash, name, user_id, last_four, expires_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(key.key_hash)
        .bind(key.name)
        .bind(key.user_id)
        .bind(key.last_four)
        .bind(key.expires_at)
        .execute(&self.pool)
        .await?
        .last_insert_id() as i64;
        self.api_key_by_id(id).await
    }

    async fn api_key_by_id(&self, id: i64) -> Result<ApiKey, sqlx::Error> {
        sqlx::query_as::<_, ApiKey>("SELECT * FROM api_keys WHERE id = ?").bind(id).fetch_one(&self.pool).await
    }

    pub async fn get_api_key_by_hash(&self, key_hash: &str) -> Result<Option<ApiKey>, sqlx::Error> {
        sqlx::query_as::<_, ApiKey>("SELECT * FROM api_keys WHERE key_hash = ? AND is_active = TRUE")
            .bind(key_hash)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn list_api_keys_for_user(&self, user_id: i64) -> Result<Vec<ApiKey>, sqlx::Error> {
        sqlx::query_as::<_, ApiKey>("SELECT * FROM api_keys WHERE user_id = ? ORDER BY created_at DESC, id DESC")
            .bind(user_id)
            .fetch_all(&self.pool)
            .await
    }

    pub async fn delete_api_key(&self, id: i64) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM api_keys WHERE id = ?").bind(id).execute(&self.pool).await?;
        self.invalidate_auth_cache();
        Ok(result.rows_affected() > 0)
    }

    pub async fn disable_api_key(&self, id: i64) -> Result<bool, sqlx::Error> {
        self.set_api_key_active(id, false).await
    }

    pub async fn enable_api_key(&self, id: i64) -> Result<bool, sqlx::Error> {
        self.set_api_key_active(id, true).await
    }

    async fn set_api_key_active(&self, id: i64, active: bool) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("UPDATE api_keys SET is_active = ? WHERE id = ?")
            .bind(active)
            .bind(id)
            .execute(&self.pool)
            .await?;
        self.invalidate_auth_cache();
        Ok(result.rows_affected() > 0)
    }

    pub async fn is_api_key_expired(&self, id: i64) -> Result<bool, sqlx::Error> {
        let expired: Option<bool> = sqlx::query_scalar(
            "SELECT expires_at IS NOT NULL AND expires_at < UTC_TIMESTAMP(3) FROM api_keys WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(expired.unwrap_or(false))
    }

    pub async fn check_model_access(&self, user_id: i64, model: &str) -> Result<ModelAccess, sqlx::Error> {
        let permissions = self.list_user_model_permissions(user_id).await?;
        let rule = permissions
            .iter()
            .find(|p| p.model == model)
            .or_else(|| permissions.iter().find(|p| p.model == "*"));
        Ok(match rule {
            Some(p) if p.allow => ModelAccess::Allow,
            Some(_) => ModelAccess::Deny,
            None if permissions.is_empty() => ModelAccess::Allow,
            None => ModelAccess::Deny,
        })
    }

    pub async fn is_for_sale(&self, model: &str) -> Result<bool, sqlx::Error> {
        Ok(self.get_model_pricing(model).await?.is_some_and(|p| p.is_advertised))
    }

    pub async fn effective_model_access(
        &self,
        user_id: i64,
        model: &str,
        sale_only_by_default: bool,
    ) -> Result<ModelAccess, sqlx::Error> {
        if sale_only_by_default
            && self.list_user_model_permissions(user_id).await?.is_empty()
            && !self.is_billing_exempt(user_id).await?
        {
            return Ok(if self.is_for_sale(model).await? { ModelAccess::Allow } else { ModelAccess::Deny });
        }
        self.check_model_access(user_id, model).await
    }

    pub async fn create_user_model_permission(
        &self,
        perm: NewUserModelPermission<'_>,
    ) -> Result<super::UserModelPermission, sqlx::Error> {
        sqlx::query("INSERT INTO user_model_permissions (user_id, model, allow) VALUES (?, ?, ?)")
            .bind(perm.user_id)
            .bind(perm.model)
            .bind(perm.allow)
            .execute(&self.pool)
            .await?;
        self.get_user_model_permission(perm.user_id, perm.model).await?.ok_or(sqlx::Error::RowNotFound)
    }

    pub async fn upsert_user_model_permission(
        &self,
        perm: NewUserModelPermission<'_>,
    ) -> Result<super::UserModelPermission, sqlx::Error> {
        sqlx::query(
            "INSERT INTO user_model_permissions (user_id, model, allow) VALUES (?, ?, ?) \
             ON DUPLICATE KEY UPDATE allow = VALUES(allow), updated_at = CURRENT_TIMESTAMP(3)",
        )
        .bind(perm.user_id)
        .bind(perm.model)
        .bind(perm.allow)
        .execute(&self.pool)
        .await?;
        self.get_user_model_permission(perm.user_id, perm.model).await?.ok_or(sqlx::Error::RowNotFound)
    }

    pub async fn get_user_model_permission(
        &self,
        user_id: i64,
        model: &str,
    ) -> Result<Option<super::UserModelPermission>, sqlx::Error> {
        sqlx::query_as::<_, super::UserModelPermission>(
            "SELECT * FROM user_model_permissions WHERE user_id = ? AND model = ?",
        )
        .bind(user_id)
        .bind(model)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn list_user_model_permissions(
        &self,
        user_id: i64,
    ) -> Result<Vec<super::UserModelPermission>, sqlx::Error> {
        sqlx::query_as::<_, super::UserModelPermission>(
            "SELECT * FROM user_model_permissions WHERE user_id = ? ORDER BY model",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn update_user_model_permission(
        &self,
        user_id: i64,
        model: &str,
        updates: UpdateUserModelPermission,
    ) -> Result<super::UserModelPermission, sqlx::Error> {
        let mut query = QueryBuilder::new("UPDATE user_model_permissions SET updated_at = CURRENT_TIMESTAMP(3)");
        if let Some(allow) = updates.allow {
            query.push(", allow = ").push_bind(allow);
        }
        query.push(" WHERE user_id = ").push_bind(user_id).push(" AND model = ").push_bind(model);
        query.build().execute(&self.pool).await?;
        self.get_user_model_permission(user_id, model).await?.ok_or(sqlx::Error::RowNotFound)
    }

    pub async fn delete_user_model_permission(&self, user_id: i64, model: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM user_model_permissions WHERE user_id = ? AND model = ?")
            .bind(user_id)
            .bind(model)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}
