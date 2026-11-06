CREATE TABLE IF NOT EXISTS connection_log (
    id INTEGER PRIMARY KEY,
    host_id INTEGER NOT NULL REFERENCES hosts(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL,
    started_at TEXT NOT NULL DEFAULT (datetime('now')),
    ended_at TEXT,
    status TEXT NOT NULL DEFAULT 'connecting',
    error TEXT
);

CREATE INDEX IF NOT EXISTS idx_connection_log_host ON connection_log(host_id);
CREATE INDEX IF NOT EXISTS idx_connection_log_started ON connection_log(started_at DESC);
