// YALR (Yet another LLM router) - Metrics system
// Event-based timeseries metrics with percentile support
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{RwLock, broadcast};
use tokio::task::JoinHandle;
use crate::router::ModelRuntimeInfo;
use crate::providers::CurrencyAmount;

type InstantMillis = u64;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

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
    /// Provider balance snapshot (account credit, upstream cost tracking)
    Balance(CurrencyAmount),
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

/// Metrics emitter that sends events via broadcast channel and records in store
#[derive(Clone)]
pub struct MetricsEmitter {
    sender: broadcast::Sender<ProviderMetrics>,
    store: Arc<Mutex<VecDeque<ProviderMetrics>>>,
    max_events: usize,
    total_requests: Arc<AtomicU64>,
    total_successes: Arc<AtomicU64>,
    total_failures: Arc<AtomicU64>,
}

impl MetricsEmitter {
    pub fn with_store(buffer_size: usize, store: Arc<Mutex<VecDeque<ProviderMetrics>>>, max_events: usize) -> Self {
        let (sender, _) = broadcast::channel(buffer_size);
        Self { 
            sender,
            store,
            max_events,
            total_requests: Arc::new(AtomicU64::new(0)),
            total_successes: Arc::new(AtomicU64::new(0)),
            total_failures: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn receiver(&self) -> MetricsReceiver {
        MetricsReceiver {
            receiver: self.sender.subscribe(),
        }
    }

    /// Get total request counts (success + failure) since process start
    pub fn get_total_requests(&self) -> (u64, u64, u64) {
        (
            self.total_requests.load(Ordering::Relaxed),
            self.total_successes.load(Ordering::Relaxed),
            self.total_failures.load(Ordering::Relaxed),
        )
    }

    fn emit(&self, provider: String, model: String, event: MetricsEvent) {
        let metrics = ProviderMetrics {
            timestamp_ms: now_ms(),
            provider: provider.clone(),
            model: model.clone(),
            event: event.clone(),
        };

        // Increment monotonic counters for outcome events
        match &event {
            MetricsEvent::Success => {
                self.total_requests.fetch_add(1, Ordering::Relaxed);
                self.total_successes.fetch_add(1, Ordering::Relaxed);
            }
            MetricsEvent::Failure(_) => {
                self.total_requests.fetch_add(1, Ordering::Relaxed);
                self.total_failures.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }

        tracing::info!(
            provider = %provider,
            model = %model,
            event = ?event,
            "Metrics event emitted"
        );
        let _ = self.sender.send(metrics.clone());
        
        // Record directly in the store (synchronously with Mutex)
        let mut events = self.store.lock().unwrap();
        events.push_back(metrics);
        while events.len() > events.capacity() {
            events.pop_front();
        }
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

    /// Emit a balance snapshot for a provider.
    pub fn emit_balance(&self, provider: &str, amount: CurrencyAmount) {
        self.emit(provider.to_string(), String::new(), MetricsEvent::Balance(amount));
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
    /// Store recent events for percentile calculations (wrapped in Arc<Mutex> for shared access)
    pub(crate) events: Arc<Mutex<VecDeque<ProviderMetrics>>>,
    /// Track per-provider in-flight request counts
    provider_in_flight: Arc<RwLock<std::collections::HashMap<String, Arc<AtomicU32>>>>,
    /// Cache provider runtime info (including max_concurrency)
    provider_runtime_info: Arc<RwLock<std::collections::HashMap<String, ModelRuntimeInfo>>>,
    max_events: usize,
    health_config: HealthConfig,
    /// History snapshots for graphing (timestamp -> list of provider+model metric snapshots)
    history: Arc<RwLock<VecDeque<MetricsSnapshot>>>,
    max_history_snapshots: usize,
}

/// Type alias for MetricsStore - now cloneable with internal Arc<Mutex>
pub type SharedMetricsStore = MetricsStore;

/// Snapshot of provider+model metrics at a point in time (for history/graphing)
#[derive(Debug, Clone, Serialize)]
pub struct MetricsSnapshot {
    pub timestamp_ms: u64,
    pub providers: Vec<ProviderMetricSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderMetricSnapshot {
    pub provider: String,
    pub model: String,
    pub p50_ttft_ms: Option<u32>,
    pub p90_ttft_ms: Option<u32>,
    pub p50_output_tps: Option<f32>,
    pub p90_output_tps: Option<f32>,
    pub p50_input_tps: Option<f32>,
    pub p90_input_tps: Option<f32>,
    pub avg_latency_ms: Option<f32>,
    pub success_rate: Option<f32>,
}

impl MetricsStore {
    pub fn new(max_events: usize) -> Self {
        Self::with_health_config(max_events, None)
    }

    pub fn with_health_config(max_events: usize, health_config: Option<HealthConfig>) -> Self {
        let events = Arc::new(Mutex::new(VecDeque::with_capacity(max_events)));
        let emitter = MetricsEmitter::with_store(10000, events.clone(), max_events);
        let history_max = 288; // 24 hours at 5-minute intervals
        Self {
            emitter,
            events,
            provider_in_flight: Arc::new(RwLock::new(std::collections::HashMap::new())),
            provider_runtime_info: Arc::new(RwLock::new(std::collections::HashMap::new())),
            max_events,
            health_config: health_config.unwrap_or_default(),
            history: Arc::new(RwLock::new(VecDeque::with_capacity(history_max))),
            max_history_snapshots: history_max,
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

    /// Unregister a provider from tracking
    pub async fn unregister_provider(&self, provider_name: &str) {
        let mut load = self.provider_in_flight.write().await;
        load.remove(provider_name);
        let mut runtime_info = self.provider_runtime_info.write().await;
        runtime_info.remove(provider_name);
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
            let prev = counter.fetch_sub(1, Ordering::SeqCst);
            // Prevent underflow - if already 0, add it back and return 0
            if prev == 0 {
                counter.fetch_add(1, Ordering::SeqCst);
                0
            } else {
                prev - 1
            }
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

    /// Set runtime info for a provider (caches max_concurrency and other details)
    pub async fn set_provider_runtime_info(&self, provider_name: &str, runtime_info: ModelRuntimeInfo) {
        let mut info = self.provider_runtime_info.write().await;
        info.insert(provider_name.to_string(), runtime_info);
    }

    /// Get max concurrency for a provider
    pub async fn get_provider_max_concurrency(&self, provider_name: &str) -> Option<u32> {
        let info = self.provider_runtime_info.read().await;
        info.get(provider_name).and_then(|r| r.max_concurrency())
    }

    /// Record a metrics event
    pub async fn record(&self, event: ProviderMetrics) {
        let provider = event.provider.clone();
        let model = event.model.clone();
        let event_type = format!("{:?}", event.event);
        
        let total = {
            let mut events = self.events.lock().unwrap();
            events.push_back(event.clone());
            if events.len() > self.max_events {
                events.pop_front();
            }
            events.len()
        };

        tracing::info!(
            provider = %provider,
            model = %model,
            event_type = %event_type,
            total,
            "Metrics event recorded"
        );
    }

    /// Get all events for a specific provider and model (model optional)
    pub async fn get_events_for(&self, provider: &str, model: Option<&str>) -> Vec<ProviderMetrics> {
        let events = self.events.lock().unwrap();
        events
            .iter()
            .filter(|e| {
                let provider_match = e.provider == provider;
                let model_match = model.is_none_or(|m| e.model == m);
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
        let events = self.events.lock().unwrap();
        events
            .iter()
            .rev()
            .take(n)
            .cloned()
            .collect()
    }

    /// Take a snapshot of current provider+model metrics and append to history
    pub async fn take_snapshot(&self) {
        let now = now_ms();
        
        let provider_snapshots: Vec<ProviderMetricSnapshot> = {
            let events = self.events.lock().unwrap();
            let mut groups: std::collections::HashMap<String, Vec<&ProviderMetrics>> = std::collections::HashMap::new();
            for e in events.iter() {
                let key = format!("{}\0{}", e.provider, e.model);
                groups.entry(key).or_default().push(e);
            }
            
            groups.into_iter().filter_map(|(key, evts)| {
                let parts: Vec<&str> = key.splitn(2, '\0').collect();
                if parts.len() != 2 { return None; }
                let provider = parts[0].to_string();
                let model = parts[1].to_string();
                
                let ttft_vals: Vec<u32> = evts.iter().filter_map(|e| match &e.event { MetricsEvent::TTFT(v) => Some(*v), _ => None }).collect();
                let out_tps_vals: Vec<f32> = evts.iter().filter_map(|e| match &e.event { MetricsEvent::OutputTokensPerSecond(v) => Some(*v), _ => None }).collect();
                let in_tps_vals: Vec<f32> = evts.iter().filter_map(|e| match &e.event { MetricsEvent::InputTokensPerSecond(v) => Some(*v), _ => None }).collect();
                let lat_vals: Vec<f32> = evts.iter().filter_map(|e| match &e.event { MetricsEvent::TotalLatency(v) => Some(*v as f32), _ => None }).collect();
                
                let outcomes: Vec<&&ProviderMetrics> = evts.iter().filter(|e| matches!(e.event, MetricsEvent::Success | MetricsEvent::Failure(_))).collect();
                let successes = outcomes.iter().filter(|e| matches!(e.event, MetricsEvent::Success)).count();
                let total_outcomes = outcomes.len();
                
                if outcomes.is_empty() {
                    return None;
                }
                
                Some(ProviderMetricSnapshot {
                    provider,
                    model,
                    p50_ttft_ms: percentile(&ttft_vals, 0.50),
                    p90_ttft_ms: percentile(&ttft_vals, 0.90),
                    p50_output_tps: percentile(&out_tps_vals, 0.50),
                    p90_output_tps: percentile(&out_tps_vals, 0.90),
                    p50_input_tps: percentile(&in_tps_vals, 0.50),
                    p90_input_tps: percentile(&in_tps_vals, 0.90),
                    avg_latency_ms: if lat_vals.is_empty() { None } else { Some(lat_vals.iter().sum::<f32>() / lat_vals.len() as f32) },
                    success_rate: if total_outcomes == 0 { None } else { Some(successes as f32 / total_outcomes as f32) },
                })
            }).collect()
        };
        
        let snapshot = MetricsSnapshot { timestamp_ms: now, providers: provider_snapshots };
        let mut history = self.history.write().await;
        history.push_back(snapshot);
        while history.len() > self.max_history_snapshots {
            history.pop_front();
        }
    }

    /// Get history snapshots (for graphing in admin UI)
    pub async fn get_history(&self) -> Vec<MetricsSnapshot> {
        let history = self.history.read().await;
        history.iter().cloned().collect()
    }

    /// Start background task to take periodic snapshots for history
    pub fn start_history_snapshots(&self, interval_secs: u64) -> JoinHandle<()> {
        let store = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                store.take_snapshot().await;
            }
        })
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
        let events = self.events.lock().unwrap();
        let provider_events: Vec<ProviderMetrics> = events
            .iter()
            .filter(|e| e.provider == provider)
            .cloned()
            .collect();
        
        Self::compute_metrics_summary(provider.to_string(), &provider_events)
    }

    /// Get model-specific metrics summary in a single lock acquisition
    pub async fn get_model_summary(&self, provider: &str, model: &str) -> ModelMetricsSummary {
        let events = self.events.lock().unwrap();
        let model_events: Vec<ProviderMetrics> = events
            .iter()
            .filter(|e| e.provider == provider && e.model == model)
            .cloned()
            .collect();
        
        Self::compute_model_metrics_summary(provider.to_string(), model.to_string(), &model_events)
    }

    /// Get summaries for all models of a provider
    pub async fn get_model_summaries_for_provider(&self, provider: &str) -> Vec<ModelMetricsSummary> {
        let events = self.events.lock().unwrap();
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

    /// Get health state for a provider - computed dynamically from recent metrics (last 5 minutes)
    pub async fn get_provider_health(&self, provider: &str) -> HealthState {
        let events = self.events.lock().unwrap();
        let now = std::time::SystemTime::now();
        let window_start = now - Duration::from_secs(300); // 5 minute window
        
        let provider_events: Vec<&ProviderMetrics> = events
            .iter()
            .filter(|e| {
                e.provider == provider
                    && e.timestamp_ms >= window_start.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64
            })
            .collect();
        
        // Check for balance issues in recent events
        let has_balance_issue = provider_events.iter().any(|e| {
            if let MetricsEvent::Balance(amount) = &e.event {
                match amount {
                    CurrencyAmount::Msats(m) => *m <= 0,
                    CurrencyAmount::Sats(s) => *s <= 0,
                    CurrencyAmount::UsdMicro(u) => *u <= 0,
                }
            } else {
                false
            }
        });
        
        // Check for recent failures
        let failure_count = provider_events
            .iter()
            .filter(|e| matches!(&e.event, MetricsEvent::Failure(_)))
            .count();
        
        // Compute health state
        if has_balance_issue {
            HealthState::Degraded
        } else if failure_count >= 5 {
            HealthState::Unhealthy
        } else if failure_count >= 2 {
            HealthState::Degraded
        } else {
            HealthState::Healthy
        }
    }

    /// Check if a provider is available for routing
    pub async fn is_provider_available(&self, provider: &str) -> bool {
        // Provider is available unless it's unhealthy
        // Degraded providers (including those with balance issues) are still available as fallback
        self.get_provider_health(provider).await != HealthState::Unhealthy
    }

    /// Get recommended backoff duration for a provider based on recent failures
    pub async fn get_provider_backoff(&self, provider: &str) -> Duration {
        let events = self.events.lock().unwrap();
        let now = std::time::SystemTime::now();
        let window_start = now - Duration::from_secs(300); // 5 minute window
        
        let provider_events: Vec<&ProviderMetrics> = events
            .iter()
            .filter(|e| {
                e.provider == provider
                    && e.timestamp_ms >= window_start.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64
            })
            .collect();
        
        // Count recent failures
        let failure_count = provider_events
            .iter()
            .filter(|e| matches!(&e.event, MetricsEvent::Failure(_)))
            .count() as u32;
        
        if failure_count == 0 {
            return Duration::from_millis(0);
        }
        
        // Exponential backoff: base * 2^failures, capped at 30 seconds
        let base_backoff = Duration::from_millis(100);
        let max_backoff = Duration::from_secs(30);
        
        let exponential_backoff = base_backoff
            .checked_mul(2_u32.saturating_pow(failure_count.saturating_sub(1).min(10)))
            .unwrap_or(max_backoff);
        
        exponential_backoff.min(max_backoff)
    }

    /// Get recent failure count for a provider (last 5 minutes)
    pub async fn get_recent_failures(&self, provider: &str) -> u32 {
        let events = self.events.lock().unwrap();
        let now = std::time::SystemTime::now();
        let window_start = now - Duration::from_secs(300); // 5 minute window
        
        events
            .iter()
            .filter(|e| {
                e.provider == provider
                    && e.timestamp_ms >= window_start.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64
                    && matches!(e.event, MetricsEvent::Failure(_))
            })
            .count() as u32
    }

    /// Get total request counts (success + failure) since process start
    pub fn get_total_requests(&self) -> (u64, u64, u64) {
        self.emitter.get_total_requests()
    }

    /// Get current provider load (in-flight requests)
    pub async fn get_provider_load(&self, provider: &str) -> Option<(u32, Option<u32>)> {
        let events = self.events.lock().unwrap();
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

    /// Get the most recent balance snapshot for a provider.
    pub async fn get_balance(&self, provider: &str) -> Option<CurrencyAmount> {
        let events = self.events.lock().unwrap();
        events
            .iter()
            .rev()
            .find_map(|e| {
                if e.provider == provider {
                    match &e.event {
                        MetricsEvent::Balance(amount) => Some(*amount),
                        _ => None,
                    }
                } else {
                    None
                }
            })
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


#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    Healthy,
    Degraded,
    Unhealthy,
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

