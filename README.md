# YALR - Yet Another LLM Router

Async LLM router with load balancing, provider abstraction, and streaming support.

## Features

- **Load Balancing**: Round-robin and other routing strategies for distributing requests across multiple LLM providers
- **Provider Abstraction**: Clean trait-based architecture supporting multiple LLM providers (OpenAI, etc.)
- **Streaming Support**: Full support for streaming responses across all providers
- **Async First**: Built on tokio for high-performance async I/O
- **Metrics**: Built-in metrics collection for monitoring provider performance
- **Health Checks**: API endpoints for monitoring router health and provider status

## Quick Start

### Docker

```bash
docker run -p 3000:3000 \
  -e OPENAI_API_KEY=your_key \
  voidic/yalr:latest
```

### From Source

```bash
cargo run --bin yalr-server
```

## Configuration

YALR can be configured via environment variables or a configuration file. See the configuration documentation for details.

## API

- `POST /v1/chat/completions` - Chat completion endpoint
- `GET /health` - Health check endpoint
- `GET /metrics` - Metrics endpoint

## License

MIT
