ALTER TABLE request_log_body
    ADD COLUMN IF NOT EXISTS size_bytes BIGINT NULL,
    ADD KEY IF NOT EXISTS idx_request_log_body_size (log_id, size_bytes),
    ADD KEY IF NOT EXISTS idx_request_log_body_unsized (size_bytes, log_id);
