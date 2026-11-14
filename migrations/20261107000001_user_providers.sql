ALTER TABLE providers
    ADD COLUMN owner_user_id BIGINT NULL,
    ADD KEY idx_providers_owner (owner_user_id),
    ADD CONSTRAINT fk_providers_owner FOREIGN KEY (owner_user_id) REFERENCES users (id) ON DELETE CASCADE;

ALTER TABLE model_pricing
    ADD COLUMN owner_user_id BIGINT NULL,
    ADD COLUMN provider_id BIGINT NULL,
    ADD COLUMN currency VARCHAR(8) NOT NULL DEFAULT 'sats',
    ADD COLUMN price_per_1m_input_milli BIGINT,
    ADD COLUMN price_per_1m_output_milli BIGINT,
    ADD COLUMN price_per_1m_cache_read_milli BIGINT,
    ADD COLUMN price_per_1m_cache_write_milli BIGINT,
    ADD COLUMN price_per_request_milli BIGINT,
    ADD COLUMN provider_key BIGINT AS (COALESCE(provider_id, 0)) PERSISTENT,
    DROP INDEX uq_model_pricing_name,
    ADD UNIQUE KEY uq_model_pricing_name_provider (model_name, provider_key),
    ADD KEY idx_model_pricing_owner (owner_user_id),
    ADD CONSTRAINT fk_model_pricing_owner FOREIGN KEY (owner_user_id) REFERENCES users (id) ON DELETE CASCADE,
    ADD CONSTRAINT fk_model_pricing_provider FOREIGN KEY (provider_id) REFERENCES providers (id) ON DELETE CASCADE;

CREATE TABLE user_currency_balances (
    id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    user_id BIGINT NOT NULL,
    currency VARCHAR(8) NOT NULL,
    balance_milli BIGINT NOT NULL DEFAULT 0,
    lifetime_deposited_milli BIGINT NOT NULL DEFAULT 0,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3),
    UNIQUE KEY uq_user_currency_balances (user_id, currency),
    CONSTRAINT fk_user_currency_balances_user FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE CASCADE
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_uca1400_as_cs;

ALTER TABLE balance_transactions
    ADD COLUMN currency VARCHAR(8) NOT NULL DEFAULT 'sats',
    ADD COLUMN amount_milli BIGINT NOT NULL DEFAULT 0;
