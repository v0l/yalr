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
}

pub struct Router {
    db: Arc<Database>,
    metrics_store: MetricsStore,
    providers: RwLock<HashMap<String, Arc<dyn Provider>>>,
    routing_tables: RwLock<HashMap<String, RoutingTable>>,
    max_retries: u32,
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

    /// Collect all candidate (provider, resolved_model) pairs for a given model,
    /// ordered by preference (available providers first, then unavailable as fallback).
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

        let mut available = Vec::new();
        let mut unavailable = Vec::new();

        for entry in &table.entries {
            let resolved_model = entry
                .model_override
                .clone()
                .unwrap_or_else(|| model.to_string());
            let pair = (entry.provider.clone(), resolved_model);
            if self.metrics_store.is_provider_available(entry.provider.name()).await {
                available.push(pair);
            } else {
                unavailable.push(pair);
            }
        }

        // Available providers first, then unavailable as fallback
        available.extend(unavailable);
        available
    }

    /// Normalize chat request messages for a given provider:
    /// 1. Convert `developer` role to `system` for providers that don't support it.
    /// 2. Move all system messages to the beginning of the array.
    ///
    /// Some providers (e.g. OpenAI) reject requests where system messages
    /// appear after user/assistant messages, and many backends don't support
    /// the `developer` role at all.
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

        // Move system messages to the beginning
        let messages = &mut request.messages;
        let first_non_system = messages.iter().position(|m| !matches!(m, ChatCompletionRequestMessage::System(_)));
        if let Some(first_non_system_idx) = first_non_system {
            let has_misplaced = messages[first_non_system_idx..]
                .iter()
                .any(|m| matches!(m, ChatCompletionRequestMessage::System(_)));
            if has_misplaced {
                let (system_msgs, other_msgs): (Vec<_>, Vec<_>) =
                    std::mem::take(messages).into_iter().partition(|m| matches!(m, ChatCompletionRequestMessage::System(_)));
                *messages = system_msgs.into_iter().chain(other_msgs.into_iter()).collect();
            }
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
                        .map(|ms| Duration::from_millis(ms))
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

                                    // If no chunks have been sent yet and the error is transient,
                                    // fail over to the next provider instead of surfacing the error.
                                    if !chunks_yielded && e.is_transient() {
                                        tracing::warn!(
                                            provider = &provider_name,
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
                                        }).map(|ms| Duration::from_millis(ms))
                                          .unwrap_or_else(|| Duration::from_millis(200 * (attempt as u64)));
                                        tokio::time::sleep(backoff).await;

                                        break; // Continue to next provider in the outer loop
                                    }

                                    // Either we already sent data (can't retry) or error is non-transient
                                    if !e.is_transient() {
                                        tracing::warn!(
                                            provider = &provider_name,
                                            attempt = attempt,
                                            error = %e,
                                            "Non-transient stream error, aborting"
                                        );
                                    } else {
                                        tracing::warn!(
                                            provider = &provider_name,
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
                                .map(|ms| Duration::from_millis(ms))
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
        let should_transform = !provider_name.to_lowercase().contains("openai");
        
        if should_transform {
            if let InputParam::Items(items) = transformed.input {
                let transformed_items: Vec<async_openai::types::responses::InputItem> = items
                    .into_iter()
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
                    .collect();
                
                transformed.input = InputParam::Items(transformed_items);
            }
        }
        
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
                        .map(|ms| Duration::from_millis(ms))
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
    fn test_normalize_moves_system_messages_to_front() {
        use crate::providers::{
            ChatCompletionRequestUserMessage, ChatCompletionRequestUserMessageContent,
        };

        let mut request = ChatCompletionRequest {
            model: "test".to_string(),
            messages: vec![
                ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                    content: ChatCompletionRequestUserMessageContent::Text("hello".to_string()),
                    name: None,
                }),
                ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage {
                    content: ChatCompletionRequestSystemMessageContent::Text("system prompt".to_string()),
                    name: None,
                }),
                ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                    content: ChatCompletionRequestUserMessageContent::Text("world".to_string()),
                    name: None,
                }),
            ],
            ..Default::default()
        };

        Router::normalize_chat_request(&mut request, "vllm");

        assert!(matches!(&request.messages[0], ChatCompletionRequestMessage::System(_)));
        assert!(matches!(&request.messages[1], ChatCompletionRequestMessage::User(_)));
        assert!(matches!(&request.messages[2], ChatCompletionRequestMessage::User(_)));
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
    fn test_normalize_developer_in_middle_moved_to_front_as_system() {
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

        // Developer converted to System and moved to front
        assert!(matches!(&request.messages[0], ChatCompletionRequestMessage::System(_)));
        assert!(matches!(&request.messages[1], ChatCompletionRequestMessage::User(_)));
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
}
