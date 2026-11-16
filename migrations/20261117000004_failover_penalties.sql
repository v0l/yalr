CREATE TABLE IF NOT EXISTS failover_penalties (
    id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    request_id VARCHAR(255) NOT NULL,
    failed_provider VARCHAR(255) NOT NULL,
    failed_owner_id BIGINT NULL,
    served_provider VARCHAR(255) NOT NULL,
    currency VARCHAR(8) NOT NULL,
    gap_milli BIGINT NOT NULL,
    penalty_milli BIGINT NOT NULL,
    platform_milli BIGINT NOT NULL,
    expected_cached_tokens BIGINT NOT NULL,
    reason TEXT,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    KEY idx_failover_penalties_provider (failed_provider, created_at),
    KEY idx_failover_penalties_owner (failed_owner_id, created_at),
    KEY idx_failover_penalties_request (request_id)
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_uca1400_as_cs;
