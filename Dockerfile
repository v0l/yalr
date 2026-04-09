FROM rust:trixie as builder

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

# Clean and copy actual source
RUN rm -rf src
COPY . .

# Build the actual binaries
RUN cargo build --release --bin yalr-server --bin yalr-cli

# Runtime image
FROM debian:trixie-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/yalr-server /usr/local/bin/
COPY --from=builder /app/target/release/yalr-cli /usr/local/bin/

EXPOSE 3000

CMD ["yalr-server"]
