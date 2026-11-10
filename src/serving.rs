use serde::Serialize;
use serde_json::{Map, Value};

use crate::metrics::MetricsStore;
use crate::payments::biller::TokenUsage;
use crate::quota_report::{serving_quota, ServingQuota};

pub const REPORTED_COST_FIELD: &str = "yalr_reported_cost_usd";

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ServingReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota: Option<ServingQuota>,
    #[serde(skip)]
    pub model: Option<ModelServing>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelServing {
    pub name: String,
    pub exhausted: Option<bool>,
    pub next_available_at: Option<i64>,
}

impl ServingReport {
    pub async fn compute(
        store: &MetricsStore,
        provider: &str,
        models: &[&str],
        usage: &TokenUsage,
        reported_cost: Option<f64>,
    ) -> Self {
        let quota = store.get_quota(provider).await.and_then(|q| serving_quota(&q));
        Self { cost_usd: crate::list_price::cost_usd(models, usage, reported_cost), quota, model: None }
    }

    pub fn cost_usd_micro(&self) -> i64 {
        self.cost_usd.map_or(0, crate::list_price::to_micro)
    }

    pub fn patch_usage(&self, usage: &mut Map<String, Value>) {
        if let Some(cost) = self.cost_usd {
            usage.insert("cost_usd".into(), Value::from(cost));
        }
        let mut quota = match &self.quota {
            Some(quota) => serde_json::to_value(quota).unwrap_or_default(),
            None => Value::Object(Map::new()),
        };
        if let (Some(model), Some(object)) = (&self.model, quota.as_object_mut()) {
            object.insert("model".into(), Value::from(model.name.clone()));
        }
        if quota.as_object().is_some_and(|o| !o.is_empty()) {
            usage.insert("quota".into(), quota);
        }
    }

    pub fn patch_body(&self, body: &mut Value) {
        if let Some(usage) = body.get_mut("usage").and_then(Value::as_object_mut) {
            self.patch_usage(usage);
        }
    }

    pub fn headers(&self) -> Vec<(&'static str, String)> {
        let mut out = Vec::new();
        if let Some(quota) = &self.quota {
            if let Some(pct) = quota.used_pct {
                out.push(("x-yalr-quota-used-pct", format!("{pct}")));
            }
            if let Some(window) = &quota.window {
                out.push(("x-yalr-quota-window", window.clone()));
            }
            if let Some(reset) = quota.resets_at {
                out.push(("x-yalr-quota-resets-at", reset.to_string()));
            }
        }
        if let Some(model) = &self.model {
            out.push(("x-yalr-quota-model", model.name.clone()));
            if let Some(exhausted) = model.exhausted {
                out.push(("x-yalr-quota-exhausted", exhausted.to_string()));
            }
            if let Some(at) = model.next_available_at {
                out.push(("x-yalr-quota-next-available-at", at.to_string()));
            }
        }
        if let Some(cost) = self.cost_usd {
            out.push(("x-yalr-cost-usd", format!("{cost:.6}")));
        }
        out
    }

    pub fn apply_headers(&self, response: &mut axum::response::Response) {
        for (name, value) in self.headers() {
            if let Ok(value) = axum::http::HeaderValue::from_str(&value) {
                response.headers_mut().insert(axum::http::HeaderName::from_static(name), value);
            }
        }
    }
}

pub async fn record(
    trace: Option<&crate::request_log::Trace>,
    store: &MetricsStore,
    provider: &str,
    models: &[&str],
    usage: &TokenUsage,
    reported_cost: Option<f64>,
) {
    let Some(trace) = trace else {
        return;
    };
    trace.set_serving(ServingReport::compute(store, provider, models, usage, reported_cost).await);
}
