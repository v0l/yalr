ALTER TABLE providers
    ADD COLUMN IF NOT EXISTS extra_body TEXT NULL,
    ADD CONSTRAINT chk_providers_extra_body CHECK (extra_body IS NULL OR JSON_VALID(extra_body));

ALTER TABLE routing_config_providers
    ADD COLUMN IF NOT EXISTS extra_body TEXT NULL,
    ADD CONSTRAINT chk_rcp_extra_body CHECK (extra_body IS NULL OR JSON_VALID(extra_body));
