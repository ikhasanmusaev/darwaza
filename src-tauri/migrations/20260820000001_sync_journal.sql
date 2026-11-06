-- Подготовка к синхронизации (#180): стабильные идентификаторы и журнал изменений.
--
-- Первичные ключи НЕ меняем. Числовые ключи связывают девять таблиц, keyring
-- адресует секреты парой (вид, числовой id), и весь UI типизирован под числа.
-- Переписывать слой данных ради синхронизации, которой ещё нет на сервере, —
-- плохой размен. Синхронизации нужен не UUID-ключ, а стабильный глобальный
-- идентификатор, и он добавляется колонкой рядом.
--
-- Журнал ведут ТРИГГЕРЫ, а не приложение. Так существующие команды создания,
-- изменения и удаления остаются нетронутыми: невозможно забыть отметить
-- изменение, потому что отмечает его сама база.

-- ── Стабильные идентификаторы ───────────────────────────────────────────
--
-- Заполняются приложением при вставке. Для уже существующих строк проставляем
-- значение из hex(randomblob(16)) — этого достаточно: собственный формат UUID
-- здесь не нужен, важна лишь глобальная уникальность.

ALTER TABLE groups        ADD COLUMN sync_id TEXT;
ALTER TABLE hosts         ADD COLUMN sync_id TEXT;
ALTER TABLE identities    ADD COLUMN sync_id TEXT;
ALTER TABLE snippets      ADD COLUMN sync_id TEXT;
ALTER TABLE port_forwards ADD COLUMN sync_id TEXT;
ALTER TABLE known_hosts   ADD COLUMN sync_id TEXT;

UPDATE groups        SET sync_id = lower(hex(randomblob(16))) WHERE sync_id IS NULL;
UPDATE hosts         SET sync_id = lower(hex(randomblob(16))) WHERE sync_id IS NULL;
UPDATE identities    SET sync_id = lower(hex(randomblob(16))) WHERE sync_id IS NULL;
UPDATE snippets      SET sync_id = lower(hex(randomblob(16))) WHERE sync_id IS NULL;
UPDATE port_forwards SET sync_id = lower(hex(randomblob(16))) WHERE sync_id IS NULL;
UPDATE known_hosts   SET sync_id = lower(hex(randomblob(16))) WHERE sync_id IS NULL;

CREATE UNIQUE INDEX IF NOT EXISTS groups_sync_id        ON groups (sync_id);
CREATE UNIQUE INDEX IF NOT EXISTS hosts_sync_id         ON hosts (sync_id);
CREATE UNIQUE INDEX IF NOT EXISTS identities_sync_id    ON identities (sync_id);
CREATE UNIQUE INDEX IF NOT EXISTS snippets_sync_id      ON snippets (sync_id);
CREATE UNIQUE INDEX IF NOT EXISTS port_forwards_sync_id ON port_forwards (sync_id);
CREATE UNIQUE INDEX IF NOT EXISTS known_hosts_sync_id   ON known_hosts (sync_id);

-- ── Журнал изменений ────────────────────────────────────────────────────
--
-- Одна строка на запись. `deleted` отмечает удалённые: жёстко забывать их
-- нельзя, иначе устройство, не выходившее в сеть, зальёт запись обратно.
--
-- `revision` — версия, известная серверу. Растёт при каждом изменении и
-- сравнивается при отправке: разошлась — конфликт.
--
-- `dirty` — изменено после последней синхронизации. Именно его читает
-- отправка, а не время: часы на устройствах расходятся.

CREATE TABLE IF NOT EXISTS sync_journal (
    kind        TEXT    NOT NULL,
    sync_id     TEXT    NOT NULL,
    -- Локальный ключ. NULL у удалённых: строки уже нет.
    local_id    INTEGER,
    revision    INTEGER NOT NULL DEFAULT 1,
    dirty       INTEGER NOT NULL DEFAULT 1,
    deleted     INTEGER NOT NULL DEFAULT 0,
    updated_at  DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (kind, sync_id)
);

-- Отправка выбирает именно неотправленное — индекс под этот запрос.
CREATE INDEX IF NOT EXISTS sync_journal_dirty ON sync_journal (dirty) WHERE dirty = 1;

-- ── Состояние синхронизации ─────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS sync_state (
    key    TEXT PRIMARY KEY,
    value  TEXT NOT NULL
);

-- ── Триггеры ────────────────────────────────────────────────────────────
--
-- Вставка и изменение помечают запись как изменённую, удаление оставляет
-- надгробие. Приложение об этом не знает и знать не должно.

CREATE TRIGGER IF NOT EXISTS groups_sync_insert AFTER INSERT ON groups
BEGIN
    INSERT INTO sync_journal (kind, sync_id, local_id, revision, dirty, deleted, updated_at)
    VALUES ('groups', NEW.sync_id, NEW.id, 1, 1, 0, CURRENT_TIMESTAMP)
    ON CONFLICT(kind, sync_id) DO UPDATE SET
        local_id   = NEW.id,
        revision   = sync_journal.revision + 1,
        dirty      = 1,
        deleted    = 0,
        updated_at = CURRENT_TIMESTAMP;
END;

CREATE TRIGGER IF NOT EXISTS groups_sync_update AFTER UPDATE ON groups
BEGIN
    INSERT INTO sync_journal (kind, sync_id, local_id, revision, dirty, deleted, updated_at)
    VALUES ('groups', NEW.sync_id, NEW.id, 1, 1, 0, CURRENT_TIMESTAMP)
    ON CONFLICT(kind, sync_id) DO UPDATE SET
        local_id   = NEW.id,
        revision   = sync_journal.revision + 1,
        dirty      = 1,
        deleted    = 0,
        updated_at = CURRENT_TIMESTAMP;
END;

-- Надгробие: local_id обнуляется, потому что строки больше нет.
CREATE TRIGGER IF NOT EXISTS groups_sync_delete AFTER DELETE ON groups
BEGIN
    INSERT INTO sync_journal (kind, sync_id, local_id, revision, dirty, deleted, updated_at)
    VALUES ('groups', OLD.sync_id, NULL, 1, 1, 1, CURRENT_TIMESTAMP)
    ON CONFLICT(kind, sync_id) DO UPDATE SET
        local_id   = NULL,
        revision   = sync_journal.revision + 1,
        dirty      = 1,
        deleted    = 1,
        updated_at = CURRENT_TIMESTAMP;
END;

CREATE TRIGGER IF NOT EXISTS hosts_sync_insert AFTER INSERT ON hosts
BEGIN
    INSERT INTO sync_journal (kind, sync_id, local_id, revision, dirty, deleted, updated_at)
    VALUES ('hosts', NEW.sync_id, NEW.id, 1, 1, 0, CURRENT_TIMESTAMP)
    ON CONFLICT(kind, sync_id) DO UPDATE SET
        local_id   = NEW.id,
        revision   = sync_journal.revision + 1,
        dirty      = 1,
        deleted    = 0,
        updated_at = CURRENT_TIMESTAMP;
END;

CREATE TRIGGER IF NOT EXISTS hosts_sync_update AFTER UPDATE ON hosts
BEGIN
    INSERT INTO sync_journal (kind, sync_id, local_id, revision, dirty, deleted, updated_at)
    VALUES ('hosts', NEW.sync_id, NEW.id, 1, 1, 0, CURRENT_TIMESTAMP)
    ON CONFLICT(kind, sync_id) DO UPDATE SET
        local_id   = NEW.id,
        revision   = sync_journal.revision + 1,
        dirty      = 1,
        deleted    = 0,
        updated_at = CURRENT_TIMESTAMP;
END;

-- Надгробие: local_id обнуляется, потому что строки больше нет.
CREATE TRIGGER IF NOT EXISTS hosts_sync_delete AFTER DELETE ON hosts
BEGIN
    INSERT INTO sync_journal (kind, sync_id, local_id, revision, dirty, deleted, updated_at)
    VALUES ('hosts', OLD.sync_id, NULL, 1, 1, 1, CURRENT_TIMESTAMP)
    ON CONFLICT(kind, sync_id) DO UPDATE SET
        local_id   = NULL,
        revision   = sync_journal.revision + 1,
        dirty      = 1,
        deleted    = 1,
        updated_at = CURRENT_TIMESTAMP;
END;

CREATE TRIGGER IF NOT EXISTS identities_sync_insert AFTER INSERT ON identities
BEGIN
    INSERT INTO sync_journal (kind, sync_id, local_id, revision, dirty, deleted, updated_at)
    VALUES ('identities', NEW.sync_id, NEW.id, 1, 1, 0, CURRENT_TIMESTAMP)
    ON CONFLICT(kind, sync_id) DO UPDATE SET
        local_id   = NEW.id,
        revision   = sync_journal.revision + 1,
        dirty      = 1,
        deleted    = 0,
        updated_at = CURRENT_TIMESTAMP;
END;

CREATE TRIGGER IF NOT EXISTS identities_sync_update AFTER UPDATE ON identities
BEGIN
    INSERT INTO sync_journal (kind, sync_id, local_id, revision, dirty, deleted, updated_at)
    VALUES ('identities', NEW.sync_id, NEW.id, 1, 1, 0, CURRENT_TIMESTAMP)
    ON CONFLICT(kind, sync_id) DO UPDATE SET
        local_id   = NEW.id,
        revision   = sync_journal.revision + 1,
        dirty      = 1,
        deleted    = 0,
        updated_at = CURRENT_TIMESTAMP;
END;

-- Надгробие: local_id обнуляется, потому что строки больше нет.
CREATE TRIGGER IF NOT EXISTS identities_sync_delete AFTER DELETE ON identities
BEGIN
    INSERT INTO sync_journal (kind, sync_id, local_id, revision, dirty, deleted, updated_at)
    VALUES ('identities', OLD.sync_id, NULL, 1, 1, 1, CURRENT_TIMESTAMP)
    ON CONFLICT(kind, sync_id) DO UPDATE SET
        local_id   = NULL,
        revision   = sync_journal.revision + 1,
        dirty      = 1,
        deleted    = 1,
        updated_at = CURRENT_TIMESTAMP;
END;

CREATE TRIGGER IF NOT EXISTS snippets_sync_insert AFTER INSERT ON snippets
BEGIN
    INSERT INTO sync_journal (kind, sync_id, local_id, revision, dirty, deleted, updated_at)
    VALUES ('snippets', NEW.sync_id, NEW.id, 1, 1, 0, CURRENT_TIMESTAMP)
    ON CONFLICT(kind, sync_id) DO UPDATE SET
        local_id   = NEW.id,
        revision   = sync_journal.revision + 1,
        dirty      = 1,
        deleted    = 0,
        updated_at = CURRENT_TIMESTAMP;
END;

CREATE TRIGGER IF NOT EXISTS snippets_sync_update AFTER UPDATE ON snippets
BEGIN
    INSERT INTO sync_journal (kind, sync_id, local_id, revision, dirty, deleted, updated_at)
    VALUES ('snippets', NEW.sync_id, NEW.id, 1, 1, 0, CURRENT_TIMESTAMP)
    ON CONFLICT(kind, sync_id) DO UPDATE SET
        local_id   = NEW.id,
        revision   = sync_journal.revision + 1,
        dirty      = 1,
        deleted    = 0,
        updated_at = CURRENT_TIMESTAMP;
END;

-- Надгробие: local_id обнуляется, потому что строки больше нет.
CREATE TRIGGER IF NOT EXISTS snippets_sync_delete AFTER DELETE ON snippets
BEGIN
    INSERT INTO sync_journal (kind, sync_id, local_id, revision, dirty, deleted, updated_at)
    VALUES ('snippets', OLD.sync_id, NULL, 1, 1, 1, CURRENT_TIMESTAMP)
    ON CONFLICT(kind, sync_id) DO UPDATE SET
        local_id   = NULL,
        revision   = sync_journal.revision + 1,
        dirty      = 1,
        deleted    = 1,
        updated_at = CURRENT_TIMESTAMP;
END;

CREATE TRIGGER IF NOT EXISTS port_forwards_sync_insert AFTER INSERT ON port_forwards
BEGIN
    INSERT INTO sync_journal (kind, sync_id, local_id, revision, dirty, deleted, updated_at)
    VALUES ('port_forwards', NEW.sync_id, NEW.id, 1, 1, 0, CURRENT_TIMESTAMP)
    ON CONFLICT(kind, sync_id) DO UPDATE SET
        local_id   = NEW.id,
        revision   = sync_journal.revision + 1,
        dirty      = 1,
        deleted    = 0,
        updated_at = CURRENT_TIMESTAMP;
END;

CREATE TRIGGER IF NOT EXISTS port_forwards_sync_update AFTER UPDATE ON port_forwards
BEGIN
    INSERT INTO sync_journal (kind, sync_id, local_id, revision, dirty, deleted, updated_at)
    VALUES ('port_forwards', NEW.sync_id, NEW.id, 1, 1, 0, CURRENT_TIMESTAMP)
    ON CONFLICT(kind, sync_id) DO UPDATE SET
        local_id   = NEW.id,
        revision   = sync_journal.revision + 1,
        dirty      = 1,
        deleted    = 0,
        updated_at = CURRENT_TIMESTAMP;
END;

-- Надгробие: local_id обнуляется, потому что строки больше нет.
CREATE TRIGGER IF NOT EXISTS port_forwards_sync_delete AFTER DELETE ON port_forwards
BEGIN
    INSERT INTO sync_journal (kind, sync_id, local_id, revision, dirty, deleted, updated_at)
    VALUES ('port_forwards', OLD.sync_id, NULL, 1, 1, 1, CURRENT_TIMESTAMP)
    ON CONFLICT(kind, sync_id) DO UPDATE SET
        local_id   = NULL,
        revision   = sync_journal.revision + 1,
        dirty      = 1,
        deleted    = 1,
        updated_at = CURRENT_TIMESTAMP;
END;

CREATE TRIGGER IF NOT EXISTS known_hosts_sync_insert AFTER INSERT ON known_hosts
BEGIN
    INSERT INTO sync_journal (kind, sync_id, local_id, revision, dirty, deleted, updated_at)
    VALUES ('known_hosts', NEW.sync_id, NEW.id, 1, 1, 0, CURRENT_TIMESTAMP)
    ON CONFLICT(kind, sync_id) DO UPDATE SET
        local_id   = NEW.id,
        revision   = sync_journal.revision + 1,
        dirty      = 1,
        deleted    = 0,
        updated_at = CURRENT_TIMESTAMP;
END;

CREATE TRIGGER IF NOT EXISTS known_hosts_sync_update AFTER UPDATE ON known_hosts
BEGIN
    INSERT INTO sync_journal (kind, sync_id, local_id, revision, dirty, deleted, updated_at)
    VALUES ('known_hosts', NEW.sync_id, NEW.id, 1, 1, 0, CURRENT_TIMESTAMP)
    ON CONFLICT(kind, sync_id) DO UPDATE SET
        local_id   = NEW.id,
        revision   = sync_journal.revision + 1,
        dirty      = 1,
        deleted    = 0,
        updated_at = CURRENT_TIMESTAMP;
END;

-- Надгробие: local_id обнуляется, потому что строки больше нет.
CREATE TRIGGER IF NOT EXISTS known_hosts_sync_delete AFTER DELETE ON known_hosts
BEGIN
    INSERT INTO sync_journal (kind, sync_id, local_id, revision, dirty, deleted, updated_at)
    VALUES ('known_hosts', OLD.sync_id, NULL, 1, 1, 1, CURRENT_TIMESTAMP)
    ON CONFLICT(kind, sync_id) DO UPDATE SET
        local_id   = NULL,
        revision   = sync_journal.revision + 1,
        dirty      = 1,
        deleted    = 1,
        updated_at = CURRENT_TIMESTAMP;
END;
