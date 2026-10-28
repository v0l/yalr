use super::fingerprint::{fingerprint, tracker};
use crate::db::{Database, Record, Retention};
use super::trace::Trace;
use super::codec::pack;
#[cfg(test)]
use super::codec::unpack;
use serde_json::Value;

pub struct Inbound {
    pub request_id: String,
    pub endpoint: &'static str,
    pub stream: bool,
    pub model: String,
    pub user_id: Option<i64>,
    pub user_name: Option<String>,
    pub api_key_id: Option<i64>,
    pub api_key_name: Option<String>,
    pub client_ip: Option<String>,
    pub client_headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Default, Clone)]
pub struct Outcome {
    pub status: String,
    pub error: Option<String>,
    pub finish_reason: Option<String>,
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub cached_tokens: Option<u32>,
    pub cache_write_tokens: Option<u32>,
    pub response: Vec<u8>,
}

pub fn weighted_input(prompt: u32, cached: u32, cache_write: u32) -> i64 {
    let fresh = prompt.saturating_sub(cached).saturating_sub(cache_write) as f64;
    (fresh + cached as f64 * 0.1 + cache_write as f64 * 1.25).round() as i64
}

fn conversation_key(inbound: &Inbound) -> String {
    format!(
        "{}|{}|{}",
        inbound.user_id.unwrap_or_default(),
        inbound.api_key_id.unwrap_or_default(),
        inbound.model
    )
}

fn pairs_json(pairs: &[(String, String)]) -> String {
    serde_json::to_string(pairs).unwrap_or_default()
}

pub fn build(inbound: Inbound, trace: &Trace, outcome: Outcome, started_at_ms: i64) -> Record {
    let upstream = trace.upstream();
    let attempts = trace.attempts();
    let message_count;
    let (system_hash, tools_hash, message_hashes, diverged, previous) =
        match upstream.request_body.as_deref().and_then(|b| serde_json::from_slice::<Value>(b).ok()) {
            Some(body) => {
                let print = fingerprint(&body);
                message_count = Some(print.message_hashes.len() as i64);
                let (diverged, previous) =
                    tracker().compare_and_store(conversation_key(&inbound), &inbound.request_id, &print);
                (
                    Some(print.system_hash),
                    Some(print.tools_hash),
                    serde_json::to_string(&print.message_hashes).unwrap_or_default(),
                    diverged,
                    previous,
                )
            }
            None => {
                message_count = None;
                (None, None, String::new(), None, None)
            }
        };
    let weighted = outcome.prompt_tokens.map(|prompt| {
        weighted_input(
            prompt,
            outcome.cached_tokens.unwrap_or(0),
            outcome.cache_write_tokens.unwrap_or(0),
        )
    });
    Record {
        request_id: inbound.request_id.clone(),
        started_at_ms,
        endpoint: inbound.endpoint.to_string(),
        stream: inbound.stream,
        model: inbound.model.clone(),
        user_id: inbound.user_id,
        user_name: inbound.user_name.clone(),
        api_key_id: inbound.api_key_id,
        api_key_name: inbound.api_key_name.clone(),
        client_ip: inbound.client_ip.clone(),
        client_headers: pairs_json(&inbound.client_headers),
        provider: trace.provider(),
        upstream_url: upstream.url.clone(),
        upstream_request_headers: pairs_json(&upstream.request_headers),
        upstream_status: upstream.status.map(i64::from),
        upstream_response_headers: pairs_json(&upstream.response_headers),
        attempts: attempts.len() as i64,
        attempt_log: serde_json::to_string(&attempts).unwrap_or_default(),
        status: outcome.status.clone(),
        error: outcome.error.clone(),
        finish_reason: outcome.finish_reason.clone(),
        headers_ms: upstream.headers_ms,
        first_event_ms: upstream.first_event_ms,
        first_thinking_ms: upstream.first_thinking_ms,
        first_text_ms: upstream.first_text_ms,
        total_ms: trace.elapsed_ms(),
        prompt_tokens: outcome.prompt_tokens.map(i64::from),
        completion_tokens: outcome.completion_tokens.map(i64::from),
        cached_tokens: outcome.cached_tokens.map(i64::from),
        cache_write_tokens: outcome.cache_write_tokens.map(i64::from),
        weighted_input_tokens: weighted,
        message_count,
        system_hash,
        tools_hash,
        message_hashes,
        first_diverging_message: diverged,
        previous_request_id: previous,
        request_bytes: upstream.request_body.as_ref().map_or(0, |b| b.len() as i64),
        response_bytes: outcome.response.len() as i64,
        client_request_packed: pack(&inbound.body),
        upstream_request_packed: upstream.request_body.as_deref().and_then(pack),
        response_packed: if outcome.response.is_empty() { None } else { pack(&outcome.response) },
    }
}

const WRITE_QUEUE: usize = 256;
const WRITE_BATCH: usize = 64;

#[derive(Clone)]
pub struct Writer {
    sender: tokio::sync::mpsc::Sender<Record>,
}

impl Writer {
    pub fn disconnected() -> Self {
        let (sender, _) = tokio::sync::mpsc::channel::<Record>(1);
        Self { sender }
    }

    pub fn spawn(db: Database) -> Self {
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<Record>(WRITE_QUEUE);
        tokio::spawn(async move {
            let mut batch = Vec::with_capacity(WRITE_BATCH);
            while receiver.recv_many(&mut batch, WRITE_BATCH).await > 0 {
                if let Err(e) = db.insert_request_logs(&batch).await {
                    tracing::error!(error = %e, rows = batch.len(), "Failed to write request log");
                }
                batch.clear();
            }
        });
        Self { sender }
    }

    pub async fn write(&self, record: Record) {
        if self.sender.send(record).await.is_err() {
            tracing::error!("Request log writer stopped");
        }
    }
}

fn spawn_save(writer: Writer, inbound: Inbound, trace: Trace, outcome: Outcome, started_at_ms: i64) {
    tokio::spawn(async move {
        let record = match tokio::task::spawn_blocking(move || build(inbound, &trace, outcome, started_at_ms)).await {
            Ok(record) => record,
            Err(e) => {
                tracing::error!(error = %e, "Failed to build request log record");
                return;
            }
        };
        writer.write(record).await;
    });
}

pub struct Pending {
    writer: Writer,
    inbound: Option<Inbound>,
    trace: Trace,
    started_at_ms: i64,
    collector: super::collector::Collector,
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
    cached_tokens: Option<u32>,
    cache_write_tokens: Option<u32>,
}

impl Pending {
    pub fn new(writer: Writer, inbound: Inbound, trace: Trace) -> Self {
        Self {
            writer,
            inbound: Some(inbound),
            trace,
            started_at_ms: chrono::Utc::now().timestamp_millis(),
            collector: Default::default(),
            prompt_tokens: None,
            completion_tokens: None,
            cached_tokens: None,
            cache_write_tokens: None,
        }
    }

    pub fn trace(&self) -> &Trace {
        &self.trace
    }

    pub fn observe(&mut self, chunk: &crate::providers::StreamingChunk) {
        self.collector.push(chunk);
        if let Some(usage) = &chunk.usage {
            self.prompt_tokens = Some(usage.prompt_tokens);
            self.completion_tokens = Some(usage.completion_tokens);
            self.cached_tokens = usage.prompt_tokens_details.as_ref().and_then(|d| d.cached_tokens);
        }
        if let Some(written) = chunk.extra_fields.get(crate::providers::CACHE_WRITE_TOKENS_FIELD).and_then(|v| v.as_u64()) {
            self.cache_write_tokens = Some(written as u32);
        }
    }

    pub fn observe_usage(&mut self, usage: &async_openai::types::chat::CompletionUsage, cache_write: Option<u32>) {
        self.prompt_tokens = Some(usage.prompt_tokens);
        self.completion_tokens = Some(usage.completion_tokens);
        self.cached_tokens = usage.prompt_tokens_details.as_ref().and_then(|d| d.cached_tokens);
        self.cache_write_tokens = cache_write.or(self.cache_write_tokens);
    }

    pub fn set_response(&mut self, response: Vec<u8>, finish_reason: Option<String>) {
        self.collector.set_raw(response, finish_reason);
    }

    fn outcome(&self, status: &str, error: Option<String>) -> Outcome {
        let routed = self.trace.usage();
        Outcome {
            status: status.to_string(),
            error,
            finish_reason: self.collector.finish_reason(),
            prompt_tokens: routed.map(|u| u.prompt_tokens).or(self.prompt_tokens),
            completion_tokens: routed.map(|u| u.completion_tokens).or(self.completion_tokens),
            cached_tokens: routed.map(|u| u.cached_tokens).or(self.cached_tokens),
            cache_write_tokens: routed.map(|u| u.cache_write_tokens).or(self.cache_write_tokens),
            response: self.collector.assembled(),
        }
    }

    pub fn complete(mut self, status: &str, error: Option<String>) {
        let outcome = self.outcome(status, error);
        self.save(outcome);
    }

    fn save(&mut self, outcome: Outcome) {
        if let Some(inbound) = self.inbound.take() {
            spawn_save(self.writer.clone(), inbound, self.trace.clone(), outcome, self.started_at_ms);
        }
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        if self.inbound.is_some() {
            let outcome = self.outcome("aborted", None);
            self.save(outcome);
        }
    }
}

pub async fn prune_now(db: &Database, retention: &Retention) {
    let now = chrono::Utc::now().timestamp_millis();
    match db.prune_request_logs(now, retention).await {
        Ok(0) => {}
        Ok(n) => tracing::info!(rows = n, "Pruned request log"),
        Err(e) => tracing::error!(error = %e, "Request log prune failed"),
    }
}

pub fn spawn_pruner(db: Database, retention: Retention) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
        loop {
            interval.tick().await;
            prune_now(&db, &retention).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inbound(id: &str) -> Inbound {
        Inbound {
            request_id: id.to_string(),
            endpoint: "/v1/chat/completions",
            stream: true,
            model: format!("model-for-{id}"),
            user_id: Some(1),
            user_name: Some("kieran".into()),
            api_key_id: Some(2),
            api_key_name: Some("pi".into()),
            client_ip: Some("10.0.0.1".into()),
            client_headers: vec![("user-agent".into(), "pi".into())],
            body: b"{\"model\":\"code-high\"}".to_vec(),
        }
    }

    fn upstream_body(messages: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"system": [], "tools": [], "messages": messages})).unwrap()
    }

    fn traced(body: Vec<u8>) -> Trace {
        let trace = Trace::new();
        trace.begin_attempt("Claude Max");
        trace.upstream_request("https://api.anthropic.com/v1/messages", vec![("x-app".into(), "cli".into())], body);
        trace.upstream_response(200, vec![("request-id".into(), "req_1".into())]);
        trace.first_event();
        trace.first_thinking();
        trace.first_text();
        trace.end_attempt(None);
        trace
    }

    #[test]
    fn build_captures_the_whole_timeline_and_cost() {
        let trace = traced(upstream_body(serde_json::json!([{"role":"user","content":"a"}])));
        let outcome = Outcome {
            status: "ok".into(),
            prompt_tokens: Some(1000),
            completion_tokens: Some(50),
            cached_tokens: Some(900),
            cache_write_tokens: Some(50),
            response: b"{\"text\":\"hi\"}".to_vec(),
            ..Default::default()
        };
        let record = build(inbound("build-1"), &trace, outcome, 5);
        assert_eq!(record.provider.as_deref(), Some("Claude Max"));
        assert_eq!(record.upstream_url.as_deref(), Some("https://api.anthropic.com/v1/messages"));
        assert_eq!(record.upstream_status, Some(200));
        assert_eq!(record.attempts, 1);
        assert!(record.headers_ms.is_some() && record.first_thinking_ms.is_some() && record.first_text_ms.is_some());
        assert_eq!(record.weighted_input_tokens, Some(50 + 90 + 63));
        assert_eq!(record.message_count, Some(1));
        assert!(record.client_headers.contains("user-agent"));
        assert_eq!(unpack(record.upstream_request_packed.as_deref().unwrap()).unwrap(), trace.upstream().request_body.unwrap());
        assert_eq!(unpack(record.response_packed.as_deref().unwrap()).unwrap(), b"{\"text\":\"hi\"}");
    }

    #[test]
    fn build_links_a_conversation_and_names_the_diverging_message() {
        let user = |t: &str| serde_json::json!({"role":"user","content":[{"type":"text","text":t}]});
        let first = traced(upstream_body(serde_json::json!([user("a"), user("b")])));
        let second = traced(upstream_body(serde_json::json!([user("a"), user("B"), user("c")])));
        let ok = || Outcome { status: "ok".into(), ..Default::default() };
        let mut one = inbound("link-1");
        let mut two = inbound("link-2");
        two.model = one.model.clone();
        one.model = two.model.clone();
        let r1 = build(one, &first, ok(), 1);
        let r2 = build(two, &second, ok(), 2);
        assert_eq!(r1.previous_request_id, None);
        assert_eq!(r2.previous_request_id.as_deref(), Some("link-1"));
        assert_eq!(r2.first_diverging_message, Some(1));
    }

    #[test]
    fn build_without_an_upstream_body_still_records_the_failure() {
        let trace = Trace::new();
        let outcome = Outcome { status: "error".into(), error: Some("no provider".into()), ..Default::default() };
        let record = build(inbound("fail-1"), &trace, outcome, 1);
        assert_eq!(record.status, "error");
        assert_eq!(record.system_hash, None);
        assert_eq!(record.upstream_request_packed, None);
        assert!(record.client_request_packed.is_some());
    }

    #[test]
    fn weighted_input_matches_anthropic_billing_weights() {
        assert_eq!(weighted_input(1000, 0, 1000), 1250);
        assert_eq!(weighted_input(1000, 900, 50), 50 + 90 + 63);
        assert_eq!(weighted_input(10, 20, 0), 2);
    }
}
