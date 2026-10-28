CREATE TABLE providers (
    id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    slug VARCHAR(255) NOT NULL,
    base_url VARCHAR(2048) NOT NULL,
    api_key TEXT,
    provider_type SMALLINT UNSIGNED NOT NULL DEFAULT 0,
    oauth_access_token TEXT,
    oauth_refresh_token TEXT,
    oauth_expires_at BIGINT,
    oauth_account_id VARCHAR(255),
    client_version VARCHAR(64),
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3),
    UNIQUE KEY uq_providers_name (name),
    UNIQUE KEY uq_providers_slug (slug)
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_uca1400_as_cs;

CREATE TABLE models (
    id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    cost_per_1m_input DOUBLE NOT NULL DEFAULT 0,
    cost_per_1m_output DOUBLE NOT NULL DEFAULT 0,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3),
    UNIQUE KEY uq_models_name (name)
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_uca1400_as_cs;

CREATE TABLE model_providers (
    id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    model_id BIGINT NOT NULL,
    provider_id BIGINT NOT NULL,
    weight INT NOT NULL DEFAULT 100,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3),
    UNIQUE KEY uq_model_providers (model_id, provider_id),
    KEY idx_model_providers_provider (provider_id),
    CONSTRAINT fk_model_providers_model FOREIGN KEY (model_id) REFERENCES models (id) ON DELETE CASCADE,
    CONSTRAINT fk_model_providers_provider FOREIGN KEY (provider_id) REFERENCES providers (id) ON DELETE CASCADE
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_uca1400_as_cs;

CREATE TABLE routing_config (
    id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    name VARCHAR(255) NOT NULL DEFAULT 'default',
    strategy VARCHAR(64) NOT NULL DEFAULT 'round_robin',
    health_check_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    health_check_interval_seconds INT NOT NULL DEFAULT 30,
    health_check_timeout_seconds INT NOT NULL DEFAULT 5,
    input_modalities TEXT,
    output_modalities TEXT,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3),
    KEY idx_routing_config_name (name)
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_uca1400_as_cs;

CREATE TABLE routing_config_providers (
    id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    routing_config_id BIGINT NOT NULL,
    provider_id BIGINT NOT NULL,
    model VARCHAR(255),
    weight INT NOT NULL DEFAULT 100,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3),
    UNIQUE KEY uq_routing_config_providers (routing_config_id, provider_id, model),
    KEY idx_routing_config_providers_active (routing_config_id, is_active),
    KEY idx_routing_config_providers_provider (provider_id),
    CONSTRAINT fk_rcp_config FOREIGN KEY (routing_config_id) REFERENCES routing_config (id) ON DELETE CASCADE,
    CONSTRAINT fk_rcp_provider FOREIGN KEY (provider_id) REFERENCES providers (id) ON DELETE CASCADE
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_uca1400_as_cs;

CREATE TABLE users (
    id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    username VARCHAR(255),
    password_hash VARCHAR(255),
    external_id VARCHAR(255),
    user_type SMALLINT UNSIGNED NOT NULL DEFAULT 0,
    is_admin BOOLEAN NOT NULL DEFAULT FALSE,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3),
    CONSTRAINT chk_users_identity CHECK (username IS NOT NULL OR external_id IS NOT NULL),
    UNIQUE KEY uq_users_username (username),
    UNIQUE KEY uq_users_external (external_id, user_type),
    KEY idx_users_created (created_at, id)
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_uca1400_as_cs;

CREATE TABLE api_keys (
    id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    key_hash VARCHAR(128) NOT NULL,
    name VARCHAR(255) NOT NULL,
    user_id BIGINT NOT NULL,
    last_four VARCHAR(8) NOT NULL,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    expires_at DATETIME(3),
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    UNIQUE KEY uq_api_keys_hash (key_hash),
    KEY idx_api_keys_user (user_id),
    CONSTRAINT fk_api_keys_user FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE CASCADE
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_uca1400_as_cs;

CREATE TABLE user_model_permissions (
    id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    user_id BIGINT NOT NULL,
    model VARCHAR(255) NOT NULL,
    allow BOOLEAN NOT NULL DEFAULT TRUE,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3),
    UNIQUE KEY uq_user_model_permissions (user_id, model),
    CONSTRAINT fk_ump_user FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE CASCADE
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_uca1400_as_cs;

CREATE TABLE billing_exempt_users (
    user_id BIGINT NOT NULL PRIMARY KEY,
    CONSTRAINT fk_billing_exempt_user FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE CASCADE
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_uca1400_as_cs;

CREATE TABLE user_balances (
    id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    user_id BIGINT NOT NULL,
    balance_msat BIGINT NOT NULL DEFAULT 0,
    lifetime_deposited_msat BIGINT NOT NULL DEFAULT 0,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3),
    UNIQUE KEY uq_user_balances_user (user_id),
    KEY idx_user_balances_balance (balance_msat),
    CONSTRAINT fk_user_balances_user FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE CASCADE
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_uca1400_as_cs;

CREATE TABLE balance_transactions (
    id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    user_id BIGINT NOT NULL,
    amount_msat BIGINT NOT NULL,
    transaction_type VARCHAR(32) NOT NULL,
    reference_id VARCHAR(255),
    metadata TEXT,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    KEY idx_balance_transactions_user (user_id, created_at, id),
    KEY idx_balance_transactions_created (created_at, id),
    KEY idx_balance_transactions_reference (reference_id),
    CONSTRAINT fk_balance_transactions_user FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE CASCADE
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_uca1400_as_cs;

CREATE TABLE lightning_invoices (
    id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    user_id BIGINT NOT NULL,
    payment_hash VARCHAR(128) NOT NULL,
    bolt11 TEXT NOT NULL,
    amount_msat BIGINT NOT NULL,
    amount_sats BIGINT NOT NULL,
    status VARCHAR(32) NOT NULL DEFAULT 'pending',
    purpose VARCHAR(32) NOT NULL DEFAULT 'balance',
    public_id VARCHAR(128),
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    expires_at DATETIME(3),
    paid_at DATETIME(3),
    UNIQUE KEY uq_lightning_invoices_hash (payment_hash),
    UNIQUE KEY uq_lightning_invoices_public_id (public_id),
    KEY idx_lightning_invoices_user_status (user_id, status),
    KEY idx_lightning_invoices_status_created (status, created_at),
    KEY idx_lightning_invoices_created (created_at, id),
    KEY idx_lightning_invoices_bolt11 (bolt11(255)),
    CONSTRAINT fk_lightning_invoices_user FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE CASCADE
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_uca1400_as_cs;

CREATE TABLE model_pricing (
    id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    model_name VARCHAR(255) NOT NULL,
    is_advertised BOOLEAN NOT NULL DEFAULT TRUE,
    is_free BOOLEAN NOT NULL DEFAULT FALSE,
    price_per_1m_input_sats BIGINT,
    price_per_1m_output_sats BIGINT,
    price_per_1m_cache_read_sats BIGINT,
    price_per_1m_cache_write_sats BIGINT,
    price_per_request_sats BIGINT,
    context_window INT,
    max_output_tokens INT,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3),
    UNIQUE KEY uq_model_pricing_name (model_name)
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_uca1400_as_cs;

CREATE TABLE api_key_usage (
    api_key_id BIGINT NOT NULL,
    hour DATETIME NOT NULL,
    model VARCHAR(255) NOT NULL,
    requests BIGINT NOT NULL DEFAULT 0,
    prompt_tokens BIGINT NOT NULL DEFAULT 0,
    completion_tokens BIGINT NOT NULL DEFAULT 0,
    cached_tokens BIGINT NOT NULL DEFAULT 0,
    cache_write_tokens BIGINT NOT NULL DEFAULT 0,
    cost_msat BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (api_key_id, hour, model),
    KEY idx_api_key_usage_hour (hour)
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_uca1400_as_cs;

CREATE TABLE metrics_history (
    timestamp_ms BIGINT NOT NULL PRIMARY KEY,
    snapshot_json LONGTEXT NOT NULL
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_uca1400_as_cs;

CREATE TABLE nostr_identity (
    id TINYINT NOT NULL PRIMARY KEY,
    secret_key VARCHAR(128) NOT NULL,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    CONSTRAINT chk_nostr_identity_single CHECK (id = 1)
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_uca1400_as_cs;

CREATE TABLE server_secret (
    id TINYINT NOT NULL PRIMARY KEY,
    secret VARCHAR(128) NOT NULL,
    CONSTRAINT chk_server_secret_single CHECK (id = 1)
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_uca1400_as_cs;

CREATE TABLE request_log (
    id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    request_id VARCHAR(64) NOT NULL,
    started_at_ms BIGINT NOT NULL,
    endpoint VARCHAR(128) NOT NULL,
    stream BOOLEAN NOT NULL,
    model VARCHAR(255) NOT NULL,
    user_id BIGINT,
    user_name VARCHAR(255),
    api_key_id BIGINT,
    api_key_name VARCHAR(255),
    client_ip VARCHAR(64),
    client_headers TEXT,
    provider VARCHAR(255),
    upstream_url VARCHAR(2048),
    upstream_request_headers TEXT,
    upstream_status INT,
    upstream_response_headers TEXT,
    attempts INT NOT NULL,
    attempt_log MEDIUMTEXT,
    status VARCHAR(32) NOT NULL,
    error TEXT,
    finish_reason VARCHAR(64),
    headers_ms BIGINT,
    first_event_ms BIGINT,
    first_thinking_ms BIGINT,
    first_text_ms BIGINT,
    total_ms BIGINT,
    prompt_tokens BIGINT,
    completion_tokens BIGINT,
    cached_tokens BIGINT,
    cache_write_tokens BIGINT,
    weighted_input_tokens BIGINT,
    message_count INT,
    system_hash VARCHAR(64),
    tools_hash VARCHAR(64),
    message_hashes MEDIUMTEXT,
    first_diverging_message INT,
    previous_request_id VARCHAR(64),
    request_bytes BIGINT,
    response_bytes BIGINT,
    KEY idx_request_log_started (started_at_ms),
    KEY idx_request_log_request_id (request_id),
    KEY idx_request_log_api_key (api_key_id, id),
    KEY idx_request_log_user (user_id, id),
    KEY idx_request_log_model (model, id),
    KEY idx_request_log_status (status, id),
    KEY idx_request_log_provider (provider, id)
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_uca1400_as_cs;

CREATE TABLE request_log_body (
    log_id BIGINT NOT NULL PRIMARY KEY,
    client_request_packed LONGBLOB,
    upstream_request_packed LONGBLOB,
    response_packed LONGBLOB,
    CONSTRAINT fk_request_log_body_log FOREIGN KEY (log_id) REFERENCES request_log (id) ON DELETE CASCADE
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_uca1400_as_cs;
