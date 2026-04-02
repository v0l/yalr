// YALR (Yet another LLM router) - Metrics system
// Event-based timeseries metrics with percentile support
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

/// Provider metrics data point with timestamp and event
#[derive(Debug, Clone, Serialize)]
pub struct ProviderMetrics {
    pub provider: String,
    pub model: String,
    pub timestamp: DateTime<Utc>,
    pub event: MetricsEvent,
}

/// Metrics event types (value only, no provider/model info)
#[derive(Debug, Clone, Serialize)]
pub enum MetricsEvent {
    /// Time to First Token (ms)
    TTFT(f64),
    /// Tokens per second
    TokensPerSecond(f64),
    /// Total latency (ms)
    TotalLatency(f64),
    /// Input tokens used
    InputTokens(u32),
    /// Output tokens used
    OutputTokens(u32),
    /// Request success
    Success,
    /// Request failed
    Failure(String),
}

/// Metrics emitter that sends events via broadcast channel
#[derive(Clone)]
pub struct MetricsEmitter {
    sender: broadcast::Sender<ProviderMetrics>,
}

impl MetricsEmitter {
    pub fn new(buffer_size: usize) -> (Self, MetricsReceiver) {
        let (sender, receiver) = broadcast::channel(buffer_size);
        (
            Self { sender },
            MetricsReceiver { receiver },
        )
    }

    fn emit(&self, provider: String, model: String, event: MetricsEvent) {
        let metrics = ProviderMetrics {
            timestamp: Utc::now(),
            provider: provider.clone(),
            model: model.clone(),
            event: event.clone(),
        };
        tracing::debug!(
            provider = %provider,
            model = %model,
            event = ?event,
            "Metrics event emitted"
        );
        let _ = self.sender.send(metrics);
    }

    pub fn emit_ttft(&self, provider: &str, model: &str, value_ms: f64) {
        self.emit(provider.to_string(), model.to_string(), MetricsEvent::TTFT(value_ms));
    }

    pub fn emit_tokens_per_second(&self, provider: &str, model: &str, value: f64) {
        self.emit(provider.to_string(), model.to_string(), MetricsEvent::TokensPerSecond(value));
    }

    pub fn emit_total_latency(&self, provider: &str, model: &str, value_ms: f64) {
        self.emit(provider.to_string(), model.to_string(), MetricsEvent::TotalLatency(value_ms));
    }

    pub fn emit_input_tokens(&self, provider: &str, model: &str, value: u32) {
        self.emit(provider.to_string(), model.to_string(), MetricsEvent::InputTokens(value));
    }

    pub fn emit_output_tokens(&self, provider: &str, model: &str, value: u32) {
        self.emit(provider.to_string(), model.to_string(), MetricsEvent::OutputTokens(value));
    }

    pub fn emit_success(&self, provider: &str, model: &str) {
        self.emit(provider.to_string(), model.to_string(), MetricsEvent::Success);
    }

    pub fn emit_failure(&self, provider: &str, model: &str, error: &str) {
        self.emit(provider.to_string(), model.to_string(), MetricsEvent::Failure(error.to_string()));
    }
}

/// Receiver for metrics events
pub struct MetricsReceiver {
    receiver: broadcast::Receiver<ProviderMetrics>,
}

impl MetricsReceiver {
    pub async fn recv(&mut self) -> Result<ProviderMetrics, broadcast::error::RecvError> {
        self.receiver.recv().await
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ProviderMetrics> {
        self.receiver.resubscribe()
    }
}

/// Summary of provider metrics for routing decisions
#[derive(Debug, Clone)]
pub struct ProviderMetricsSummary {
    pub provider: String,
    pub p90_ttft: Option<f64>,
    pub p90_tokens_per_second: Option<f64>,
    pub avg_latency: Option<f64>,
    pub success_rate: Option<f64>,
}

/// Summary of model-specific metrics for routing decisions
#[derive(Debug, Clone)]
pub struct ModelMetricsSummary {
    pub provider: String,
    pub model: String,
    pub p90_ttft: Option<f64>,
    pub p90_tokens_per_second: Option<f64>,
    pub avg_latency: Option<f64>,
    pub success_rate: Option<f64>,
}

/// In-memory timeseries store for metrics with percentile support
#[derive(Clone)]
pub struct MetricsStore {
    emitter: MetricsEmitter,
    /// Store recent events for percentile calculations (wrapped in Arc<RwLock> for shared access)
    events: Arc<RwLock<VecDeque<ProviderMetrics>>>,
    max_events: usize,
}

/// Type alias for MetricsStore - now cloneable with internal Arc<RwLock>
pub type SharedMetricsStore = MetricsStore;

impl MetricsStore {
    pub fn new(emitter: MetricsEmitter, max_events: usize) -> Self {
        Self {
            emitter,
            events: Arc::new(RwLock::new(VecDeque::with_capacity(max_events))),
            max_events,
        }
    }

    pub fn emitter(&self) -> &MetricsEmitter {
        &self.emitter
    }

    /// Record a metrics event
    pub async fn record(&self, event: ProviderMetrics) {
        let provider = event.provider.clone();
        let model = event.model.clone();
        let event_type = format!("{:?}", event.event);
        
        let mut events = self.events.write().await;
        events.push_back(event);
        if events.len() > self.max_events {
            events.pop_front();
        }
        
        tracing::debug!(
            provider = %provider,
            model = %model,
            event_type = %event_type,
            total_events = events.len(),
            "Metrics event recorded"
        );
    }

    /// Get all events for a specific provider and model (model optional)
    pub async fn get_events_for(&self, provider: &str, model: Option<&str>) -> Vec<ProviderMetrics> {
        let events = self.events.read().await;
        events
            .iter()
            .filter(|e| {
                let provider_match = e.provider == provider;
                let model_match = model.map_or(true, |m| e.model == m);
                provider_match && model_match
            })
            .cloned()
            .collect()
    }

    /// Calculate p90 tokens per second for a provider/model (model optional)
    pub async fn p90_tokens_per_second(&self, provider: &str, model: Option<&str>) -> Option<f64> {
        let events = self.get_events_for(provider, model).await;
        let values: Vec<f64> = events
            .iter()
            .filter_map(|e| match &e.event {
                MetricsEvent::TokensPerSecond(value) => Some(*value),
                _ => None,
            })
            .collect();

        percentile(&values, 0.90)
    }

    /// Calculate p90 TTFT for a provider/model (model optional)
    pub async fn p90_ttft(&self, provider: &str, model: Option<&str>) -> Option<f64> {
        let events = self.get_events_for(provider, model).await;
        let values: Vec<f64> = events
            .iter()
            .filter_map(|e| match &e.event {
                MetricsEvent::TTFT(value_ms) => Some(*value_ms),
                _ => None,
            })
            .collect();

        percentile(&values, 0.90)
    }

    /// Calculate average latency for a provider (model aggregated)
    pub async fn avg_latency(&self, provider: &str, model: Option<&str>) -> Option<f64> {
        let events = self.get_events_for(provider, model).await;
        let values: Vec<f64> = events
            .iter()
            .filter_map(|e| match &e.event {
                MetricsEvent::TotalLatency(value_ms) => Some(*value_ms),
                _ => None,
            })
            .collect();

        if values.is_empty() {
            None
        } else {
            Some(values.iter().sum::<f64>() / values.len() as f64)
        }
    }

    /// Calculate success rate for a provider (model aggregated)
    pub async fn success_rate(&self, provider: &str, model: Option<&str>) -> Option<f64> {
        let events = self.get_events_for(provider, model).await;
        if events.is_empty() {
            return None;
        }

        let successes = events
            .iter()
            .filter(|e| matches!(e.event, MetricsEvent::Success))
            .count();

        Some(successes as f64 / events.len() as f64)
    }

    /// Get recent events (last N events)
    pub async fn recent_events(&self, n: usize) -> Vec<ProviderMetrics> {
        let events = self.events.read().await;
        events
            .iter()
            .rev()
            .take(n)
            .cloned()
            .collect()
    }

    /// Compute metrics summary from events (internal helper)
    fn compute_metrics_summary(provider: String, events: &[ProviderMetrics]) -> ProviderMetricsSummary {
        let ttft_values: Vec<f64> = events
            .iter()
            .filter_map(|e| match &e.event {
                MetricsEvent::TTFT(v) => Some(*v),
                _ => None,
            })
            .collect();
        
        let tps_values: Vec<f64> = events
            .iter()
            .filter_map(|e| match &e.event {
                MetricsEvent::TokensPerSecond(v) => Some(*v),
                _ => None,
            })
            .collect();
        
        let latency_values: Vec<f64> = events
            .iter()
            .filter_map(|e| match &e.event {
                MetricsEvent::TotalLatency(v) => Some(*v),
                _ => None,
            })
            .collect();
        
        let successes = events.iter().filter(|e| matches!(e.event, MetricsEvent::Success)).count();
        let total = events.len();

        ProviderMetricsSummary {
            provider,
            p90_ttft: percentile(&ttft_values, 0.90),
            p90_tokens_per_second: percentile(&tps_values, 0.90),
            avg_latency: if latency_values.is_empty() { None } else { Some(latency_values.iter().sum::<f64>() / latency_values.len() as f64) },
            success_rate: if total == 0 { None } else { Some(successes as f64 / total as f64) },
        }
    }

    /// Compute model-specific metrics summary (internal helper)
    fn compute_model_metrics_summary(provider: String, model: String, events: &[ProviderMetrics]) -> ModelMetricsSummary {
        let ttft_values: Vec<f64> = events
            .iter()
            .filter_map(|e| match &e.event {
                MetricsEvent::TTFT(v) => Some(*v),
                _ => None,
            })
            .collect();
        
        let tps_values: Vec<f64> = events
            .iter()
            .filter_map(|e| match &e.event {
                MetricsEvent::TokensPerSecond(v) => Some(*v),
                _ => None,
            })
            .collect();
        
        let latency_values: Vec<f64> = events
            .iter()
            .filter_map(|e| match &e.event {
                MetricsEvent::TotalLatency(v) => Some(*v),
                _ => None,
            })
            .collect();
        
        let successes = events.iter().filter(|e| matches!(e.event, MetricsEvent::Success)).count();
        let total = events.len();

        ModelMetricsSummary {
            provider,
            model,
            p90_ttft: percentile(&ttft_values, 0.90),
            p90_tokens_per_second: percentile(&tps_values, 0.90),
            avg_latency: if latency_values.is_empty() { None } else { Some(latency_values.iter().sum::<f64>() / latency_values.len() as f64) },
            success_rate: if total == 0 { None } else { Some(successes as f64 / total as f64) },
        }
    }

    /// Get all metrics for a provider in a single lock acquisition
    pub async fn get_provider_summary(&self, provider: &str) -> ProviderMetricsSummary {
        let events = self.events.read().await;
        let provider_events: Vec<ProviderMetrics> = events
            .iter()
            .filter(|e| e.provider == provider)
            .cloned()
            .collect();
        
        Self::compute_metrics_summary(provider.to_string(), &provider_events)
    }

    /// Get model-specific metrics summary in a single lock acquisition
    pub async fn get_model_summary(&self, provider: &str, model: &str) -> ModelMetricsSummary {
        let events = self.events.read().await;
        let model_events: Vec<ProviderMetrics> = events
            .iter()
            .filter(|e| e.provider == provider && e.model == model)
            .cloned()
            .collect();
        
        Self::compute_model_metrics_summary(provider.to_string(), model.to_string(), &model_events)
    }

    /// Get summaries for all models of a provider
    pub async fn get_model_summaries_for_provider(&self, provider: &str) -> Vec<ModelMetricsSummary> {
        let events = self.events.read().await;
        let provider_events: Vec<&ProviderMetrics> = events
            .iter()
            .filter(|e| e.provider == provider)
            .collect();
        
        let mut model_map: std::collections::HashMap<String, Vec<&ProviderMetrics>> = std::collections::HashMap::new();
        for event in provider_events {
            model_map.entry(event.model.clone()).or_default().push(event);
        }
        
        model_map
            .into_iter()
            .map(|(model, events)| {
                let cloned_events: Vec<ProviderMetrics> = events.iter().cloned().cloned().collect();
                Self::compute_model_metrics_summary(provider.to_string(), model, &cloned_events)
            })
            .collect()
    }
}

fn percentile(values: &[f64], p: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }

    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let index = ((p * (sorted.len() - 1) as f64).round() as usize).min(sorted.len() - 1);
    Some(sorted[index])
}