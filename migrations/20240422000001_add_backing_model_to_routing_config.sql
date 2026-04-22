-- Add routing_config_providers junction table
-- This links routing configs to providers with model specifications
-- Allows each routing config to have multiple providers with specific model configurations

CREATE TABLE IF NOT EXISTS routing_config_providers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    routing_config_id INTEGER NOT NULL,
    provider_id INTEGER NOT NULL,
    model TEXT,  -- Specific model to use, or NULL for any model from that provider
    weight INTEGER DEFAULT 100,
    is_active BOOLEAN DEFAULT 1,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (routing_config_id) REFERENCES routing_config(id) ON DELETE CASCADE,
    FOREIGN KEY (provider_id) REFERENCES providers(id) ON DELETE CASCADE,
    UNIQUE(routing_config_id, provider_id, model)
);

-- Create indexes for faster lookups
CREATE INDEX IF NOT EXISTS idx_routing_config_providers_config ON routing_config_providers(routing_config_id);
CREATE INDEX IF NOT EXISTS idx_routing_config_providers_provider ON routing_config_providers(provider_id);
