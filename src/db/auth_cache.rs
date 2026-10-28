use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{ApiKey, Database, User};

const TTL: Duration = Duration::from_secs(30);
const MAX_ENTRIES: usize = 10_000;

#[derive(Clone, Default)]
pub struct AuthCache {
    entries: Arc<Mutex<HashMap<String, (Instant, ApiKey, User)>>>,
    generation: Arc<AtomicU64>,
}

fn expires_within_ttl(key: &ApiKey) -> bool {
    key.expires_at
        .is_some_and(|at| at.timestamp() - chrono::Utc::now().timestamp() < TTL.as_secs() as i64)
}

impl AuthCache {
    fn get(&self, key_hash: &str) -> Option<(ApiKey, User)> {
        let mut entries = self.entries.lock().unwrap();
        match entries.get(key_hash) {
            Some((at, key, user)) if at.elapsed() < TTL => Some((key.clone(), user.clone())),
            Some(_) => {
                entries.remove(key_hash);
                None
            }
            None => None,
        }
    }

    fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    fn put(&self, generation: u64, key_hash: &str, key: ApiKey, user: User) {
        if expires_within_ttl(&key) {
            return;
        }
        let mut entries = self.entries.lock().unwrap();
        if generation != self.generation() {
            return;
        }
        if entries.len() >= MAX_ENTRIES {
            entries.retain(|_, (at, _, _)| at.elapsed() < TTL);
            if entries.len() >= MAX_ENTRIES {
                entries.clear();
            }
        }
        entries.insert(key_hash.to_string(), (Instant::now(), key, user));
    }

    pub fn clear(&self) {
        let mut entries = self.entries.lock().unwrap();
        self.generation.fetch_add(1, Ordering::SeqCst);
        entries.clear();
    }
}

impl Database {
    pub async fn authenticate_api_key(&self, key_hash: &str) -> Result<Option<(ApiKey, User)>, sqlx::Error> {
        if let Some(hit) = self.auth_cache.get(key_hash) {
            return Ok(Some(hit));
        }
        let generation = self.auth_cache.generation();
        let Some(key) = self.get_api_key_by_hash(key_hash).await? else {
            return Ok(None);
        };
        if !key.is_active || self.is_api_key_expired(key.id).await? {
            return Ok(None);
        }
        let Some(user) = self.get_user_by_id(key.user_id).await? else {
            return Ok(None);
        };
        self.auth_cache.put(generation, key_hash, key.clone(), user.clone());
        Ok(Some((key, user)))
    }

    pub fn invalidate_auth_cache(&self) {
        self.auth_cache.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{NewApiKey, NewUser, UserType};

    async fn db_with_key() -> (Database, i64) {
        let db = Database::test().await;
        let user = db
            .create_user(NewUser { username: Some("u"), password_hash: None, external_id: None, user_type: UserType::Internal, is_admin: false })
            .await
            .unwrap();
        let key = db
            .create_api_key(NewApiKey { key_hash: "h", name: "k", user_id: user.id, last_four: "abcd", expires_at: None })
            .await
            .unwrap();
        (db, key.id)
    }

    #[tokio::test]
    async fn repeated_lookups_skip_the_database() {
        let (db, _) = db_with_key().await;
        assert!(db.authenticate_api_key("h").await.unwrap().is_some());
        db.pool.close().await;
        assert!(db.authenticate_api_key("h").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn a_lookup_racing_a_flush_is_not_cached() {
        let (db, _) = db_with_key().await;
        let generation = db.auth_cache.generation();
        let (key, user) = db.authenticate_api_key("h").await.unwrap().unwrap();
        db.invalidate_auth_cache();
        db.auth_cache.put(generation, "h", key, user);
        assert!(db.auth_cache.get("h").is_none());
    }

    #[tokio::test]
    async fn keys_about_to_expire_are_not_cached() {
        let db = Database::test().await;
        let user = db
            .create_user(NewUser { username: Some("u"), password_hash: None, external_id: None, user_type: UserType::Internal, is_admin: false })
            .await
            .unwrap();
        let soon = chrono::Utc::now() + chrono::Duration::seconds(5);
        db.create_api_key(NewApiKey { key_hash: "e", name: "k", user_id: user.id, last_four: "abcd", expires_at: Some(soon) })
            .await
            .unwrap();
        assert!(db.authenticate_api_key("e").await.unwrap().is_some());
        assert!(db.auth_cache.get("e").is_none());
    }

    #[tokio::test]
    async fn keys_with_a_distant_expiry_are_cached() {
        let db = Database::test().await;
        let user = db
            .create_user(NewUser { username: Some("u"), password_hash: None, external_id: None, user_type: UserType::Internal, is_admin: false })
            .await
            .unwrap();
        let later = chrono::Utc::now() + chrono::Duration::days(30);
        db.create_api_key(NewApiKey { key_hash: "l", name: "k", user_id: user.id, last_four: "abcd", expires_at: Some(later) })
            .await
            .unwrap();
        assert!(db.authenticate_api_key("l").await.unwrap().is_some());
        assert!(db.auth_cache.get("l").is_some());
    }

    #[tokio::test]
    async fn expired_keys_are_rejected() {
        let db = Database::test().await;
        let user = db.add_test_user("u", false).await;
        let past = chrono::Utc::now() - chrono::Duration::minutes(1);
        db.create_api_key(NewApiKey { key_hash: "x", name: "k", user_id: user.id, last_four: "abcd", expires_at: Some(past) })
            .await
            .unwrap();
        assert!(db.authenticate_api_key("x").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn disabling_a_key_takes_effect_immediately() {
        let (db, id) = db_with_key().await;
        assert!(db.authenticate_api_key("h").await.unwrap().is_some());
        db.disable_api_key(id).await.unwrap();
        assert!(db.authenticate_api_key("h").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn deleting_a_key_takes_effect_immediately() {
        let (db, id) = db_with_key().await;
        assert!(db.authenticate_api_key("h").await.unwrap().is_some());
        db.delete_api_key(id).await.unwrap();
        assert!(db.authenticate_api_key("h").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn deleting_the_user_takes_effect_immediately() {
        let (db, _) = db_with_key().await;
        let (_, user) = db.authenticate_api_key("h").await.unwrap().unwrap();
        db.delete_user(user.id).await.unwrap();
        assert!(db.authenticate_api_key("h").await.unwrap().is_none());
    }
}
