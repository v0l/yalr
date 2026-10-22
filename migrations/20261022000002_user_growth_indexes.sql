CREATE INDEX IF NOT EXISTS idx_users_created ON users (created_at);
CREATE INDEX IF NOT EXISTS idx_user_balances_balance ON user_balances (balance_msat);
CREATE INDEX IF NOT EXISTS idx_lightning_invoices_user_status ON lightning_invoices (user_id, status);
CREATE INDEX IF NOT EXISTS idx_api_key_usage_hour ON api_key_usage (hour);
