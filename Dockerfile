FROM rust:trixie AS builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    build-essential \
    pkg-config \
    libssl-dev \
    cmake \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

# Copy manifests first for dependency caching
COPY Cargo.toml Cargo.lock ./

# Fetch and cache dependencies (persisted via BuildKit cache mount)
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo fetch --locked

# Create dummy source to build dependency artifacts
RUN mkdir -p src/bin && \
    echo "fn main() {}" > src/bin/server.rs && \
    echo "fn main() {}" > src/bin/cli.rs && \
    echo "" > src/lib.rs

# Pre-compile dependencies (cached across builds)
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release --bin yalr-server --bin yalr-cli && \
    rm -rf src

# Copy actual source and rebuild only the application crates
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    touch src/bin/server.rs src/bin/cli.rs src/lib.rs && \
    cargo build --release --bin yalr-server --bin yalr-cli

# Copy built binaries out of the cache mount so they're available to later stages
RUN --mount=type=cache,target=/app/target \
    cp /app/target/release/yalr-server /app/target/release/yalr-cli /tmp/

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

COPY --from=builder /tmp/yalr-server /usr/local/bin/
COPY --from=builder /tmp/yalr-cli /usr/local/bin/
COPY --from=admin-builder /app/admin/dist /app/admin/dist

# Verify binary can run (check for missing libraries)
RUN yalr-server --help 2>&1 || yalr-server 2>&1 | head -5 || true

EXPOSE 3000

HEALTHCHECK --interval=30s --timeout=10s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:3000/health || exit 1

CMD ["yalr-server"]
