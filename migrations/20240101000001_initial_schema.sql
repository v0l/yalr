-- YALR Database Schema
-- Single migration file for initial database setup

-- Providers table with retry configuration
CREATE TABLE IF NOT EXISTS providers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT UNIQUE NOT NULL,
    slug TEXT UNIQUE NOT NULL,
    provider_type TEXT NOT NULL DEFAULT 'openai',
    base_url TEXT NOT NULL,
    api_key_env TEXT,
    api_key TEXT,
    is_active BOOLEAN DEFAULT 1,
    max_retries INTEGER DEFAULT 3,
    retry_delay_ms INTEGER DEFAULT 1000,
    backoff_multiplier REAL DEFAULT 2.0,
    max_backoff_ms INTEGER DEFAULT 30000,
    timeout_ms INTEGER DEFAULT 60000,
    metadata JSON,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Canonical model definitions (model families/types)
CREATE TABLE IF NOT EXISTS models (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT UNIQUE NOT NULL,
    display_name TEXT,
    model_family TEXT,
    parameter_size TEXT,
    is_active BOOLEAN DEFAULT 1,
    metadata JSON,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Provider-specific model configurations
CREATE TABLE IF NOT EXISTS provider_model_configs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    model_id INTEGER NOT NULL,
    provider_id INTEGER NOT NULL,
    served_model_name TEXT NOT NULL,
    weight INTEGER DEFAULT 100,
    is_active BOOLEAN DEFAULT 1,
    context_window INTEGER,
    max_output_tokens INTEGER,
    cost_per_1m_input REAL DEFAULT 0.0,
    cost_per_1m_output REAL DEFAULT 0.0,
    quantization TEXT,
    variant TEXT,
    max_requests_per_minute INTEGER,
    max_requests_per_hour INTEGER,
    max_tokens_per_minute INTEGER,
    max_tokens_per_hour INTEGER,
    metadata JSON,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (model_id) REFERENCES models(id) ON DELETE CASCADE,
    FOREIGN KEY (provider_id) REFERENCES providers(id) ON DELETE CASCADE,
    UNIQUE(model_id, provider_id, served_model_name)
);

-- Routing configuration
CREATE TABLE IF NOT EXISTS routing_config (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    strategy TEXT NOT NULL DEFAULT 'round_robin',
    global_max_retries INTEGER DEFAULT 3,
    global_retry_delay_ms INTEGER DEFAULT 1000,
    global_backoff_multiplier REAL DEFAULT 2.0,
    global_timeout_ms INTEGER DEFAULT 60000,
    health_check_enabled BOOLEAN DEFAULT 1,
    health_check_interval_seconds INTEGER DEFAULT 30,
    health_check_timeout_seconds INTEGER DEFAULT 5,
    health_check_failure_threshold INTEGER DEFAULT 3,
    health_check_recovery_threshold INTEGER DEFAULT 1,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Rate limiting configuration per provider
CREATE TABLE IF NOT EXISTS rate_limits (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_id INTEGER NOT NULL,
    requests_per_second INTEGER DEFAULT 10,
    requests_per_minute INTEGER DEFAULT 100,
    requests_per_hour INTEGER DEFAULT 1000,
    tokens_per_minute INTEGER DEFAULT 10000,
    tokens_per_hour INTEGER DEFAULT 100000,
    burst_size INTEGER DEFAULT 20,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (provider_id) REFERENCES providers(id) ON DELETE CASCADE,
    UNIQUE(provider_id)
);

-- Quota configuration per provider
CREATE TABLE IF NOT EXISTS quotas (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_id INTEGER NOT NULL,
    daily_token_limit INTEGER,
    monthly_token_limit INTEGER,
    daily_request_limit INTEGER,
    monthly_request_limit INTEGER,
    daily_cost_limit REAL,
    monthly_cost_limit REAL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (provider_id) REFERENCES providers(id) ON DELETE CASCADE,
    UNIQUE(provider_id)
);

-- Quota usage tracking
CREATE TABLE IF NOT EXISTS quota_usage (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_id INTEGER NOT NULL,
    usage_date DATE NOT NULL,
    usage_month DATE NOT NULL,
    tokens_used INTEGER DEFAULT 0,
    requests_used INTEGER DEFAULT 0,
    cost_used REAL DEFAULT 0.0,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (provider_id) REFERENCES providers(id) ON DELETE CASCADE,
    UNIQUE(provider_id, usage_date, usage_month)
);

-- Request logging
CREATE TABLE IF NOT EXISTS request_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    model_id INTEGER,
    provider_id INTEGER,
    request_tokens INTEGER,
    response_tokens INTEGER,
    total_tokens INTEGER,
    cost REAL DEFAULT 0.0,
    latency_ms INTEGER,
    status TEXT NOT NULL,
    error_message TEXT,
    retry_count INTEGER DEFAULT 0,
    fallback_count INTEGER DEFAULT 0,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (model_id) REFERENCES models(id) ON DELETE SET NULL,
    FOREIGN KEY (provider_id) REFERENCES providers(id) ON DELETE SET NULL
);

-- Provider health logs
CREATE TABLE IF NOT EXISTS health_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_id INTEGER NOT NULL,
    health_state TEXT NOT NULL,
    consecutive_failures INTEGER DEFAULT 0,
    consecutive_successes INTEGER DEFAULT 0,
    last_error TEXT,
    backoff_ms INTEGER,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (provider_id) REFERENCES providers(id) ON DELETE CASCADE
);

-- Performance indexes
CREATE INDEX IF NOT EXISTS idx_providers_slug ON providers(slug);
CREATE INDEX IF NOT EXISTS idx_providers_is_active ON providers(is_active);
CREATE INDEX IF NOT EXISTS idx_models_name ON models(name);
CREATE INDEX IF NOT EXISTS idx_models_family ON models(model_family);
CREATE INDEX IF NOT EXISTS idx_models_active ON models(is_active);
CREATE INDEX IF NOT EXISTS idx_provider_model_configs_model ON provider_model_configs(model_id);
CREATE INDEX IF NOT EXISTS idx_provider_model_configs_provider ON provider_model_configs(provider_id);
CREATE INDEX IF NOT EXISTS idx_provider_model_configs_active ON provider_model_configs(is_active);
CREATE INDEX IF NOT EXISTS idx_provider_model_configs_served_name ON provider_model_configs(served_model_name);
CREATE INDEX IF NOT EXISTS idx_routing_config_strategy ON routing_config(strategy);
CREATE INDEX IF NOT EXISTS idx_quota_usage_date ON quota_usage(usage_date);
CREATE INDEX IF NOT EXISTS idx_quota_usage_month ON quota_usage(usage_month);
CREATE INDEX IF NOT EXISTS idx_request_logs_created ON request_logs(created_at);
CREATE INDEX IF NOT EXISTS idx_request_logs_model ON request_logs(model_id);
CREATE INDEX IF NOT EXISTS idx_request_logs_provider ON request_logs(provider_id);
CREATE INDEX IF NOT EXISTS idx_health_logs_provider ON health_logs(provider_id);
CREATE INDEX IF NOT EXISTS idx_health_logs_created ON health_logs(created_at);
