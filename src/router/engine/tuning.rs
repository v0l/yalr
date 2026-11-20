use super::*;
use crate::providers::effort::{parse_stored, EffortMap};

#[derive(Clone, Default)]
pub(crate) struct Tuning {
    pub(crate) ttft_deadline: Option<Duration>,
    pub(crate) effort_map: Option<EffortMap>,
}

pub(crate) type TuningMap = HashMap<String, Tuning>;

pub(crate) fn tuning_from_records(records: &[crate::db::Provider]) -> TuningMap {
    records
        .iter()
        .filter_map(|r| {
            let tuning = Tuning {
                ttft_deadline: r
                    .ttft_deadline_ms
                    .filter(|ms| *ms > 0)
                    .map(|ms| Duration::from_millis(ms as u64)),
                effort_map: parse_stored(r.reasoning_effort_map.as_deref()),
            };
            (tuning.ttft_deadline.is_some() || tuning.effort_map.is_some()).then(|| (r.slug.clone(), tuning))
        })
        .collect()
}

pub(crate) fn apply_effort_map(request: &mut ChatRequest, tuning: &TuningMap, slug: &str) {
    if let Some(map) = tuning.get(slug).and_then(|t| t.effort_map.as_ref()) {
        request.map_reasoning_effort(map);
    }
}

pub(crate) fn ttft_deadline(tuning: &TuningMap, slug: &str) -> Option<Duration> {
    tuning.get(slug).and_then(|t| t.ttft_deadline)
}

pub(crate) async fn next_within<S: futures::Stream + Unpin>(
    stream: &mut S,
    deadline: Option<Duration>,
) -> (Option<S::Item>, bool) {
    match deadline {
        None => (stream.next().await, false),
        Some(limit) => match tokio::time::timeout(limit, stream.next()).await {
            Ok(item) => (item, false),
            Err(_) => (stream.next().await, true),
        },
    }
}

impl Router {
    pub(crate) fn tuning_snapshot(&self) -> Arc<TuningMap> {
        self.provider_tuning.read().unwrap().clone()
    }

    pub(crate) fn set_provider_tuning(&self, records: &[crate::db::Provider]) {
        *self.provider_tuning.write().unwrap() = Arc::new(tuning_from_records(records));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;

    #[tokio::test]
    async fn a_slow_first_item_is_flagged_but_still_delivered() {
        let mut slow = Box::pin(stream::once(async {
            tokio::time::sleep(Duration::from_millis(60)).await;
            7
        }));
        let (item, missed) = next_within(&mut slow, Some(Duration::from_millis(10))).await;
        assert_eq!((item, missed), (Some(7), true));
    }

    #[tokio::test]
    async fn a_fast_first_item_is_not_flagged() {
        let mut fast = Box::pin(stream::iter([1]));
        let (item, missed) = next_within(&mut fast, Some(Duration::from_millis(50))).await;
        assert_eq!((item, missed), (Some(1), false));
    }

    #[tokio::test]
    async fn no_deadline_never_flags() {
        let mut slow = Box::pin(stream::once(async {
            tokio::time::sleep(Duration::from_millis(30)).await;
            1
        }));
        assert_eq!(next_within(&mut slow, None).await, (Some(1), false));
    }

    #[test]
    fn effort_map_is_applied_only_for_the_named_provider() {
        let mut tuning = TuningMap::new();
        tuning.insert(
            "a".into(),
            Tuning { ttft_deadline: None, effort_map: Some([("xhigh".to_string(), "high".to_string())].into_iter().collect()) },
        );
        let build = || {
            ChatRequest::from_value(serde_json::json!({"model": "m", "messages": [], "reasoning_effort": "xhigh"})).unwrap()
        };
        let mut hit = build();
        apply_effort_map(&mut hit, &tuning, "a");
        assert_eq!(hit.extra_fields().get("reasoning_effort").unwrap(), "high");
        let mut miss = build();
        apply_effort_map(&mut miss, &tuning, "b");
        assert_eq!(miss.extra_fields().get("reasoning_effort").unwrap(), "xhigh");
    }
}
