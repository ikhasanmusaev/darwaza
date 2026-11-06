-- Migration #1: initial schema
-- See docs/01-architecture/database.md

CREATE TABLE IF NOT EXISTS groups (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT    NOT NULL,
    parent_id   INTEGER REFERENCES groups(id) ON DELETE SET NULL,
    sort_order  INTEGER NOT NULL DEFAULT 0,
    created_at  DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at  DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS identities (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    label           TEXT    NOT NULL,
    kind            TEXT    NOT NULL CHECK (kind IN ('password', 'private_key', 'agent')),
    key_path        TEXT,
    keyring_ref     TEXT,
    has_passphrase  INTEGER NOT NULL DEFAULT 0,
    public_key      TEXT,
    created_at      DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at      DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS hosts (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    group_id             INTEGER REFERENCES groups(id) ON DELETE SET NULL,
    name                 TEXT    NOT NULL,
    hostname             TEXT    NOT NULL,
    port                 INTEGER NOT NULL DEFAULT 22,
    username             TEXT    NOT NULL,
    identity_id          INTEGER REFERENCES identities(id) ON DELETE SET NULL,
    auth_method          TEXT    NOT NULL CHECK (auth_method IN ('password', 'publickey', 'agent')),
    color                TEXT,
    icon                 TEXT,
    jump_host_id         INTEGER REFERENCES hosts(id) ON DELETE SET NULL,
    keep_alive_interval  INTEGER NOT NULL DEFAULT 30,
    description          TEXT,
    last_connected_at    DATETIME,
    created_at           DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at           DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS known_hosts (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    hostname     TEXT NOT NULL,
    key_type     TEXT NOT NULL,
    fingerprint  TEXT NOT NULL,
    accepted_at  DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS settings (
    key    TEXT PRIMARY KEY,
    value  TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_hosts_group ON hosts(group_id);
CREATE INDEX IF NOT EXISTS idx_hosts_last_connected ON hosts(last_connected_at DESC);
CREATE INDEX IF NOT EXISTS idx_known_hosts_hostname ON known_hosts(hostname);
CREATE UNIQUE INDEX IF NOT EXISTS uniq_known_hosts_host_keytype ON known_hosts(hostname, key_type);
