ALTER TABLE request_log_body
    ADD COLUMN size_bytes BIGINT NULL,
    ADD KEY idx_request_log_body_size (log_id, size_bytes),
    ADD KEY idx_request_log_body_unsized (size_bytes, log_id);
