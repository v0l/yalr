use serde::Serialize;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Debug, Clone, Default, Serialize)]
pub struct Attempt {
    pub provider: String,
    pub upstream_url: Option<String>,
    pub upstream_status: Option<u16>,
    pub error: Option<String>,
    pub at_ms: i64,
}

#[derive(Debug, Clone, Default)]
pub struct Upstream {
    pub url: Option<String>,
    pub request_headers: Vec<(String, String)>,
    pub request_body: Option<Vec<u8>>,
    pub status: Option<u16>,
    pub response_headers: Vec<(String, String)>,
    pub headers_ms: Option<i64>,
    pub first_event_ms: Option<i64>,
    pub first_thinking_ms: Option<i64>,
    pub first_text_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cached_tokens: u32,
    pub cache_write_tokens: u32,
}

#[derive(Debug, Default)]
struct Inner {
    usage: Option<Usage>,
    provider: Option<String>,
    upstream: Upstream,
    attempts: Vec<Attempt>,
}

#[derive(Debug, Clone)]
pub struct Trace {
    started: Instant,
    inner: Arc<Mutex<Inner>>,
}

impl Default for Trace {
    fn default() -> Self {
        Self::new()
    }
}

impl Trace {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            inner: Arc::new(Mutex::new(Inner::default())),
        }
    }

    pub fn elapsed_ms(&self) -> i64 {
        self.started.elapsed().as_millis() as i64
    }

    fn with<R>(&self, f: impl FnOnce(&mut Inner) -> R) -> Option<R> {
        self.inner.lock().ok().map(|mut inner| f(&mut inner))
    }

    pub fn begin_attempt(&self, provider: &str) {
        self.with(|inner| {
            inner.provider = Some(provider.to_string());
            inner.upstream = Upstream::default();
        });
    }

    pub fn upstream_request(&self, url: &str, headers: Vec<(String, String)>, body: Vec<u8>) {
        self.with(|inner| {
            inner.upstream.url = Some(url.to_string());
            inner.upstream.request_headers = headers;
            inner.upstream.request_body = Some(body);
        });
    }

    pub fn upstream_response(&self, status: u16, headers: Vec<(String, String)>) {
        let at = self.elapsed_ms();
        self.with(|inner| {
            inner.upstream.status = Some(status);
            inner.upstream.response_headers = headers;
            inner.upstream.headers_ms.get_or_insert(at);
        });
    }

    pub fn first_event(&self) {
        let at = self.elapsed_ms();
        self.with(|inner| {
            inner.upstream.first_event_ms.get_or_insert(at);
        });
    }

    pub fn first_thinking(&self) {
        let at = self.elapsed_ms();
        self.with(|inner| {
            inner.upstream.first_thinking_ms.get_or_insert(at);
        });
    }

    pub fn first_text(&self) {
        let at = self.elapsed_ms();
        self.with(|inner| {
            inner.upstream.first_text_ms.get_or_insert(at);
        });
    }

    pub fn end_attempt(&self, error: Option<String>) {
        let at = self.elapsed_ms();
        self.with(|inner| {
            let attempt = Attempt {
                provider: inner.provider.clone().unwrap_or_default(),
                upstream_url: inner.upstream.url.clone(),
                upstream_status: inner.upstream.status,
                error,
                at_ms: at,
            };
            inner.attempts.push(attempt);
        });
    }

    pub fn set_usage(&self, usage: Usage) {
        self.with(|inner| inner.usage = Some(usage));
    }

    pub fn usage(&self) -> Option<Usage> {
        self.with(|inner| inner.usage).flatten()
    }

    pub fn attempts(&self) -> Vec<Attempt> {
        self.with(|inner| inner.attempts.clone()).unwrap_or_default()
    }

    pub fn provider(&self) -> Option<String> {
        self.with(|inner| inner.provider.clone()).flatten()
    }

    pub fn upstream(&self) -> Upstream {
        self.with(|inner| inner.upstream.clone()).unwrap_or_default()
    }
}

pub fn mark_chunk(trace: &Trace, chunk: &crate::providers::StreamingChunk) {
    for choice in &chunk.choices {
        let delta = &choice.delta;
        let reasoning = delta.reasoning_content.as_deref().is_some_and(|t| !t.is_empty())
            || delta.extra_fields.get("reasoning").and_then(|v| v.as_str()).is_some_and(|t| !t.is_empty());
        if reasoning {
            trace.first_thinking();
        }
        if delta.content.as_deref().is_some_and(|t| !t.is_empty()) || delta.tool_calls.is_some() {
            trace.first_text();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_attempt_clears_the_previous_upstream_but_keeps_the_history() {
        let trace = Trace::new();
        trace.begin_attempt("a");
        trace.upstream_request("https://a/x", vec![], b"{}".to_vec());
        trace.upstream_response(429, vec![]);
        trace.end_attempt(Some("rate limited".into()));
        trace.begin_attempt("b");
        assert_eq!(trace.upstream().url, None);
        assert_eq!(trace.attempts().len(), 1);
        assert_eq!(trace.attempts()[0].upstream_status, Some(429));
        assert_eq!(trace.provider().as_deref(), Some("b"));
    }

    #[test]
    fn usage_survives_a_later_attempt() {
        let trace = Trace::new();
        trace.begin_attempt("a");
        trace.set_usage(Usage { prompt_tokens: 10, completion_tokens: 2, cached_tokens: 5, cache_write_tokens: 1 });
        trace.end_attempt(None);
        assert_eq!(trace.usage().map(|u| u.cached_tokens), Some(5));
    }

    #[test]
    fn first_marks_keep_the_earliest_time() {
        let trace = Trace::new();
        trace.begin_attempt("a");
        trace.first_event();
        let first = trace.upstream().first_event_ms;
        std::thread::sleep(std::time::Duration::from_millis(5));
        trace.first_event();
        assert_eq!(trace.upstream().first_event_ms, first);
    }
}
