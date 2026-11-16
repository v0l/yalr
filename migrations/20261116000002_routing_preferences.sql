ALTER TABLE providers ADD COLUMN max_concurrency INT NULL;

CREATE TABLE user_routing_preferences (
    id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    user_id BIGINT NOT NULL,
    model_key VARCHAR(255) NOT NULL DEFAULT '',
    preference VARCHAR(16) NOT NULL,
    UNIQUE KEY uq_user_routing_preferences (user_id, model_key),
    CONSTRAINT fk_user_routing_preferences_user FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE CASCADE
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_uca1400_as_cs;
