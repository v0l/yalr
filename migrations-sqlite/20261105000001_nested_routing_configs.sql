CREATE TABLE routing_config_providers_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    routing_config_id INTEGER NOT NULL,
    provider_id INTEGER,
    child_config_id INTEGER,
    model TEXT,
    weight INTEGER DEFAULT 100,
    is_active BOOLEAN DEFAULT 1,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (routing_config_id) REFERENCES routing_config(id) ON DELETE CASCADE,
    FOREIGN KEY (provider_id) REFERENCES providers(id) ON DELETE CASCADE,
    FOREIGN KEY (child_config_id) REFERENCES routing_config(id) ON DELETE CASCADE,
    UNIQUE(routing_config_id, provider_id, model),
    UNIQUE(routing_config_id, child_config_id, model),
    CHECK ((provider_id IS NULL) <> (child_config_id IS NULL))
);

INSERT INTO routing_config_providers_new
    (id, routing_config_id, provider_id, model, weight, is_active, created_at, updated_at)
SELECT id, routing_config_id, provider_id, model, weight, is_active, created_at, updated_at
FROM routing_config_providers;

DROP TABLE routing_config_providers;
ALTER TABLE routing_config_providers_new RENAME TO routing_config_providers;

CREATE INDEX idx_routing_config_providers_config ON routing_config_providers(routing_config_id);
CREATE INDEX idx_routing_config_providers_provider ON routing_config_providers(provider_id);
