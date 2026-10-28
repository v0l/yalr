-- Persist timeseries metric snapshots for survival across restarts.
-- Retains 288 snapshots (24 hours at 5-minute intervals).
CREATE TABLE IF NOT EXISTS metrics_history (
    timestamp_ms INTEGER NOT NULL PRIMARY KEY,
    snapshot_json TEXT NOT NULL
);
