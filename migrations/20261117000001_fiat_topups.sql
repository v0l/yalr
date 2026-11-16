CREATE TABLE fiat_orders (
    id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    public_id VARCHAR(32) NOT NULL,
    user_id BIGINT NOT NULL,
    provider VARCHAR(16) NOT NULL DEFAULT 'revolut',
    external_id VARCHAR(128) NULL,
    kind VARCHAR(16) NOT NULL DEFAULT 'topup',
    currency VARCHAR(8) NOT NULL,
    amount_milli BIGINT NOT NULL,
    status VARCHAR(16) NOT NULL DEFAULT 'pending',
    save_card TINYINT(1) NOT NULL DEFAULT 0,
    failure_reason VARCHAR(255) NULL,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3),
    completed_at DATETIME(3) NULL,
    UNIQUE KEY uq_fiat_orders_public (public_id),
    UNIQUE KEY uq_fiat_orders_external (provider, external_id),
    KEY idx_fiat_orders_user (user_id, created_at),
    KEY idx_fiat_orders_status (status, created_at),
    CONSTRAINT fk_fiat_orders_user FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE CASCADE
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_uca1400_as_cs;

CREATE TABLE user_payment_methods (
    id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    user_id BIGINT NOT NULL,
    provider VARCHAR(16) NOT NULL DEFAULT 'revolut',
    external_customer_id TEXT NOT NULL,
    external_id VARCHAR(128) NOT NULL,
    card_brand VARCHAR(32) NULL,
    card_last_four VARCHAR(4) NULL,
    exp_month SMALLINT UNSIGNED NULL,
    exp_year SMALLINT UNSIGNED NULL,
    is_default TINYINT(1) NOT NULL DEFAULT 0,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    UNIQUE KEY uq_user_payment_methods_external (provider, external_id),
    KEY idx_user_payment_methods_user (user_id),
    CONSTRAINT fk_user_payment_methods_user FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE CASCADE
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
    CONSTRAINT fk_auto_reload_user FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE CASCADE,
    CONSTRAINT fk_auto_reload_method FOREIGN KEY (payment_method_id) REFERENCES user_payment_methods (id) ON DELETE CASCADE
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_uca1400_as_cs;
