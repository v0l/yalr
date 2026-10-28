CREATE TABLE IF NOT EXISTS request_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id TEXT NOT NULL,
    started_at_ms INTEGER NOT NULL,
    endpoint TEXT NOT NULL,
    stream INTEGER NOT NULL,
    model TEXT NOT NULL,
    user_id INTEGER,
    user_name TEXT,
    api_key_id INTEGER,
    api_key_name TEXT,
    client_ip TEXT,
    client_headers TEXT,
    provider TEXT,
    upstream_url TEXT,
    upstream_request_headers TEXT,
    upstream_status INTEGER,
    upstream_response_headers TEXT,
    attempts INTEGER NOT NULL,
    attempt_log TEXT,
    status TEXT NOT NULL,
    error TEXT,
    finish_reason TEXT,
    headers_ms INTEGER,
    first_event_ms INTEGER,
    first_thinking_ms INTEGER,
    first_text_ms INTEGER,
    total_ms INTEGER,
    prompt_tokens INTEGER,
    completion_tokens INTEGER,
    cached_tokens INTEGER,
    cache_write_tokens INTEGER,
    weighted_input_tokens INTEGER,
    message_count INTEGER,
    system_hash TEXT,
    tools_hash TEXT,
    message_hashes TEXT,
    first_diverging_message INTEGER,
    previous_request_id TEXT,
    request_bytes INTEGER,
    response_bytes INTEGER
);

CREATE INDEX IF NOT EXISTS idx_request_log_started ON request_log (started_at_ms);

CREATE INDEX IF NOT EXISTS idx_request_log_request_id ON request_log (request_id);

CREATE INDEX IF NOT EXISTS idx_request_log_model ON request_log (model, started_at_ms);

CREATE TABLE IF NOT EXISTS request_log_body (
    log_id INTEGER PRIMARY KEY REFERENCES request_log (id) ON DELETE CASCADE,
    client_request_gz BLOB,
    upstream_request_gz BLOB,
    response_gz BLOB
);
