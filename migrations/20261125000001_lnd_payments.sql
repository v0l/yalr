ALTER TABLE gateway_payments
    ADD COLUMN provider VARCHAR(16) NOT NULL DEFAULT 'gateway',
    ADD KEY idx_gateway_payments_provider_status (provider, status, created_at);

CREATE TABLE payment_cursors (
    name VARCHAR(64) NOT NULL PRIMARY KEY,
    position BIGINT UNSIGNED NOT NULL DEFAULT 0,
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3)
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_uca1400_as_cs;
