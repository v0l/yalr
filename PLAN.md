# YALR - Implementation Plan

## Overview

YALR (Yet Another LLM Router) is an async LLM routing proxy with load balancing, provider abstraction, multi-tenant support, and admin UI.

---

## Current State Summary

| Component | Status | Notes |
|-----------|--------|-------|
| Provider Trait & Implementations | ✅ Complete | OpenAI, LlamaCpp, Routstr, PPQ, Anthropic, OpenRouter |
| Routing Engine | ✅ Complete | Round-robin, model-prefixed routing, retry/fallback |
| Health & Metrics | ✅ Complete | MetricsStore, provider health states, backoff |
| API Layer | ✅ Complete | OpenAI-compatible + admin endpoints |
| Authentication | ✅ Complete | NIP-98, sessions, API keys |
| User Management | ✅ Complete | Internal + Nostr users, admin/regular roles |
| Payments (Beta) | 🟡 Partial | Routstr balance tracking, top-ups, billing guards |
| Rate Limiting/Quotas | ❌ Pending | Phase 6 - not yet implemented |
| Per-Model Access Control | ❌ Pending | Future enhancement |

---

## Architecture

### Core Components

1. **Provider Trait Interface** (`src/providers/provider_trait.rs`)
   - Abstract trait for LLM providers with streaming support
   - OpenAI, LlamaCpp, Routstr, PPQ implementations
   - Metrics tracking via shared `MetricsStore`
   - Error classification (rate limit, server error, timeout, etc.)

2. **Routing Engine** (`src/router/engine.rs`, `src/router/model_router.rs`)
   - Model-prefixed routing (`provider/model` → direct to provider)
   - Load-balanced routing (round-robin, configurable strategies)
   - Health-aware provider selection
   - Retry with exponential backoff + automatic fallback

3. **API Layer** (`src/api/handlers.rs`, `src/api/server.rs`)
   - OpenAI-compatible: `/v1/chat/completions`, `/v1/models`, `/v1/responses`
   - Admin API: `/api/*` (providers, users, metrics, routing config, payments)
   - WebSocket: `/api/metrics/ws` for real-time health updates

4. **Authentication & Authorization** (`src/auth/`)
   - NIP-98 (Nostr HTTP auth)
   - Session-based (browser/admin UI)
   - API keys (SHA-256 hashed, named, expirable)
   - Admin vs regular user roles

5. **Database** (`src/db/mod.rs`)
   - SQLite via sqlx
   - Tables: providers, models, routing_config, users, api_keys, payments
   - Migrations in `./migrations/`

6. **Metrics & Health** (`src/metrics.rs`)
   - Per-provider health states (Healthy, Degraded, Unhealthy)
   - In-flight request counts, latency, token tracking
   - Exponential backoff with automatic recovery

7. **Payments** (`src/payments/`)
   - Balance tracking (msats, sats, USD micro)
   - Billing guards (reservation + finalization)
   - Routstr integration (balance info, refunds, invoices)
   - Provider top-up via Lightning

---

## Project Structure

```
src/
├── bin/
│   ├── server.rs          # Main server entry point
│   └── cli.rs             # CLI tool (optional)
├── api/
│   ├── server.rs          # Axum router setup
│   └── handlers.rs        # Request handlers (chat, admin, metrics)
├── auth/
│   ├── nip98.rs           # NIP-98 extraction & validation
│   ├── admin.rs           # Sessions, login/logout, middleware
│   └── api_keys.rs        # API key CRUD
├── config.rs              # Config loading (YAML + DB)
├── db/
│   └── mod.rs             # Database connection, queries, schema
├── metrics.rs             # MetricsStore, health tracking
├── payments/
│   ├── mod.rs             # Payments module
│   ├── guard.rs           # Billing guard (reservation/finalization)
│   ├── biller.rs          # Usage-based billing
│   ├── instructions.rs    # Top-up payment instructions
│   └── routstr.rs         # Routstr provider integration
├── providers/
│   ├── provider_trait.rs  # Provider trait definition
│   ├── openai.rs          # OpenAI implementation
│   ├── llamacpp.rs        # LlamaCpp implementation
│   ├── routstr.rs         # Routstr implementation
│   └── ppq.rs             # PPQ implementation
├── router/
│   ├── engine.rs          # Core routing logic
│   └── model_router.rs    # ModelRequestRouter (prefixed routing)
└── state.rs               # AppState (shared across handlers)

migrations/
├── 20240101000001_initial_schema.sql
├── 20240421000001_add_name_to_routing_config.sql
├── 20240422000001_add_backing_model_to_routing_config.sql
├── 20240424000001_add_provider_type.sql
└── 20250514000001_add_payments.sql
```

---

## Key Dependencies

```toml
# Async runtime
tokio = { version = "1", features = ["full"] }

# HTTP server
axum = { version = "0.7", features = ["macros", "ws"] }
tower = "0.4"
tower-http = { version = "0.5", features = ["trace", "cors"] }

# HTTP client
reqwest = { version = "0.11", features = ["json", "stream"] }

# Async OpenAI
async-openai = "0.21"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Async utilities
futures = "0.3"
async-stream = "0.3"

# Database
sqlx = { version = "0.7", features = ["sqlite", "runtime-tokio-rustls"] }

# Error handling
thiserror = "1"
anyhow = "1"

# Logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# Time
chrono = { version = "0.4", features = ["serde"] }

# Auth
nostr = "0.31"
argon2 = "0.5"
sha2 = "0.10"
uuid = { version = "1", features = ["v4"] }

# Testing
wiremock = "0.5"
```

---

## Database Schema

### Providers
```sql
CREATE TABLE providers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT UNIQUE NOT NULL,
    slug TEXT UNIQUE NOT NULL,
    base_url TEXT NOT NULL,
    api_key TEXT,
    provider_type INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
```

### Users
```sql
CREATE TABLE users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    username TEXT UNIQUE,
    password_hash TEXT,
    external_id TEXT,
    user_type INTEGER NOT NULL DEFAULT 0,  -- internal, nostr, oauth
    is_admin BOOLEAN DEFAULT 0,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    CHECK (username IS NOT NULL OR external_id IS NOT NULL),
    UNIQUE(external_id, user_type)
);
```

### API Keys
```sql
CREATE TABLE api_keys (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    key_hash TEXT UNIQUE NOT NULL,
    name TEXT NOT NULL,
    user_id INTEGER NOT NULL,
    last_four TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP,
    is_active BOOLEAN DEFAULT 1,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);
```

### Routing Config
```sql
CREATE TABLE routing_config (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT,
    strategy TEXT NOT NULL DEFAULT 'round_robin',
    health_check_enabled BOOLEAN DEFAULT 1,
    health_check_interval_seconds INTEGER DEFAULT 30,
    health_check_timeout_seconds INTEGER DEFAULT 5,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE routing_config_providers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    routing_config_id INTEGER NOT NULL,
    provider_id INTEGER NOT NULL,
    model TEXT,  -- Optional model-specific routing
    weight INTEGER DEFAULT 100,
    is_active BOOLEAN DEFAULT 1,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (routing_config_id) REFERENCES routing_config(id) ON DELETE CASCADE,
    FOREIGN KEY (provider_id) REFERENCES providers(id) ON DELETE CASCADE
);
```

### Payments
```sql
CREATE TABLE balances (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL,
    provider_id INTEGER,
    currency TEXT NOT NULL,  -- msats, sats, usd_micro
    amount BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (provider_id) REFERENCES providers(id) ON DELETE CASCADE
);

CREATE TABLE transactions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL,
    provider_id INTEGER,
    amount BIGINT NOT NULL,
    transaction_type TEXT NOT NULL,  -- credit, debit, refund
    description TEXT,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);
```

---

## Implementation Phases

### Phase 1-5: Foundation ✅ COMPLETED

#### Phase 1: Foundation
- [x] Update Cargo.toml with dependencies
- [x] Set up database schema and migrations
- [x] Implement Provider trait (`providers/provider_trait.rs`)
- [x] Implement OpenAI provider (`providers/openai.rs`)
- [x] Set up config system with DB access

#### Phase 2: Routing Engine
- [x] Create routing strategy trait
- [x] Implement RoundRobin strategy
- [x] Build routing engine (`router/engine.rs`)
- [x] Model-prefixed routing (`router/model_router.rs`)

#### Phase 3: API Layer
- [x] Set up Axum server (`api/server.rs`)
- [x] Implement OpenAI-compatible handler
- [x] Add streaming support (SSE)
- [x] Add config management endpoints (CRUD for providers, models, routing)
- [x] Add health/metrics endpoints

#### Phase 4: Metrics & Health
- [x] Implement metrics collector (`metrics.rs`)
- [x] Add latency, cost, token tracking
- [x] Implement health check system
- [x] Provider health states with exponential backoff
- [x] Automatic provider recovery

#### Phase 5: Polish
- [x] Add comprehensive error handling
- [x] Add logging (tracing)
- [x] Add configuration hot-reload
- [x] Implement retry/fallback mechanism
- [x] Health-driven provider selection

---

### Phase 6: Authentication & Authorization ✅ COMPLETED

#### Phase 6a: NIP-98 Authentication
- [x] Implement NIP-98 event extraction (`auth/nip98.rs`)
- [x] Validate event signatures, timestamps, tags
- [x] Auto-create Nostr users on first auth
- [x] Optional pubkey whitelist

#### Phase 6b: Session-based Auth
- [x] Session store with in-memory tokens
- [x] Login/logout endpoints
- [x] 24-hour session expiry
- [x] Admin UI integration

#### Phase 6c: API Keys
- [x] SHA-256 hashed API key storage
- [x] Named keys with optional expiration
- [x] Enable/disable without deletion
- [x] Admin can create keys for any user

#### Phase 6d: Authorization Middleware
- [x] `auth_middleware` - extract user from any auth method
- [x] `admin_middleware` - require admin role
- [x] Route-level protection

**See**: [AUTH.md](AUTH.md) for detailed documentation.

---

### Phase 7: Payments (Beta) ✅ IN PROGRESS

#### Phase 7a: Balance Tracking
- [x] Per-user, per-provider balance tables
- [x] Multi-currency support (msats, sats, USD micro)
- [x] Balance query endpoints

#### Phase 7b: Billing Guards
- [x] Usage reservation before request
- [x] Finalization after completion
- [x] Insufficient funds handling (402 Payment Required)

#### Phase 7c: Provider Top-ups
- [x] Top-up instructions per provider
- [x] PPQ account creation via API
- [x] Routstr Lightning invoice generation
- [x] Payment tracking

#### Phase 7d: Admin Billing
- [x] Admin credit/debit endpoints
- [x] Transaction history
- [x] Balance details per user

**Status**: Core billing flow working, integration tests pending.

---

### Phase 8: Rate Limiting & Quotas ❌ PENDING

#### Phase 8a: Rate Limiter
- [ ] Token bucket algorithm per provider
- [ ] Configurable limits (req/sec, req/min, tokens/min, tokens/hour)
- [ ] Return 429 Too Many Requests when exceeded
- [ ] Per-provider rate limit config in DB

#### Phase 8b: Usage Quotas
- [ ] Daily/monthly token quotas per provider
- [ ] Daily/monthly request quotas
- [ ] Quota tracking table with usage aggregation
- [ ] Admin API to view/reset quotas

#### Phase 8c: Admin APIs
- [ ] GET /api/usage/:user_id - View user usage
- [ ] GET /api/providers/:id/usage - View provider usage
- [ ] POST /api/quotas/reset - Reset quota
- [ ] GET /api/rate-limits - View rate limit config
- [ ] PUT /api/rate-limits/:id - Update rate limits

---

### Phase 9: Access Control (Future) ❌ PENDING

#### Phase 9a: Model Access Control
- [ ] Create `user_model_permissions` table
- [ ] Allow/deny model access per user or role
- [ ] Check permissions in chat handler
- [ ] Admin UI for permission management

#### Phase 9b: Role-Based Access (Optional)
- [ ] Define roles (admin, power_user, regular, restricted)
- [ ] Role-model associations
- [ ] Simplified permission management

---

### Phase 10: Integration Testing ❌ PENDING

- [ ] Wiremock integration tests for providers
- [ ] End-to-end routing tests
- [ ] Auth flow tests (NIP-98, sessions, API keys)
- [ ] Payment flow tests (billing, top-ups)
- [ ] Load testing with concurrent requests

---

## Health Tracking System

### Health States
- **Healthy**: Normal operation, accepting requests
- **Degraded**: Elevated errors, accepting with backoff
- **Unhealthy**: High failure rate, temporarily blocked

### State Transitions
```
Healthy → Degraded (consecutive_failures ≥ 2)
Degraded → Unhealthy (consecutive_failures ≥ 5)
Any → Healthy (successful request)
```

### Backoff Formula
```
backoff_ms = min(base_backoff * 2^consecutive_failures, max_backoff)
```

### Error Classification
- **RateLimit**: 429 with `retry_after`
- **ServerError**: 5xx responses
- **Timeout**: Request timeout
- **Authentication**: 401/403
- **NotFound**: 404
- **Other**: Unclassified

---

## Configuration

### config.yaml
```yaml
server:
  host: "0.0.0.0"
  port: 3000
  admin_ui_path: "admin/dist"

database:
  url: "sqlite:llm_router.db?mode=rwc"

auth:
  enabled: true
  allowed_pubkeys:
    - "your_nostr_pubkey_here"

payments:
  enabled: false
  default_pricing:
    price_per_1m_input_sats: 0
    price_per_1m_output_sats: 0
    price_per_request_sats: 0
  lnd:
    url: "https://lnd:10009"
    tls_cert_path: "admin.cert"
    macaroon_path: "admin.macaroon"
```

### Environment Variables
- `HOST` - Server host (overrides config)
- `PORT` - Server port (overrides config)
- `RUST_LOG` - Logging level (e.g., `info`, `debug`, `yalr=trace`)

---

## Testing Conventions

- **Inline tests**: `#[cfg(test)] mod tests { ... }` at bottom of file
- **No separate test files** (except integration tests in `/tests/`)
- **Wiremock** for HTTP provider mocking
- **In-memory SQLite** (`sqlite::memory:`) for DB tests

---

## Deployment

### Docker
```dockerfile
# Multi-stage build: Rust builder → runtime
FROM rust:1.75 as builder
# ... build steps

FROM debian:bookworm-slim
# ... runtime setup
```

### docker-compose
```yaml
services:
  yalr:
    image: yalr:latest
    volumes:
      - ./data:/app/data
      - ./config.yaml:/app/config.yaml
    ports:
      - "3000:3000"
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:3000/health"]
```

### Admin UI
- React/Vite built with Bun
- Served at `/admin` path
- Built into Docker image

---

## Development Guidelines

1. **Async First**: All I/O must be async (tokio)
2. **Trait-Based**: Use traits for abstractions (testing, extension)
3. **Streaming Support**: All providers must support streaming
4. **Error Handling**: Use `thiserror`, proper propagation
5. **Logging**: Use `tracing` with structured fields
6. **Testing**: Inline tests, aim for >80% coverage on core logic

---

## Related Documentation

- [AGENTS.md](AGENTS.md) - Development guidelines, commands
- [AUTH.md](AUTH.md) - Authentication & authorization
- [README.md](README.md) - Project overview
