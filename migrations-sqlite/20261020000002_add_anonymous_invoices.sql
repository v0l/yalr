ALTER TABLE lightning_invoices ADD COLUMN purpose TEXT NOT NULL DEFAULT 'balance';
ALTER TABLE lightning_invoices ADD COLUMN public_id TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS idx_lightning_invoices_public_id ON lightning_invoices(public_id);

CREATE TABLE IF NOT EXISTS server_secret (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    secret TEXT NOT NULL
);
