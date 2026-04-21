# YALR - Yet Another LLM Router

Async LLM router with load balancing, provider management, and streaming support.

## Features

- **Load Balancing**: Round-robin and other routing strategies for distributing requests across multiple LLM providers
- **Provider Abstraction**: Clean trait-based architecture supporting multiple LLM providers (OpenAI, LlamaCpp, etc.)
- **Streaming Support**: Full support for streaming responses across all providers
- **Async First**: Built on tokio for high-performance async I/O
- **Metrics Collection**: Built-in metrics for monitoring provider performance (latency, throughput, success rates)
- **Health Checks**: API endpoints for monitoring router health and provider status
- **Admin UI**: Web-based interface for managing providers, API keys, and viewing metrics
- **Authentication**: Session-based auth with API key support for production use

## Quick Start

### Docker

```bash
docker run -p 3000:3000 \
  -v $(pwd)/data:/app/data \
  voidic/yalr:latest
```

Access the admin UI at http://localhost:3000

### From Source

```bash
# Build and run server
cargo run --bin yalr-server

# Or run with debug info
cargo run --bin yalr-server -- --verbose
```

Access the admin UI at http://localhost:3000

## Configuration

YALR can be configured via environment variables or a configuration file (`config.yaml`).

### Environment Variables

- `HOST`: Server host (default: `0.0.0.0`)
- `PORT`: Server port (default: `3000`)
- `RUST_LOG`: Logging level (e.g., `info`, `debug`, `trace`)

### Configuration File

Create a `config.yaml` in the working directory:

```yaml
server:
  host: "0.0.0.0"
  port: 3000

database:
  url: "sqlite:data/llm_router.db?mode=rwc"

auth:
  enabled: true
  allowed_pubkeys:
    - "your-nostr-pubkey"
```

## API Endpoints

### Admin UI

The admin UI provides a web interface for:
- **Dashboard**: Overview of system status and metrics
- **Providers**: Create, list, and delete LLM providers
- **Router Config**: View routing strategy and provider configurations
- **Metrics**: Real-time provider performance metrics
- **API Keys**: Manage API keys for authentication

### REST API

#### Authentication
- `POST /api/auth/setup` - Setup first admin user
- `POST /api/auth/login` - Login and get session token
- `POST /api/auth/logout` - Logout current session
- `GET /api/auth/status` - Check authentication status

#### Providers
- `GET /api/providers` - List all providers
- `POST /api/providers` - Create new provider
- `DELETE /api/providers/:slug` - Delete provider by slug

#### Configuration
- `GET /api/config` - Get router configuration (strategy, providers, metrics)

#### Metrics
- `GET /api/metrics` - Get provider performance metrics

#### API Keys
- `GET /api/api-keys` - List API keys
- `POST /api/api-keys` - Create new API key
- `POST /api/api-keys/:id/disable` - Disable API key
- `POST /api/api-keys/:id/enable` - Enable API key
- `DELETE /api/api-keys/:id` - Delete API key

#### Router
- `GET /health` - Health check
- `POST /v1/chat/completions` - Chat completion endpoint
- `GET /v1/models` - List available models

## Architecture

```
┌─────────────────┐
│   Admin UI      │
│  (React/TS)     │
└────────┬────────┘
         │
┌────────▼────────┐
│   YALR Server   │
│   (Rust/Axum)   │
└────────┬────────┘
         │
    ┌────┴────┐
    │         │
┌───▼──┐  ┌──▼───┐
│Route │  │Auth  │
│Engine│  │Keys  │
└───┬──┘  └──┬───┘
    │        │
┌───▼────────▼──┐
│  Providers    │
│ (OpenAI, etc) │
└───────────────┘
```

## Development

### Building

```bash
# Build server
cargo build --release --bin yalr-server

# Build admin UI
cd admin && bun run build

# Build Docker image
docker build -t yalr .
```

### Testing

```bash
cargo test
```

## License

MIT
