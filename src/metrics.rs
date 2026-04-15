// YALR (Yet another LLM router) - Metrics system
// Event-based timeseries metrics with percentile support
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::sync::{broadcast, RwLock};

/// Provider metrics data point with timestamp and event
#[derive(Debug, Clone, Serialize)]
pub struct ProviderMetrics {
    pub provider: String,
    pub model: String,
    pub timestamp_ms: u64,
    pub event: MetricsEvent,
}

/// Metrics event types (value only, no provider/model info)
#[derive(Debug, Clone, Serialize)]
pub enum MetricsEvent {
    /// Time to First Token (ms)
    TTFT(u32),
    /// Output tokens per second
    OutputTokensPerSecond(f32),
    /// Input tokens per second (prefill speed)
    InputTokensPerSecond(f32),
    /// Total latency (ms)
    TotalLatency(u32),
    /// Input tokens used
    InputTokens(u32),
    /// Output tokens used
    OutputTokens(u32),
    /// Request success
    Success,
    /// Request failed with error details
    Failure(FailureDetails),
    /// Provider load event (in-flight requests)
    ProviderLoad {
        in_flight: u32,
        max_concurrency: Option<u32>,
    },
}

/// Error details for failure events
#[derive(Debug, Clone, Serialize)]
pub struct FailureDetails {
    pub error_type: ErrorType,
    pub error_code: Option<String>,
    pub error_message: String,
    pub retry_after_ms: Option<u64>,
    pub status_code: Option<u16>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum ErrorType {
    RateLimit,
    ServerError,
    Timeout,
    Authentication,
    NotFound,
    Other,
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
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            provider: provider.clone(),
            model: model.clone(),
            event: event.clone(),
        };
        tracing::info!(
            provider = %provider,
            model = %model,
            event = ?event,
            "Metrics event emitted"
        );
        let _ = self.sender.send(metrics);
    }

    pub fn emit_ttft(&self, provider: &str, model: &str, value_ms: u32) {
        self.emit(provider.to_string(), model.to_string(), MetricsEvent::TTFT(value_ms));
    }

    pub fn emit_output_tokens_per_second(&self, provider: &str, model: &str, value: f32) {
        self.emit(provider.to_string(), model.to_string(), MetricsEvent::OutputTokensPerSecond(value));
    }

    pub fn emit_input_tokens_per_second(&self, provider: &str, model: &str, value: f32) {
        self.emit(provider.to_string(), model.to_string(), MetricsEvent::InputTokensPerSecond(value));
    }

    pub fn emit_total_latency(&self, provider: &str, model: &str, value_ms: u32) {
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

    pub fn emit_failure(
        &self,
        provider: &str,
        model: &str,
        error_type: ErrorType,
        error_message: &str,
    ) {
        self.emit_failure_with_details(
            provider,
            model,
            error_type,
            None,
            error_message,
            None,
            None,
        );
    }

    pub fn emit_failure_with_details(
        &self,
        provider: &str,
        model: &str,
        error_type: ErrorType,
        error_code: Option<String>,
        error_message: &str,
        retry_after_ms: Option<u64>,
        status_code: Option<u16>,
    ) {
        let details = FailureDetails {
            error_type,
            error_code,
            error_message: error_message.to_string(),
            retry_after_ms,
            status_code,
        };
        self.emit(
            provider.to_string(),
            model.to_string(),
            MetricsEvent::Failure(details),
        );
    }

    pub fn emit_rate_limit(
        &self,
        provider: &str,
        model: &str,
        retry_after_ms: u64,
        status_code: Option<u16>,
    ) {
        self.emit_failure_with_details(
            provider,
            model,
            ErrorType::RateLimit,
            None,
            "Rate limit exceeded",
            Some(retry_after_ms),
            status_code,
        );
    }

    pub fn emit_provider_load(&self, provider: &str, in_flight: u32, max_concurrency: Option<u32>) {
        self.emit(
            provider.to_string(),
            String::new(),
            MetricsEvent::ProviderLoad {
                in_flight,
                max_concurrency,
            },
        );
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

use std::time::{Duration, Instant};

/// Summary of provider metrics for routing decisions
#[derive(Debug, Clone)]
pub struct ProviderMetricsSummary {
    pub provider: String,
    pub p90_ttft: Option<u32>,
    pub p90_output_tokens_per_second: Option<f32>,
    pub p90_input_tokens_per_second: Option<f32>,
    pub avg_latency: Option<f32>,
    pub success_rate: Option<f32>,
}

/// Summary of model-specific metrics for routing decisions
#[derive(Debug, Clone)]
pub struct ModelMetricsSummary {
    pub provider: String,
    pub model: String,
    pub p90_ttft: Option<u32>,
    pub p90_output_tokens_per_second: Option<f32>,
    pub p90_input_tokens_per_second: Option<f32>,
    pub avg_latency: Option<f32>,
    pub success_rate: Option<f32>,
}

/// In-memory timeseries store for metrics with percentile support
#[derive(Clone)]
pub struct MetricsStore {
    emitter: MetricsEmitter,
    /// Store recent events for percentile calculations (wrapped in Arc<RwLock> for shared access)
    events: Arc<RwLock<VecDeque<ProviderMetrics>>>,
    /// Track provider health states
    provider_health: Arc<RwLock<std::collections::HashMap<String, ProviderHealthState>>>,
    /// Track per-provider in-flight request counts
    provider_in_flight: Arc<RwLock<std::collections::HashMap<String, Arc<AtomicU32>>>>,
    max_events: usize,
    health_config: HealthConfig,
}

/// Type alias for MetricsStore - now cloneable with internal Arc<RwLock>
pub type SharedMetricsStore = MetricsStore;

impl MetricsStore {
    pub fn new(emitter: MetricsEmitter, max_events: usize) -> Self {
        Self::with_health_config(emitter, max_events, None)
    }

    pub fn with_health_config(
        emitter: MetricsEmitter,
        max_events: usize,
        health_config: Option<HealthConfig>,
    ) -> Self {
        Self {
            emitter,
            events: Arc::new(RwLock::new(VecDeque::with_capacity(max_events))),
            provider_health: Arc::new(RwLock::new(std::collections::HashMap::new())),
            provider_in_flight: Arc::new(RwLock::new(std::collections::HashMap::new())),
            max_events,
            health_config: health_config.unwrap_or_default(),
        }
    }

    pub fn emitter(&self) -> &MetricsEmitter {
        &self.emitter
    }

    /// Register a provider for in-flight tracking
    pub async fn register_provider(&self, provider_name: &str) {
        let mut load = self.provider_in_flight.write().await;
        load.entry(provider_name.to_string())
            .or_insert_with(|| Arc::new(AtomicU32::new(0)));
    }

    /// Increment in-flight count for a provider and return the new count
    pub async fn increment_in_flight(&self, provider_name: &str) -> u32 {
        let load = self.provider_in_flight.read().await;
        if let Some(counter) = load.get(provider_name) {
            counter.fetch_add(1, Ordering::SeqCst) + 1
        } else {
            drop(load);
            let mut write_load = self.provider_in_flight.write().await;
            let counter = write_load
                .entry(provider_name.to_string())
                .or_insert_with(|| Arc::new(AtomicU32::new(0)))
                .clone();
            drop(write_load);
            counter.fetch_add(1, Ordering::SeqCst) + 1
        }
    }

    /// Decrement in-flight count for a provider and return the new count
    pub async fn decrement_in_flight(&self, provider_name: &str) -> u32 {
        let load = self.provider_in_flight.read().await;
        if let Some(counter) = load.get(provider_name) {
            let current = counter.fetch_sub(1, Ordering::SeqCst) - 1;
            current
        } else {
            0
        }
    }

    /// Get current in-flight count for a provider
    pub async fn get_in_flight(&self, provider_name: &str) -> u32 {
        let load = self.provider_in_flight.read().await;
        load.get(provider_name)
            .map(|c| c.load(Ordering::SeqCst))
            .unwrap_or(0)
    }

    /// Record a metrics event and update health state
    pub async fn record(&self, event: ProviderMetrics) {
        let provider = event.provider.clone();
        let model = event.model.clone();
        let event_type = format!("{:?}", event.event);
        
        let mut events = self.events.write().await;
        events.push_back(event.clone());
        if events.len() > self.max_events {
            events.pop_front();
        }
        
        self.update_health_from_event(&event).await;
        
        tracing::info!(
            provider = %provider,
            model = %model,
            event_type = %event_type,
            total_events = events.len(),
            "Metrics event recorded"
        );
    }

    /// Update provider health state from a metrics event
    async fn update_health_from_event(&self, event: &ProviderMetrics) {
        let mut health = self.provider_health.write().await;
        let provider_health = health
            .entry(event.provider.clone())
            .or_insert_with(|| ProviderHealthState::new(Some(self.health_config.clone())));

        match &event.event {
            MetricsEvent::Success => {
                provider_health.record_success();
            }
            MetricsEvent::Failure(details) => {
                let retry_after = details.retry_after_ms
                    .map(|ms| Duration::from_millis(ms));
                provider_health.record_failure(retry_after);
            }
            _ => {}
        }
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

    /// Calculate p90 output tokens per second for a provider/model (model optional)
    pub async fn p90_output_tokens_per_second(&self, provider: &str, model: Option<&str>) -> Option<f32> {
        let events = self.get_events_for(provider, model).await;
        let values: Vec<f32> = events
            .iter()
            .filter_map(|e| match &e.event {
                MetricsEvent::OutputTokensPerSecond(value) => Some(*value),
                _ => None,
            })
            .collect();

        percentile(&values, 0.90)
    }

    /// Calculate p90 input tokens per second for a provider/model (model optional)
    pub async fn p90_input_tokens_per_second(&self, provider: &str, model: Option<&str>) -> Option<f32> {
        let events = self.get_events_for(provider, model).await;
        let values: Vec<f32> = events
            .iter()
            .filter_map(|e| match &e.event {
                MetricsEvent::InputTokensPerSecond(value) => Some(*value),
                _ => None,
            })
            .collect();

        percentile(&values, 0.90)
    }

    /// Calculate p90 TTFT for a provider/model (model optional)
    pub async fn p90_ttft(&self, provider: &str, model: Option<&str>) -> Option<u32> {
        let events = self.get_events_for(provider, model).await;
        let values: Vec<u32> = events
            .iter()
            .filter_map(|e| match &e.event {
                MetricsEvent::TTFT(value_ms) => Some(*value_ms),
                _ => None,
            })
            .collect();

        percentile(&values, 0.90)
    }

    /// Calculate average latency for a provider (model aggregated)
    pub async fn avg_latency(&self, provider: &str, model: Option<&str>) -> Option<f32> {
        let events = self.get_events_for(provider, model).await;
        let values: Vec<f32> = events
            .iter()
            .filter_map(|e| match &e.event {
                MetricsEvent::TotalLatency(value_ms) => Some(*value_ms as f32),
                _ => None,
            })
            .collect();

        if values.is_empty() {
            None
        } else {
            Some(values.iter().sum::<f32>() / values.len() as f32)
        }
    }

    /// Calculate success rate for a provider (model aggregated)
    /// Only counts Success and Failure events in the denominator
    pub async fn success_rate(&self, provider: &str, model: Option<&str>) -> Option<f64> {
        let events = self.get_events_for(provider, model).await;
        let outcomes: Vec<_> = events
            .iter()
            .filter(|e| matches!(e.event, MetricsEvent::Success | MetricsEvent::Failure(_)))
            .collect();

        if outcomes.is_empty() {
            return None;
        }

        let successes = outcomes
            .iter()
            .filter(|e| matches!(e.event, MetricsEvent::Success))
            .count();

        Some(successes as f64 / outcomes.len() as f64)
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
        let ttft_values: Vec<u32> = events
            .iter()
            .filter_map(|e| match &e.event {
                MetricsEvent::TTFT(v) => Some(*v),
                _ => None,
            })
            .collect();
        
        let output_tps_values: Vec<f32> = events
            .iter()
            .filter_map(|e| match &e.event {
                MetricsEvent::OutputTokensPerSecond(v) => Some(*v),
                _ => None,
            })
            .collect();
        
        let input_tps_values: Vec<f32> = events
            .iter()
            .filter_map(|e| match &e.event {
                MetricsEvent::InputTokensPerSecond(v) => Some(*v),
                _ => None,
            })
            .collect();
        
        let latency_values: Vec<f32> = events
            .iter()
            .filter_map(|e| match &e.event {
                MetricsEvent::TotalLatency(v) => Some(*v as f32),
                _ => None,
            })
            .collect();
        
        let outcome_events: Vec<_> = events.iter().filter(|e| matches!(e.event, MetricsEvent::Success | MetricsEvent::Failure(_))).collect();
        let successes = outcome_events.iter().filter(|e| matches!(e.event, MetricsEvent::Success)).count();
        let total = outcome_events.len();

        ProviderMetricsSummary {
            provider,
            p90_ttft: percentile(&ttft_values, 0.90),
            p90_output_tokens_per_second: percentile(&output_tps_values, 0.90),
            p90_input_tokens_per_second: percentile(&input_tps_values, 0.90),
            avg_latency: if latency_values.is_empty() { None } else { Some(latency_values.iter().sum::<f32>() / latency_values.len() as f32) },
            success_rate: if total == 0 { None } else { Some(successes as f32 / total as f32) },
        }
    }

    /// Compute model-specific metrics summary (internal helper)
    fn compute_model_metrics_summary(provider: String, model: String, events: &[ProviderMetrics]) -> ModelMetricsSummary {
        let ttft_values: Vec<u32> = events
            .iter()
            .filter_map(|e| match &e.event {
                MetricsEvent::TTFT(v) => Some(*v),
                _ => None,
            })
            .collect();
        
        let output_tps_values: Vec<f32> = events
            .iter()
            .filter_map(|e| match &e.event {
                MetricsEvent::OutputTokensPerSecond(v) => Some(*v),
                _ => None,
            })
            .collect();
        
        let input_tps_values: Vec<f32> = events
            .iter()
            .filter_map(|e| match &e.event {
                MetricsEvent::InputTokensPerSecond(v) => Some(*v),
                _ => None,
            })
            .collect();
        
        let latency_values: Vec<f32> = events
            .iter()
            .filter_map(|e| match &e.event {
                MetricsEvent::TotalLatency(v) => Some(*v as f32),
                _ => None,
            })
            .collect();
        
        let outcome_events: Vec<_> = events.iter().filter(|e| matches!(e.event, MetricsEvent::Success | MetricsEvent::Failure(_))).collect();
        let successes = outcome_events.iter().filter(|e| matches!(e.event, MetricsEvent::Success)).count();
        let total = outcome_events.len();

        ModelMetricsSummary {
            provider,
            model,
            p90_ttft: percentile(&ttft_values, 0.90),
            p90_output_tokens_per_second: percentile(&output_tps_values, 0.90),
            p90_input_tokens_per_second: percentile(&input_tps_values, 0.90),
            avg_latency: if latency_values.is_empty() { None } else { Some(latency_values.iter().sum::<f32>() / latency_values.len() as f32) },
            success_rate: if total == 0 { None } else { Some(successes as f32 / total as f32) },
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

    /// Get health state for a provider
    pub async fn get_provider_health(&self, provider: &str) -> HealthState {
        let health = self.provider_health.read().await;
        health
            .get(provider)
            .map(|h| h.state())
            .unwrap_or(HealthState::Healthy)
    }

    /// Check if a provider is available for routing
    pub async fn is_provider_available(&self, provider: &str) -> bool {
        let health = self.provider_health.read().await;
        health
            .get(provider)
            .map(|h| h.is_available())
            .unwrap_or(true)
    }

    /// Get recommended backoff duration for a provider
    pub async fn get_provider_backoff(&self, provider: &str) -> Duration {
        let health = self.provider_health.read().await;
        health
            .get(provider)
            .map(|h| h.wait_time())
            .unwrap_or_default()
    }

    /// Get recent failure count for a provider
    pub async fn get_recent_failures(&self, provider: &str) -> u32 {
        let health = self.provider_health.read().await;
        health
            .get(provider)
            .map(|h| h.consecutive_failures)
            .unwrap_or(0)
    }

    /// Get current provider load (in-flight requests)
    pub async fn get_provider_load(&self, provider: &str) -> Option<(u32, Option<u32>)> {
        let events = self.events.read().await;
        let provider_events: Vec<&ProviderMetrics> = events
            .iter()
            .filter(|e| e.provider == provider)
            .collect();
        
        provider_events
            .iter()
            .rev()
            .find_map(|e| match &e.event {
                MetricsEvent::ProviderLoad { in_flight, max_concurrency } => {
                    Some((*in_flight, *max_concurrency))
                }
                _ => None,
            })
    }

    /// Get load score for routing (0.0 = fully loaded, 1.0 = completely idle)
    pub async fn get_provider_load_score(&self, provider: &str) -> Option<f32> {
        let (in_flight, max_concurrency) = self.get_provider_load(provider).await?;
        
        if let Some(max) = max_concurrency {
            if max == 0 {
                Some(0.0)
            } else {
                Some(((max - in_flight) as f32 / max as f32).max(0.0))
            }
        } else {
            Some(1.0)
        }
    }

    /// Compute health from recent metrics (for external health calculation)
    pub async fn compute_health_from_metrics(&self, provider: &str) -> (HealthState, f32, u32) {
        let events = self.get_events_for(provider, None).await;
        
        let total = events.len();
        if total == 0 {
            return (HealthState::Healthy, 1.0, 0);
        }

        let successes = events
            .iter()
            .filter(|e| matches!(e.event, MetricsEvent::Success))
            .count();

        let recent_failures = events
            .iter()
            .filter(|e| matches!(e.event, MetricsEvent::Failure(_)))
            .count() as u32;

        let success_rate = successes as f32 / total as f32;

        let state = if success_rate < 0.5 {
            HealthState::Unhealthy
        } else if success_rate < 0.8 {
            HealthState::Degraded
        } else {
            HealthState::Healthy
        };

        (state, success_rate, recent_failures)
    }
}

fn percentile<T: Copy + PartialOrd>(values: &[T], p: f32) -> Option<T> {
    if values.is_empty() {
        return None;
    }

    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let index = ((p * (sorted.len() - 1) as f32).round() as usize).min(sorted.len() - 1);
    Some(sorted[index])
}

#[cfg(test)]
mod health_tests {
    use super::*;

    #[test]
    fn test_initial_state_healthy() {
        let health = ProviderHealthState::new(None);
        assert_eq!(health.state(), HealthState::Healthy);
        assert!(health.is_available());
    }

    #[test]
    fn test_record_success() {
        let mut health = ProviderHealthState::new(Some(HealthConfig {
            failure_threshold: 3,
            ..Default::default()
        }));

        health.record_failure(None);
        health.record_failure(None);
        health.record_success();

        assert_eq!(health.state(), HealthState::Degraded);
        assert_eq!(health.consecutive_failures, 0);
    }

    #[test]
    fn test_exponential_backoff() {
        let config = HealthConfig {
            base_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(1),
            ..Default::default()
        };
        let mut health = ProviderHealthState::new(Some(config));

        health.record_failure(None);
        let backoff1 = health.calculate_backoff();
        assert!(backoff1 >= Duration::from_millis(100));

        health.record_failure(None);
        let backoff2 = health.calculate_backoff();
        assert!(backoff2 >= backoff1);
        assert!(backoff2 <= Duration::from_secs(1));
    }

   #[test]
    fn test_retry_after_respected() {
        let retry_after = Duration::from_secs(30);
        let mut health = ProviderHealthState::new(None);

        health.record_failure(Some(retry_after));
        let backoff = health.calculate_backoff();
        
        assert!(backoff >= Duration::from_secs(29));
        assert!(backoff <= Duration::from_secs(30));
    }

    #[test]
    fn test_recovery_after_success() {
        let config = HealthConfig {
            failure_threshold: 5,
            recovery_window: Duration::from_millis(100),
            ..Default::default()
        };
        let mut health = ProviderHealthState::new(Some(config));

        for _ in 0..5 {
            health.record_failure(None);
        }

        assert_eq!(health.state(), HealthState::Unhealthy);

        std::thread::sleep(Duration::from_millis(150));
        health.record_success();

        assert_eq!(health.state(), HealthState::Healthy);
    }

    #[test]
    fn test_unhealthy_after_threshold() {
        let config = HealthConfig {
            failure_threshold: 3,
            ..Default::default()
        };
        let mut health = ProviderHealthState::new(Some(config));

        assert!(health.is_available());

        health.record_failure(None);
        health.record_failure(None);
        health.record_failure(None);

        assert!(!health.is_available());
        assert_eq!(health.state(), HealthState::Unhealthy);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum HealthState {
    Healthy,
    Degraded,
    Unhealthy,
}

#[derive(Clone)]
pub struct ProviderHealthState {
    state: HealthState,
    consecutive_failures: u32,
    last_failure_time: Option<Instant>,
    rate_limit_until: Option<Instant>,
    config: HealthConfig,
}

#[derive(Clone)]
pub struct HealthConfig {
    pub failure_threshold: u32,
    pub recovery_window: Duration,
    pub base_backoff: Duration,
    pub max_backoff: Duration,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            recovery_window: Duration::from_secs(60),
            base_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(60),
        }
    }
}

impl ProviderHealthState {
    pub fn new(config: Option<HealthConfig>) -> Self {
        Self {
            state: HealthState::Healthy,
            consecutive_failures: 0,
            last_failure_time: None,
            rate_limit_until: None,
            config: config.unwrap_or_default(),
        }
    }

    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
        
        if self.state == HealthState::Degraded {
            if let Some(last_failure) = self.last_failure_time {
                if Instant::now().duration_since(last_failure) > self.config.recovery_window {
                    self.state = HealthState::Healthy;
                }
            }
        }
    }

    pub fn record_failure(&mut self, retry_after: Option<Duration>) {
        self.consecutive_failures += 1;
        self.last_failure_time = Some(Instant::now());

        if let Some(retry_after) = retry_after {
            self.rate_limit_until = Some(Instant::now() + retry_after);
        }

        if self.consecutive_failures >= self.config.failure_threshold {
            self.state = HealthState::Unhealthy;
        } else if self.consecutive_failures >= self.config.failure_threshold / 2 {
            self.state = HealthState::Degraded;
        }
    }

    pub fn update_from_metrics(&mut self, success_rate: f32, recent_failures: u32) {
        self.consecutive_failures = recent_failures;

        if success_rate < 0.5 {
            self.state = HealthState::Unhealthy;
        } else if success_rate < 0.8 {
            self.state = HealthState::Degraded;
        } else {
            if self.consecutive_failures < self.config.failure_threshold / 2 {
                self.state = HealthState::Healthy;
            }
        }
    }

   pub fn state(&self) -> HealthState {
        if let Some(rate_limit_until) = self.rate_limit_until {
            if Instant::now() < rate_limit_until {
                return HealthState::Unhealthy;
            }
        }
        
        if self.state == HealthState::Unhealthy && self.consecutive_failures == 0 {
            return HealthState::Healthy;
        }
        
        self.state.clone()
    }

    pub fn is_available(&self) -> bool {
        self.state() != HealthState::Unhealthy
    }

    pub fn calculate_backoff(&self) -> Duration {
        if let Some(rate_limit_until) = self.rate_limit_until {
            if Instant::now() < rate_limit_until {
                return rate_limit_until.duration_since(Instant::now());
            }
        }

        if self.consecutive_failures == 0 {
            return Duration::from_millis(0);
        }

        let exponential_backoff = self
            .config
            .base_backoff
            .checked_mul(
                2_u32
                    .saturating_pow(self.consecutive_failures.saturating_sub(1).min(10)),
            )
            .unwrap_or(self.config.max_backoff);

        exponential_backoff.min(self.config.max_backoff)
    }

    pub fn wait_time(&self) -> Duration {
        if !self.is_available() {
            self.calculate_backoff()
        } else {
            Duration::from_millis(0)
        }
    }
}