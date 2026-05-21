-- Model Access Control: per-user model permissions
-- Allows admins to grant/deny specific model access per user

CREATE TABLE IF NOT EXISTS user_model_permissions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL,
    model TEXT NOT NULL,  -- exact model name or '*' for all models
    allow BOOLEAN NOT NULL DEFAULT 1,  -- true=allow, false=deny
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    UNIQUE(user_id, model)
);

CREATE INDEX IF NOT EXISTS idx_user_model_permissions_user ON user_model_permissions(user_id);
CREATE INDEX IF NOT EXISTS idx_user_model_permissions_model ON user_model_permissions(model);
