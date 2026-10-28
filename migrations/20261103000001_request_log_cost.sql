ALTER TABLE request_log
    ADD COLUMN IF NOT EXISTS cost_msat BIGINT NULL,
    ADD KEY IF NOT EXISTS idx_request_log_endpoint (endpoint, id);
