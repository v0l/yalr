use crate::db::Database;
use crate::metrics::MetricsStore;
use crate::providers::{create_provider, Provider};
use crate::router::strategies::{ProviderEntry, RoutingStrategy};
use crate::{ChatCompletionRequest, ChatCompletionResponse, ProviderError};
use crate::providers::StreamingChunk;
use async_openai::types::responses::{CreateResponse, Response as ApiResponse};
use async_stream::stream;
use futures::stream::BoxStream;
use futures::StreamExt;
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
    strategy_name: String,
    entries: Vec<ProviderEntry>,
}

pub struct Router {
    db: Arc<Database>,
    metrics_store: MetricsStore,
    providers: RwLock<HashMap<String, Arc<dyn Provider>>>,
    routing_tables: RwLock<HashMap<String, RoutingTable>>,
    strategies: HashMap<String, Arc<dyn RoutingStrategy>>,
    max_retries: u32,
}

impl Router {
    pub fn new(
        default_strategy: Arc<dyn RoutingStrategy>,
        metrics_store: MetricsStore,
        db: Arc<Database>,
    ) -> Self {
        let mut strategies = HashMap::new();
        strategies.insert(default_strategy.name().to_string(), default_strategy);

        Self {
            db,
            metrics_store,
            providers: RwLock::new(HashMap::new()),
            routing_tables: RwLock::new(HashMap::new()),
            strategies,
            max_retries: 3,
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
                RoutingTable {
                    strategy_name: rc.strategy.clone(),
                    entries,
                },
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
                .or_insert_with(|| RoutingTable {
                    strategy_name: "round_robin".to_string(),
                    entries: Vec::new(),
                })
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
                RoutingTable {
                    strategy_name: "round_robin".to_string(),
                    entries,
                },
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
            .or_insert_with(|| RoutingTable {
                strategy_name: "round_robin".to_string(),
                entries: Vec::new(),
            });
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

    async fn resolve_route(&self, model: &str) -> Option<(Arc<dyn Provider>, String)> {
        self.resolve_route_excluding(model, &[]).await
    }

    async fn resolve_route_excluding(
        &self,
        model: &str,
        excluded_providers: &[String],
    ) -> Option<(Arc<dyn Provider>, String)> {
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
                tracing::info!(
                    model = model,
                    slug = slug_prefix,
                    resolved_model = actual_model,
                    provider = provider.name(),
                    "Routed by provider prefix"
                );
                return Some((provider.clone(), actual_model.to_string()));
            }
            tracing::warn!(slug = slug_prefix, "No provider found for slug prefix");
            return None;
        }

        let tables = self.routing_tables.read().await;
        let table = match tables.get(model).or_else(|| tables.get("default")) {
            Some(t) => t,
            None => {
                tracing::warn!(model = model, "No routing table found for model");
                return None;
            }
        };

        if table.entries.is_empty() {
            tracing::warn!(model = model, "Routing table has no providers");
            return None;
        }

        let strategy_name = table.strategy_name.clone();

        // Collect entries that are not explicitly excluded, then prefer available ones
        let not_excluded: Vec<ProviderEntry> = table
            .entries
            .iter()
            .filter(|e| !excluded_providers.contains(&e.provider.name().to_string()))
            .cloned()
            .collect();
        drop(tables);

        if not_excluded.is_empty() {
            tracing::warn!(
                model = model,
                excluded = ?excluded_providers,
                "All providers excluded, no fallback available"
            );
            return None;
        }

        // Prefer providers that are available (not in backoff/unhealthy state)
        let mut available = Vec::new();
        let mut unavailable = Vec::new();
        for entry in not_excluded {
            if self.metrics_store.is_provider_available(entry.provider.name()).await {
                available.push(entry);
            } else {
                unavailable.push(entry);
            }
        }

        let entries = if !available.is_empty() {
            available
        } else {
            tracing::warn!(
                model = model,
                "All non-excluded providers are unavailable, using fallback"
            );
            unavailable
        };

        let strategy = match self
            .strategies
            .get(&strategy_name)
            .or_else(|| self.strategies.values().next())
        {
            Some(s) => s,
            None => return None,
        };

        let idx = strategy.select(&entries, model).await?;
        let entry = &entries[idx];
        let resolved_model = entry
            .model_override
            .clone()
            .unwrap_or_else(|| model.to_string());

        tracing::info!(
            model = model,
            resolved_model = resolved_model,
            provider = entry.provider.name(),
            strategy = strategy.name(),
            "Routed via routing table"
        );

        Some((entry.provider.clone(), resolved_model))
    }

    pub async fn chat_completions(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, RouterError> {
        let start = Instant::now();
        let original_model = request.model.clone();

        let mut excluded_providers: Vec<String> = Vec::new();
        let mut last_error: Option<RouterError> = None;

        for attempt in 0..self.max_retries {
            let (provider, resolved_model) = self
                .resolve_route_excluding(&request.model, &excluded_providers)
                .await
                .ok_or_else(|| {
                    last_error.unwrap_or(RouterError::NoAvailableProvider)
                })?;
            let provider_name = provider.name().to_string();

            let mut actual_request = request.clone();
            actual_request.model = resolved_model.clone();

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

            if !self.metrics_store.is_provider_available(&provider_name).await {
                let backoff = self.metrics_store.get_provider_backoff(&provider_name).await;
                tracing::warn!(
                    provider = &provider_name,
                    attempt = attempt,
                    backoff_ms = backoff.as_millis(),
                    "Provider unavailable, waiting before retry"
                );
                tokio::time::sleep(backoff).await;
            }

            let result = provider.chat_completions(&actual_request).await;
            let total_latency = start.elapsed();

            match result {
                Ok(response) => {
                    guard.decrement();
                    drop(guard);

                    let latency_ms = total_latency.as_millis() as u32;
                    self.metrics_store
                        .emitter()
                        .emit_total_latency(&provider_name, &original_model, latency_ms);
                    self.metrics_store
                        .emitter()
                        .emit_success(&provider_name, &original_model);

                    if let Some(tokens) = response.usage.as_ref() {
                        let output_tokens_per_sec = tokens.completion_tokens as f32
                            / (total_latency.as_secs_f64().max(0.001)) as f32;
                        let input_tokens_per_sec = tokens.prompt_tokens as f32
                            / (total_latency.as_secs_f64().max(0.001)) as f32;

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
                            tokens.prompt_tokens as u32,
                        );
                        self.metrics_store.emitter().emit_output_tokens(
                            &provider_name,
                            &original_model,
                            tokens.completion_tokens as u32,
                        );
                    }

                    return Ok(response);
                }
                Err(e) => {
                    guard.decrement();
                    drop(guard);

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
                        // Exclude this provider and try another on next iteration
                        excluded_providers.push(provider_name.clone());
                        tracing::warn!(
                            provider = &provider_name,
                            attempt = attempt,
                            error = %e,
                            "Transient error, failing over to another provider"
                        );
                    } else {
                        // Non-transient error (auth, not found) - don't retry
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
                        .map(|ms| Duration::from_millis(ms))
                        .unwrap_or_else(|| Duration::from_secs(2_u64.saturating_pow(attempt)));

                    tokio::time::sleep(backoff).await;
                }
            }
        }

        Err(last_error.unwrap_or(RouterError::ProviderError(
            ProviderError::Other("Max retries exceeded".to_string().into()),
        )))
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

        let mut excluded_providers: Vec<String> = Vec::new();
        let mut last_error: Option<RouterError> = None;

        for attempt in 0..self.max_retries {
            let route_result = self
                .resolve_route_excluding(&request.model, &excluded_providers)
                .await;
            
            let (provider, resolved_model) = match route_result {
                Some(route) => route,
                None => {
                    return Err(last_error.unwrap_or(RouterError::NoAvailableProvider));
                }
            };
            let provider_name = provider.name().to_string();

            let mut actual_request = request.clone();
            actual_request.model = resolved_model.clone();

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

            let metrics_store = self.metrics_store.clone();
            let provider_name_stream = provider_name.clone();

            match provider.chat_completions_stream(&actual_request) {
                Ok(provider_stream) => {
                    // Stream created successfully - guard will be decremented when stream ends
                    let stream = stream! {
                        let start = Instant::now();
                        let mut first_token = true;
                        let mut total_tokens = 0u32;
                        let mut prompt_tokens = 0u32;
                        let mut completion_tokens = 0u32;
                        let mut ttft_ms = 0u32;
                        let mut stream_error: Option<ProviderError> = None;

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

                                    yield Ok(chunk);
                                }
                                Err(e) => {
                                    stream_error = Some(e.clone());
                                    metrics_store.emitter().emit_failure_with_details(
                                        &provider_name,
                                        &original_model,
                                        e.error_type(),
                                        None,
                                        &e.to_string(),
                                        e.retry_after_ms(),
                                        e.status_code(),
                                    );
                                    yield Err(RouterError::ProviderError(e));
                                    break;
                                }
                            }
                        }

                        if stream_error.is_none() && !first_token {
                            metrics_store.emitter().emit_success(&provider_name, &original_model);
                            let total_latency_ms = start.elapsed().as_millis() as u32;
                            metrics_store.emitter().emit_total_latency(&provider_name, &original_model, total_latency_ms);

                            if total_tokens > 0 {
                                let generation_time_ms = total_latency_ms.saturating_sub(ttft_ms) as f32;
                                let output_tokens_per_sec = completion_tokens as f32 / (generation_time_ms / 1000.0).max(0.001);
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
                        }

                        guard.decrement();
                    };

                    return Ok(Box::pin(stream));
                }
                Err(e) => {
                    guard.decrement();
                    drop(guard);

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
                        excluded_providers.push(provider_name.clone());
                        tracing::warn!(
                            provider = &provider_name,
                            attempt = attempt,
                            error = %e,
                            "Transient stream setup error, failing over to another provider"
                        );
                    } else {
                        tracing::warn!(
                            provider = &provider_name,
                            attempt = attempt,
                            error = %e,
                            "Non-transient stream setup error, aborting"
                        );
                        return Err(last_error.unwrap());
                    }

                    let backoff = e
                        .retry_after_ms()
                        .map(|ms| Duration::from_millis(ms))
                        .unwrap_or_else(|| Duration::from_secs(2_u64.saturating_pow(attempt)));

                    tokio::time::sleep(backoff).await;
                }
            }
        }

        Err(last_error.unwrap_or(RouterError::ProviderError(
            ProviderError::Other("Max retries exceeded".to_string().into()),
        )))
    }

    pub async fn responses(
        &self,
        request: &CreateResponse,
    ) -> Result<ApiResponse, RouterError> {
        
        let start = Instant::now();
        let original_model = request.model.clone().unwrap_or_default();

        let mut excluded_providers: Vec<String> = Vec::new();
        let mut last_error: Option<RouterError> = None;

        for attempt in 0..self.max_retries {
            let route_result = self
                .resolve_route_excluding(&original_model, &excluded_providers)
                .await;
            
            let (provider, _resolved_model) = match route_result {
                Some(route) => route,
                None => {
                    return Err(last_error.unwrap_or(RouterError::NoAvailableProvider));
                }
            };
            let provider_name = provider.name().to_string();

            let in_flight = self.metrics_store.increment_in_flight(&provider_name).await;
            let mut guard = InFlightGuard::new(
                self.metrics_store.clone(),
                provider_name.clone(),
            );

            // Check if provider supports responses API
            if !self.metrics_store.is_provider_available(&provider_name).await {
                let backoff = self.metrics_store.get_provider_backoff(&provider_name).await;
                tracing::warn!(
                    provider = &provider_name,
                    attempt = attempt,
                    backoff_ms = backoff.as_millis(),
                    "Provider unavailable, waiting before retry"
                );
                guard.decrement();
                drop(guard);
                tokio::time::sleep(backoff).await;
                continue;
            }

            let result = provider.responses(request).await;
            let total_latency = start.elapsed();

            match result {
                Ok(response) => {
                    guard.decrement();
                    drop(guard);

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
                    drop(guard);

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
                        excluded_providers.push(provider_name.clone());
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
                        .map(|ms| Duration::from_millis(ms))
                        .unwrap_or_else(|| Duration::from_secs(2_u64.saturating_pow(attempt)));

                    tokio::time::sleep(backoff).await;
                }
            }
        }

        Err(last_error.unwrap_or(RouterError::ProviderError(
            ProviderError::Other("Max retries exceeded".to_string().into()),
        )))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RouterError {
    #[error("No available provider for routing")]
    NoAvailableProvider,

    #[error("Provider error: {0}")]
    ProviderError(ProviderError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::openai::OpenAiProvider;
    use crate::router::strategies::round_robin::RoundRobinStrategy;
    use crate::metrics::{MetricsStore, ProviderMetrics, MetricsEvent, FailureDetails, ErrorType};
    use std::sync::Arc;

    async fn setup_test_router() -> (Router, MetricsStore) {
        let db = Arc::new(Database::new("sqlite::memory:").await.unwrap());
        let metrics_store = MetricsStore::new(1000);
        
        let router = Router::new(
            Arc::new(RoundRobinStrategy::new()),
            metrics_store.clone(),
            db.clone(),
        );
        
        (router, metrics_store)
    }

    #[tokio::test]
    async fn test_resolve_route_prefers_available_providers() {
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
        
        // Resolve route for "default" model - should prefer provider1 (available)
        let result = router.resolve_route_excluding("default", &[]).await;
        
        assert!(result.is_some(), "Should find a route");
        let (resolved_provider, _) = result.unwrap();
        assert_eq!(resolved_provider.name(), "Provider1", "Should prefer available provider");
    }

    #[tokio::test]
    async fn test_resolve_route_fallback_to_unavailable_when_all_excluded() {
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
        
        // Exclude provider1 - should still fall back to provider2 (unavailable but not excluded)
        let result = router.resolve_route_excluding("default", &["Provider1".to_string()]).await;
        
        assert!(result.is_some(), "Should find a fallback route even if unavailable");
        let (resolved_provider, _) = result.unwrap();
        assert_eq!(resolved_provider.name(), "Provider2", "Should fall back to unavailable but not-excluded provider");
    }

    #[tokio::test]
    async fn test_resolve_route_excludes_explicitly_excluded_providers() {
        let (router, _metrics_store) = setup_test_router().await;
        
        let provider1 = Arc::new(OpenAiProvider::new("Provider1", Some("provider1"), "http://localhost:8001", Some("key")));
        let provider2 = Arc::new(OpenAiProvider::new("Provider2", Some("provider2"), "http://localhost:8002", Some("key")));
        
        router.add_provider(provider1.clone()).await;
        router.add_provider(provider2.clone()).await;
        
        // Exclude provider1
        let result = router.resolve_route_excluding("default", &["Provider1".to_string()]).await;
        
        assert!(result.is_some(), "Should find a route");
        let (resolved_provider, _) = result.unwrap();
        assert_eq!(resolved_provider.name(), "Provider2", "Should exclude explicitly excluded provider");
    }
}
