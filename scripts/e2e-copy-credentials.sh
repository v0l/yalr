#!/usr/bin/env bash
# ── Copy LND Credentials from Docker Containers ───────────────────────────
# Copies Bob's LND TLS cert and admin macaroon from the docker container
# to a local temp directory, so the Rust test process can connect.
#
# Run this after `docker compose -f docker-compose.e2e.yaml up -d`

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
COMPOSE_FILE="$SCRIPT_DIR/../docker-compose.e2e.yaml"
DEST_DIR="$SCRIPT_DIR/../lnd-data-bob-temp"

rm -rf "$DEST_DIR"
mkdir -p "$DEST_DIR"

echo "Copying Bob's LND data from container..."
docker compose -f "$COMPOSE_FILE" cp lnd-bob:/root/.lnd "$DEST_DIR" 2>/dev/null || {
    echo "ERROR: Could not copy from lnd-bob container."
    echo "Make sure 'docker compose -f docker-compose.e2e.yaml up -d' is running."
    exit 1
}

# docker cp may nest the target as .lnd/ — flatten if needed
if [ -d "$DEST_DIR/.lnd" ]; then
    mv "$DEST_DIR"/.lnd/* "$DEST_DIR"/
    rmdir "$DEST_DIR/.lnd"
fi

echo "Verifying copied files..."
if [ -f "$DEST_DIR/tls.cert" ] && [ -f "$DEST_DIR/data/chain/bitcoin/regtest/admin.macaroon" ]; then
    echo "OK: TLS cert and admin macaroon copied to $DEST_DIR"
else
    echo "ERROR: Missing expected files"
    echo "  tls.cert: $([ -f "$DEST_DIR/tls.cert" ] && echo 'found' || echo 'MISSING')"
    echo "  macaroon: $([ -f "$DEST_DIR/data/chain/bitcoin/regtest/admin.macaroon" ] && echo 'found' || echo 'MISSING')"
    find "$DEST_DIR" -type f | head -20
    exit 1
fi

echo ""
echo "Credentials copied. Tests can now connect to Bob's LND at localhost:10029"
echo ""
echo "Run tests: cargo test --test e2e_tests -- --nocapture"
