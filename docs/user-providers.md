# User-owned providers

Users can connect their own providers, set their own prices and earn from
consumers. Withdrawals do not exist yet: earnings stay on the owner's balance.

## Schema (migration `20261107000001_user_providers.sql`)

- `providers.owner_user_id` nullable FK to `users`. NULL is platform owned.
- `model_pricing` gains `owner_user_id`, `provider_id` (nullable FKs),
  `currency` (`sats` default, or a lowercase ISO 4217 code such as `usd`, `eur`)
  and `price_per_1m_{input,output,cache_read,cache_write}_milli` plus
  `price_per_request_milli`. Fiat prices are in thousandths of the currency's
  minor unit (`usd`: millicents, 1e-5 USD). Sats rows keep using the `_sats`
  columns. The unique key is `(model_name, provider_key)` where `provider_key`
  is `COALESCE(provider_id, 0)`, so there is one platform row per model and one
  row per provider.
- `user_currency_balances (user_id, currency, balance_milli,
  lifetime_deposited_milli)`: one row per user and fiat currency. Any currency
  works, none is special. Sats stay in `user_balances` (unchanged, so existing
  readers keep working).
- `balance_transactions` gains `currency` (default `sats`) and `amount_milli`.
  Sats rows keep `amount_msat` (and `amount_milli` is 0); fiat rows set
  `amount_milli` and `amount_msat` is 0.

`migrations-sqlite/` is the frozen legacy schema used only by
`yalr-cli import-sqlite`, which writes into MariaDB. It is not migrated.

## Settlement

On each billed request, in one database transaction:

1. debit the consumer in the price's currency (may go below zero after the
   upstream call, as today; admission already returned 402 if the balance in
   that currency was below the per-request price);
2. credit the provider owner the same amount minus `provider_fee_ppm` of it
   (config `payments.provider_fee_ppm`, default 0). The fee is credited to no
   one;
3. write a `usage` row for the consumer and a `provider_earning` row for the
   owner, both with the request id as `reference_id`.

Platform-owned providers debit only. A user calling their own provider is not
charged. There is no conversion between currencies: a price in `eur` is paid
from the consumer's `eur` balance.

## Routing

A requested model containing `/` that has no explicit routing config of that
exact name, and whose prefix is not a provider slug, is a full repo id such as
`qwen/qwen3.8-27b`. The router builds a round-robin pool from every loaded
provider whose discovered model list contains the id (case-insensitive), with
the usual weights, health tiers and failover. Each request is billed at the
price row of the provider that served it. Short names never reach user-owned
providers. `/v1/models` lists each full-name model once with provider count and
min/max price.

## SSRF guard

A user-owned provider's `base_url` must be http or https. The host is resolved
and refused when any address is loopback, private (RFC 1918), link-local
(including 169.254.169.254), CGNAT (100.64/10), unspecified, multicast, ULA
(fc00::/7), or IPv4-mapped forms of those. The check runs when the provider is
saved and again on every request through a re-resolving transport, so DNS
rebinding cannot bypass it. Admin-owned providers are exempt.

Provider API keys are write-only and are never returned by any endpoint.
