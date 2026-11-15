use std::sync::Arc;
use std::time::Duration;

use nostr::{Filter, Kind, PublicKey};
use nostr_sdk::Client;
use serde::Deserialize;

use crate::db::{Database, User};

const FETCH_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_AGE_HOURS: i64 = 24;
const SWEEP: Duration = Duration::from_secs(60 * 60);
const MAX_NAME_CHARS: usize = 100;

#[derive(Debug, Default, Deserialize, PartialEq)]
pub struct Profile {
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    picture: Option<String>,
    #[serde(default)]
    nip05: Option<String>,
}

fn clean(value: Option<String>, max: usize) -> Option<String> {
    let value = value?;
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.chars().take(max).collect())
}

impl Profile {
    pub fn parse(content: &str) -> Option<Profile> {
        serde_json::from_str(content).ok()
    }

    pub fn shown_name(&self) -> Option<String> {
        clean(self.display_name.clone(), MAX_NAME_CHARS).or_else(|| clean(self.name.clone(), MAX_NAME_CHARS))
    }

    pub fn picture(&self) -> Option<String> {
        clean(self.picture.clone(), 512).filter(|p| p.starts_with("https://") || p.starts_with("http://"))
    }

    pub fn nip05(&self) -> Option<String> {
        clean(self.nip05.clone(), 255)
    }
}

pub async fn fetch(relays: &[String], pubkey_hex: &str) -> Option<Profile> {
    let author = PublicKey::from_hex(pubkey_hex).ok()?;
    let client = Client::default();
    for relay in relays {
        let _ = client.add_relay(relay.as_str()).await;
    }
    client.connect().await;
    let filter = Filter::new().kind(Kind::Metadata).author(author).limit(1);
    let events = client.fetch_events(filter, FETCH_TIMEOUT).await;
    client.shutdown().await;
    let newest = events.ok()?.into_iter().max_by_key(|e| e.created_at)?;
    Profile::parse(&newest.content)
}

pub async fn refresh(db: &Database, relays: &[String], user: &User) {
    let Some(pubkey) = user.external_id.as_deref() else { return };
    let profile = fetch(relays, pubkey).await;
    let result = match &profile {
        Some(p) => db.store_user_profile(user.id, p.shown_name().as_deref(), p.picture().as_deref(), p.nip05().as_deref()).await,
        None => db.store_user_profile(user.id, user.display_name.as_deref(), user.picture.as_deref(), user.nip05.as_deref()).await,
    };
    if let Err(e) = result {
        tracing::warn!(user_id = user.id, error = %e, "Could not store Nostr profile");
    }
}

pub fn refresh_in_background(db: Arc<Database>, relays: Vec<String>, user: User) {
    if user.user_type != crate::db::UserType::Nostr {
        return;
    }
    let stale = user
        .profile_fetched_at
        .map_or(true, |at| chrono::Utc::now() - at > chrono::Duration::hours(MAX_AGE_HOURS));
    if !stale {
        return;
    }
    tokio::spawn(async move {
        if db.claim_profile_refresh(user.id, MAX_AGE_HOURS).await.unwrap_or(false) {
            refresh(&db, &relays, &user).await;
        }
    });
}

pub fn spawn_refresher(db: Arc<Database>, relays: Vec<String>) {
    tokio::spawn(async move {
        loop {
            for user in db.nostr_users_needing_profile(MAX_AGE_HOURS).await.unwrap_or_default() {
                if db.claim_profile_refresh(user.id, MAX_AGE_HOURS).await.unwrap_or(false) {
                    refresh(&db, &relays, &user).await;
                }
            }
            tokio::time::sleep(SWEEP).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Metadata};
    use nostr_relay_builder::MockRelay;

    async fn publish(relay: &MockRelay, keys: &Keys, metadata: &str) {
        let client = Client::new(keys.clone());
        client.add_relay(relay.url().await.to_string()).await.unwrap();
        client.connect().await;
        let event = EventBuilder::new(Kind::Metadata, metadata).sign_with_keys(keys).unwrap();
        client.send_event(&event).await.unwrap();
        client.shutdown().await;
    }

    #[test]
    fn display_name_wins_over_name_and_junk_is_dropped() {
        let p = Profile::parse(r#"{"display_name":"  Kieran ","name":"v0l","picture":"javascript:x","nip05":"k@v0l.io"}"#).unwrap();
        assert_eq!(p.shown_name().as_deref(), Some("Kieran"));
        assert_eq!(p.picture(), None);
        assert_eq!(p.nip05().as_deref(), Some("k@v0l.io"));
        let p = Profile::parse(r#"{"display_name":"","name":"v0l"}"#).unwrap();
        assert_eq!(p.shown_name().as_deref(), Some("v0l"));
        assert!(Profile::parse("not json").is_none());
    }

    #[tokio::test]
    async fn a_profile_on_a_relay_is_fetched_and_stored_on_the_user() {
        let relay = MockRelay::run().await.unwrap();
        let keys = Keys::generate();
        let _ = Metadata::new();
        publish(&relay, &keys, r#"{"display_name":"Ripper Co","name":"ripper","picture":"https://img.example/a.png","nip05":"r@example.com"}"#).await;

        let db = Database::test().await;
        let user = db.get_or_create_user_by_nostr_pubkey(&keys.public_key().to_hex()).await.unwrap();
        assert_eq!(user.display_name, None);
        refresh(&db, &[relay.url().await.to_string()], &user).await;

        let stored = db.get_user_by_id(user.id).await.unwrap().unwrap();
        assert_eq!(stored.display_name.as_deref(), Some("Ripper Co"));
        assert_eq!(stored.picture.as_deref(), Some("https://img.example/a.png"));
        assert_eq!(stored.nip05.as_deref(), Some("r@example.com"));
        assert!(stored.profile_fetched_at.is_some());
        assert_eq!(stored.shown_name(), "Ripper Co");
    }

    #[tokio::test]
    async fn no_profile_falls_back_to_a_short_npub_never_the_username() {
        let relay = MockRelay::run().await.unwrap();
        let keys = Keys::generate();
        let db = Database::test().await;
        let user = db.get_or_create_user_by_nostr_pubkey(&keys.public_key().to_hex()).await.unwrap();
        refresh(&db, &[relay.url().await.to_string()], &user).await;

        let stored = db.get_user_by_id(user.id).await.unwrap().unwrap();
        assert!(stored.profile_fetched_at.is_some());
        let shown = stored.shown_name();
        assert!(shown.starts_with("npub1") && shown.contains('…'), "{shown}");
        assert!(!shown.contains("nostr_"));
    }

    #[tokio::test]
    async fn a_profile_is_claimed_once_per_day() {
        let db = Database::test().await;
        let keys = Keys::generate();
        let user = db.get_or_create_user_by_nostr_pubkey(&keys.public_key().to_hex()).await.unwrap();
        assert!(db.claim_profile_refresh(user.id, 24).await.unwrap());
        assert!(!db.claim_profile_refresh(user.id, 24).await.unwrap());
    }
}
