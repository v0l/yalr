#!/usr/bin/env bash
# ── E2E Setup Script ─────────────────────────────────────────────────────
# Sets up the regtest environment:
#   1. Mine enough blocks to mature coinbase
#   2. Fund both LND wallets
#   3. Open a channel from Bob → Alice
#   4. Mine blocks to confirm the channel
#
# Requires:
#   - docker compose -f docker-compose.e2e.yaml up -d  (all services healthy)
#   - bitcoin-cli in PATH (uses docker exec)
#   - lncli in PATH (uses docker exec)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
COMPOSE_FILE="$SCRIPT_DIR/../docker-compose.e2e.yaml"

# ── Helpers ───────────────────────────────────────────────────────────────

bc() { docker compose -f "$COMPOSE_FILE" exec -T bitcoind bitcoin-cli -regtest -rpcuser=polaruser -rpcpassword=polarpass "$@"; }
lncli_alice() { docker compose -f "$COMPOSE_FILE" exec -T lnd-alice lncli --network=regtest "$@"; }
lncli_bob() { docker compose -f "$COMPOSE_FILE" exec -T lnd-bob lncli --network=regtest "$@"; }

echo "=== E2E Setup: Mining initial blocks ==="
# Generate 101 blocks to a wallet so coinbase is mature
bc createwallet "miner" 2>/dev/null || true
ADDR=$(bc getnewaddress)
bc generatetoaddress 101 "$ADDR"

echo "=== E2E Setup: Funding LND wallets ==="
# Alice gets ~10 BTC (on-chain)
ALICE_ADDR=$(lncli_alice newaddress p2wkh | jq -r '.address')
echo "Alice address: $ALICE_ADDR"
bc sendtoaddress "$ALICE_ADDR" 10
bc generatetoaddress 6 "$ADDR"
sleep 3

# Bob gets ~5 BTC (on-chain)
BOB_ADDR=$(lncli_bob newaddress p2wkh | jq -r '.address')
echo "Bob address: $BOB_ADDR"
bc sendtoaddress "$BOB_ADDR" 5
bc generatetoaddress 6 "$ADDR"
sleep 3

# Wait for wallets to sync
echo "Waiting for LND wallets to confirm balance..."
sleep 5

# Check balances
echo "Alice wallet balance:"
lncli_alice walletbalance
echo "Bob wallet balance:"
lncli_bob walletbalance

echo "=== E2E Setup: Opening Bob → Alice channel ==="
# Bob connects to Alice
ALICE_PUBKEY=$(lncli_alice getinfo | jq -r '.identity_pubkey')
ALICE_PORT="10009"
echo "Alice pubkey: $ALICE_PUBKEY"

# Connect Bob to Alice
lncli_bob connect "${ALICE_PUBKEY}@lnd-alice:${ALICE_PORT}" 2>/dev/null || true

# Open a 1M sat channel from Bob to Alice 
# (large enough for multiple test payments)
lncli_bob openchannel \
  --node_key="$ALICE_PUBKEY" \
  --local_amt=1000000 \
  --sat_per_vbyte=1 2>/dev/null || true

sleep 2

# Mine blocks to confirm the channel
bc generatetoaddress 6 "$ADDR"
sleep 5

echo "=== E2E Setup: Checking channel status ==="
echo "Alice channels:"
lncli_alice listchannels
echo "Bob channels:"
lncli_bob listchannels

echo ""
echo "=== E2E Setup Complete ==="
echo "Alice LND node is now funded and has an incoming channel from Bob."
echo "YALR should be reachable at http://localhost:3099"
echo ""
echo "Run tests with: cargo test --test e2e_tests -- --nocapture"
