ALTER TABLE request_log
    ADD COLUMN IF NOT EXISTS upstream_model VARCHAR(255) NULL AFTER route;
