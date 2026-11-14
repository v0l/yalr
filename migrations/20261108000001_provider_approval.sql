ALTER TABLE providers
    ADD COLUMN status VARCHAR(16) NOT NULL DEFAULT 'approved',
    ADD COLUMN status_reason TEXT NULL,
    ADD COLUMN status_locked BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN is_reference BOOLEAN NOT NULL DEFAULT FALSE,
    ADD KEY idx_providers_status (status);

CREATE TABLE provider_verifications (
    id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    run_id VARCHAR(36) NOT NULL,
    provider_id BIGINT NOT NULL,
    model_name VARCHAR(255) NOT NULL,
    reference_provider_id BIGINT NULL,
    outcome VARCHAR(16) NOT NULL,
    passed BOOLEAN NOT NULL DEFAULT FALSE,
    reason TEXT NULL,
    scores LONGTEXT NULL,
    results LONGTEXT NULL,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    KEY idx_provider_verifications_provider (provider_id, created_at),
    KEY idx_provider_verifications_run (run_id),
    CONSTRAINT fk_provider_verifications_provider FOREIGN KEY (provider_id) REFERENCES providers (id) ON DELETE CASCADE
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_uca1400_as_cs;
