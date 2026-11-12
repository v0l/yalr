ALTER TABLE routing_config_providers
    MODIFY provider_id BIGINT NULL,
    ADD COLUMN IF NOT EXISTS child_config_id BIGINT NULL AFTER provider_id,
    ADD KEY IF NOT EXISTS idx_routing_config_providers_child (child_config_id),
    ADD UNIQUE KEY IF NOT EXISTS uq_routing_config_children (routing_config_id, child_config_id, model),
    ADD CONSTRAINT fk_rcp_child FOREIGN KEY IF NOT EXISTS (child_config_id) REFERENCES routing_config (id) ON DELETE CASCADE,
    ADD CONSTRAINT chk_rcp_one_member CHECK ((provider_id IS NULL) <> (child_config_id IS NULL));

ALTER TABLE request_log
    ADD COLUMN IF NOT EXISTS route VARCHAR(512) NULL;
