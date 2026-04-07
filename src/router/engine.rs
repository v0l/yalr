use crate::metrics::{MetricsStore};
use crate::{
    ChatCompletionChunk, ChatCompletionRequest, ChatCompletionResponse, ProviderError,
};
use crate::providers::Provider;
use crate::router::strategies::{RoutingEngine, RoutingStrategy};
use async_stream::stream;
use futures::stream::BoxStream;
use futures::StreamExt;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

pub struct Router {
    engine: Arc<RwLock<RoutingEngine>>,
    metrics_store: MetricsStore,
    max_retries: u32,
}

impl Router {
    pub fn new(strategy: Box<dyn RoutingStrategy>, metrics_store: MetricsStore) -> Self {
        let engine = RoutingEngine::new(Arc::from(strategy));
        Self {
            engine: Arc::new(RwLock::new(engine)),
            metrics_store,
            max_retries: 3,
        }
    }

    pub async fn add_provider(&self, provider: Arc<dyn Provider>) {
        let engine = self.engine.write().await;
        engine.add_provider(provider.clone()).await;
    }

   pub async fn chat_completions(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, RouterError> {
        let start = Instant::now();
        
        let engine = self.engine.read().await;
        let provider = self.select_provider(&engine, &request.model).await
            .ok_or(RouterError::NoAvailableProvider)?;
        let provider_name = provider.name().to_string();
        let model = request.model.clone();

        let mut attempt = 0;
        let mut last_error = None;

        loop {
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

            if attempt >= self.max_retries {
                return Err(last_error.unwrap_or(RouterError::ProviderError(
                    ProviderError::ProviderError("Max retries exceeded".to_string())
                )));
            }

            let result = provider.chat_completions(request).await;
            let total_latency = start.elapsed();

            match result {
                Ok(response) => {
                    let latency_ms = total_latency.as_millis() as u32;
                    self.metrics_store
                        .emitter()
                        .emit_total_latency(&provider_name, &model, latency_ms);
                    self.metrics_store
                        .emitter()
                        .emit_success(&provider_name, &model);
                    
                    if let Some(tokens) = response.usage.as_ref() {
                        let output_tokens_per_sec = tokens.completion_tokens as f32 / (total_latency.as_secs_f64().max(0.001)) as f32;
                        let input_tokens_per_sec = tokens.prompt_tokens as f32 / (total_latency.as_secs_f64().max(0.001)) as f32;
                        
                        tracing::info!(
                            provider = %provider_name,
                            model = %model,
                            prompt_tokens = tokens.prompt_tokens,
                            completion_tokens = tokens.completion_tokens,
                            total_tokens = tokens.total_tokens,
                            total_latency_ms = latency_ms,
                            output_tokens_per_second = output_tokens_per_sec,
                            input_tokens_per_second = input_tokens_per_sec,
                            "Emitting tokens metrics"
                        );
                        
                        self.metrics_store
                            .emitter()
                            .emit_output_tokens_per_second(&provider_name, &model, output_tokens_per_sec);
                        self.metrics_store
                            .emitter()
                            .emit_input_tokens_per_second(&provider_name, &model, input_tokens_per_sec);
                        self.metrics_store
                            .emitter()
                            .emit_input_tokens(&provider_name, &model, tokens.prompt_tokens as u32);
                        self.metrics_store
                            .emitter()
                            .emit_output_tokens(&provider_name, &model, tokens.completion_tokens as u32);
                    }
                    
                    return Ok(response);
                }
                Err(e) => {
                    last_error = Some(RouterError::ProviderError(e.clone()));
                    
                    self.metrics_store.emitter().emit_failure_with_details(
                        &provider_name,
                        &model,
                        e.error_type(),
                        None,
                        &e.to_string(),
                        e.retry_after_ms(),
                        e.status_code(),
                    );

                    if attempt >= self.max_retries - 1 {
                        return Err(last_error.unwrap());
                    }

                    let backoff = e.retry_after_ms()
                        .map(|ms| Duration::from_millis(ms))
                        .unwrap_or_else(|| Duration::from_secs(2_u64.saturating_pow(attempt)));

                    tracing::warn!(
                        provider = &provider_name,
                        attempt = attempt,
                        error = %last_error.as_ref().unwrap(),
                        backoff_ms = backoff.as_millis(),
                        "Request failed, retrying after backoff"
                    );

                    tokio::time::sleep(backoff).await;
                    attempt += 1;
                }
            }
        }
    }

pub async fn chat_completions_stream(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<BoxStream<'static, Result<ChatCompletionChunk, RouterError>>, RouterError> {
        let model = request.model.clone();
        tracing::info!(
            model = &model,
            stream = true,
            "Routing streaming request"
        );
        
        let (provider_name, model, provider) = {
            let engine = self.engine.read().await;
            let provider = self.select_provider(&engine, &request.model)
                .await
                .ok_or(RouterError::NoAvailableProvider)?;
            (provider.name().to_string(), request.model.clone(), provider)
        };

        let metrics_store = self.metrics_store.clone();
        let request = request.clone();

        let stream = stream! {
            let start = Instant::now();
            let mut first_token = true;
            let mut total_tokens = 0u32;
            let mut prompt_tokens = 0u32;
            let mut completion_tokens = 0u32;
            let mut ttft_ms = 0u32;

            let provider_stream = match provider.chat_completions_stream(&request) {
                Ok(stream) => stream,
                Err(e) => {
                    metrics_store.emitter().emit_failure_with_details(
                        &provider_name,
                        &model,
                        e.error_type(),
                        None,
                        &e.to_string(),
                        e.retry_after_ms(),
                        e.status_code(),
                    );
                    yield Err(RouterError::ProviderError(e));
                    return;
                }
            };

            futures::pin_mut!(provider_stream);

            while let Some(result) = provider_stream.next().await {
                match result {
                    Ok(chunk) => {
                        if first_token {
                            first_token = false;
                            ttft_ms = start.elapsed().as_millis() as u32;
                            metrics_store.emitter().emit_ttft(&provider_name, &model, ttft_ms);
                        }
                        
                        if let Some(usage) = chunk.usage.clone() {
                            prompt_tokens = usage.prompt_tokens;
                            completion_tokens = usage.completion_tokens;
                            total_tokens = usage.total_tokens;
                        }
                        
                        yield Ok(chunk);
                    }
                    Err(e) => {
                        metrics_store.emitter().emit_failure_with_details(
                            &provider_name,
                            &model,
                            e.error_type(),
                            None,
                            &e.to_string(),
                            e.retry_after_ms(),
                            e.status_code(),
                        );
                        yield Err(RouterError::ProviderError(e));
                        return;
                    }
                }
            }

            if !first_token {
                metrics_store.emitter().emit_success(&provider_name, &model);
                let total_latency_ms = start.elapsed().as_millis() as u32;
                metrics_store.emitter().emit_total_latency(&provider_name, &model, total_latency_ms);
                
                if total_tokens > 0 {
                    let generation_time_ms = total_latency_ms.saturating_sub(ttft_ms) as f32;
                    let output_tokens_per_sec = completion_tokens as f32 / (generation_time_ms / 1000.0).max(0.001);
                    let input_tokens_per_sec = prompt_tokens as f32 / (start.elapsed().as_secs_f64().max(0.001)) as f32;
                    
                    tracing::info!(
                        provider = %provider_name,
                        model = %model,
                        prompt_tokens = prompt_tokens,
                        completion_tokens = completion_tokens,
                        total_tokens = total_tokens,
                        total_latency_ms = total_latency_ms,
                        output_tokens_per_second = output_tokens_per_sec,
                        input_tokens_per_second = input_tokens_per_sec,
                        "Emitting tokens metrics"
                    );
                    
                    metrics_store.emitter().emit_output_tokens_per_second(&provider_name, &model, output_tokens_per_sec);
                    metrics_store.emitter().emit_input_tokens_per_second(&provider_name, &model, input_tokens_per_sec);
                    metrics_store.emitter().emit_input_tokens(&provider_name, &model, prompt_tokens);
                    metrics_store.emitter().emit_output_tokens(&provider_name, &model, completion_tokens);
                }
            }
        };

        Ok(Box::pin(stream))
    }

    async fn select_provider(
        &self,
        engine: &RoutingEngine,
        model: &str,
    ) -> Option<Arc<dyn Provider>> {
        if let Some((slug_prefix, _actual_model)) = model.split_once('/') {
            return engine.route_by_slug(slug_prefix).await;
        }
        
        engine.route(model).await
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RouterError {
    #[error("No available provider for routing")]
    NoAvailableProvider,

    #[error("Provider error: {0}")]
    ProviderError(ProviderError),
}
