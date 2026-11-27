#!/usr/bin/env bash
set -euo pipefail

prefix="${YALR_REGTEST_PREFIX:-yalr-regtest}"
net="$prefix-net"
dir="${YALR_REGTEST_DIR:-/tmp/$prefix}"
rpc_port="${YALR_REGTEST_LND_PORT:-10019}"

if [[ "${1:-}" == "down" ]]; then
    docker rm -f "$prefix-btc" "$prefix-yalr" "$prefix-payer" >/dev/null 2>&1 || true
    docker network rm "$net" >/dev/null 2>&1 || true
    rm -rf "$dir"
    exit 0
fi

mkdir -p "$dir"
docker network inspect "$net" >/dev/null 2>&1 || docker network create "$net" >/dev/null

docker rm -f "$prefix-btc" >/dev/null 2>&1 || true
docker run -d --name "$prefix-btc" --network "$net" \
    -e BITCOIN_NETWORK=regtest \
    -e "BITCOIN_EXTRA_ARGS=rpcuser=u
rpcpassword=p
rpcallowip=0.0.0.0/0
rpcbind=0.0.0.0
server=1
txindex=1
fallbackfee=0.00001
zmqpubrawblock=tcp://0.0.0.0:28332
zmqpubrawtx=tcp://0.0.0.0:28333" \
    btcpayserver/bitcoin:26.0 >/dev/null

btc() { docker exec "$prefix-btc" bitcoin-cli -regtest -rpcuser=u -rpcpassword=p "$@"; }
for _ in $(seq 1 60); do btc getblockchaininfo >/dev/null 2>&1 && break; sleep 1; done

lnd_node() {
    local name="$1" publish="$2"
    docker rm -f "$prefix-$name" >/dev/null 2>&1 || true
    docker run -d --name "$prefix-$name" --network "$net" $publish lightninglabs/lnd:v0.18.5-beta \
        --noseedbackup --norest --bitcoin.active --bitcoin.regtest --bitcoin.node=bitcoind \
        --bitcoind.rpchost="$prefix-btc:18443" --bitcoind.rpcuser=u --bitcoind.rpcpass=p \
        --bitcoind.zmqpubrawblock="tcp://$prefix-btc:28332" --bitcoind.zmqpubrawtx="tcp://$prefix-btc:28333" \
        --rpclisten=0.0.0.0:10009 --tlsextradomain=localhost --tlsextraip=127.0.0.1 --tlsextradomain="$prefix-$name" \
        --protocol.wumbo-channels >/dev/null
}

lnd_node yalr "-p 127.0.0.1:$rpc_port:10009"
lnd_node payer ""

cli() { docker exec "$1" lncli --network=regtest "${@:2}"; }
for node in yalr payer; do
    for _ in $(seq 1 60); do cli "$prefix-$node" getinfo >/dev/null 2>&1 && break; sleep 1; done
done

btc createwallet miner >/dev/null 2>&1 || btc loadwallet miner >/dev/null 2>&1 || true
miner=$(btc -rpcwallet=miner getnewaddress)
btc generatetoaddress 101 "$miner" >/dev/null
field() { sed -n "s/.*\"$1\": *\"\([^\"]*\)\".*/\1/p"; }
payer_addr=$(cli "$prefix-payer" newaddress p2wkh | field address)
btc -rpcwallet=miner sendtoaddress "$payer_addr" 5 >/dev/null
btc generatetoaddress 6 "$miner" >/dev/null

yalr_pub=$(cli "$prefix-yalr" getinfo | field identity_pubkey)
for _ in $(seq 1 30); do
    cli "$prefix-payer" connect "$yalr_pub@$prefix-yalr:9735" >/dev/null 2>&1 && break
    cli "$prefix-payer" listpeers | grep -q "$yalr_pub" && break
    sleep 1
done
for _ in $(seq 1 30); do
    cli "$prefix-payer" openchannel --node_key "$yalr_pub" --local_amt 5000000 >/dev/null 2>&1 && break
    sleep 2
done
btc generatetoaddress 6 "$miner" >/dev/null
for _ in $(seq 1 60); do
    cli "$prefix-payer" listchannels | grep -q '"active": *true' && break
    btc generatetoaddress 1 "$miner" >/dev/null
    sleep 1
done

docker cp "$prefix-yalr:/root/.lnd/tls.cert" "$dir/tls.cert"
docker cp "$prefix-yalr:/root/.lnd/data/chain/bitcoin/regtest/admin.macaroon" "$dir/admin.macaroon"

echo "url: https://127.0.0.1:$rpc_port"
echo "tls_cert_path: $dir/tls.cert"
echo "macaroon_path: $dir/admin.macaroon"
echo "pay with: docker exec $prefix-payer lncli --network=regtest payinvoice --force <bolt11>"
