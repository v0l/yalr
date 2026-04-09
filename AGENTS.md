# YALR - Agent Instructions

## Overview

Async LLM router with load balancing, provider abstraction, and streaming support.

## Developer Commands

```bash
# Run server
cargo run --bin yalr-server

# Run CLI
cargo run --bin yalr-cli

# Run all tests
cargo test

# Run library tests only
cargo test --lib

# Check compilation
cargo check
```

## Architecture

- **Entry points**: `src/bin/server.rs`, `src/bin/cli.rs`
- **Core router**: `src/router/engine.rs` - request routing and provider selection
- **Providers**: `src/providers/` - OpenAI, LlamaCpp implementations
- **Metrics**: `src/metrics.rs` - shared metrics store for health/load tracking
- **Routing strategies**: `src/router/strategies/` - round-robin, etc.

## Testing Conventions

- All tests inline in source files: `#[cfg(test)] mod tests { ... }` at bottom of file
- No separate `*_test.rs` files
- Use `wiremock` for HTTP mocking in provider tests

## Provider Implementation Rules

- All providers implement the `Provider` trait in `src/providers/provider_trait.rs`
- **Always use shared `MetricsStore`** for provider health and load tracking - never implement provider-specific tracking outside it
- Every provider must include unit tests for trait methods, error handling, and edge cases
- Providers are wrapped in `Arc<dyn Provider>` and stored in `RoutingEngine`

## Metrics Tracking

All Router instances share the same `MetricsStore` for:
- Per-provider in-flight request counts
- Provider health state and backoff timing
- Request outcomes, latency, token usage, throughput

## Configuration

Providers loaded from SQLite database (`llm_router.db`) via `src/config.rs`. See `config.yaml` for example.

## Implementation Plan

See [PLAN.md](./PLAN.md) for implementation roadmap.
