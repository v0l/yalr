//! Example that warms vLLM's automatic prefix cache by sending many identical
//! streaming requests, then verifies `prompt_tokens_details.cached_tokens`
//?  shows > 0 after the cache fills up.
//!
//! Also demonstrates the YALR router path if you point it at localhost:3000.
//!
//! Usage (direct vLLM):
//!   cargo run --example vllm_prefix_cache
//!
//! Usage (through YALR router):
//!   YALR_BASE_URL=http://localhost:3000 \
//!   YALR_AUTH_TOKEN=<token> \
//!   cargo run --example vllm_prefix_cache
//!
//! The direct mode sends 15 cold + 15 warm requests so you see 0 → N cache
//! transition clearly. Through YALR, each request carries the same big system
//! prompt so the shared prefix caches up automatically.

use futures::StreamExt;
use serde_json::{json, Value};
use std::env;
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let base_url = env::var("VLLM_URL")
        .unwrap_or_else(|_| {
            // If YALR auth token is set, go through the YALR router instead.
            if env::var("YALR_AUTH_TOKEN").is_ok() || env::var("YALR_BEARER").is_ok() {
                format!("{}/v1/chat/completions", env::var("YALR_BASE_URL").unwrap_or("http://localhost:3000".into()))
            } else {
                "http://localhost:8001/v1/chat/completions".into()
            }
        });
    let model = env::var("VLLM_MODEL").unwrap_or_else(|_| "qwen3.8-27b-fp8".to_string());

    // A large system prompt (~650 tokens) that stays identical across every
    // request. vLLM's APC uses this shared prefix to serve warm requests
    // from cache after ~3–5 calls.
    let system_prompt = "You are a concise technical assistant who only writes code comments. When asked anything, respond with exactly one line starting with '# ' followed by a relevant observation.\n\n"
        .repeat(12); // ~780 tokens of shared prefix

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .build()?;

    let header: Option<String> = env::var("YALR_BEARER").ok().or_else(|| {
        env::var("YALR_AUTH_TOKEN").ok()
    });

    println!("== vLLM prefix-cache warmup & verification ==");
    println!("url:   {base_url}");
    println!("model: {model}\n");

    let total_requests = 30;
    println!("Sending {} requests with the same {}-token system prompt...\n",
             total_requests,
             system_prompt.len() / 10);

    let mut hits = 0u32;
    let mut misses = 0u32;
    let start = Instant::now();

    for i in 1..=total_requests {
        let body = json!({
            "model": model,
            "stream": true,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": format!("Request #{}", i)}
            ],
            "stream_options": {"include_usage": true}
        });

        let resp = build_request(&client, &base_url, &header, &body).await?;
        let result = parse_stream(resp).await?;

        let cache_val = result.cached_tokens.unwrap_or(0);
        if cache_val > 0 {
            hits += 1;
        } else {
            misses += 1;
        }

        // Print progress every 5th request
        if i % 5 == 0 || i <= 3 {
            println!(
                "  [{:3}/{:<3}] {:4} tok | cached={:<5} | elapsed={:.1}s",
                i, total_requests, result.prompt_tokens, cache_val,
                start.elapsed().as_secs_f64()
            );
        }
    }

    println!("\n--- Summary ---");
    println!("  Cache hits : {} ({:.0}%)", hits, hits as f64 / total_requests as f64 * 100.0);
    println!("  Cache misses: {} ({:.0}%)", misses, misses as f64 / total_requests as f64 * 100.0);
    println!("  Total time : {:.1}s", start.elapsed().as_secs_f64());
    println!("\nIf hits > 0 your prefix cache is working. For the YALR router,");
    println!("cache stats will show up in the metrics event stream (CACHE chip).");

    Ok(())
}

/// Build a reqwest request, optionally adding Bearer auth for YALR.
async fn build_request(
    client: &reqwest::Client,
    url: &str,
    bearer: &Option<String>,
    body: &Value,
) -> Result<reqwest::Response, Box<dyn std::error::Error>> {
    let mut req = client.post(url).json(body);
    if let Some(token) = bearer {
        req = req.header("Authorization", format!("Bearer {token}"));
    }
    Ok(req.send().await?)
}

struct StreamResult {
    prompt_tokens: u64,
    cached_tokens: Option<u64>,
}

async fn parse_stream(mut resp: reqwest::Response) -> Result<StreamResult, Box<dyn std::error::Error>> {
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("{status} {text}").into());
    }

    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    let mut usage_chunk: Option<Value> = None;
    let mut content_len = 0usize;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        buf.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(idx) = buf.find("\n\n") {
            let event = buf[..idx].to_string();
            buf.drain(..idx + 2);

            for line in event.lines() {
                let Some(data) = line.strip_prefix("data: ") else { continue };
                if data == "[DONE]" { continue; }
                let parsed: Value = match serde_json::from_str(data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                // Extract content before moving `parsed` into usage_chunk.
                if let Some(c) = parsed["choices"][0]["delta"]["content"].as_str() {
                    content_len += c.len();
                }
                if !parsed["usage"].is_null() {
                    usage_chunk = Some(parsed);
                }
            }
        }
    }

    let ut = usage_chunk.as_ref().and_then(|u| u["usage"]["prompt_tokens"].as_u64()).unwrap_or(0);
    let ct = usage_chunk.as_ref().and_then(|u| u["usage"]["prompt_tokens_details"]["cached_tokens"].as_u64());

    Ok(StreamResult {
        prompt_tokens: ut,
        cached_tokens: ct,
    })
}
