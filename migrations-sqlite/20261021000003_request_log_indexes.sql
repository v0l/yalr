CREATE INDEX IF NOT EXISTS idx_request_log_started ON request_log (started_at_ms);

CREATE INDEX IF NOT EXISTS idx_request_log_request_id ON request_log (request_id);

CREATE INDEX IF NOT EXISTS idx_request_log_model ON request_log (model, started_at_ms);

CREATE INDEX IF NOT EXISTS idx_request_log_api_key ON request_log (api_key_id, id);

CREATE INDEX IF NOT EXISTS idx_request_log_user ON request_log (user_id, id);
