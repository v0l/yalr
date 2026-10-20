use axum::{
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
};
use async_trait::async_trait;
use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use nostr::{Event, JsonUtil, Kind, TagKind, Timestamp};
use std::collections::HashSet;

const DEFAULT_EXPIRATION_SECS: u64 = 60 * 10; // 10 minutes

#[derive(Clone)]
#[derive(Default)]
pub struct AuthConfig {
    pub enabled: bool,
    pub allowed_pubkeys: HashSet<String>,
}


pub fn validate_event(
    event: &Event,
    method: &str,
    path: &str,
) -> Result<(), (StatusCode, &'static str)> {
    if event.kind != Kind::HttpAuth {
        return Err((StatusCode::UNAUTHORIZED, "Wrong event kind"));
    }

    let expiration = event
        .tags
        .find(TagKind::Expiration)
        .and_then(|t| t.content())
        .and_then(|s: &str| s.parse::<u64>().ok())
        .unwrap_or_else(|| event.created_at.as_secs() + DEFAULT_EXPIRATION_SECS);

    let now = Timestamp::now().as_secs();

    if event.created_at.as_secs() > now + 60 {
        return Err((StatusCode::UNAUTHORIZED, "Event created_at is in the future"));
    }

    if now > expiration {
        return Err((StatusCode::UNAUTHORIZED, "Event has expired"));
    }

    let url_matched = event.tags.filter(TagKind::u()).any(|tag| {
        tag.content()
            .and_then(|s: &str| s.parse::<url::Url>().ok())
            .map(|u| u.path() == path)
            .unwrap_or(false)
    });
    if !url_matched {
        return Err((StatusCode::UNAUTHORIZED, "U tag does not match request URL"));
    }

    let method_matched = event.tags.filter(TagKind::Method).any(|tag| {
        tag.content()
            .map(|m: &str| m.eq_ignore_ascii_case(method))
            .unwrap_or(false)
    });
    if !method_matched {
        return Err((StatusCode::UNAUTHORIZED, "Method tag does not match request method"));
    }

    event
        .verify()
        .map_err(|_| (StatusCode::UNAUTHORIZED, "Event signature invalid"))
}

pub struct Nip98Auth {
    pub content_type: Option<String>,
    pub content_length: Option<u64>,
    pub event: Event,
    pub pubkey: String,
}

#[async_trait]
impl<S> FromRequestParts<S> for Nip98Auth
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let auth = parts
            .headers
            .get("authorization")
            .ok_or((StatusCode::FORBIDDEN, "Auth header not found"))?
            .to_str()
            .map_err(|_| (StatusCode::FORBIDDEN, "Invalid auth header"))?;

        if !auth.starts_with("Nostr ") {
            return Err((StatusCode::FORBIDDEN, "Auth scheme must be Nostr"));
        }

        let event_bytes = BASE64_STANDARD
            .decode(&auth[6..])
            .map_err(|_| (StatusCode::FORBIDDEN, "Invalid auth string"))?;

        let event =
            Event::from_json(event_bytes).map_err(|_| (StatusCode::FORBIDDEN, "Invalid nostr event"))?;

        validate_event(&event, parts.method.as_str(), parts.uri.path())?;

        let pubkey = event.pubkey.to_string();

        let content_type = parts
            .headers
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string());

        let content_length = parts
            .headers
            .get("content-length")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.parse().ok());

        Ok(Nip98Auth {
            event,
            content_type,
            content_length,
            pubkey,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Tag};

    fn signed(url: &str, method: &str, age_secs: u64) -> Event {
        let keys = Keys::generate();
        let tags = vec![
            Tag::parse(["u", url]).unwrap(),
            Tag::parse(["method", method]).unwrap(),
        ];
        EventBuilder::new(Kind::HttpAuth, "")
            .tags(tags)
            .custom_created_at(Timestamp::from_secs(Timestamp::now().as_secs() - age_secs))
            .sign_with_keys(&keys)
            .unwrap()
    }

    #[test]
    fn accepts_matching_event() {
        let e = signed("https://yalr.example/v1/models", "GET", 0);
        assert!(validate_event(&e, "GET", "/v1/models").is_ok());
    }

    #[test]
    fn rejects_wrong_path_and_method() {
        let e = signed("https://yalr.example/v1/models", "GET", 0);
        assert!(validate_event(&e, "GET", "/v1/chat/completions").is_err());
        assert!(validate_event(&e, "POST", "/v1/models").is_err());
    }

    #[test]
    fn rejects_expired_event() {
        let e = signed("https://yalr.example/v1/models", "GET", DEFAULT_EXPIRATION_SECS + 5);
        assert_eq!(
            validate_event(&e, "GET", "/v1/models").unwrap_err().1,
            "Event has expired"
        );
    }

    #[test]
    fn rejects_wrong_kind() {
        let keys = Keys::generate();
        let e = EventBuilder::new(Kind::TextNote, "").sign_with_keys(&keys).unwrap();
        assert!(validate_event(&e, "GET", "/").is_err());
    }
}
