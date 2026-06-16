FROM rust:trixie AS rust-deps

WORKDIR /app

# Keep CI builds lean and reproducible.
ENV CARGO_INCREMENTAL=0 \
    CARGO_NET_RETRY=10 \
    CARGO_TERM_COLOR=never

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

# Build dependencies only. NO cache mounts: artifacts must land in the image
# layer so they are exported via `cache-to: type=gha,mode=max` and reused on the
# next run. BuildKit `type=cache` mounts are NOT persisted across GHA runners,
# which previously caused every dependency to recompile from scratch (twice).
# This layer is invalidated only when Cargo.toml / Cargo.lock change.
RUN cargo build --release --bin yalr-server --bin yalr-cli && \
    rm -f target/release/*yalr* target/release/deps/*yalr*

# Build the admin UI
FROM oven/bun:1 AS admin-builder

WORKDIR /app/admin

COPY admin/package.json admin/bun.lock ./
RUN bun install --frozen-lockfile

COPY admin/ ./
RUN bun run build

# Rust application build
FROM rust-deps AS rust-build

# Copy actual source code (specific files needed for build)
COPY src ./src
COPY migrations ./migrations

# Touch entry points so Cargo sees them as changed vs. the stubs above
RUN touch src/lib.rs src/bin/server.rs src/bin/cli.rs

# Rebuild - dependencies cached from the layer above, only our crate recompiles.
RUN cargo build --release --bin yalr-server --bin yalr-cli

# Run tests reusing the release artifacts (avoids a full dev-profile recompile).
RUN cargo test --release --lib

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
