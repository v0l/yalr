use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use serde::Serialize;

pub const PROVIDER_KIND: u16 = 38421;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub provider_id: String,
    pub urls: Vec<String>,
    pub models: Vec<String>,
    pub version: String,
    pub name: String,
    pub about: String,
}

#[derive(Serialize)]
struct Metadata<'a> {
    name: &'a str,
    about: &'a str,
}

impl Listing {
    fn tags(&self) -> Vec<Tag> {
        let mut tags = vec![vec!["d".to_string(), self.provider_id.clone()]];
        tags.extend(self.urls.iter().map(|u| vec!["u".to_string(), u.clone()]));
        if !self.models.is_empty() {
            let mut models = vec!["models".to_string()];
            models.extend(self.models.iter().cloned());
            tags.push(models);
        }
        tags.push(vec!["version".to_string(), self.version.clone()]);
        tags.into_iter().filter_map(|t| Tag::parse(t).ok()).collect()
    }

    pub fn sign(&self, keys: &Keys) -> Result<Event, nostr::event::builder::Error> {
        let content = serde_json::to_string(&Metadata {
            name: &self.name,
            about: &self.about,
        })
        .unwrap_or_default();
        EventBuilder::new(Kind::from(PROVIDER_KIND), content)
            .tags(self.tags())
            .sign_with_keys(keys)
    }
}

pub fn is_public_url(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    match parsed.host() {
        Some(url::Host::Domain(d)) => {
            let d = d.to_ascii_lowercase();
            d != "localhost" && !d.ends_with(".localhost")
        }
        Some(url::Host::Ipv4(ip)) => {
            !(ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified())
        }
        Some(url::Host::Ipv6(ip)) => !(ip.is_loopback() || ip.is_unspecified()),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing() -> Listing {
        Listing {
            provider_id: "yalr.v0l.io".into(),
            urls: vec!["https://yalr.v0l.io".into()],
            models: vec!["gpt-4o".into(), "llama".into()],
            version: "0.1.0".into(),
            name: "YALR".into(),
            about: "LLM router".into(),
        }
    }

    fn values<'a>(event: &'a Event, key: &str) -> Vec<Vec<&'a str>> {
        event
            .tags
            .iter()
            .map(|t| t.as_slice())
            .filter(|t| t.first().map(|k| k.as_str()) == Some(key))
            .map(|t| t[1..].iter().map(|v| v.as_str()).collect())
            .collect()
    }

    #[test]
    fn signs_a_valid_provider_event() {
        let keys = Keys::generate();
        let event = listing().sign(&keys).unwrap();
        assert_eq!(event.kind.as_u16(), PROVIDER_KIND);
        assert_eq!(event.pubkey, keys.public_key());
        assert!(event.verify().is_ok());
    }

    #[test]
    fn carries_rip02_tags() {
        let event = listing().sign(&Keys::generate()).unwrap();
        assert_eq!(values(&event, "d"), vec![vec!["yalr.v0l.io"]]);
        assert_eq!(values(&event, "u"), vec![vec!["https://yalr.v0l.io"]]);
        assert_eq!(values(&event, "models"), vec![vec!["gpt-4o", "llama"]]);
        assert_eq!(values(&event, "version"), vec![vec!["0.1.0"]]);
        assert!(values(&event, "mint").is_empty());
    }

    #[test]
    fn content_is_profile_metadata() {
        let event = listing().sign(&Keys::generate()).unwrap();
        assert_eq!(event.content, r#"{"name":"YALR","about":"LLM router"}"#);
    }

    #[test]
    fn omits_models_tag_when_empty() {
        let mut l = listing();
        l.models.clear();
        let event = l.sign(&Keys::generate()).unwrap();
        assert!(values(&event, "models").is_empty());
    }

    #[test]
    fn rejects_unroutable_urls() {
        assert!(is_public_url("https://yalr.v0l.io"));
        assert!(!is_public_url("http://localhost:3000"));
        assert!(!is_public_url("http://127.0.0.1:3000"));
        assert!(!is_public_url("http://10.100.2.5"));
        assert!(!is_public_url("not a url"));
    }
}
