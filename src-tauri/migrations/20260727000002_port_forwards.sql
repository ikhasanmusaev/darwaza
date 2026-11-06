-- Migration #4: port_forwards (#130)
-- Черновик схемы был в docs/01-architecture/database.md; здесь он уточнён.

CREATE TABLE IF NOT EXISTS port_forwards (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    host_id      INTEGER NOT NULL REFERENCES hosts(id) ON DELETE CASCADE,
    kind         TEXT    NOT NULL CHECK (kind IN ('local', 'remote', 'dynamic')),
    label        TEXT,
    -- Для local/dynamic — где слушаем у себя.
    -- Для remote — куда на нашей стороне отправлять пришедшее с сервера.
    local_host   TEXT    NOT NULL DEFAULT '127.0.0.1',
    local_port   INTEGER NOT NULL,
    -- Для local — куда идти со стороны сервера.
    -- Для remote — что сервер должен слушать.
    -- Для dynamic не используются: адрес назначения приходит по SOCKS5.
    remote_host  TEXT,
    remote_port  INTEGER,
    auto_start   INTEGER NOT NULL DEFAULT 0,
    created_at   DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at   DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_port_forwards_host ON port_forwards(host_id);
