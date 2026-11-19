UPDATE lightning_invoices SET status = 'expired' WHERE status = 'pending';
UPDATE fiat_orders SET status = 'failed', failure_reason = 'closed at gateway cutover' WHERE status = 'pending';

RENAME TABLE lightning_invoices TO legacy_lightning_invoices,
             fiat_orders TO legacy_fiat_orders,
             user_payment_methods TO legacy_user_payment_methods,
             auto_reload_settings TO legacy_auto_reload_settings;

CREATE TABLE gateway_payments (
    id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    public_id VARCHAR(64) NOT NULL,
    gateway_payment_id VARCHAR(128) NULL,
    user_id BIGINT NOT NULL,
    kind VARCHAR(16) NOT NULL DEFAULT 'topup',
    purpose VARCHAR(16) NULL,
    currency VARCHAR(8) NOT NULL,
    amount_milli BIGINT NOT NULL,
    status VARCHAR(16) NOT NULL DEFAULT 'pending',
    bolt11 TEXT NULL,
    payment_hash VARCHAR(128) NULL,
    pay_url TEXT NULL,
    failure_reason VARCHAR(255) NULL,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3),
    expires_at DATETIME(3) NULL,
    completed_at DATETIME(3) NULL,
    UNIQUE KEY uq_gateway_payments_public (public_id),
    UNIQUE KEY uq_gateway_payments_remote (gateway_payment_id),
    KEY idx_gateway_payments_user (user_id, created_at),
    KEY idx_gateway_payments_status (status, created_at),
    KEY idx_gateway_payments_hash (payment_hash),
    KEY idx_gateway_payments_bolt11 (bolt11(255)),
    CONSTRAINT fk_gateway_payments_user FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE CASCADE
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_uca1400_as_cs;

CREATE TABLE auto_reload_settings (
    id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    user_id BIGINT NOT NULL,
    currency VARCHAR(8) NOT NULL,
    enabled TINYINT(1) NOT NULL DEFAULT 0,
    threshold_milli BIGINT NOT NULL,
    amount_milli BIGINT NOT NULL,
    monthly_cap_milli BIGINT NOT NULL,
    payment_method_id BIGINT NOT NULL,
    consecutive_failures INT NOT NULL DEFAULT 0,
    last_attempt_at DATETIME(3) NULL,
    last_failure_at DATETIME(3) NULL,
    disabled_reason VARCHAR(255) NULL,
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3),
    UNIQUE KEY uq_auto_reload_user_currency (user_id, currency),
    CONSTRAINT fk_auto_reload_settings_user FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE CASCADE
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_uca1400_as_cs;
