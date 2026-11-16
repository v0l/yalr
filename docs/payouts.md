# Provider payouts

Providers earn into their YALR balance (`provider_earning` rows from `settle_usage`).
This document records the research and the decisions for letting them withdraw, in
sats over Lightning and in EUR/USD through the Revolut Business API.

Sources: the official OpenAPI spec (`revolut-engineering/revolut-openapi`, `json/business.json`,
tag `2026-11-10`), payments-rs (`~/git/payments-rs`) and LNVPS (`~/git/lnvps_api`).
developer.revolut.com sits behind a bot challenge that neither the fetcher nor the headless
browser passes, so the prose guides were not read. Items that only a guide would settle are
marked **unverified** and must be checked by someone logged in before going live.

## What already exists

- payments-rs has no Revolut Business code. `src/fiat/revolut.rs` is the Merchant API only
  (orders, refunds, webhooks). Its webhook verifier (`RevolutWebhookBody`) is reusable.
- LNVPS has no Business API code either. Its only Revolut sandbox credentials
  (`config.local.yaml`) are for the **Merchant** sandbox. No Business sandbox credentials exist
  on this machine, so no sandbox payout link was created.
- payments-rs `LightningNode::pay_invoice(PayInvoiceRequest)` (feature `method-lnd`) pays a BOLT11.

## Revolut Business API

Base URLs: `https://b2b.revolut.com/api/1.0` (production), `https://sandbox-b2b.revolut.com/api/1.0`.

### Auth

- Current method: an **API key** created in the Business web app (API Console), sent as
  `Authorization: Bearer <key>`. Per-key scopes are chosen at creation: `payout-links:write`,
  `payout-links:read`, `payments:write`, `webhooks:write`, plus read scopes for accounts.
- A key may have **signing enabled**: every request then also carries
  `Revolut-Api-Timestamp` (unix ms) and `Revolut-Api-Signature` (base64 Ed25519 over the request
  data). The message construction is in the API Console guide (**unverified**, not readable).
  The client must support both modes.
- The OAuth flow in the issue (client certificate, JWT client assertion, `refresh_token`, 40 minute
  `access_token`) is marked **deprecated** in the spec ("use API key authentication instead").
  Decision: build for API keys. No refresh logic is needed, which removes the token-refresh
  state from the DB. The scope of the key is the only thing limiting damage, so give it
  payout-links and webhooks only, no `payments:write`, unless bank transfer is enabled.

### Payout link (default fiat method)

| Action | Request | Notes |
| --- | --- | --- |
| Create | `POST /payout-links` | scope `payout-links:write`, returns 201 with the link |
| Get | `GET /payout-links/{id}` | scope `payout-links:read` |
| List | `GET /payout-links` | paged by `created_at` cursor and `limit` |
| Cancel | `POST /payout-links/{id}/cancel` | empty response, only before it is claimed |

Create body (required: `counterparty_name`, `request_id`, `account_id`, `amount`, `currency`,
`reference`):

```json
{
  "counterparty_name": "Jane Doe",
  "request_id": "<payout id>",
  "account_id": "<our Business account uuid for that currency>",
  "amount": 105.6,
  "currency": "EUR",
  "reference": "YALR payout 42",
  "payout_methods": ["revolut", "bank_account", "card"],
  "expiry_period": "P7D",
  "save_counterparty": false
}
```

- `request_id` is the idempotency key. YALR sends the payout id, so a retry of the create can
  never produce a second link.
- Response fields: `id`, `state`, `url` (only while `active`), `expiry_date`, `counterparty_id` and
  `transaction_id` (after the claim), `cancellation_reason`.
- States: `created` (amount not yet blocked), `awaiting` (waiting for approval in Revolut),
  `active` (redeemable, `url` present), `processing`, `processed` (money transferred), and the
  dead ends `failed`, `expired`, `cancelled`.
- Expiry: default and maximum 7 days (`expiry_period`, ISO 8601).
- Amount: between 1 GBP and 2,500 GBP or the equivalent in the chosen currency, per link.
- Recipient choices (`payout_methods`): `revolut` (P2P), `bank_account`, `card`. `card` is
  available in the UK and EEA and **not in the sandbox**.
- Availability: the spec says "available in the UK, the EEA, AU, and SG". Whether that limits the
  sender (our Business account) or also the recipient is **unverified**.
- The link is the only thing YALR stores and shows the provider. The recipient types their
  bank or card details on Revolut's page, so YALR never holds a PAN, IBAN or account number.
- Whether a `card` payout can go to a non-Revolut card is **unverified**. The spec only says the
  method is a "Card transfer" for UK and EEA. The recipient can always fall back to `bank_account`.
- Fees: not stated in the spec, **unverified**. Business plan fee schedules apply to the
  resulting transfer. YALR treats the fee as a configurable flat amount plus ppm deducted from the
  provider's payout (config `fee_*`), reconciled against the real transaction later.

### Bank transfer (fallback, not the default)

1. `POST /counterparty` with `company_name` or `individual_name`, `bank_country`, `currency` and
   `iban` (or `account_no` + `sort_code`/`routing_number`...). The response `id` is the counterparty id.
   `GET /counterparties/fields` and `/counterparties/countries` list requirements per corridor.
2. `POST /pay` with `request_id` (idempotency key), `account_id`, `receiver.counterparty_id`
   (plus `receiver.account_id` when the counterparty has several accounts), `amount`, `currency`,
   `reference`, `charge_bearer`. Returns a transaction with state
   `created | pending | completed | declined | failed | reverted`.
3. Requires `counterparties` and `payments:write` scopes and, for the recipient, YALR must
   store IBAN-class data at Revolut (not locally; only the counterparty id is kept).

Decision: **payout link is the default and the only fiat method built in this slice.** It keeps
bank and card data off YALR, needs a narrower key, and the recipient can still pick bank
account. Method `revolut_bank` stays in the enum and schema, unimplemented, so adding it later
needs no migration.

### Webhooks (v2)

- `POST /webhooks` `{url, events}` returns `{id, url, events, signing_secret}`. HTTPS only,
  at most 10 per business. Events: `TransactionCreated`, `TransactionStateChanged`,
  `PayoutLinkCreated`, `PayoutLinkStateChanged`.
- Retries: three more attempts, 10 minutes apart, after an HTTP error.
- `POST /webhooks/{id}/rotate-signing-secret`, `GET /webhooks/{id}/failed-events`.
- Signature: HMAC-SHA256 with the signing secret over `v1.<Revolut-Request-Timestamp>.<raw body>`,
  header `Revolut-Signature: v1=<hex>` (comma separated during rotation). This is the same
  scheme as the Merchant webhooks payments-rs already verifies, and the Business guide is said to
  match, but it is **unverified** for the Business API. The implementation reuses the
  payments-rs verifier; a mismatch shows up on the first sandbox delivery.
- The payload carries the payout link id and new state. YALR never trusts the body for amounts:
  on any event it calls `GET /payout-links/{id}` and applies the state from the response.
- Because delivery is not guaranteed past the retries, a reconciliation poll runs over payouts
  in `sent` older than a few minutes.

### Sandbox

`sandbox-b2b.revolut.com` exists. Test counterparties by revtag (`john1pvki` ...) exist for
bank-transfer flows. The `card` payout method is not available there. Credentials needed from
the user to run it: a sandbox API key, the sandbox account id(s) per currency, and the webhook
signing secret.

## Lightning

- Method `lightning`, destination a BOLT11 invoice or a Lightning address (`name@host`).
- BOLT11: must carry an amount equal to the payout, not expired, network matches. Paid with
  `pay_invoice` and a fee limit (config `lightning_max_fee_ppm` and a floor in sats).
- Lightning address: LNURL-pay (`https://host/.well-known/lnurlp/name`, then the `callback` with
  `amount` in msat) to obtain a BOLT11 for the exact amount, then as above.
- The payment hash of the paid invoice is the external id. A second payout with an invoice whose
  hash already exists is rejected by a unique key.

## Ledger model

Coordinated with #670 (fiat top-ups): it owns `fiat_orders`, saved cards and auto-reload and
makes `add` and `record` in `src/db/currency.rs` `pub(super)`. Payouts reuse those two helpers
and add nothing to the existing tables, so the only shared surface is that visibility change.
Whichever branch lands second rebases onto it.

### `payouts` table

`id`, `user_id`, `currency` (`sats` or an ISO code), `amount` (msat for sats, milli otherwise),
`fee` (same unit, deducted from the amount sent), `method` (`lightning | revolut_link | revolut_bank`),
`destination` (BOLT11 or Lightning address; for Revolut the link id once created, never account
data), `status` (`requested | held | sent | completed | failed | cancelled`), `external_id`
(unique, nullable: payment hash or Revolut link id), `failure_reason`, `requested_at`,
`updated_at`, `completed_at`.

- `requested`: waiting for admin approval. No money moved yet and nothing is held.
- `held`: approved, the amount is already debited into the hold (below), not yet sent.
- `sent`: handed to LND or Revolut. For a link this means the link exists and is claimable.
- `completed`: payment settled (LND success, link `processed`).
- `failed`, `cancelled`: hold returned.

Because approval is needed for the first payout, a `requested` payout is not a hold. The
withdrawable amount is still reduced by every non-terminal-failed payout (below), so a user
cannot request twice against the same earnings.

### Hold

Moving to `held` is one DB transaction: `UPDATE ... balance >= amount` debit through the shared
`add` helper plus a `balance_transactions` row of type `payout_hold`, and the payout status
change. Failure or cancel is one transaction: credit `payout_release` and status change.
Completion writes nothing to balances (the money already left) and appends a `payout_complete`
marker row with amount 0. Every status transition is `UPDATE payouts SET status = new WHERE
id = ? AND status = old`; the caller proceeds only when one row changed, so two racing
webhooks, polls and retries cannot hold, release or complete twice.

External id idempotency: the Revolut `request_id` and the LND payment attempt are keyed by the
payout id. A retry after a crash between "sent" and the status update re-sends with the same
key and gets the existing object back; LND rejects a duplicate payment hash.

### Earned-only withdrawals

Deposits are never withdrawable. The withdrawable amount per currency is:

```
min(balance, matured_earnings - committed_payouts)
```

- `matured_earnings` = sum of `provider_earning` rows in `balance_transactions` for that user and
  currency with `created_at <= now - hold_period` (default 7 days).
- `committed_payouts` = sum of `amount` of payouts in `requested | held | sent | completed`
  (failed and cancelled are excluded, so they free up the earnings again).
- Capped by the balance so spending the earned money on inference first reduces what can leave.

So a user who tops up 100 and has earned 0 can withdraw 0, and a user who earned 10 and topped up
100 can withdraw at most 10. No new column is needed on `user_currency_balances`, which keeps
the migration surface away from #670.

### Rules (config, `payouts` section, disabled when absent)

- `min_fiat_milli` = 10 units, `min_msat` = 5,000 sats.
- `daily_cap_fiat_milli`, `daily_cap_msat` per user, counting payouts requested in the last 24 h.
- `hold_period_days` = 7.
- `approval_threshold_*`: first payout per user and anything above needs admin approval, shown
  in `/approvals`. Below threshold after the first one, payouts go straight to `held`.
- `fee_ppm`, `fee_flat_*`: deducted from what the recipient gets (passed on), never from YALR.
- `revolut`: `base_url`, `api_key` (env), `account_ids` per currency, `webhook_secret` (env).
- Fiat currencies for Revolut: EUR and USD (the same set #670 accepts).

## Credentials the user must provide

- Revolut Business API key (production) with scopes `payout-links:read`, `payout-links:write`,
  `webhooks:write`; the Ed25519 signing key pair if signing is enabled for that key.
- Our Business account id for EUR and for USD.
- The webhook signing secret from `POST /webhooks` with URL
  `https://app.yalr.net/api/webhooks/revolut-business`.
- For a sandbox run: the same three items from `sandbox-b2b.revolut.com`.
- Lightning payouts use the LND node already configured for top-ups, with a macaroon that may
  send payments.

All of these go into the k8s Secret or env, never into this repository.
