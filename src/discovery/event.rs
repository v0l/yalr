use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use serde::Serialize;
use std::collections::BTreeMap;

pub const PROVIDER_KIND: u16 = 38421;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SatsPricing {
    pub prompt: f64,
    pub completion: f64,
    pub request: f64,
    pub input_cache_read: f64,
    pub input_cache_write: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelDetail {
    pub id: String,
    pub context_length: i32,
    pub max_output_tokens: Option<i32>,
    pub vision: bool,
    pub pricing: SatsPricing,
    pub usd_pricing: Option<SatsPricing>,
}

#[derive(Serialize)]
struct ModelPrices<'a> {
    sats: &'a SatsPricing,
    #[serde(skip_serializing_if = "Option::is_none")]
    usd: Option<&'a SatsPricing>,
}

impl ModelDetail {
    fn capabilities(&self) -> String {
        let mut parts = vec![format!("max_tokens:{}", self.context_length)];
        if let Some(out) = self.max_output_tokens {
            parts.push(format!("max_output:{out}"));
        }
        parts.push(format!("vision:{}", self.vision));
        parts.join(",")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Listing {
    pub provider_id: String,
    pub urls: Vec<String>,
    pub models: Vec<ModelDetail>,
    pub version: String,
    pub name: String,
    pub about: String,
}

#[derive(Serialize)]
struct Metadata<'a> {
    name: &'a str,
    about: &'a str,
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    models: Vec<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pricing: BTreeMap<&'a str, ModelPrices<'a>>,
}

impl Listing {
    fn tags(&self) -> Vec<Tag> {
        let mut tags = vec![vec!["d".to_string(), self.provider_id.clone()]];
        tags.extend(self.urls.iter().map(|u| vec!["u".to_string(), u.clone()]));
        let ids = self.model_ids();
        if !ids.is_empty() {
            let mut models = vec!["models".to_string()];
            models.extend(ids);
            tags.push(models);
        }
        for model in self.unique_models() {
            tags.push(vec!["model-cap".to_string(), model.id.clone(), model.capabilities()]);
        }
        tags.push(vec!["version".to_string(), self.version.clone()]);
        tags.into_iter().filter_map(|t| Tag::parse(t).ok()).collect()
    }

    fn unique_models(&self) -> Vec<&ModelDetail> {
        let mut seen = std::collections::HashSet::new();
        self.models.iter().filter(|m| seen.insert(m.id.as_str())).collect()
    }

    fn model_ids(&self) -> Vec<String> {
        self.unique_models().into_iter().map(|m| m.id.clone()).collect()
    }

    pub fn sign(&self, keys: &Keys) -> Result<Event, nostr::event::builder::Error> {
        let content = serde_json::to_string(&Metadata {
            name: &self.name,
            about: &self.about,
            models: self.model_ids(),
            pricing: self.unique_models().into_iter().map(|m| (m.id.as_str(), ModelPrices { sats: &m.pricing, usd: m.usd_pricing.as_ref() })).collect(),
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

    fn detail(id: &str, context: i32, vision: bool) -> ModelDetail {
        ModelDetail {
            id: id.into(),
            context_length: context,
            max_output_tokens: Some(4096),
            vision,
            pricing: SatsPricing { prompt: 0.00042, completion: 0.00084, request: 1.0, input_cache_read: 0.000042, input_cache_write: 0.000042 },
            usd_pricing: Some(SatsPricing { prompt: 0.00026, completion: 0.00052, request: 0.62, input_cache_read: 0.000026, input_cache_write: 0.000026 }),
        }
    }

    fn listing() -> Listing {
        Listing {
            provider_id: "yalr.v0l.io".into(),
            urls: vec!["https://yalr.v0l.io".into()],
            models: vec![detail("gpt-4o", 128_000, true), detail("llama", 32_768, false), detail("gpt-4o", 128_000, true)],
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
        assert_eq!(values(&event, "models").len(), 1);
        assert!(values(&event, "mint").is_empty());
    }

    #[test]
    fn content_is_profile_metadata() {
        let event = listing().sign(&Keys::generate()).unwrap();
        let content: serde_json::Value = serde_json::from_str(&event.content).unwrap();
        assert_eq!(content["name"], "YALR");
        assert_eq!(content["about"], "LLM router");
    }

    #[test]
    fn models_tag_is_deduplicated_in_order() {
        let event = listing().sign(&Keys::generate()).unwrap();
        assert_eq!(values(&event, "models"), vec![vec!["gpt-4o", "llama"]]);
        assert_eq!(values(&event, "model-cap").len(), 2);
    }

    #[test]
    fn model_cap_tags_carry_context_output_and_vision() {
        let event = listing().sign(&Keys::generate()).unwrap();
        assert_eq!(
            values(&event, "model-cap"),
            vec![
                vec!["gpt-4o", "max_tokens:128000,max_output:4096,vision:true"],
                vec!["llama", "max_tokens:32768,max_output:4096,vision:false"],
            ]
        );
    }

    #[test]
    fn content_carries_per_model_sats_pricing_per_token() {
        let event = listing().sign(&Keys::generate()).unwrap();
        let content: serde_json::Value = serde_json::from_str(&event.content).unwrap();
        assert_eq!(content["models"], serde_json::json!(["gpt-4o", "llama"]));
        assert_eq!(content["pricing"]["gpt-4o"]["sats"]["prompt"], 0.00042);
        assert_eq!(content["pricing"]["gpt-4o"]["usd"]["prompt"], 0.00026);
        assert_eq!(content["pricing"]["llama"]["sats"]["request"], 1.0);
        assert_eq!(content["pricing"].as_object().unwrap().len(), 2);
    }

    #[test]
    fn usd_pricing_is_left_out_without_a_rate() {
        let mut l = listing();
        l.models[0].usd_pricing = None;
        let event = l.sign(&Keys::generate()).unwrap();
        let content: serde_json::Value = serde_json::from_str(&event.content).unwrap();
        assert!(content["pricing"]["gpt-4o"].get("usd").is_none());
        assert!(content["pricing"]["gpt-4o"].get("sats").is_some());
    }

    #[test]
    fn several_urls_become_several_u_tags() {
        let mut l = listing();
        l.urls = vec!["https://yalr.v0l.io".into(), "https://app.yalr.net".into()];
        let event = l.sign(&Keys::generate()).unwrap();
        assert_eq!(values(&event, "u"), vec![vec!["https://yalr.v0l.io"], vec!["https://app.yalr.net"]]);
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
