DROP INDEX IF EXISTS idx_request_log_model;
CREATE INDEX IF NOT EXISTS idx_request_log_model ON request_log (model, id);
CREATE INDEX IF NOT EXISTS idx_request_log_status ON request_log (status, id);
DROP INDEX IF EXISTS idx_balance_transactions_user;
CREATE INDEX IF NOT EXISTS idx_balance_transactions_user ON balance_transactions (user_id, created_at, id);
CREATE INDEX IF NOT EXISTS idx_balance_transactions_created ON balance_transactions (created_at);
CREATE INDEX IF NOT EXISTS idx_lightning_invoices_created ON lightning_invoices (created_at);
CREATE INDEX IF NOT EXISTS idx_lightning_invoices_bolt11 ON lightning_invoices (bolt11);
