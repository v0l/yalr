CREATE TABLE payouts (
    id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    user_id BIGINT NOT NULL,
    currency VARCHAR(8) NOT NULL,
    amount BIGINT NOT NULL,
    fee BIGINT NOT NULL DEFAULT 0,
    method VARCHAR(16) NOT NULL,
    destination VARCHAR(1024) NOT NULL,
    status VARCHAR(16) NOT NULL,
    external_id VARCHAR(255),
    failure_reason TEXT,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3),
    completed_at DATETIME(3),
    UNIQUE KEY uq_payouts_external (method, external_id),
    KEY idx_payouts_user (user_id, created_at, id),
    KEY idx_payouts_status (status, updated_at),
    CONSTRAINT fk_payouts_user FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE CASCADE
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_uca1400_as_cs;
