use crate::db::Database;
use crate::metrics::MetricsStore;
use crate::providers::{create_provider, Provider, ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage, ChatCompletionRequestSystemMessageContent};
use crate::router::strategies::ProviderEntry;
use crate::{ChatCompletionRequest, ChatCompletionResponse, ProviderError};
use crate::providers::StreamingChunk;
use async_openai::types::responses::{CreateResponse, Response as ApiResponse, InputParam, InputRole, MessageItem, InputMessage};
use async_stream::stream;
use futures::stream::BoxStream;
use futures::StreamExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Guard that decrements in-flight count when dropped.
/// Ensures in-flight tracking is correct even on early returns or panics.
struct InFlightGuard {
    metrics_store: MetricsStore,
    provider_name: String,
    decremented: bool,
}

impl InFlightGuard {
    fn new(metrics_store: MetricsStore, provider_name: String) -> Self {
        Self {
            metrics_store,
            provider_name,
            decremented: false,
        }
    }

    fn decrement(&mut self) {
        if !self.decremented {
            let metrics = self.metrics_store.clone();
            let name = self.provider_name.clone();
            tokio::spawn(async move {
                let _ = metrics.decrement_in_flight(&name).await;
                let current = metrics.get_in_flight(&name).await;
                let max_conc = metrics.get_provider_max_concurrency(&name).await;
                metrics.emitter().emit_provider_load(&name, current, max_conc);
            });
            self.decremented = true;
        }
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.decrement();
    }
}

struct RoutingTable {
    entries: Vec<ProviderEntry>,
    /// Counter for weighted round-robin. Each call to `collect_candidates`
    /// advances this so that over many requests the distribution matches the
    /// configured weights.
    rr_counter: AtomicUsize,
}

impl RoutingTable {
    fn new(entries: Vec<ProviderEntry>) -> Self {
        Self {
            entries,
            rr_counter: AtomicUsize::new(0),
        }
    }

    /// Return entries reordered for a single weighted round-robin step.
    ///
    /// Weighted round-robin works by expanding each entry into `weight` virtual
    /// slots, then cycling through those slots.  A 3:1 split produces the
    /// repeating sequence A A A B — so A gets 75 % of traffic, B gets 25 %.
    fn weighted_rr_order(&self) -> Vec<&ProviderEntry> {
        if self.entries.is_empty() {
            return vec![];
        }

        // If all weights are equal (or zero/negative), fall back to plain RR
        let first_weight = self.entries[0].weight.max(1);
        let all_equal = self.entries.iter().all(|e| e.weight.max(1) == first_weight);

        if all_equal {
            let idx = self.rr_counter.fetch_add(1, Ordering::Relaxed) % self.entries.len();
            // Rotate so the chosen index is first
            let mut ordered: Vec<&ProviderEntry> = self.entries.iter().collect();
            ordered.rotate_left(idx);
            return ordered;
        }

        // Weighted: expand into virtual slots, pick the next slot
        let total_weight: i32 = self.entries.iter().map(|e| e.weight.max(1)).sum();
        let slot = self.rr_counter.fetch_add(1, Ordering::Relaxed) % (total_weight as usize);

        // Determine which entry the slot belongs to
        let mut accumulated = 0i32;
        let mut primary_idx = 0;
        for (i, entry) in self.entries.iter().enumerate() {
            accumulated += entry.weight.max(1);
            if slot < accumulated as usize {
                primary_idx = i;
                break;
            }
        }

        // Rotate so the selected entry is first (the rest follow as fallbacks)
        let mut ordered: Vec<&ProviderEntry> = self.entries.iter().collect();
        ordered.rotate_left(primary_idx);
        ordered
    }
}

/// Helper struct for health check tasks that only needs metrics access
#[derive(Clone)]
struct HealthCheckRouter {
    metrics_store: MetricsStore,
}

impl HealthCheckRouter {
    async fn record_success(&self, provider_name: &str) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        self.metrics_store.record(crate::metrics::ProviderMetrics {
            provider: provider_name.to_string(),
            model: String::new(),
            timestamp_ms: now,
            event: crate::metrics::MetricsEvent::Success,
        }).await;
    }

    async fn record_failure(&self, provider_name: &str, error_type: crate::metrics::ErrorType, message: &str) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        self.metrics_store.record(crate::metrics::ProviderMetrics {
            provider: provider_name.to_string(),
            model: String::new(),
            timestamp_ms: now,
            event: crate::metrics::MetricsEvent::Failure(crate::metrics::FailureDetails {
                error_type,
                error_code: None,
                error_message: message.to_string(),
                retry_after_ms: None,
                status_code: None,
            }),
        }).await;
    }
}

pub struct Router {
    db: Arc<Database>,
    metrics_store: MetricsStore,
    providers: RwLock<HashMap<String, Arc<dyn Provider>>>,
    routing_tables: RwLock<HashMap<String, RoutingTable>>,
    max_retries: u32,
    health_check_handles: RwLock<HashMap<String, tokio::task::JoinHandle<()>>>,
}

impl Router {
    pub fn new(
        metrics_store: MetricsStore,
        db: Arc<Database>,
    ) -> Self {
        Self {
            db,
            metrics_store,
            providers: RwLock::new(HashMap::new()),
            routing_tables: RwLock::new(HashMap::new()),
            max_retries: 3,
            health_check_handles: RwLock::new(HashMap::new()),
        }
    }

    pub async fn reload_config(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let provider_records = self.db.list_providers().await?;

        let mut providers = HashMap::new();
        let mut id_to_slug: HashMap<i64, String> = HashMap::new();

        for record in &provider_records {
            let provider = create_provider(
                &record.name,
                Some(&record.slug),
                &record.base_url,
                record.api_key.as_deref(),
                record.provider_type,
            );
            self.metrics_store.register_provider(&record.name).await;
            id_to_slug.insert(record.id, record.slug.clone());
            providers.insert(record.slug.clone(), provider);
        }

        let mut tables = HashMap::new();

        let routing_configs = self.db.list_routing_configs().await?;
        for rc in &routing_configs {
            let rcp_records = self.db.list_active_routing_config_providers(rc.id).await?;
            let mut entries = Vec::new();

            for rcp in &rcp_records {
                let slug = match id_to_slug.get(&rcp.provider_id) {
                    Some(s) => s,
                    None => continue,
                };
                let provider = match providers.get(slug) {
                    Some(p) => p,
                    None => continue,
                };
                entries.push(ProviderEntry {
                    provider: provider.clone(),
                    model_override: rcp.model.clone(),
                    weight: rcp.weight,
                });
            }

            tracing::info!(
                routing_config = rc.name,
                strategy = rc.strategy,
                provider_count = entries.len(),
                "Loaded routing config"
            );

            tables.insert(
                rc.name.clone(),
                RoutingTable::new(entries),
            );
        }

        let model_records = self.db.list_models().await?;
        let mp_records = self.db.list_model_providers().await?;

        let mut model_id_to_name: HashMap<i64, String> = HashMap::new();
        for model in &model_records {
            model_id_to_name.insert(model.id, model.name.clone());
        }

        for mp in &mp_records {
            if !mp.is_active {
                continue;
            }

            let model_name = match model_id_to_name.get(&mp.model_id) {
                Some(n) => n,
                None => continue,
            };

            if tables.contains_key(model_name.as_str()) {
                continue;
            }

            let slug = match id_to_slug.get(&mp.provider_id) {
                Some(s) => s,
                None => continue,
            };
            let provider = match providers.get(slug) {
                Some(p) => p,
                None => continue,
            };

            tables
                .entry(model_name.clone())
                .or_insert_with(|| RoutingTable::new(Vec::new()))
                .entries.push(ProviderEntry {
                    provider: provider.clone(),
                    model_override: None,
                    weight: mp.weight,
                });
        }

        if !tables.contains_key("default") && !providers.is_empty() {
            let entries: Vec<ProviderEntry> = providers
                .values()
                .map(|provider| ProviderEntry {
                    provider: provider.clone(),
                    model_override: None,
                    weight: 100,
                })
                .collect();

            tables.insert(
                "default".to_string(),
                RoutingTable::new(entries),
            );
        }

        *self.providers.write().await = providers;
        *self.routing_tables.write().await = tables;

        let provider_count = self.providers.read().await.len();
        let table_names: Vec<String> = self.routing_tables.read().await.keys().cloned().collect();
        tracing::info!(
            providers_loaded = provider_count,
            routing_tables = ?table_names,
            "Router config reloaded"
        );

        // Restart health checks with the new config
        if let Err(e) = self.start_health_checks().await {
            tracing::warn!(error = %e, "Failed to start health checks after config reload");
        }

        Ok(())
    }

    /// Start background health check tasks that cover all loaded providers.
    /// The interval and timeout are taken from the first routing config that has
    /// health checks enabled, falling back to 30s / 10s defaults.
    pub async fn start_health_checks(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut handles = self.health_check_handles.write().await;
        
        // Cancel existing health check tasks
        for (_, handle) in handles.drain() {
            handle.abort();
        }
        
        // Gather all loaded providers (deduplicated by name)
        let provs = self.providers.read().await;
        let mut all_providers = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for provider in provs.values() {
            if seen.insert(provider.name().to_string()) {
                all_providers.push(provider.clone());
            }
        }
        drop(provs);

        if all_providers.is_empty() {
            return Ok(());
        }

        // Pick interval/timeout from the first routing config with health checks enabled,
        // or fall back to sensible defaults.
        let routing_configs = self.db.list_routing_configs().await?;
        let health_config = routing_configs.iter().find(|rc| rc.health_check_enabled);
        let interval = Duration::from_secs(
            health_config.map_or(30, |rc| rc.health_check_interval_seconds as u64),
        );
        let timeout = Duration::from_secs(
            health_config.map_or(10, |rc| rc.health_check_timeout_seconds as u64),
        );

        let count = all_providers.len();
        let router = HealthCheckRouter {
            metrics_store: self.metrics_store.clone(),
        };

        let handle = tokio::spawn(async move {
            let mut interval_tick = tokio::time::interval(interval);

            loop {
                interval_tick.tick().await;

                let health_check_tasks: Vec<_> = all_providers
                    .iter()
                    .map(|provider| {
                        let provider_name = provider.name().to_string();
                        let provider_clone = provider.clone();
                        let router_clone = router.clone();
                        let timeout_duration = timeout;

                        tokio::spawn(async move {
                            // Fetch and emit balance snapshot
                            if let Some(amount) = provider_clone.fetch_balance().await {
                                router_clone.metrics_store.emitter().emit_balance(&provider_name, amount);
                            }

                            let result = tokio::time::timeout(timeout_duration, provider_clone.health_check()).await;
                            match result {
                                Ok(Ok(true)) => {
                                    let was_down = !router_clone.metrics_store.is_provider_available(&provider_name).await;
                                    if was_down {
                                        tracing::info!(provider = %provider_name, "Health check recovered");
                                    }
                                    router_clone.record_success(&provider_name).await;
                                }
                                Ok(Ok(false)) => {
                                    tracing::warn!(provider = %provider_name, "Health check unhealthy");
                                    router_clone.record_failure(
                                        &provider_name,
                                        crate::metrics::ErrorType::ServerError,
                                        "Health check returned unhealthy",
                                    ).await;
                                }
                                Ok(Err(e)) => {
                                    tracing::warn!(provider = %provider_name, error = %e, "Health check error");
                                    router_clone.record_failure(&provider_name, e.error_type(), &e.to_string()).await;
                                }
                                Err(_) => {
                                    tracing::warn!(provider = %provider_name, "Health check timeout");
                                    router_clone.record_failure(&provider_name, crate::metrics::ErrorType::Timeout, "Health check timed out").await;
                                }
                            }
                        })
                    })
                    .collect();

                for task in health_check_tasks {
                    let _ = task.await;
                }
            }
        });

        handles.insert("all-providers".to_string(), handle);
        tracing::info!(
            interval_seconds = interval.as_secs(),
            total_providers = count,
            "Started health check task for all providers"
        );

        Ok(())
    }

    pub async fn get_providers(&self) -> Vec<Arc<dyn Provider>> {
        self.providers.read().await.values().cloned().collect()
    }

    pub async fn add_provider(&self, provider: Arc<dyn Provider>) {
        let provider_name = provider.name().to_string();
        self.metrics_store.register_provider(&provider_name).await;
        let slug = provider.slug().to_string();
        self.providers.write().await.insert(slug.clone(), provider.clone());

        let mut tables = self.routing_tables.write().await;
        let default = tables
            .entry("default".to_string())
            .or_insert_with(|| RoutingTable::new(Vec::new()));
        default.entries.push(ProviderEntry {
            provider,
            model_override: None,
            weight: 100,
        });
    }

    pub async fn remove_provider(&self, slug: &str) {
        self.providers.write().await.remove(slug);

        let mut tables = self.routing_tables.write().await;
        for table in tables.values_mut() {
            table.entries.retain(|e| e.provider.slug() != slug);
        }
    }

    /// Collect all candidate (provider, resolved_model) pairs for a given model,
    /// ordered by preference (available providers first, then unavailable as fallback).
    ///
    /// Within available providers, candidates are sorted by load: providers with
    /// fewer in-flight requests (relative to their weight) are preferred. This
    /// prevents a slow provider from absorbing all traffic just because the
    /// round-robin counter keeps cycling back to it while it's still processing.
    async fn collect_candidates(
        &self,
        model: &str,
    ) -> Vec<(Arc<dyn Provider>, String)> {
        // Handle prefixed model (provider-slug/model)
        if let Some((slug_prefix, actual_model)) = model.split_once('/') {
            let providers = self.providers.read().await;
            let provider = providers
                .get(slug_prefix)
                .cloned()
                .or_else(|| {
                    providers
                        .values()
                        .find(|p| p.slug().starts_with(slug_prefix))
                        .cloned()
                });
            if let Some(provider) = provider {
                return vec![(provider, actual_model.to_string())];
            }
            return vec![];
        }

        let tables = self.routing_tables.read().await;
        let table = match tables.get(model).or_else(|| tables.get("default")) {
            Some(t) => t,
            None => return vec![],
        };

        if table.entries.is_empty() {
            return vec![];
        }

        // Get entries in weighted round-robin order
        let ordered = table.weighted_rr_order();

        // Build candidate list with load information for available providers.
        // Load score = in_flight / max(weight, 1). Lower is better.
        let mut available: Vec<(f32, (Arc<dyn Provider>, String))> = Vec::new();
        let mut unavailable = Vec::new();

        for entry in ordered {
            let resolved_model = entry
                .model_override
                .clone()
                .unwrap_or_else(|| model.to_string());
            let pair = (entry.provider.clone(), resolved_model);
            if self.metrics_store.is_provider_available(entry.provider.name()).await {
                let in_flight = self.metrics_store.get_in_flight(entry.provider.name()).await;
                let weight = entry.weight.max(1) as f32;
                let load_score = in_flight as f32 / weight;
                available.push((load_score, pair));
            } else {
                unavailable.push(pair);
            }
        }

        // Sort available providers by load score (ascending) — least loaded first.
        // This means a slow provider with many in-flight requests gets naturally
        // deprioritized even if the round-robin counter picked it as primary.
        available.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        let mut candidates: Vec<(Arc<dyn Provider>, String)> = available
            .into_iter()
            .map(|(_, pair)| pair)
            .collect();

        // Only fall back to unavailable providers if there are no healthy ones at all.
        // This avoids hammering providers that are known to be down/dead/gone.
        if candidates.is_empty() {
            tracing::warn!(
                model = %model,
                fallback_count = unavailable.len(),
                "No available providers, falling back to unavailable providers"
            );
            candidates.extend(unavailable);
        }
        candidates
    }

    /// Normalize chat request messages for a given provider:
    /// Convert `developer` role to `system` for providers that don't support it.
    ///
    /// Many backends don't support the `developer` role at all.
    /// Message ordering is the caller's responsibility.
    fn normalize_chat_request(request: &mut ChatCompletionRequest, provider_name: &str) {
        let should_convert_developer = !provider_name.to_lowercase().contains("openai");

        if should_convert_developer {
            request.messages = std::mem::take(&mut request.messages)
                .into_iter()
                .map(|m| match m {
                    ChatCompletionRequestMessage::Developer(dev) => {
                        let content = match dev.content {
                            async_openai::types::chat::ChatCompletionRequestDeveloperMessageContent::Text(t) => {
                                ChatCompletionRequestSystemMessageContent::Text(t)
                            }
                            async_openai::types::chat::ChatCompletionRequestDeveloperMessageContent::Array(parts) => {
                                ChatCompletionRequestSystemMessageContent::Array(
                                    parts.into_iter().map(|p| match p {
                                        async_openai::types::chat::ChatCompletionRequestDeveloperMessageContentPart::Text(t) => {
                                            async_openai::types::chat::ChatCompletionRequestSystemMessageContentPart::Text(t)
                                        }
                                    }).collect()
                                )
                            }
                        };
                        ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage {
                            content,
                            name: dev.name,
                        })
                    }
                    other => other,
                })
                .collect();
        }
    }

    pub async fn chat_completions(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, RouterError> {
        let start = Instant::now();
        let original_model = request.model.clone();

        let candidates = self.collect_candidates(&request.model).await;
        if candidates.is_empty() {
            return Err(RouterError::NoAvailableProvider);
        }

        let mut last_error: Option<RouterError> = None;
        let mut attempt: u32 = 0;

        for (provider, resolved_model) in candidates {
            if attempt >= self.max_retries {
                break;
            }
            attempt += 1;

            let provider_name = provider.name().to_string();

            let mut actual_request = request.clone();
            actual_request.model = resolved_model.clone();
            Self::normalize_chat_request(&mut actual_request, &provider_name);

            let in_flight = self.metrics_store.increment_in_flight(&provider_name).await;
            let mut guard = InFlightGuard::new(
                self.metrics_store.clone(),
                provider_name.clone(),
            );

            // Fetch and cache runtime info to get max_concurrency
            let max_concurrency = self.metrics_store.get_provider_max_concurrency(&provider_name).await;
            let max_concurrency = if max_concurrency.is_none() {
                if let Ok(Some(info)) = provider.get_runtime_info(&resolved_model).await {
                    let max_conc = info.max_concurrency();
                    self.metrics_store.set_provider_runtime_info(&provider_name, info).await;
                    max_conc
                } else {
                    None
                }
            } else {
                max_concurrency
            };

            self.metrics_store
                .emitter()
                .emit_provider_load(&provider_name, in_flight, max_concurrency);

            let result = provider.chat_completions(&actual_request).await;
            let total_latency = start.elapsed();

            match result {
                Ok(response) => {
                    guard.decrement();

                    let latency_ms = total_latency.as_millis() as u32;
                    self.metrics_store
                        .emitter()
                        .emit_total_latency(&provider_name, &original_model, latency_ms);
                    self.metrics_store
                        .emitter()
                        .emit_success(&provider_name, &original_model);

                    if let Some(tokens) = response.usage.as_ref() {
                        let latency_secs = total_latency.as_secs_f64().max(0.001);
                        let output_tokens_per_sec = tokens.completion_tokens as f32 / latency_secs as f32;
                        let input_tokens_per_sec = tokens.prompt_tokens as f32 / latency_secs as f32;

                        tracing::info!(
                            provider = %provider_name,
                            model = %original_model,
                            prompt_tokens = tokens.prompt_tokens,
                            completion_tokens = tokens.completion_tokens,
                            total_tokens = tokens.total_tokens,
                            total_latency_ms = latency_ms,
                            output_tokens_per_second = output_tokens_per_sec,
                            input_tokens_per_second = input_tokens_per_sec,
                            "Emitting tokens metrics"
                        );

                        self.metrics_store.emitter().emit_output_tokens_per_second(
                            &provider_name,
                            &original_model,
                            output_tokens_per_sec,
                        );
                        self.metrics_store.emitter().emit_input_tokens_per_second(
                            &provider_name,
                            &original_model,
                            input_tokens_per_sec,
                        );
                        self.metrics_store.emitter().emit_input_tokens(
                            &provider_name,
                            &original_model,
                            tokens.prompt_tokens,
                        );
                        self.metrics_store.emitter().emit_output_tokens(
                            &provider_name,
                            &original_model,
                            tokens.completion_tokens,
                        );
                    }

                    return Ok(response);
                }
                Err(e) => {
                    guard.decrement();

                    last_error = Some(RouterError::ProviderError(e.clone()));

                    self.metrics_store.emitter().emit_failure_with_details(
                        &provider_name,
                        &original_model,
                        e.error_type(),
                        None,
                        &e.to_string(),
                        e.retry_after_ms(),
                        e.status_code(),
                    );

                    if e.is_transient() {
                        tracing::warn!(
                            provider = &provider_name,
                            attempt = attempt,
                            error = %e,
                            "Transient error, failing over to another provider"
                        );
                    } else {
                        tracing::warn!(
                            provider = &provider_name,
                            attempt = attempt,
                            error = %e,
                            "Non-transient error, aborting"
                        );
                        return Err(last_error.unwrap());
                    }

                    let backoff = e
                        .retry_after_ms()
                        .map(Duration::from_millis)
                        .unwrap_or_else(|| Duration::from_millis(200 * (attempt as u64)));

                    tokio::time::sleep(backoff).await;
                }
            }
        }

        Err(last_error.unwrap_or(RouterError::NoAvailableProvider))
    }

    pub async fn chat_completions_stream(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<BoxStream<'static, Result<StreamingChunk, RouterError>>, RouterError>
    {
        let original_model = request.model.clone();
        tracing::info!(
            model = &original_model,
            stream = true,
            "Routing streaming request"
        );

        let candidates = self.collect_candidates(&request.model).await;
        if candidates.is_empty() {
            return Err(RouterError::NoAvailableProvider);
        }

        let metrics_store = self.metrics_store.clone();
        let max_retries = self.max_retries;
        let request = request.clone();

        let stream = stream! {
            let mut last_error: Option<RouterError> = None;
            let mut chunks_yielded = false;
            let mut attempt: u32 = 0;

            for (provider, resolved_model) in candidates {
                if attempt >= max_retries {
                    break;
                }
                attempt += 1;

                let provider_name = provider.name().to_string();

                let mut actual_request = request.clone();
                actual_request.model = resolved_model.clone();
                Self::normalize_chat_request(&mut actual_request, &provider_name);

                let in_flight = metrics_store.increment_in_flight(&provider_name).await;
                let mut guard = InFlightGuard::new(
                    metrics_store.clone(),
                    provider_name.clone(),
                );

                // Fetch and cache runtime info to get max_concurrency
                let max_concurrency = metrics_store.get_provider_max_concurrency(&provider_name).await;
                let max_concurrency = if max_concurrency.is_none() {
                    if let Ok(Some(info)) = provider.get_runtime_info(&resolved_model).await {
                        let max_conc = info.max_concurrency();
                        metrics_store.set_provider_runtime_info(&provider_name, info).await;
                        max_conc
                    } else {
                        None
                    }
                } else {
                    max_concurrency
                };

                metrics_store
                    .emitter()
                    .emit_provider_load(&provider_name, in_flight, max_concurrency);

                match provider.chat_completions_stream(&actual_request) {
                    Ok(provider_stream) => {
                        let start = Instant::now();
                        let mut first_token = true;
                        let mut total_tokens = 0u32;
                        let mut prompt_tokens = 0u32;
                        let mut completion_tokens = 0u32;
                        let mut ttft_ms = 0u32;

                        let mut stream: futures::stream::BoxStream<'static, Result<StreamingChunk, ProviderError>> = provider_stream;

                        while let Some(result) = stream.next().await {
                            match result {
                                Ok(chunk) => {
                                    if first_token {
                                        first_token = false;
                                        ttft_ms = start.elapsed().as_millis() as u32;
                                        metrics_store.emitter().emit_ttft(&provider_name, &original_model, ttft_ms);
                                    }

                                    if let Some(usage) = chunk.usage.clone() {
                                        prompt_tokens = usage.prompt_tokens;
                                        completion_tokens = usage.completion_tokens;
                                        total_tokens = usage.total_tokens;
                                    }

                                    chunks_yielded = true;
                                    yield Ok(chunk);
                                }
                                Err(e) => {
                                    metrics_store.emitter().emit_failure_with_details(
                                        &provider_name,
                                        &original_model,
                                        e.error_type(),
                                        None,
                                        &e.to_string(),
                                        e.retry_after_ms(),
                                        e.status_code(),
                                    );

                                    // Log detailed error context for debugging
                                    tracing::error!(
                                        provider = &provider_name,
                                        model = &original_model,
                                        attempt = attempt,
                                        chunks_yielded = chunks_yielded,
                                        error_type = ?e.error_type(),
                                        error = %e,
                                        "Stream error occurred"
                                    );

                                    // If no chunks have been sent yet and the error is transient,
                                    // fail over to the next provider instead of surfacing the error.
                                    if !chunks_yielded && e.is_transient() {
                                        tracing::warn!(
                                            provider = &provider_name,
                                            model = &original_model,
                                            attempt = attempt,
                                            error = %e,
                                            "Transient stream error before any data, failing over to another provider"
                                        );
                                        last_error = Some(RouterError::ProviderError(e));
                                        guard.decrement();

                                        // Backoff before trying next provider
                                        let backoff = last_error.as_ref().and_then(|e| {
                                            if let RouterError::ProviderError(pe) = e {
                                                pe.retry_after_ms()
                                            } else {
                                                None
                                            }
                                        }).map(Duration::from_millis)
                                          .unwrap_or_else(|| Duration::from_millis(200 * (attempt as u64)));
                                        tokio::time::sleep(backoff).await;

                                        break; // Continue to next provider in the outer loop
                                    }

                                    // Either we already sent data (can't retry) or error is non-transient
                                    if !e.is_transient() {
                                        tracing::warn!(
                                            provider = &provider_name,
                                            model = &original_model,
                                            attempt = attempt,
                                            error = %e,
                                            "Non-transient stream error, aborting"
                                        );
                                    } else {
                                        tracing::warn!(
                                            provider = &provider_name,
                                            model = &original_model,
                                            attempt = attempt,
                                            chunks_yielded = chunks_yielded,
                                            error = %e,
                                            "Transient stream error after data already sent, cannot fail over"
                                        );
                                    }
                                    yield Err(RouterError::ProviderError(e));
                                    guard.decrement();
                                    // Prevent further retries
                                    chunks_yielded = true;
                                    break;
                                }
                            }
                        }

                        // Stream completed normally (no error)
                        if !first_token {
                            metrics_store.emitter().emit_success(&provider_name, &original_model);
                            let total_latency_ms = start.elapsed().as_millis() as u32;
                            metrics_store.emitter().emit_total_latency(&provider_name, &original_model, total_latency_ms);

                            if total_tokens > 0 {
                                let generation_time_ms = total_latency_ms.saturating_sub(ttft_ms) as f32;
                                // Use total latency for output tok/s when generation time
                                // is negligible (< 100ms). Short responses finish so fast
                                // that generation_time ≈ 0 produces meaningless numbers.
                                let effective_output_time_secs = if generation_time_ms > 100.0 {
                                    generation_time_ms / 1000.0
                                } else {
                                    total_latency_ms as f32 / 1000.0
                                };
                                let output_tokens_per_sec = completion_tokens as f32 / effective_output_time_secs.max(0.001);
                                let input_tokens_per_sec = prompt_tokens as f32 / (start.elapsed().as_secs_f64().max(0.001)) as f32;

                                tracing::info!(
                                    provider = %provider_name,
                                    model = %original_model,
                                    prompt_tokens = prompt_tokens,
                                    completion_tokens = completion_tokens,
                                    total_tokens = total_tokens,
                                    total_latency_ms = total_latency_ms,
                                    output_tokens_per_second = output_tokens_per_sec,
                                    input_tokens_per_second = input_tokens_per_sec,
                                    "Emitting tokens metrics"
                                );

                                metrics_store.emitter().emit_output_tokens_per_second(&provider_name, &original_model, output_tokens_per_sec);
                                metrics_store.emitter().emit_input_tokens_per_second(&provider_name, &original_model, input_tokens_per_sec);
                                metrics_store.emitter().emit_input_tokens(&provider_name, &original_model, prompt_tokens);
                                metrics_store.emitter().emit_output_tokens(&provider_name, &original_model, completion_tokens);
                            }

                            guard.decrement();
                            break; // Stream completed successfully, don't try more providers
                        }

                        // Empty stream (no chunks, no error) — guard still needs decrement.
                        // Continue to next provider.
                        guard.decrement();
                        continue;
                    }
                    Err(e) => {
                        guard.decrement();

                        last_error = Some(RouterError::ProviderError(e.clone()));

                        metrics_store.emitter().emit_failure_with_details(
                            &provider_name,
                            &original_model,
                            e.error_type(),
                            None,
                            &e.to_string(),
                            e.retry_after_ms(),
                            e.status_code(),
                        );

                        if e.is_transient() {
                            tracing::warn!(
                                provider = &provider_name,
                                attempt = attempt,
                                error = %e,
                                "Transient stream setup error, failing over to another provider"
                            );

                            // Backoff before trying next provider
                            let backoff = e
                                .retry_after_ms()
                                .map(Duration::from_millis)
                                .unwrap_or_else(|| Duration::from_millis(200 * (attempt as u64)));
                            tokio::time::sleep(backoff).await;
                        } else {
                            tracing::warn!(
                                provider = &provider_name,
                                attempt = attempt,
                                error = %e,
                                "Non-transient stream setup error, aborting"
                            );
                            yield Err(last_error.clone().unwrap());
                            break;
                        }
                    }
                }
            }

            // If we exhausted all providers without yielding anything, emit the last error
            if !chunks_yielded {
                if let Some(e) = last_error {
                    yield Err(e);
                } else if attempt == 0 {
                    yield Err(RouterError::NoAvailableProvider);
                }
            }
        };

        Ok(Box::pin(stream))
    }

    fn transform_request(request: &CreateResponse, provider_name: &str) -> CreateResponse {
        let mut transformed = request.clone();
        
        // Only transform developer role for providers that don't support it
        // OpenAI supports developer role natively, but vLLM and other backends may not
        // Message ordering is the caller's responsibility.
        let should_transform = !provider_name.to_lowercase().contains("openai");

        let items = match std::mem::replace(&mut transformed.input, InputParam::Items(vec![])) {
            InputParam::Items(items) => items,
            other => {
                transformed.input = other;
                return transformed;
            }
        };
        
        let items: Vec<async_openai::types::responses::InputItem> = if should_transform {
            items.into_iter()
                .map(|item| {
                    if let async_openai::types::responses::InputItem::Item(
                        async_openai::types::responses::Item::Message(MessageItem::Input(InputMessage {
                            role: InputRole::Developer,
                            content,
                            status,
                        }))
                    ) = item
                    {
                        async_openai::types::responses::InputItem::Item(
                            async_openai::types::responses::Item::Message(MessageItem::Input(InputMessage {
                                role: InputRole::System,
                                content,
                                status,
                            }))
                        )
                    } else {
                        item
                    }
                })
                .collect()
        } else {
            items
        };
        
        transformed.input = InputParam::Items(items);
        transformed
    }

    pub async fn responses(
        &self,
        request: &CreateResponse,
    ) -> Result<ApiResponse, RouterError> {
        let start = Instant::now();
        let original_model = request.model.clone().unwrap_or_default();

        let candidates = self.collect_candidates(&original_model).await;
        if candidates.is_empty() {
            return Err(RouterError::NoAvailableProvider);
        }

        let mut last_error: Option<RouterError> = None;
        let mut attempt: u32 = 0;

        for (provider, resolved_model) in candidates {
            if attempt >= self.max_retries {
                break;
            }
            attempt += 1;

            let provider_name = provider.name().to_string();

            let mut actual_request = Self::transform_request(request, &provider_name);
            actual_request.model = Some(resolved_model.clone());

            let in_flight = self.metrics_store.increment_in_flight(&provider_name).await;
            let mut guard = InFlightGuard::new(
                self.metrics_store.clone(),
                provider_name.clone(),
            );

            // Fetch and cache runtime info to get max_concurrency
            let max_concurrency = self.metrics_store.get_provider_max_concurrency(&provider_name).await;
            let max_concurrency = if max_concurrency.is_none() {
                if let Ok(Some(info)) = provider.get_runtime_info(&resolved_model).await {
                    let max_conc = info.max_concurrency();
                    self.metrics_store.set_provider_runtime_info(&provider_name, info).await;
                    max_conc
                } else {
                    None
                }
            } else {
                max_concurrency
            };

            self.metrics_store
                .emitter()
                .emit_provider_load(&provider_name, in_flight, max_concurrency);

            let result = provider.responses(&actual_request).await;
            let total_latency = start.elapsed();

            match result {
                Ok(response) => {
                    guard.decrement();

                    let latency_ms = total_latency.as_millis() as u32;
                    self.metrics_store
                        .emitter()
                        .emit_total_latency(&provider_name, &original_model, latency_ms);
                    self.metrics_store
                        .emitter()
                        .emit_success(&provider_name, &original_model);

                    tracing::info!(
                        provider = provider_name,
                        model = original_model,
                        latency_ms = latency_ms,
                        "Responses API request completed successfully"
                    );

                    return Ok(response);
                }
                Err(e) => {
                    guard.decrement();

                    last_error = Some(RouterError::ProviderError(e.clone()));

                    self.metrics_store.emitter().emit_failure_with_details(
                        &provider_name,
                        &original_model,
                        e.error_type(),
                        None,
                        &e.to_string(),
                        e.retry_after_ms(),
                        e.status_code(),
                    );

                    if e.is_transient() {
                        tracing::warn!(
                            provider = &provider_name,
                            attempt = attempt,
                            error = %e,
                            "Transient responses error, failing over to another provider"
                        );
                    } else {
                        tracing::warn!(
                            provider = &provider_name,
                            attempt = attempt,
                            error = %e,
                            "Non-transient responses error, aborting"
                        );
                        return Err(last_error.unwrap());
                    }

                    let backoff = e
                        .retry_after_ms()
                        .map(Duration::from_millis)
                        .unwrap_or_else(|| Duration::from_millis(200 * (attempt as u64)));

                    tokio::time::sleep(backoff).await;
                }
            }
        }

        Err(last_error.unwrap_or(RouterError::NoAvailableProvider))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RouterError {
    #[error("No available provider for routing")]
    NoAvailableProvider,

    #[error("Provider error: {0}")]
    ProviderError(ProviderError),
}

impl Clone for RouterError {
    fn clone(&self) -> Self {
        match self {
            RouterError::NoAvailableProvider => RouterError::NoAvailableProvider,
            RouterError::ProviderError(e) => RouterError::ProviderError(e.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::openai::OpenAiProvider;
    use crate::metrics::{MetricsStore, ProviderMetrics, MetricsEvent, FailureDetails, ErrorType};
    use std::sync::Arc;

    async fn setup_test_router() -> (Router, MetricsStore) {
        let db = Arc::new(Database::new("sqlite::memory:").await.unwrap());
        let metrics_store = MetricsStore::new(1000);
        
        let router = Router::new(
            metrics_store.clone(),
            db.clone(),
        );
        
        (router, metrics_store)
    }

    #[tokio::test]
    async fn test_collect_candidates_prefers_available_providers() {
        let (router, metrics_store) = setup_test_router().await;
        
        // Create two providers
        let provider1 = Arc::new(OpenAiProvider::new("Provider1", Some("provider1"), "http://localhost:8001", Some("key")));
        let provider2 = Arc::new(OpenAiProvider::new("Provider2", Some("provider2"), "http://localhost:8002", Some("key")));
        
        // Register providers
        router.add_provider(provider1.clone()).await;
        router.add_provider(provider2.clone()).await;
        
        // Mark provider2 as unavailable by recording 5 failures (hits failure_threshold)
        for _ in 0..5 {
            metrics_store.record(ProviderMetrics {
                provider: "Provider2".to_string(),
                model: "default".to_string(),
                timestamp_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
                event: MetricsEvent::Failure(FailureDetails {
                    error_type: ErrorType::Other,
                    error_code: None,
                    error_message: "test failure".to_string(),
                    retry_after_ms: None,
                    status_code: None,
                }),
            }).await;
        }
        
        // Verify provider2 is now unavailable
        assert!(!metrics_store.is_provider_available("Provider2").await);
        assert!(metrics_store.is_provider_available("Provider1").await);
        
        // Collect candidates for "default" model - should list provider1 first
        let candidates = router.collect_candidates("default").await;
        
        assert!(!candidates.is_empty(), "Should find candidates");
        assert_eq!(candidates[0].0.name(), "Provider1", "Should list available provider first");
    }

    #[tokio::test]
    async fn test_collect_candidates_includes_unavailable_as_fallback() {
        let (router, metrics_store) = setup_test_router().await;
        
        let provider1 = Arc::new(OpenAiProvider::new("Provider1", Some("provider1"), "http://localhost:8001", Some("key")));
        let provider2 = Arc::new(OpenAiProvider::new("Provider2", Some("provider2"), "http://localhost:8002", Some("key")));
        
        router.add_provider(provider1.clone()).await;
        router.add_provider(provider2.clone()).await;
        
        // Mark both as unavailable
        for _ in 0..5 {
            metrics_store.record(ProviderMetrics {
                provider: "Provider1".to_string(),
                model: "default".to_string(),
                timestamp_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
                event: MetricsEvent::Failure(FailureDetails {
                    error_type: ErrorType::Other,
                    error_code: None,
                    error_message: "test failure".to_string(),
                    retry_after_ms: None,
                    status_code: None,
                }),
            }).await;
            
            metrics_store.record(ProviderMetrics {
                provider: "Provider2".to_string(),
                model: "default".to_string(),
                timestamp_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
                event: MetricsEvent::Failure(FailureDetails {
                    error_type: ErrorType::Other,
                    error_code: None,
                    error_message: "test failure".to_string(),
                    retry_after_ms: None,
                    status_code: None,
                }),
            }).await;
        }
        
        // Even though both are unavailable, they should still be returned as candidates
        let candidates = router.collect_candidates("default").await;
        
        assert_eq!(candidates.len(), 2, "Should include unavailable providers as fallback");
    }

    #[tokio::test]
    async fn test_collect_candidates_prefixed_model() {
        let (router, _metrics_store) = setup_test_router().await;
        
        let provider1 = Arc::new(OpenAiProvider::new("Provider1", Some("provider1"), "http://localhost:8001", Some("key")));
        let provider2 = Arc::new(OpenAiProvider::new("Provider2", Some("provider2"), "http://localhost:8002", Some("key")));
        
        router.add_provider(provider1.clone()).await;
        router.add_provider(provider2.clone()).await;
        
        // Prefixed model should route to the specific provider only
        let candidates = router.collect_candidates("provider2/gpt-4").await;
        
        assert_eq!(candidates.len(), 1, "Should return exactly one candidate for prefixed model");
        assert_eq!(candidates[0].0.name(), "Provider2", "Should route to the prefixed provider");
        assert_eq!(candidates[0].1, "gpt-4", "Should extract the actual model name");
    }

    #[tokio::test]
    async fn test_collect_candidates_empty_when_no_providers() {
        let (router, _metrics_store) = setup_test_router().await;
        
        // No providers added
        let candidates = router.collect_candidates("gpt-4").await;
        
        assert!(candidates.is_empty(), "Should return empty when no providers configured");
    }

    #[test]
    fn test_normalize_converts_developer_to_system_for_non_openai() {
        use async_openai::types::chat::{
            ChatCompletionRequestDeveloperMessage,
            ChatCompletionRequestDeveloperMessageContent,
            ChatCompletionRequestUserMessage, ChatCompletionRequestUserMessageContent,
        };

        let mut request = ChatCompletionRequest {
            model: "test".to_string(),
            messages: vec![
                ChatCompletionRequestMessage::Developer(ChatCompletionRequestDeveloperMessage {
                    content: ChatCompletionRequestDeveloperMessageContent::Text("dev instructions".to_string()),
                    name: None,
                }),
                ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                    content: ChatCompletionRequestUserMessageContent::Text("hello".to_string()),
                    name: None,
                }),
            ],
            ..Default::default()
        };

        Router::normalize_chat_request(&mut request, "vllm-backend");

        // Developer should be converted to System
        assert!(matches!(&request.messages[0], ChatCompletionRequestMessage::System(_)));
        assert!(matches!(&request.messages[1], ChatCompletionRequestMessage::User(_)));
        // No Developer messages remain
        assert!(!request.messages.iter().any(|m| matches!(m, ChatCompletionRequestMessage::Developer(_))));
    }

    #[test]
    fn test_normalize_preserves_developer_for_openai() {
        use async_openai::types::chat::{
            ChatCompletionRequestDeveloperMessage,
            ChatCompletionRequestDeveloperMessageContent,
            ChatCompletionRequestUserMessage, ChatCompletionRequestUserMessageContent,
        };

        let mut request = ChatCompletionRequest {
            model: "test".to_string(),
            messages: vec![
                ChatCompletionRequestMessage::Developer(ChatCompletionRequestDeveloperMessage {
                    content: ChatCompletionRequestDeveloperMessageContent::Text("dev instructions".to_string()),
                    name: None,
                }),
                ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                    content: ChatCompletionRequestUserMessageContent::Text("hello".to_string()),
                    name: None,
                }),
            ],
            ..Default::default()
        };

        Router::normalize_chat_request(&mut request, "OpenAI");

        // Developer message should be preserved for OpenAI
        assert!(matches!(&request.messages[0], ChatCompletionRequestMessage::Developer(_)));
        assert!(matches!(&request.messages[1], ChatCompletionRequestMessage::User(_)));
    }

    #[test]
    fn test_normalize_developer_in_middle_preserves_position_for_openai() {
        use async_openai::types::chat::{
            ChatCompletionRequestDeveloperMessage,
            ChatCompletionRequestDeveloperMessageContent,
            ChatCompletionRequestUserMessage, ChatCompletionRequestUserMessageContent,
        };

        let mut request = ChatCompletionRequest {
            model: "test".to_string(),
            messages: vec![
                ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                    content: ChatCompletionRequestUserMessageContent::Text("hello".to_string()),
                    name: None,
                }),
                ChatCompletionRequestMessage::Developer(ChatCompletionRequestDeveloperMessage {
                    content: ChatCompletionRequestDeveloperMessageContent::Text("dev prompt".to_string()),
                    name: None,
                }),
                ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                    content: ChatCompletionRequestUserMessageContent::Text("world".to_string()),
                    name: None,
                }),
            ],
            ..Default::default()
        };

        Router::normalize_chat_request(&mut request, "OpenAI");

        // Developer preserved in place — not converted, not moved
        assert!(matches!(&request.messages[0], ChatCompletionRequestMessage::User(_)));
        assert!(matches!(&request.messages[1], ChatCompletionRequestMessage::Developer(_)));
        assert!(matches!(&request.messages[2], ChatCompletionRequestMessage::User(_)));
    }

    #[test]
    fn test_normalize_developer_in_middle_converts_preserves_position_for_non_openai() {
        use async_openai::types::chat::{
            ChatCompletionRequestDeveloperMessage,
            ChatCompletionRequestDeveloperMessageContent,
            ChatCompletionRequestUserMessage, ChatCompletionRequestUserMessageContent,
        };

        let mut request = ChatCompletionRequest {
            model: "test".to_string(),
            messages: vec![
                ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                    content: ChatCompletionRequestUserMessageContent::Text("hello".to_string()),
                    name: None,
                }),
                ChatCompletionRequestMessage::Developer(ChatCompletionRequestDeveloperMessage {
                    content: ChatCompletionRequestDeveloperMessageContent::Text("dev prompt".to_string()),
                    name: None,
                }),
                ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                    content: ChatCompletionRequestUserMessageContent::Text("world".to_string()),
                    name: None,
                }),
            ],
            ..Default::default()
        };

        Router::normalize_chat_request(&mut request, "my-vllm");

        // Developer converted to System but stays in place
        assert!(matches!(&request.messages[0], ChatCompletionRequestMessage::User(_)));
        assert!(matches!(&request.messages[1], ChatCompletionRequestMessage::System(_)));
        assert!(matches!(&request.messages[2], ChatCompletionRequestMessage::User(_)));
    }

    #[test]
    fn test_normalize_no_change_when_already_correct() {
        use crate::providers::{
            ChatCompletionRequestUserMessage, ChatCompletionRequestUserMessageContent,
        };

        let mut request = ChatCompletionRequest {
            model: "test".to_string(),
            messages: vec![
                ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage {
                    content: ChatCompletionRequestSystemMessageContent::Text("system".to_string()),
                    name: None,
                }),
                ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                    content: ChatCompletionRequestUserMessageContent::Text("hello".to_string()),
                    name: None,
                }),
            ],
            ..Default::default()
        };

        Router::normalize_chat_request(&mut request, "vllm");

        assert!(matches!(&request.messages[0], ChatCompletionRequestMessage::System(_)));
        assert!(matches!(&request.messages[1], ChatCompletionRequestMessage::User(_)));
    }

    #[tokio::test]
    async fn test_weighted_round_robin_3_to_1_distribution() {
        let (router, _metrics_store) = setup_test_router().await;

        let provider1 = Arc::new(OpenAiProvider::new("Heavy", Some("heavy"), "http://localhost:8001", Some("key")));
        let provider2 = Arc::new(OpenAiProvider::new("Light", Some("light"), "http://localhost:8002", Some("key")));

        // Manually build a routing table with 3:1 weights
        {
            let mut tables = router.routing_tables.write().await;
            tables.insert(
                "test-model".to_string(),
                RoutingTable::new(vec![
                    ProviderEntry {
                        provider: provider1.clone(),
                        model_override: None,
                        weight: 3,
                    },
                    ProviderEntry {
                        provider: provider2.clone(),
                        model_override: None,
                        weight: 1,
                    },
                ]),
            );
        }

        // Collect candidates many times and count which provider is picked first
        let mut heavy_count = 0usize;
        let mut light_count = 0usize;

        for _ in 0..40 {
            let candidates = router.collect_candidates("test-model").await;
            match candidates[0].0.name() {
                "Heavy" => heavy_count += 1,
                "Light" => light_count += 1,
                other => panic!("Unexpected provider: {other}"),
            }
        }

        // With 3:1 weights, expect ~30 heavy and ~10 light (±some tolerance)
        assert_eq!(heavy_count + light_count, 40, "All 40 requests should have a first candidate");
        assert!(
            heavy_count >= 25 && heavy_count <= 35,
            "Expected ~30 heavy selections, got {heavy_count}"
        );
        assert!(
            light_count >= 5 && light_count <= 15,
            "Expected ~10 light selections, got {light_count}"
        );
    }

    #[tokio::test]
    async fn test_collect_candidates_deprioritizes_loaded_provider() {
        let (router, metrics_store) = setup_test_router().await;

        let provider1 = Arc::new(OpenAiProvider::new("Slow", Some("slow"), "http://localhost:8001", Some("key")));
        let provider2 = Arc::new(OpenAiProvider::new("Fast", Some("fast"), "http://localhost:8002", Some("key")));

        // Build routing table with 3:3 weights (equal preference)
        {
            let mut tables = router.routing_tables.write().await;
            tables.insert(
                "load-test".to_string(),
                RoutingTable::new(vec![
                    ProviderEntry {
                        provider: provider1.clone(),
                        model_override: None,
                        weight: 3,
                    },
                    ProviderEntry {
                        provider: provider2.clone(),
                        model_override: None,
                        weight: 3,
                    },
                ]),
            );
        }

        // Simulate 10 in-flight requests on "Slow" provider
        for _ in 0..10 {
            metrics_store.increment_in_flight("Slow").await;
        }

        // "Fast" has 0 in-flight, "Slow" has 10.
        // Load score: Slow = 10/3 ≈ 3.33, Fast = 0/3 = 0.0
        // Fast should always be picked first.
        for _ in 0..10 {
            let candidates = router.collect_candidates("load-test").await;
            assert_eq!(
                candidates[0].0.name(), "Fast",
                "Less-loaded provider should be preferred over heavily loaded one"
            );
        }
    }

    #[tokio::test]
    async fn test_collect_candidates_weight_adjusts_load_tolerance() {
        let (router, metrics_store) = setup_test_router().await;

        let heavy = Arc::new(OpenAiProvider::new("Heavy", Some("heavy"), "http://localhost:8001", Some("key")));
        let light = Arc::new(OpenAiProvider::new("Light", Some("light"), "http://localhost:8002", Some("key")));

        // Heavy has weight 3, Light has weight 1
        {
            let mut tables = router.routing_tables.write().await;
            tables.insert(
                "weight-load-test".to_string(),
                RoutingTable::new(vec![
                    ProviderEntry {
                        provider: heavy.clone(),
                        model_override: None,
                        weight: 3,
                    },
                    ProviderEntry {
                        provider: light.clone(),
                        model_override: None,
                        weight: 1,
                    },
                ]),
            );
        }

        // Heavy has 2 in-flight (load_score = 2/3 ≈ 0.67)
        // Light has 0 in-flight (load_score = 0/1 = 0.0)
        // Light should be preferred since it's less loaded relative to its weight
        metrics_store.increment_in_flight("Heavy").await;
        metrics_store.increment_in_flight("Heavy").await;

        let candidates = router.collect_candidates("weight-load-test").await;
        assert_eq!(
            candidates[0].0.name(), "Light",
            "Provider with lower load/weight ratio should be preferred"
        );

        // Now: Heavy has 2 in-flight (0.67), Light has 1 in-flight (1.0)
        // Heavy should now be preferred since its load/weight is lower
        metrics_store.increment_in_flight("Light").await;

        let candidates = router.collect_candidates("weight-load-test").await;
        assert_eq!(
            candidates[0].0.name(), "Heavy",
            "Provider with lower load/weight ratio should be preferred even with higher absolute in-flight"
        );
    }

    #[tokio::test]
    async fn test_equal_weights_rotates_fairly() {
        let (router, _metrics_store) = setup_test_router().await;

        let provider1 = Arc::new(OpenAiProvider::new("P1", Some("p1"), "http://localhost:8001", Some("key")));
        let provider2 = Arc::new(OpenAiProvider::new("P2", Some("p2"), "http://localhost:8002", Some("key")));

        {
            let mut tables = router.routing_tables.write().await;
            tables.insert(
                "equal-model".to_string(),
                RoutingTable::new(vec![
                    ProviderEntry {
                        provider: provider1.clone(),
                        model_override: None,
                        weight: 100,
                    },
                    ProviderEntry {
                        provider: provider2.clone(),
                        model_override: None,
                        weight: 100,
                    },
                ]),
            );
        }

        let mut p1_count = 0usize;
        let mut p2_count = 0usize;

        for _ in 0..20 {
            let candidates = router.collect_candidates("equal-model").await;
            match candidates[0].0.name() {
                "P1" => p1_count += 1,
                "P2" => p2_count += 1,
                other => panic!("Unexpected provider: {other}"),
            }
        }

        // With equal weights, should be roughly 10:10
        assert_eq!(p1_count + p2_count, 20);
        assert!(
            p1_count >= 7 && p1_count <= 13,
            "Expected ~10 P1 selections, got {p1_count}"
        );
        assert!(
            p2_count >= 7 && p2_count <= 13,
            "Expected ~10 P2 selections, got {p2_count}"
        );
    }
}
