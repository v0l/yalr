# YALR - Agent Instructions

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

# Run specific test module
cargo test --lib router::model_router

# Check compilation
cargo check

# Build Docker image
docker build -t yalr .
```

## Architecture

**Entry points**: `src/bin/server.rs`, `src/bin/cli.rs`

**Core routing**:
- `src/router/engine.rs` - Router with provider selection and retry logic
- `src/router/model_router.rs` - ModelRequestRouter for prefixed model routing
- `src/router/strategies/` - Routing strategies (round_robin)

**Providers**: `src/providers/` - OpenAI, LlamaCpp implementations (all implement `Provider` trait in `provider_trait.rs`)

**API**: `src/api/handlers.rs` - Chat completion handlers use both routers

**Metrics**: `src/metrics.rs` - Shared metrics store for health/load tracking

**Database**: `src/db/mod.rs` - SQLite via sqlx with migrations in `./migrations/`

## Model Routing Rules

**Prefixed models** (`provider-1/gpt-4`):
- Split on `/` to get provider slug (`provider-1`) and actual model (`gpt-4`)
- Route directly to that provider via `RoutingEngine::route_by_slug()`
- Bypasses load balancing, goes straight to specified provider

**Unprefixed models** (`gpt-4`):
- Route through `RoutingEngine` for load-balanced selection
- Engine matches model name against `routing_config_providers` table
- Uses round-robin strategy to select from active providers configured for that model
- Falls back to first available routing config if no model-specific match

**Key methods**: `ModelRequestRouter::is_prefixed()`, `extract_prefix()`, `extract_model()`, `RoutingEngine::route_by_slug()`, `RoutingEngine::route()`

## Testing Conventions

- All tests inline in source files: `#[cfg(test)] mod tests { ... }` at bottom of file
- No separate `*_test.rs` files (integration tests in `/tests/` are exceptions)
- Use `wiremock` for HTTP mocking in provider tests
- Use in-memory SQLite (`sqlite::memory:`) for DB tests

## Provider Implementation Rules

- All providers implement `Provider` trait in `src/providers/provider_trait.rs`
- **Always use shared `MetricsStore`** for provider health and load tracking - never implement provider-specific tracking
- Every provider must include unit tests for trait methods, error handling, edge cases
- Providers wrapped in `Arc<dyn Provider>` and stored in `RoutingEngine`

## Metrics Tracking

All Router instances share the same `MetricsStore` for:
- Per-provider in-flight request counts
- Provider health state and backoff timing
- Request outcomes, latency, token usage, throughput

## Configuration

- Providers loaded from SQLite database (`llm_router.db`) via `src/config.rs`
- Config file (`config.yaml`) loaded from working directory - see example in repo
- Environment vars: `HOST`, `PORT`, `RUST_LOG`
- Auth uses NIP-98 (Nostr) - pubkeys configured in `config.yaml`

## Deployment

- **Docker**: Multi-stage build with Rust builder, Bun for admin UI, Debian slim runtime
- **docker-compose**: Volumes for `data/` and `config.yaml`, health check on `/health`
- **Admin UI**: React/Vite built with Bun, served at `/admin` path
- **Database migrations**: Run via `sqlx::migrate!("./migrations")` at startup

## Implementation Plan

See [PLAN.md](./PLAN.md) for implementation roadmap.
