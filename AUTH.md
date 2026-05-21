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
  - Full access to `/api/*` routes
  - Can manage providers, users, billing, routing config
  - Can create API keys for any user

- **Regular users**:
  - Access to chat/completions endpoints (`/v1/chat/completions`)
  - Manage own API keys
  - View own billing data

### Route Protection

| Route Pattern | Auth Required | Admin Only? | Methods |
|---------------|---------------|-------------|---------|
| `/api/auth/*` | ❌ No | No | Login, setup |
| `/api/health` | ❌ No | No | GET |
| `/v1/models` | ❌ No | No | GET |
| `/v1/chat/completions` | ✅ Yes | No | POST |
| `/v1/responses` | ✅ Yes | No | POST |
| `/api/providers/*` | ✅ Yes | **Yes** | All |
| `/api/metrics/*` | ✅ Yes | **Yes** | All |
| `/api/users/*` | ✅ Yes | **Yes** | All |
| `/api/payments/*` | ✅ Yes | **Yes** | All |
| `/api/routing-configs/*` | ✅ Yes | **Yes** | All |

### Middleware

- **`auth_middleware`**: Extracts user from NIP-98, session, or API key
- **`admin_middleware`**: Builds on auth_middleware, requires `is_admin = true`

**See**: [`src/api/server.rs`](src/api/server.rs) - Route definitions

---

## Model Access

**Current Status**: ⚠️ **No per-model access control**

All authenticated users can access all models. There is no:
- User-model permission system
- Model whitelisting/blacklisting
- Role-based model restrictions

**Future Work**: See [PLAN.md](PLAN.md) for potential model access control implementation.

---

## Configuration

### config.yaml Example

```yaml
server:
  host: "0.0.0.0"
  port: 3000
  admin_ui_path: "admin/dist"

database:
  url: "sqlite:llm_router.db?mode=rwc"

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
