ALTER TABLE request_log
    ADD COLUMN IF NOT EXISTS failure_attribution ENUM('provider', 'request', 'unknown') NULL AFTER status;
