use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use futures::StreamExt;
use nostr::nips::nip65::{self, RelayMetadata};
use nostr::{Event, Filter, Kind, PublicKey};
use nostr_sdk::Client;
use serde::Deserialize;

use crate::db::{Database, User};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(6);
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_OUTBOX_RELAYS: usize = 4;
const LATE_RELAY_GRACE: Duration = Duration::from_millis(1500);
const MAX_AGE_HOURS: i64 = 24;
const SWEEP: Duration = Duration::from_secs(5 * 60);
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

async fn query_relay(relay: String, filter: Filter) -> Vec<Event> {
    let client = Client::default();
    if client.add_relay(relay.as_str()).await.is_err() {
        return Vec::new();
    }
    if client.try_connect_relay(relay.as_str(), CONNECT_TIMEOUT).await.is_err() {
        client.shutdown().await;
        return Vec::new();
    }
    let events = client
        .fetch_events_from([relay.as_str()], filter, FETCH_TIMEOUT)
        .await
        .map(|e| e.into_iter().collect())
        .unwrap_or_default();
    client.shutdown().await;
    events
}

async fn query_relays(relays: &[String], filter: &Filter) -> Vec<Event> {
    let mut pending: futures::stream::FuturesUnordered<_> =
        relays.iter().map(|r| query_relay(r.clone(), filter.clone())).collect();
    let mut events = Vec::new();
    let mut deadline = None;
    loop {
        let next = match deadline {
            Some(at) => match tokio::time::timeout_at(at, pending.next()).await {
                Ok(next) => next,
                Err(_) => break,
            },
            None => pending.next().await,
        };
        let Some(found) = next else { break };
        if !found.is_empty() && deadline.is_none() {
            deadline = Some(tokio::time::Instant::now() + LATE_RELAY_GRACE);
        }
        events.extend(found);
    }
    events
}

fn newest_of_kind(events: &[Event], kind: Kind) -> Option<&Event> {
    events.iter().filter(|e| e.kind == kind).max_by_key(|e| e.created_at)
}

fn write_relays(list: &Event, already: &[String]) -> Vec<String> {
    nip65::extract_relay_list(list)
        .filter(|(_, meta)| !matches!(meta, Some(RelayMetadata::Read)))
        .map(|(url, _)| url.to_string())
        .filter(|url| !already.iter().any(|r| r.trim_end_matches('/') == url.trim_end_matches('/')))
        .take(MAX_OUTBOX_RELAYS)
        .collect()
}

pub async fn fetch(relays: &[String], pubkey_hex: &str) -> Option<Profile> {
    let author = PublicKey::from_hex(pubkey_hex).ok()?;
    let filter = Filter::new().kinds([Kind::Metadata, Kind::RelayList]).author(author);
    let mut events = query_relays(relays, &filter).await;
    if let Some(list) = newest_of_kind(&events, Kind::RelayList) {
        let outbox = write_relays(list, relays);
        if !outbox.is_empty() {
            let filter = Filter::new().kind(Kind::Metadata).author(author);
            events.extend(query_relays(&outbox, &filter).await);
        }
    }
    Profile::parse(&newest_of_kind(&events, Kind::Metadata)?.content)
}

fn retry_delay_minutes(failures: u32) -> i64 {
    if failures <= 1 {
        15
    } else {
        60
    }
}

fn failures() -> &'static Mutex<HashMap<i64, u32>> {
    static FAILURES: OnceLock<Mutex<HashMap<i64, u32>>> = OnceLock::new();
    FAILURES.get_or_init(Default::default)
}

fn note_failure(user_id: i64) -> u32 {
    let mut map = failures().lock().unwrap();
    let count = map.entry(user_id).or_insert(0);
    *count += 1;
    *count
}

fn clear_failures(user_id: i64) {
    failures().lock().unwrap().remove(&user_id);
}

pub async fn refresh(db: &Database, relays: &[String], user: &User) {
    let Some(pubkey) = user.external_id.as_deref() else { return };
    let Some(p) = fetch(relays, pubkey).await else {
        let minutes = retry_delay_minutes(note_failure(user.id));
        tracing::warn!(user_id = user.id, retry_minutes = minutes, "Nostr profile not found");
        if let Err(e) = db.defer_profile_refresh(user.id, MAX_AGE_HOURS, minutes).await {
            tracing::warn!(user_id = user.id, error = %e, "Could not defer Nostr profile refresh");
        }
        return;
    };
    clear_failures(user.id);
    if let Err(e) = db.store_user_profile(user.id, p.shown_name().as_deref(), p.picture().as_deref(), p.nip05().as_deref()).await {
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

    async fn ws_relay(drop_after_handshake: bool) -> String {
        use axum::extract::ws::WebSocketUpgrade;
        use axum::routing::get;
        let app = axum::Router::new().route(
            "/",
            get(move |ws: WebSocketUpgrade| async move {
                ws.on_upgrade(move |socket| async move {
                    if drop_after_handshake {
                        drop(socket);
                    } else {
                        let _socket = socket;
                        tokio::time::sleep(Duration::from_secs(600)).await;
                    }
                })
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        url
    }

    #[tokio::test]
    async fn a_silent_relay_does_not_hide_a_profile_on_another() {
        let silent_url = ws_relay(false).await;
        let resetting_url = ws_relay(true).await;
        let refused = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let refused_url = format!("ws://{}", refused.local_addr().unwrap());
        drop(refused);
        let relay = MockRelay::run().await.unwrap();
        let keys = Keys::generate();
        publish(&relay, &keys, r#"{"display_name":"Kieran"}"#).await;
        let upstream = relay.url().await.to_string().trim_start_matches("ws://").trim_end_matches('/').to_string();
        let slow = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let slow_url = format!("ws://{}", slow.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = slow.accept().await {
                let upstream = upstream.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(1500)).await;
                    if let Ok(mut target) = tokio::net::TcpStream::connect(upstream).await {
                        let _ = tokio::io::copy_bidirectional(&mut socket, &mut target).await;
                    }
                });
            }
        });

        let relays = [resetting_url, silent_url, refused_url, slow_url];
        let started = std::time::Instant::now();
        let profile = fetch(&relays, &keys.public_key().to_hex()).await.unwrap();
        assert!(started.elapsed() < Duration::from_secs(5), "took {:?}", started.elapsed());
        assert_eq!(profile.shown_name().as_deref(), Some("Kieran"));
    }

    #[tokio::test]
    #[ignore]
    async fn live_relays_return_the_profile() {
        let relays: Vec<String> = crate::discovery::DEFAULT_RELAYS.iter().map(|r| r.to_string()).collect();
        let hex = "63fe6318dc58583cfe16810f86dd09e18bfd76aabc24a0081ce2856f330504ed";
        let profile = fetch(&relays, hex).await.unwrap();
        println!("live name: {:?}", profile.shown_name());
        assert_eq!(profile.shown_name().as_deref(), Some("Kieran"));
    }

    #[tokio::test]
    async fn a_failed_fetch_retries_in_minutes_not_a_day() {
        let relay = MockRelay::run().await.unwrap();
        let keys = Keys::generate();
        let db = Database::test().await;
        let user = db.get_or_create_user_by_nostr_pubkey(&keys.public_key().to_hex()).await.unwrap();
        assert!(db.claim_profile_refresh(user.id, 24).await.unwrap());
        refresh(&db, &[relay.url().await.to_string()], &user).await;
        assert!(db.nostr_users_needing_profile(24).await.unwrap().is_empty());
        let stamped = db.get_user_by_id(user.id).await.unwrap().unwrap().profile_fetched_at.unwrap();
        let wait = chrono::Duration::hours(24) - (chrono::Utc::now() - stamped);
        assert!(wait <= chrono::Duration::minutes(61), "next try in {wait}");
        assert!(wait > chrono::Duration::minutes(10), "next try in {wait}");
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
