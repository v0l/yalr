FROM rust:trixie AS rust-deps

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

# Create dummy source for dependency compilation
RUN mkdir -p src/bin && \
    echo "fn main() {}" > src/bin/server.rs && \
    echo "fn main() {}" > src/bin/cli.rs && \
    echo "// Empty lib for dependency caching" > src/lib.rs

# Step 1: Build dependencies only (cached unless Cargo.lock changes)
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release --bin yalr-server --bin yalr-cli

# Step 2: Remove stub binary artifacts so real source rebuilds
RUN rm -f target/release/*yalr*

# Build the admin UI
FROM oven/bun:1 AS admin-builder

WORKDIR /app/admin

# Cache node_modules for faster installs
RUN --mount=type=cache,target=/root/.bun/install/cache \
    mkdir -p /app/admin

COPY admin/package.json admin/bun.lock ./
RUN --mount=type=cache,target=/root/.bun/install/cache \
    bun install --frozen-lockfile

COPY admin/ ./
RUN --mount=type=cache,target=/root/.bun/install/cache \
    bun run build

# Rust application build
FROM rust-deps AS rust-build

# Copy actual source code (specific files needed for build)
COPY src ./src
COPY migrations ./migrations

# Touch entry points so Cargo sees them as changed vs. the stubs above
RUN touch src/lib.rs src/bin/server.rs src/bin/cli.rs

# Rebuild - dependencies cached, only source files recompile
RUN cargo build --release --bin yalr-server --bin yalr-cli

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

COPY --from=rust-build /app/target/release/yalr-server /usr/local/bin/
COPY --from=rust-build /app/target/release/yalr-cli /usr/local/bin/
COPY --from=admin-builder /app/admin/dist /app/admin/dist

RUN yalr-server --version

EXPOSE 3000

HEALTHCHECK --interval=30s --timeout=10s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:3000/health || exit 1

CMD ["yalr-server"]
