# YALR - Implementation Plan

## Overview

Build an async LLM router similar to OmniRoute/Bifrost with load balancing, provider abstraction, and future web UI support.

## Architecture

### Core Components

1. **Provider Trait Interface**
   - Abstract trait for LLM providers with streaming support
   - OpenAI implementation as primary provider
   - Designed for easy extension to other providers

2. **Load Balancing Engine**
   - Routing strategy trait with multiple implementations:
     - RoundRobin: Simple cyclic distribution
     - Weighted: Custom weights per endpoint
     - CostBased: Route to cheapest available model
     - LatencyBased: Route to fastest healthy endpoint
   - Health tracking per endpoint
   - Metrics collection for routing decisions

3. **API Layer**
   - OpenAI-compatible endpoint (`/v1/chat/completions`)
   - Config management endpoints
   - Health/metrics endpoints

4. **Configuration System**
   - Database-backed configuration (SQLite/PostgreSQL)
   - REST API for config management
   - Schema for providers, models, routing policies
   - Real-time config updates via API

5. **Async Runtime**
   - Tokio for async operations
   - HTTP server (Axum/Hyper)
   - Reqwest for HTTP client with streaming support
   - Tokio streams for SSE (Server-Sent Events)

## Project Structure

```
src/
├── main.rs              # Entry point, app setup
├── config.rs            # Config loading & hot-reload
├── db/
│   ├── mod.rs           # Database connection & migrations
│   └── schema.rs        # Database schema definitions
├── router/
│   ├── mod.rs
│   ├── engine.rs        # Core routing logic
│   └── strategies/
│       ├── mod.rs
│       └── round_robin.rs
├── providers/
│   ├── mod.rs
│   ├── trait.rs         # Provider trait definition
│   └── openai.rs        # OpenAI implementation
├── api/
│   ├── mod.rs
│   ├── server.rs        # HTTP server setup
│   └── handlers.rs      # Request handlers
├── metrics/
│   ├── mod.rs
│   └── collector.rs     # Latency, cost, health tracking
└── quota/
    ├── mod.rs
    ├── tracker.rs       # Rate limit & quota tracking
    └── limiter.rs       # Rate limiting middleware
```

## Key Dependencies

- `tokio` - Async runtime with full features
- `axum` - HTTP server framework
- `reqwest` - HTTP client with streaming
- `serde` + `serde_json` - Config serialization
- `tokio-stream` - SSE streaming support
- `metrics` or `prometheus-client` - Metrics collection
- `chrono` - Timestamps and time handling
- `thiserror` - Error handling
- `async-trait` - Async trait support
- `sqlx` or `deadpool` + `r2d2` - Database connection pool
- `sqlite` or `postgres` - Database driver
- `tokio` + `parking_lot` - Rate limit token buckets

## Config Schema

### Providers Table
```sql
CREATE TABLE providers (
    id INTEGER PRIMARY KEY,
    name TEXT UNIQUE NOT NULL,
    base_url TEXT NOT NULL,
    api_key_env TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
```

### Rate Limit Config
```sql
CREATE TABLE rate_limits (
    id INTEGER PRIMARY KEY,
    provider_id INTEGER REFERENCES providers(id),
    requests_per_second INTEGER DEFAULT 10,
    requests_per_minute INTEGER DEFAULT 100,
    requests_per_hour INTEGER DEFAULT 1000,
    tokens_per_minute INTEGER DEFAULT 10000,
    tokens_per_hour INTEGER DEFAULT 100000,
    UNIQUE(provider_id)
);
```

### Quota Config
```sql
CREATE TABLE quotas (
    id INTEGER PRIMARY KEY,
    provider_id INTEGER REFERENCES providers(id),
    daily_token_limit INTEGER,
    monthly_token_limit INTEGER,
    daily_request_limit INTEGER,
    monthly_request_limit INTEGER,
    UNIQUE(provider_id)
);
```

### Quota Tracking
```sql
CREATE TABLE quota_usage (
    id INTEGER PRIMARY KEY,
    provider_id INTEGER REFERENCES providers(id),
    usage_date DATE NOT NULL,
    usage_month DATE NOT NULL,
    tokens_used INTEGER DEFAULT 0,
    requests_used INTEGER DEFAULT 0,
    UNIQUE(provider_id, usage_date, usage_month)
);
```

### Models Table
```sql
CREATE TABLE models (
    id INTEGER PRIMARY KEY,
    name TEXT UNIQUE NOT NULL,
    cost_per_1m_input REAL,
    cost_per_1m_output REAL
);
```

### Model Providers (Association)
```sql
CREATE TABLE model_providers (
    id INTEGER PRIMARY KEY,
    model_id INTEGER REFERENCES models(id),
    provider_id INTEGER REFERENCES providers(id),
    weight INTEGER DEFAULT 100,
    is_active BOOLEAN DEFAULT TRUE
);
```

### Routing Configuration
```sql
CREATE TABLE routing_config (
    id INTEGER PRIMARY KEY,
    strategy TEXT NOT NULL,  -- round_robin | weighted | cost_based | latency_based
    health_check_enabled BOOLEAN DEFAULT TRUE,
    health_check_interval_seconds INTEGER DEFAULT 30,
    health_check_timeout_seconds INTEGER DEFAULT 5
);
```

## Implementation Phases

### Phase 1: Foundation
- [x] Update Cargo.toml with dependencies
- [x] Set up database schema and migrations (`db/mod.rs`, `db/schema.rs`)
- [x] Implement Provider trait (`providers/trait.rs`)
- [x] Implement OpenAI provider (`providers/openai.rs`)
- [x] Set up config system with DB access (`config.rs`)

### Phase 2: Routing Engine
- [x] Create routing strategy trait (`router/strategies/mod.rs`)
- [x] Implement RoundRobin strategy
- [ ] Build routing engine (`router/engine.rs`)

### Phase 3: API Layer
- [x] Set up Axum server (`api/server.rs`)
- [x] Implement OpenAI-compatible handler (`api/handlers.rs`)
- [x] Add streaming support (SSE)
- [x] Add config management endpoints (CRUD for providers, models, routing)
- [x] Add health/metrics endpoints

### Phase 4: Metrics & Health
- [x] Implement metrics collector (`metrics.rs`)
- [x] Add latency tracking
- [x] Add cost tracking
- [ ] Implement health check system

### Phase 5: Polish
- [x] Add comprehensive error handling
- [x] Add logging (tracing)
- [x] Add configuration hot-reload
- [ ] Integration testing

### Phase 6: Quota & Rate Limiting
- [ ] Implement rate limiter (`quota/limiter.rs`)
- [ ] Token bucket algorithm per provider
- [ ] Track requests per provider (per second/minute/hour)
- [ ] Track tokens per provider (per minute/hour)
- [ ] Track total usage per provider (daily/monthly quotas)
- [ ] Store rate limit config in DB per provider
- [ ] Check rate limits before routing in engine
- [ ] Return 429 Too Many Requests when limit exceeded
- [ ] Admin API to view provider usage stats
- [ ] Admin API to configure/update rate limits
- [ ] Admin API to reset quotas

## Future Enhancements

- Web UI for configuration (Phase 6)
- Additional provider implementations (Anthropic, Google, etc.)
- Request/response transformation
- Caching layer
- Rate limiting
- Authentication
- Multi-tenant support

## Development Guidelines

1. **Async First**: All I/O operations must be async using tokio
2. **Trait-Based**: Use traits for all abstractions to enable testing and extension
3. **Streaming Support**: All providers must support streaming responses
4. **Error Handling**: Use `thiserror` for library errors, proper error propagation
5. **Logging**: Use `tracing` for structured logging
6. **Testing**: Write unit tests using `#[cfg(test)] mod tests {}` inline within the source file, not as separate test files

### Testing Convention

- All tests should be written as inline modules: `#[cfg(test)] mod tests { ... }`
- Place the test module at the bottom of the file being tested
- Do not create separate `*_test.rs` files
