-- Payments: balances, invoices, model pricing
-- Feature gated via config.yaml payments.enabled

CREATE TABLE IF NOT EXISTS user_balances (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL UNIQUE,
    balance_msat INTEGER NOT NULL DEFAULT 0,
    lifetime_deposited_msat INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS balance_transactions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL,
    amount_msat INTEGER NOT NULL,
    transaction_type TEXT NOT NULL,
    reference_id TEXT,
    metadata TEXT,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS lightning_invoices (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL,
    payment_hash TEXT UNIQUE NOT NULL,
    bolt11 TEXT NOT NULL,
    amount_msat INTEGER NOT NULL,
    amount_sats INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP,
    paid_at TIMESTAMP,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS model_pricing (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    model_name TEXT NOT NULL UNIQUE,
    is_advertised BOOLEAN NOT NULL DEFAULT 1,
    is_free BOOLEAN NOT NULL DEFAULT 0,
    price_per_1m_input_sats INTEGER,
    price_per_1m_output_sats INTEGER,
    price_per_request_sats INTEGER,
    context_window INTEGER,
    max_output_tokens INTEGER,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_user_balances_user ON user_balances(user_id);
CREATE INDEX IF NOT EXISTS idx_balance_transactions_user ON balance_transactions(user_id);
CREATE INDEX IF NOT EXISTS idx_lightning_invoices_user ON lightning_invoices(user_id);
CREATE INDEX IF NOT EXISTS idx_lightning_invoices_hash ON lightning_invoices(payment_hash);
CREATE INDEX IF NOT EXISTS idx_model_pricing_name ON model_pricing(model_name);
