# Payment providers

YALR takes money through one payment provider, picked in `config.yaml`. Balances, the
ledger, pricing and settlement stay in YALR whichever you pick. Revolut payouts are
separate and unchanged.

| | `gateway` (Apex payment gateway) | `lnd` (your own LND) |
| --- | --- | --- |
| Lightning top-ups | yes, bolt11 from the gateway | yes, bolt11 from your node |
| Card top-ups (EUR, USD) | yes, hosted pay page | no |
| Saved cards and auto-reload | yes | no |
| On-chain | yes, through the gateway pay page | no |
| Anonymous keys (`/v2/lightning/invoice`) | yes | yes, same key derivation |
| Routstr `/lightning/invoice`, `/v1/balance/lightning/invoice` | yes | yes |
| `/v1/balance/refund` | refunded against a past gateway deposit | paid from your node |
| Lightning payouts | held for an operator to settle | paid from your node |
| Needs | a gateway tenant (API key, webhook secret) | an LND node, its `tls.cert` and a macaroon |

Pick `lnd` when you run a Lightning node and only want sats. Pick `gateway` when you need
cards or do not want to run a node. The hosted instance uses the gateway.

## Choosing

```yaml
payments:
  enabled: true
  provider: lnd        # or gateway; optional
```

Without `provider`, a `payments.gateway` section selects the gateway, otherwise a
`payments.lnd` section selects LND. With both sections and no `provider`, the gateway
wins, so an older config keeps working. YALR refuses to start with a clear message when
the named provider has no section, the LND url is empty, the cert or macaroon file cannot
be read, or `network` is unknown.

## Direct LND

```yaml
payments:
  enabled: true
  provider: lnd
  lnd:
    url: https://127.0.0.1:10009
    tls_cert_path: /lnd/tls.cert
    macaroon_path: /lnd/admin.macaroon
    network: mainnet            # mainnet, testnet, signet or regtest; shown in the top-up dialog
    invoice_expiry_secs: 3600
    repair_interval_secs: 300
```

The macaroon needs `invoices:read invoices:write` for top-ups and `offchain:read
offchain:write` for refunds and payouts. An `invoice.macaroon` is enough when you do not
use refunds or Lightning payouts.

How settlement works:

- Each invoice is a row in `gateway_payments` with `provider = 'lnd'` and the payment hash
  as its remote id. Crediting is one transaction that only moves a pending row to
  completed, so a replayed settle event never credits twice.
- YALR follows `SubscribeInvoices` from the last stored `settle_index`
  (`payment_cursors.lnd_settle_index`). When the stream ends or LND restarts it
  resubscribes from that cursor.
- On every resubscribe and every `repair_interval_secs` it looks up each pending invoice
  with `LookupInvoice`, so invoices paid while YALR was down are credited even if the
  cursor was lost.
- An invoice settled for less than asked is marked failed and not credited. A cancelled
  or expired invoice credits nothing.

Card endpoints (`/api/account/payment-methods`, `/api/account/auto-reload`) answer 404
saying cards need `payments.gateway`, and a top-up in a currency other than sats is
refused with the same message.

To try it locally against a regtest node pair:

```bash
scripts/lnd-regtest.sh          # bitcoind and two LND nodes; prints the lnd: settings
scripts/lnd-regtest.sh down
```

## Gateway

See [payments-gateway-migration.md](payments-gateway-migration.md). The settings stay as
they were: `payments.gateway` in `config.yaml`, `GATEWAY_API_KEY` and
`GATEWAY_WEBHOOK_SECRET` in the environment, webhook at `/api/webhooks/gateway`.

## Code

`src/payments/provider.rs` holds the `PaymentProvider` trait (create a top-up, refresh its
status, webhook, refund, amount rules). `src/payments/gateway/` and `src/payments/lnd/`
implement it, and `PaymentsState::provider()` returns the configured one. The same
conformance cases run against a fake gateway and a fake LND in
`src/api/payment_provider_tests.rs`.
