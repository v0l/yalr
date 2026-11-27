# Payments move to the Apex payment gateway

YALR stops collecting money itself. Every top-up (Lightning, card, on-chain) goes
through the Apex payment gateway (`pay.apexstrata.ie`, sandbox `pay-sandbox.apexstrata.ie`).
YALR stays the system of record for balances, pricing, the ledger and settlement
accounting. Only collecting money in moved.

Gateway API used: `docs/reference.md` in the payment-gateway repo and
`GET /v1/openapi.json`. Auth is `Authorization: Bearer <api key>`. Amounts are minor
units (msats for `BTC`, cents for `EUR` and `USD`). Webhooks carry
`X-Gateway-Event-Id` and `X-Gateway-Signature: t=<unix>,v1=<hex>` where
`v1 = HMAC-SHA256(webhook_secret, "{t}.{body}")`.

## Config

The gateway is one of two payment providers now; see
[payment-providers.md](payment-providers.md) for direct LND.

```yaml
payments:
  enabled: true
  gateway:
    url: https://pay.apexstrata.ie
    currencies: [btc, eur, usd]     # btc credits sats, the rest credit per-currency balances
    min_amount: 5
    max_amount: 500
    success_url: https://yalr.v0l.io/payments   # must be in the gateway tenant allowed_origins
```

Secrets come from the environment, never the file:

| Variable | Purpose |
| --- | --- |
| `GATEWAY_API_KEY` | tenant API key (`pgw_live_...`) |
| `GATEWAY_WEBHOOK_SECRET` | tenant webhook signing secret |

`payments.lnd`, `payments.revolut`, `REVOLUT_API_KEY`, `REVOLUT_WEBHOOK_SECRET` and
`YALR_ENCRYPTION_KEY` are gone. `payments.provider_fee_ppm`, `default_pricing` stay.

## What was analysed

Money moving paths in the old `src/payments/` (4,067 lines) and their callers.

| Old path | What it did | Disposition |
| --- | --- | --- |
| `payments/lightning.rs` `create_invoice` (LND add_invoice) | user top-up invoice | gateway payment (BTC) + `lightning` attempt, bolt11 from `details.bolt11` |
| `payments/lightning.rs` settlement listener (LND subscribe) | credit on settle | gateway webhook `payment.paid` |
| `payments/lightning.rs` `pay_invoice` | refunds and Lightning payouts | removed, see Payouts |
| `payments/anonymous.rs` | `anon_*` user and key per paid invoice | same flow on a gateway payment, same `derive_api_key` |
| `payments/revolut/mod.rs` `create_topup` | Revolut order + in-page card field | gateway payment (EUR/USD), user is sent to `pay_url` (hosted page) |
| `payments/revolut/mod.rs` `reconcile`, runner | poll Revolut | webhook, plus a reconciler that `GET /v1/payments/{id}` for pending rows |
| `payments/revolut/mod.rs` cards | store customer and payment method ids | gateway saved methods (`save_method` on the attempt, `customer_ref = user-<id>`) |
| `payments/revolut/charge.rs` auto-reload | off-session charge | `POST /v1/saved-methods/{id}/charge` |
| `payments/routstr.rs` | Routstr node protocol: `/v1/info`, `/v1/balance/info`, balance refund, `/lightning/invoice` | `info` and `balance_info` stay (listing). `/lightning/invoice` and status become gateway backed. `balance_refund` becomes a payout request. `create_provider_invoice` proxies an upstream provider's invoice, it is not YALR money, it stays |
| `providers/routstr.rs`, `providers/ppq.rs` `create_topup` | YALR topping up its own upstream balance | untouched, not a collection path |
| `payouts/` | holds, approval, limits, Lightning via LND, fiat via Revolut Business payout links | see Payouts |

HTTP routes:

| Old | New |
| --- | --- |
| `POST /lightning/invoice`, `GET /lightning/invoice/:hash/status` | same paths, gateway backed (status is by payment hash, looked up in `gateway_payments`) |
| `POST /v2/lightning/invoice`, `GET /v2/lightning/invoice/:id/status`, `POST /v2/lightning/recover`, `/v1/balance/lightning/invoice*` | same paths and shapes. `recover` also reads `legacy_lightning_invoices` |
| `GET/POST /api/account/topup/fiat*`, `/account/topup/fiat/config` | `POST /api/account/topup`, `GET /api/account/topup`, `GET /api/account/topup/:id`, `GET /api/account/topup/config` (all currencies, Lightning and card) |
| `GET/DELETE /api/account/payment-methods*`, `PUT .../default` | `GET/DELETE /api/account/payment-methods` backed by gateway saved methods. Default is dropped, auto-reload names the method |
| `GET/PUT /api/account/auto-reload` | same, `payment_method_id` is the gateway saved method id |
| `GET /api/payments/fiat-orders` | `GET /api/payments/topups` (admin) |
| `POST /api/webhooks/revolut` | `POST /api/webhooks/gateway` |
| `GET /api/payments/invoices` | reads `gateway_payments` |

## New tables

`gateway_payments` is the single order table for every top-up:
`public_id` (our id, used as the gateway `reference`), `gateway_payment_id` (unique),
`user_id`, `kind` (`topup`, `auto_reload`, `anon_create`, `anon_topup`), `currency`,
`amount_milli` (balance units, 1000 per minor unit for fiat, msats for BTC),
`status` (`pending`, `completed`, `expired`, `cancelled`, `failed`), `bolt11`,
`payment_hash`, `pay_url`, `expires_at`, `completed_at`.

The balance credit and the `pending -> completed` flip happen in one transaction,
guarded by `WHERE status = 'pending'`, so a replayed or duplicated webhook credits once.
Ledger reference is `gw-<public_id>`, type `deposit`.

## Webhook rules

1. Reject a body whose signature does not match, or whose timestamp is older than 5 minutes.
2. `payment.paid` and `payment.late_payment` credit a pending (or expired) row.
3. The credit is the amount the gateway reports paid, only if it is at least our expected
   amount and the currency matches. A mismatch marks the row `failed` with a reason and
   credits nothing.
4. `payment.expired`, `payment.cancelled`, `attempt.failed` (no live attempt) close a pending row.
5. Unknown events answer 200. Unknown payment ids answer 200 and are logged.
6. A reconciler runs every 30 s over pending rows older than 2 minutes and fetches the
   payment, so a missed webhook still settles.

## Anonymous flow

`POST /v2/lightning/invoice {amount_sats, purpose}`: create the gateway payment and a
`lightning` attempt, store a `gateway_payments` row with `public_id`. For `create` the
`anon_<public_id[..12]>` user and key are created up front exactly as today, so
`derive_api_key(server_secret, public_id)` is unchanged. Existing anon keys keep working
because users, keys and `server_secret` are not touched. Status returns the key once
`completed`. The unpaid pruner now looks at `gateway_payments` and `legacy_lightning_invoices`.

## Payouts

The gateway has no `/v1` withdrawal API. Withdrawals exist only in the operator console
(`/console/api/projects/{id}/withdrawals`, session auth, owner role) and move the tenant's
held BTC balance. It also has no fiat payout. So:

* YALR keeps the payout request, hold period, limits, approval and cancel logic and the
  accounting (`payouts` table, debit on request, refund on reject).
* The LND client is removed, so YALR can no longer send Lightning itself. A Lightning payout
  reaches `held` and waits. An operator sends it from the gateway console (or the LND
  behind it) and records it with `POST /api/payouts/:id/settle {external_id}`, which marks it
  `sent` then `completed`. `reject` and `cancel` still release the debit.
* Revolut Business payout links (fiat payouts) are a different Revolut product and API from
  the Merchant client being removed. They stay as they are.
* `POST /v1/balance/refund` becomes a Lightning payout request for the invoice amount.
* A gateway issue is filed for a `/v1/withdrawals` API. When it ships, `settle` is replaced
  by an automatic call in `PayoutService::process`.

## Cutover rules (supervisor step, after user approval and payment-gateway #852 live)

1. Create the YALR tenant on `pay.apexstrata.ie`: `POST /admin/tenants` with
   `webhook_url=https://yalr.v0l.io/api/webhooks/gateway`, add the providers (Lightning on the
   gateway LND, Revolut live, optional on-chain), set `allowed_origins` to the YALR origin.
2. Put `GATEWAY_API_KEY` and `GATEWAY_WEBHOOK_SECRET` in a new Secret `yalr-gateway`.
3. Back up the YALR database.
4. Drain: stop creating new invoices (deploy the new image behind the old one is not safe),
   wait for the old pending set to settle or expire:
   * Lightning invoices expire after 1 h at most. Wait 1 h after the last old-version
     invoice, with the old pod still running so the LND listener credits late payments.
   * Revolut orders: `reconcile_stale` settles within 2 minutes of payment. Anything still
     `pending` after 1 h is abandoned by the card flow.
5. Deploy the new image (step 6). The migration is the one-shot cutover:
   * balances, `balance_transactions` and `payouts` are untouched,
   * any still-`pending` `lightning_invoices` become `expired` and `fiat_orders` become
     `failed` (reason `closed at gateway cutover`) before the tables are renamed. A payer who
     paid after the drain is credited by hand with `/api/payments/credit`, using the
     `legacy_*` rows to find them,
   * `auto_reload_settings` is recreated empty (the old rows are kept in
     `legacy_auto_reload_settings`), so users set it up again with a new gateway card.
6. The sqlx migration renames `lightning_invoices`, `fiat_orders`,
   `user_payment_methods` to `legacy_*` and creates `gateway_payments`. Nothing is dropped.
7. Register no Revolut webhook for YALR any more. The gateway registers its own. Delete the
   old YALR Revolut merchant webhook after the new deploy is healthy.
8. Remove the `lnd` and `yalr-revolut` Secret mounts and the `YALR_ENCRYPTION_KEY`,
   `REVOLUT_*` env entries from `yalr/yalr.yml`.
9. Smoke test: a small Lightning top-up, a card top-up, confirm both credited once.
10. A later migration (after a backup and a quiet month) drops the `legacy_*` tables.

## Saved cards

The gateway cannot import Revolut customer or payment method ids (no endpoint accepts
them), and saved methods are tied to its own provider config. Decision: saved cards are not
moved. `legacy_user_payment_methods` is kept read-only, users add a card again with
"save card" on their next top-up, and auto-reload stays off until they pick the new card.

## Payout destinations and history

`payouts` rows and their destinations stay in YALR (YALR still does payout accounting).
Nothing is copied into the gateway. History stays readable.

## Secrets and dependencies removed

k8s Secret mounts to remove (listed for the supervisor, not edited here): `lnd` (TLS cert,
macaroon), `yalr-revolut` (`api-key`, `webhook-secret`, `encryption-key`).
Cargo: `payments-rs` (with `method-lnd`, `method-revolut`) is removed. The gateway is reached
through the official `payment-gateway-client` crate (git dependency, pinned rev), including
`verify_webhook`.
