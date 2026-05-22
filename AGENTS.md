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

## Serde &amp; API Parsing Conventions

- **Always use proper serde structs** for API request/response bodies — never `serde_json::Value` for parsing external API responses
- **Never use `serde_json::json!` macro** — define a `#[derive(Serialize)]` request struct instead
- Request structs use `#[derive(Debug, Serialize)]`, response structs use `#[derive(Debug, Deserialize)]`
- Place request/response structs near the method that uses them, marked private (`struct`) unless shared
- Use `#[serde(alias = "...")]` for handling multiple possible field names from upstream APIs
- Use `#[serde(skip_serializing_if = "Option::is_none")]` for optional request fields
- Use `#[serde(default)]` for optional response fields that may be absent

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

## Admin UI

### Tech Stack
- **Framework**: React 19 + TypeScript 6 + Vite 8
- **Styling**: Tailwind CSS v4 + shadcn/ui (radix-nova style, neutral base)
- **Libraries**: React Router 7, Recharts, Zustand, assistant-ui (chat), lucide-react icons
- **Font**: Geist Variable (via @fontsource-variable/geist)
- **Package manager**: Bun
- **Location**: `admin/` (separate package from Rust workspace)

### Directory Structure
```
admin/
├── src/
│   ├── main.tsx                     # Entry point, wraps App with ThemeProvider
│   ├── App.tsx                      # Routes: /, /setup, /login, /providers, /config, /metrics, /users, /users/:id, /payments, /chat
│   ├── App.css                      # Empty (all styles in index.css)
│   ├── index.css                    # Tailwind v4 + shadcn + dark mode vars + custom layers
│   ├── api/
│   │   └── client.ts               # All API calls (fetch-based, Bearer token auth)
│   ├── components/
│   │   ├── TopupDialog.tsx          # Top-up dialog for routstr/ppq providers
│   │   └── ui/                      # shadcn/ui components (alert-dialog, alert, badge, button, card, checkbox, dialog, input, label, select, separator, skeleton, table)
│   ├── context/
│   │   └── ThemeContext.tsx          # Light/dark theme toggle, persisted to localStorage
│   ├── layout/
│   │   └── Layout.tsx               # Top navbar + <Outlet/> for authenticated pages
│   ├── lib/
│   │   └── utils.ts                 # cn() helper, formatBalance()
│   ├── pages/
│   │   ├── Chat.tsx                 # assistant-ui chat interface, model selector, SSE streaming
│   │   ├── Config.tsx               # Routing configs CRUD + provider assignments per config
│   │   ├── Dashboard.tsx            # Stats cards, provider health table, WebSocket live updates
│   │   ├── Login.tsx                # Username/password login form
│   │   ├── Metrics.tsx              # Real-time WebSocket metrics, charts (recharts), event stream, health panel
│   │   ├── Payments.tsx             # Tabbed: Balances, Model Pricing, Transactions, Invoices
│   │   ├── Providers.tsx            # Provider CRUD, quick-add common providers, API key gen
│   │   ├── Setup.tsx                # Initial setup wizard (first admin user)
│   │   ├── UserDetail.tsx           # Single user view: detail, API keys, model permissions
│   │   └── Users.tsx                # User list table, create/delete
│   └── types/
│       └── index.ts                 # All TypeScript interfaces (Provider, User, Metrics*, Payments*, etc.)
├── components.json                  # shadcn config (radix-nova, neutral base, lucide icons)
├── vite.config.ts
├── package.json
└── index.html
```

### Route Map
| Path | Page | Auth | Notes |
|------|------|------|-------|
| `/setup` | Setup | Public | Initial admin setup |
| `/login` | Login | Public | Username/password |
| `/` | Dashboard | Private + SetupCheck | Stats overview, provider health |
| `/providers` | Providers | Private + SetupCheck | CRUD, quick-add templates |
| `/config` | Config | Private + SetupCheck | Routing configs with nested provider assignments |
| `/metrics` | Metrics | Private + SetupCheck | WebSocket live events, charts, model breakdown |
| `/users` | Users | Private + SetupCheck | User CRUD |
| `/users/:id` | UserDetail | Private + SetupCheck | User detail, API keys, model permissions |
| `/payments` | Payments | Private + SetupCheck | 4 tabs: balances, pricing, transactions, invoices |
| `/chat` | Chat | Private + SetupCheck | assistant-ui chat with streaming |

### Auth Flow
1. `App.tsx` wraps authenticated routes in `<SetupCheckRoute>` → `<PrivateRoute>` → `<Layout />`
2. `PrivateRoute` checks `localStorage.getItem('token')` then calls `GET /api/auth/status`
3. `SetupCheckRoute` calls `GET /api/setup/status` to see if initial admin exists
4. Login stores `token` + `user` (username, isAdmin) in localStorage
5. API client (`client.ts`) auto-attaches `Authorization: Bearer ${token}` to all requests

### Key State Dependencies
- `ThemeContext`: persisted theme (light/dark), toggled by button in Layout
- `localStorage`: token, user object, theme preference
- WebSocket: Dashboard and Metrics pages connect to `/api/metrics/ws?token=...` for real-time events
- Recharts: Metrics page has 2 charts (P90 TTFT, P90 Tokens/Second) fed from `/api/metrics/history`

## Admin UI Design Philosophy

- **Refined minimalism** — Stripe-dashboard inspired. No wasted chrome, tight spacing.
- **Sections are bordered panels**, not heavy cards. Use `rounded-lg border bg-card p-4` instead of `Card`/`CardHeader`/`CardContent` for internal pages. Cards are for primary dashboard tiles only.
- **No `Separator` components** between sections — 16px spacing (`gap-4`) is enough.
- **Typography**: Section headers are `text-sm font-semibold`. Labels are `text-[10px] uppercase tracking-wider`. Use `tabular-nums` on all numeric values.
- **Badges are compact**: `text-[10px] px-1.5 py-0 h-5` consistently across the app.
- **Tables favor lightness**: `border-b border-border/50` for rows, `hover:bg-muted/30` for hover, hide less-important columns on smaller breakpoints.
- **Two-column grids on lg+** (`grid-cols-1 lg:grid-cols-2 gap-4`), single-column below. Balance + Permissions side-by-side on desktop.
- **Info boxes are collapsible** behind a `?` toggle instead of always-visible blocks.
- **Page wrapper**: `space-y-4 p-4 sm:p-5` — full width, no max-width clamp. Layout handles padding at the nav level.
- **User identity banner**: Not a card — just a header row with badges inline. Username, type badge, admin badge, created date, external ID all on one compact line.
- API keys table uses raw `<table>` instead of shadcn `Table` for finer control over column visibility at breakpoints.

## Authentication & Authorization

See [AUTH.md](AUTH.md) for authentication methods, access control, and user management.
