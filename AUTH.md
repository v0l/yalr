# Authentication & Authorization

This document describes the authentication and authorization system for YALR (LLM Router).

## Overview

YALR supports three authentication methods:

1. **NIP-98 (Nostr)** - Primary auth for API clients
2. **Session-based** - Browser/Admin UI authentication
3. **API Keys** - Long-lived tokens for programmatic access

All methods support both regular users and admin users.

---

## Authentication Methods

### 1. NIP-98 (Nostr HTTP Auth)

**Best for**: Nostr-native clients, decentralized authentication

**How it works**:
- Client creates a signed Nostr event (Kind 22219) with:
  - `u` tag: Request URL
  - `method` tag: HTTP method (GET, POST, etc.)
  - `expiration` tag: Optional expiry timestamp
- Sends `Authorization: Nostr <base64(event)>`

**Validation**:
- Event signature verified
- Timestamp/expiration checked
- URL and method tags match request
- User auto-created/retrieved by pubkey

**Config**:
```yaml
auth:
  enabled: true
  allowed_pubkeys:  # Optional whitelist
    - "your_pubkey_here"
```

**See**: [`src/auth/nip98.rs`](src/auth/nip98.rs)

---

### 2. Session-based Authentication

**Best for**: Admin UI, browser-based access

**Flow**:
1. POST `/api/auth/login` with username/password
2. Server validates (Argon2 hash) and returns session token
3. Client sends `Authorization: Bearer <session_token>`
4. Session stored in-memory, 24-hour expiry

**Endpoints**:
- `POST /api/auth/login` - Login
- `POST /api/auth/logout` - Logout
- `GET /api/auth/status` - Check auth status
- `POST /api/auth/setup` - Create first admin user

**See**: [`src/auth/admin.rs`](src/auth/admin.rs)

---

### 3. API Keys

**Best for**: Long-lived programmatic access, service-to-service

**Features**:
- SHA-256 hashed storage
- Named keys for tracking
- Optional expiration
- Enable/disable without deletion
- Admin can create keys for any user

**Endpoints**:
- `POST /api/api-keys` - Create new key (authenticated user)
- `GET /api/api-keys` - List own keys
- `POST /api/users/:id/api-keys` - Admin create for user
- `DELETE /api/api-keys/:id` - Delete key
- `POST /api/api-keys/:id/disable` - Disable key
- `POST /api/api-keys/:id/enable` - Enable key

**Usage**:
```
Authorization: Bearer <api_key>
```

**See**: [`src/auth/api_keys.rs`](src/auth/api_keys.rs)

---

## Authorization & Access Control

### User Types

| Type | Description |
|------|-------------|
| **Internal** | Username/password users (created via setup/login) |
| **Nostr** | Auto-created from NIP-98 pubkey auth |
| **OAuth** | Reserved for future OAuth providers |

### Admin vs Regular Users

- **Admin users** (`is_admin = true`):
  - See and manage every row behind `/api/*`
  - Can manage platform providers, users, billing, routing config
  - Can create API keys for any user, and link a provider to an owner

- **Regular users** (tenants) see the same admin UI pages, scoped to their own rows:
  - **Dashboard / providers**: only providers they own. New ones get a `u{id}-` slug and a `username/` name prefix, must have a public base URL (SSRF guard) and cannot be OAuth
  - **Config**: only routing configs they own, named `username/...`. Only their own providers can be assigned, and the router only routes an owned provider inside its owner's configs
  - **Metrics**: health, throughput and events of their own providers only, on the REST endpoints and the metrics WebSocket
  - **Requests**: only requests they made. Upstream URL, headers, route and the upstream body are hidden unless the request was served by their own provider
  - **Payments**: own balance, transactions and invoices, Lightning top up, and pricing of their own providers. Credit and debit stay admin-only
  - **Users**: hidden

### Route Protection

| Route Pattern | Auth Required | Scope | Methods |
|---------------|---------------|-------|---------|
| `/api/auth/*` | ❌ No | - | Login, setup |
| `/api/health` | ❌ No | - | GET |
| `/v1/models` | ❌ No | - | GET |
| `/v1/chat/completions` | ✅ Yes | Own account | POST |
| `/v1/responses` | ✅ Yes | Own account | POST |
| `/api/providers/*` | ✅ Yes | Tenant scoped | All |
| `/api/metrics/*`, `/api/metrics/ws` | ✅ Yes | Tenant scoped | GET |
| `/api/request-log/*` | ✅ Yes | Tenant scoped | GET |
| `/api/routing-configs/*` | ✅ Yes | Tenant scoped | All |
| `/api/model-pricing`, `/api/model-pricing/id/:id` | ✅ Yes | Tenant scoped | All |
| `/api/payments/{balances,transactions,invoices,currencies}` | ✅ Yes | Tenant scoped | GET |
| `/api/payments/{credit,debit}`, `/api/payments/balances/:user_id` | ✅ Yes | **Admin** | All |
| `/api/users/*`, `/api/api-keys/*`, `/api/oauth/*`, `/api/config` | ✅ Yes | **Admin** | All |

Tenant scoping lives in one place, `Scope` in [`src/api/tenant.rs`](src/api/tenant.rs): an admin sees every row, a regular user sees only rows whose `owner_user_id` is theirs, and a row they cannot see answers 404. [`src/api/tenant_tests.rs`](src/api/tenant_tests.rs) drives the HTTP app as two users and an admin to prove it.

### Middleware

- **`auth_middleware`**: Extracts user from NIP-98, session, or API key. Guards the tenant scoped routes
- **`admin_middleware`**: Builds on auth_middleware, requires `is_admin = true`

**See**: [`src/api/routes.rs`](src/api/routes.rs) - Route definitions

---

## Model Access Control

**Current Status**: ✅ **Implemented**

Per-model access control is enforced on all model interactions. Each user can have per-model allow/deny rules stored in `user_model_permissions`.

### How It Works

1. **`check_model_access(user_id, model)`** in [`src/db/mod.rs`](src/db/mod.rs) checks permissions with two-tier matching:
   - **Exact match first**: looks for a permission row where `model` equals the requested model
   - **Wildcard fallback**: checks for a `model = '*'` row as a catch-all
   - Returns `None` (no rules exist → default-allow), `Some(true)` (explicit allow), or `Some(false)` (explicit deny)

2. **Chat completions** ([`src/api/chat.rs`](src/api/chat.rs)): calls `check_model_access` before routing. If a rule returns `allow = false`, the request is rejected with `403` and error type `"model_access_denied"`.

3. **Model listing** ([`src/api/models.rs`](src/api/models.rs)): filters the models returned based on user permissions. A wildcard deny (`* = false`) hides everything; a wildcard allow (`* = true`) shows all unless an explicit deny exists for a specific model. If permissions are defined but no rule matches a model, the model is hidden (default-deny when any rules exist).

### Permission Rules

| Scenario | Rules | Result for `gpt-4` |
|---|---|---|
| No rules at all | (none) | ✅ Allowed (default-open) |
| Wildcard allow | `* = true` | ✅ Allowed |
| Wildcard deny | `* = false` | ❌ Denied |
| Explicit allow | `gpt-4 = true` | ✅ Allowed |
| Explicit deny | `gpt-4 = false` | ❌ Denied |
| Exact overrides wildcard | `* = true`, `gpt-4 = false` | ❌ Denied (exact match wins) |
| Wildcard allow, no exact | `* = true` (no `claude-3` rule) | ✅ Allowed for `claude-3` |

### API Endpoints

| Method | Endpoint | Description |
|---|---|---|
| `GET` | `/api/users/:user_id/models` | List user's model permissions |
| `POST` | `/api/users/:user_id/models` | Create a new model permission |
| `DELETE` | `/api/users/:user_id/models/:model` | Delete a model permission |

All endpoints are admin-only. See [`src/api/users.rs`](src/api/users.rs) and [`src/api/server.rs`](src/api/server.rs) for route definitions.

---

## Configuration

### config.yaml Example

```yaml
server:
  host: "0.0.0.0"
  port: 3000
  admin_ui_path: "admin/dist"

database:
  url: "mysql://yalr:yalr@127.0.0.1:3306/yalr"

auth:
  enabled: true
  allowed_pubkeys:  # Optional - if set, only these pubkeys can auth
    - "your_nostr_pubkey_here"
    - "another_trusted_pubkey"

payments:
  enabled: false
  # ... (see PAYMENTS.md)
```

---

## User Management

### Creating Users

**First user (setup)**:
```bash
POST /api/auth/setup
{
  "username": "admin",
  "password": "secure_password"
}
```

**Subsequent users**:
- Admin only via `POST /api/users`
- Requires admin authentication

### API Key Lifecycle

1. **Create**: Generate fresh key (shown once)
2. **Store**: Client must save key securely
3. **Use**: Send as `Authorization: Bearer <key>`
4. **Manage**: Enable/disable via API
5. **Revoke**: Delete when compromised/no longer needed

---

## Security Considerations

### Password Storage
- Argon2 hashing with random salt
- Never stored in plain text

### API Key Storage
- SHA-256 hashed (like password hashes)
- Only last 4 chars visible in UI/logs

### Session Security
- In-memory storage (lost on restart)
- 24-hour default expiry
- No persistent cookies

### NIP-98 Security
- Event signature verification
- Timestamp/expiration validation
- URL/method binding prevents replay

---

## Implementation Files

```
src/
├── auth/
│   ├── nip98.rs        # NIP-98 extraction & validation
│   ├── admin.rs        # Sessions, login/logout, middleware
│   └── api_keys.rs     # API key CRUD
├── api/
│   └── server.rs       # Route definitions & middleware wiring
├── db/
│   └── mod.rs          # User, ApiKey schema & queries
└── config.rs           # AuthConfig, AppConfig
```

---

## Related Documentation

- [AGENTS.md](AGENTS.md) - Development guidelines
- [PAYMENTS.md](PAYMENTS.md) - Billing & payment integration
- [API.md](API.md) - Full API reference
