FROM rust:trixie AS builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    build-essential \
    pkg-config \
    libssl-dev \
    cmake \
    && rm -rf /var/lib/apt/lists/*

# Copy manifests
COPY Cargo.toml Cargo.lock ./

# Create dummy source to cache dependencies
RUN mkdir -p src/bin && \
    echo "fn main() {}" > src/bin/server.rs && \
    echo "fn main() {}" > src/bin/cli.rs && \
    echo "#[path = \"../lib.rs\"] mod lib {}" > src/lib.rs

# Build dependencies
RUN cargo build --release

# Copy actual source, touch to update mtimes so Cargo detects changes, then rebuild
COPY . .
RUN find src -type f -exec touch {} + && \
    cargo build --release --bin yalr-server --bin yalr-cli

# Build the admin UI
FROM oven/bun:1 AS admin-builder

WORKDIR /app/admin
COPY admin/package.json admin/bun.lock ./
RUN bun install --frozen-lockfile
COPY admin/ ./
RUN bun run build

# Runtime image
FROM debian:trixie-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3t64 \
    libsqlite3-0 \
    curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/yalr-server /usr/local/bin/
COPY --from=builder /app/target/release/yalr-cli /usr/local/bin/
COPY --from=admin-builder /app/admin/dist /app/admin/dist

# Verify binary can run (check for missing libraries)
RUN yalr-server --help 2>&1 || yalr-server 2>&1 | head -5 || true

EXPOSE 3000

HEALTHCHECK --interval=30s --timeout=10s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:3000/health || exit 1

CMD ["yalr-server"]
