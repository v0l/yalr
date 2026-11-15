CREATE TABLE provider_health_checks (
    id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    provider_id BIGINT NOT NULL,
    checked_at_ms BIGINT NOT NULL,
    healthy BOOLEAN NOT NULL,
    KEY idx_provider_health_checks_provider (provider_id, checked_at_ms),
    KEY idx_provider_health_checks_at (checked_at_ms),
    CONSTRAINT fk_provider_health_checks_provider FOREIGN KEY (provider_id) REFERENCES providers (id) ON DELETE CASCADE
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_uca1400_as_cs;

ALTER TABLE request_log ADD KEY idx_request_log_provider_started (provider, started_at_ms);
