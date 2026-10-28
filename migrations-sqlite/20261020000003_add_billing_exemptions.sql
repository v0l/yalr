CREATE TABLE IF NOT EXISTS billing_exempt_users (
    user_id INTEGER PRIMARY KEY,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

INSERT OR IGNORE INTO billing_exempt_users (user_id) SELECT id FROM users;
