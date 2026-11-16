# Card top-ups, saved cards and auto-reload (Revolut Merchant)

Users top up EUR or USD balances with a card through the in-page Revolut card field.
Each currency credits the matching `user_currency_balances` row in millicents, exactly the
amount paid. No conversion. Lightning top-ups are unchanged.

The feature is off unless `payments.revolut` is in `config.yaml` and all three secrets are set.
Without it the routes below return 404 and the Card tab is hidden.

## Config

```yaml
payments:
  enabled: true
  revolut:
    sandbox: false          # true uses https://sandbox-merchant.revolut.com
    api_version: "2024-09-01"
    currencies: [eur, usd]
    min_amount: 5
    max_amount: 500
    reconcile_after_secs: 120
    auto_reload_cooldown_secs: 3600
```

Secrets come from the environment, never the file:

| Variable | Purpose |
| --- | --- |
| `REVOLUT_API_KEY` | Merchant API secret key |
| `REVOLUT_WEBHOOK_SECRET` | Signing secret returned when the webhook is created |
| `YALR_ENCRYPTION_KEY` | 64 hex characters (32 bytes), encrypts the Revolut customer id at rest |

Suggested k8s Secret: `yalr-revolut` with keys `api-key`, `webhook-secret` and `encryption-key`,
mapped to the three variables with `secretKeyRef` in `yalr/yalr.yml`. Losing the encryption key
makes saved cards unusable (users re-save them); the ledger is unaffected.

## Setup

1. In the Revolut Business merchant dashboard create an API key (production or sandbox).
2. Register the webhook once, using `RevolutApi::create_webhook` from payments-rs with the
   events `ORDER_COMPLETED`, `ORDER_CANCELLED` and `ORDER_FAILED` and the URL
   `https://app.yalr.net/api/webhooks/revolut`. Keep the returned `signing_secret` as
   `REVOLUT_WEBHOOK_SECRET`.
3. Set the three variables and add the `revolut` block, then restart.

## Flow

1. `POST /api/account/topup/fiat {currency, amount, save_card, email?}` creates a pending
   `fiat_orders` row and a Revolut order, and returns the public order `token` and `mode`.
2. The admin SPA mounts `RevolutCheckout(token, mode).createCardField(...)`. "Save card" passes
   `savePaymentMethodFor: "merchant"` and creates the order with a customer.
3. Credit happens only when Revolut reports the order `completed`: through the verified webhook
   (`POST /api/webhooks/revolut`, HMAC signature and timestamp checked), the status endpoint, or
   the reconciliation task that polls pending orders older than `reconcile_after_secs`. All three
   go through one database transaction that locks the order row, so a webhook plus a poll
   credits once. An amount or currency that differs from the local order is never credited.
4. After a completed order that asked to save the card, the merchant-saved payment methods of
   the customer are stored (brand, last four, expiry, Revolut ids; no card data). The customer id
   is AES-256-GCM encrypted.

## API

All under `/api`, tenant scoped unless noted.

| Route | Purpose |
| --- | --- |
| `GET /account/topup/fiat/config` | mode, currencies, limits |
| `POST /account/topup/fiat` | create order |
| `GET /account/topup/fiat/:id` | status (also reconciles) |
| `GET /account/topup/fiat` | own order history |
| `GET /account/payment-methods`, `DELETE /account/payment-methods/:id`, `PUT /account/payment-methods/:id/default` | saved cards; delete also removes the method at Revolut |
| `GET/PUT /account/auto-reload` | per currency settings |
| `GET /payments/fiat-orders` | all orders (admin) |
| `POST /webhooks/revolut` | Revolut webhook (public, signature checked) |

## Auto-reload

Per user and currency: threshold, reload amount (within min and max), monthly cap, card. A
background task (every 30 s) finds enabled settings whose balance is under the threshold and
claims one off-session charge under a row lock: at most one pending auto-reload per user and
currency, none inside the failure cooldown, none that would pass the monthly cap. The charge
uses `create_off_session_order`-style order plus `pay_order` with `initiator: merchant` and is
credited by the same path as a normal top-up. Two consecutive failures disable it and the
reason shows on the Card tab. Nothing is charged in a request handler.

## Sandbox check

Point `sandbox: true` at sandbox credentials, create a top-up in the Card tab and pay with a
Revolut test card. If the sandbox cannot reach the server, the reconciliation task still
credits the order after `reconcile_after_secs`.
