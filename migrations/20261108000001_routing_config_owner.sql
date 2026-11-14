ALTER TABLE routing_config
    ADD COLUMN IF NOT EXISTS owner_user_id BIGINT NULL,
    ADD KEY IF NOT EXISTS idx_routing_config_owner (owner_user_id),
    ADD CONSTRAINT fk_routing_config_owner FOREIGN KEY IF NOT EXISTS (owner_user_id) REFERENCES users (id) ON DELETE CASCADE;
