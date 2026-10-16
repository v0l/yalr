use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;

const TRACKED_CONVERSATIONS: usize = 512;

#[derive(Debug, Clone, PartialEq)]
pub struct Fingerprint {
    pub system_hash: String,
    pub tools_hash: String,
    pub message_hashes: Vec<String>,
}

pub const DIVERGED_SYSTEM: i64 = -1;
pub const DIVERGED_TOOLS: i64 = -2;

fn short_hash(bytes: &[u8]) -> String {
    hex::encode(&Sha256::digest(bytes)[..6])
}

fn strip_cache_control(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("cache_control");
            map.values_mut().for_each(strip_cache_control);
        }
        Value::Array(items) => items.iter_mut().for_each(strip_cache_control),
        _ => {}
    }
}

fn canonical_hash(value: Option<&Value>) -> String {
    let mut value = value.cloned().unwrap_or(Value::Null);
    strip_cache_control(&mut value);
    short_hash(&serde_json::to_vec(&value).unwrap_or_default())
}

pub fn fingerprint(body: &Value) -> Fingerprint {
    let mut running = String::new();
    let message_hashes = body
        .get("messages")
        .and_then(Value::as_array)
        .map(|messages| {
            messages
                .iter()
                .map(|message| {
                    let mut message = message.clone();
                    strip_cache_control(&mut message);
                    let mut bytes = running.clone().into_bytes();
                    bytes.extend(serde_json::to_vec(&message).unwrap_or_default());
                    running = short_hash(&bytes);
                    running.clone()
                })
                .collect()
        })
        .unwrap_or_default();
    Fingerprint {
        system_hash: canonical_hash(body.get("system")),
        tools_hash: canonical_hash(body.get("tools")),
        message_hashes,
    }
}

const SESSIONS_PER_KEY: usize = 8;

pub fn common_prefix(previous: &Fingerprint, current: &Fingerprint) -> usize {
    previous
        .message_hashes
        .iter()
        .zip(&current.message_hashes)
        .take_while(|(a, b)| a == b)
        .count()
}

pub fn first_divergence(previous: &Fingerprint, current: &Fingerprint) -> Option<i64> {
    if previous.system_hash != current.system_hash {
        return Some(DIVERGED_SYSTEM);
    }
    if previous.tools_hash != current.tools_hash {
        return Some(DIVERGED_TOOLS);
    }
    let shared = common_prefix(previous, current);
    let comparable = previous.message_hashes.len().min(current.message_hashes.len());
    (shared < comparable).then_some(shared as i64)
}

fn is_related(previous: &Fingerprint, current: &Fingerprint) -> bool {
    previous.system_hash == current.system_hash || common_prefix(previous, current) > 0
}

fn score(previous: &Fingerprint, current: &Fingerprint) -> (bool, bool, usize) {
    (
        previous.system_hash == current.system_hash,
        previous.tools_hash == current.tools_hash,
        common_prefix(previous, current),
    )
}

struct Previous {
    request_id: String,
    fingerprint: Fingerprint,
}

#[derive(Default)]
pub struct Tracker {
    sessions: Mutex<HashMap<String, Vec<Previous>>>,
}

impl Tracker {
    pub fn compare_and_store(
        &self,
        key: String,
        request_id: &str,
        current: &Fingerprint,
    ) -> (Option<i64>, Option<String>) {
        let Ok(mut sessions) = self.sessions.lock() else {
            return (None, None);
        };
        if sessions.len() >= TRACKED_CONVERSATIONS && !sessions.contains_key(&key) {
            sessions.clear();
        }
        let list = sessions.entry(key).or_default();
        let best = list
            .iter()
            .enumerate()
            .max_by_key(|(_, previous)| score(&previous.fingerprint, current))
            .map(|(index, _)| index);
        let related = best.filter(|&index| is_related(&list[index].fingerprint, current));
        let outcome = related
            .map(|index| {
                (
                    first_divergence(&list[index].fingerprint, current),
                    Some(list[index].request_id.clone()),
                )
            })
            .unwrap_or((None, None));
        let extends_best = related.is_some_and(|index| {
            let previous = &list[index].fingerprint;
            outcome.0.is_none() && previous.message_hashes.len() <= current.message_hashes.len()
        });
        if extends_best {
            if let Some(index) = related {
                list.remove(index);
            }
        }
        list.push(Previous {
            request_id: request_id.to_string(),
            fingerprint: current.clone(),
        });
        if list.len() > SESSIONS_PER_KEY {
            list.remove(0);
        }
        outcome
    }
}

pub fn tracker() -> &'static Tracker {
    static TRACKER: std::sync::OnceLock<Tracker> = std::sync::OnceLock::new();
    TRACKER.get_or_init(Tracker::default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn body(messages: Value) -> Value {
        json!({
            "system": [{"type": "text", "text": "sys", "cache_control": {"type": "ephemeral"}}],
            "tools": [{"name": "read"}],
            "messages": messages,
        })
    }

    fn user(text: &str, marked: bool) -> Value {
        let mut block = json!({"type": "text", "text": text});
        if marked {
            block["cache_control"] = json!({"type": "ephemeral"});
        }
        json!({"role": "user", "content": [block]})
    }

    #[test]
    fn moving_breakpoint_is_not_a_divergence() {
        let first = fingerprint(&body(json!([user("a", true)])));
        let second = fingerprint(&body(json!([
            user("a", false),
            {"role": "assistant", "content": [{"type": "text", "text": "r"}]},
            user("b", true)
        ])));
        assert_eq!(first_divergence(&first, &second), None);
    }

    #[test]
    fn edited_early_message_reports_its_index() {
        let first = fingerprint(&body(json!([user("a", false), user("b", false), user("c", true)])));
        let second = fingerprint(&body(json!([user("a", false), user("B", false), user("c", false), user("d", true)])));
        assert_eq!(first_divergence(&first, &second), Some(1));
    }

    #[test]
    fn system_and_tool_changes_are_distinguished() {
        let base = fingerprint(&body(json!([user("a", true)])));
        let mut system_changed = body(json!([user("a", true)]));
        system_changed["system"][0]["text"] = json!("other");
        assert_eq!(first_divergence(&base, &fingerprint(&system_changed)), Some(DIVERGED_SYSTEM));
        let mut tools_changed = body(json!([user("a", true)]));
        tools_changed["tools"] = json!([{"name": "write"}]);
        assert_eq!(first_divergence(&base, &fingerprint(&tools_changed)), Some(DIVERGED_TOOLS));
    }

    #[test]
    fn interleaved_sessions_compare_against_their_own_history() {
        let tracker = Tracker::default();
        let a1 = fingerprint(&body(json!([user("a", true)])));
        let b1 = fingerprint(&body(json!([user("zzz", true)])));
        let a2 = fingerprint(&body(json!([user("a", false), user("a2", true)])));
        let b2 = fingerprint(&body(json!([user("zzz", false), user("z2", true)])));
        tracker.compare_and_store("k".into(), "a1", &a1);
        tracker.compare_and_store("k".into(), "b1", &b1);
        assert_eq!(tracker.compare_and_store("k".into(), "a2", &a2), (None, Some("a1".into())));
        assert_eq!(tracker.compare_and_store("k".into(), "b2", &b2), (None, Some("b1".into())));
    }

    #[test]
    fn an_unrelated_conversation_is_not_linked() {
        let tracker = Tracker::default();
        let one = fingerprint(&body(json!([user("alpha", true)])));
        let mut other_body = body(json!([user("omega", true)]));
        other_body["system"][0]["text"] = json!("a different billing header");
        let other = fingerprint(&other_body);
        tracker.compare_and_store("k".into(), "r1", &one);
        assert_eq!(tracker.compare_and_store("k".into(), "r2", &other), (None, None));
    }

    #[test]
    fn a_changed_system_prompt_mid_conversation_is_reported() {
        let tracker = Tracker::default();
        let one = fingerprint(&body(json!([user("a", true)])));
        let mut changed = body(json!([user("a", false), user("b", true)]));
        changed["system"][0]["text"] = json!("pi edited a prompt section");
        let two = fingerprint(&changed);
        tracker.compare_and_store("k".into(), "r1", &one);
        assert_eq!(
            tracker.compare_and_store("k".into(), "r2", &two),
            (Some(DIVERGED_SYSTEM), Some("r1".to_string()))
        );
    }

    #[test]
    fn tracker_links_consecutive_requests_of_one_conversation() {
        let tracker = Tracker::default();
        let first = fingerprint(&body(json!([user("a", true)])));
        let second = fingerprint(&body(json!([user("a", false), user("b", true)])));
        assert_eq!(tracker.compare_and_store("k".into(), "r1", &first), (None, None));
        assert_eq!(
            tracker.compare_and_store("k".into(), "r2", &second),
            (None, Some("r1".to_string()))
        );
    }
}
