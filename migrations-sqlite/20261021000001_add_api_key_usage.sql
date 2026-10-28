CREATE TABLE IF NOT EXISTS api_key_usage (
    api_key_id INTEGER NOT NULL,
    hour TEXT NOT NULL,
    model TEXT NOT NULL,
    requests INTEGER NOT NULL DEFAULT 0,
    prompt_tokens INTEGER NOT NULL DEFAULT 0,
    completion_tokens INTEGER NOT NULL DEFAULT 0,
    cached_tokens INTEGER NOT NULL DEFAULT 0,
    cache_write_tokens INTEGER NOT NULL DEFAULT 0,
    cost_msat INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (api_key_id, hour, model)
);
