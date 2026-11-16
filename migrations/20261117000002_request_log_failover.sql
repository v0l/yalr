ALTER TABLE request_log
    ADD COLUMN IF NOT EXISTS failover_note TEXT NULL;
