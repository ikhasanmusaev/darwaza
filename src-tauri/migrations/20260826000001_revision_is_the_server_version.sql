-- Локальная правка больше не увеличивает `revision`.
--
-- Прежние триггеры делали и то и другое сразу: помечали запись изменённой И
-- увеличивали версию. Комментарий к схеме сам себе противоречил — «версия,
-- известная серверу» и тут же «растёт при каждом изменении».
--
-- Сервер принимает изменение, только если присланная версия СОВПАДАЕТ с его.
-- Значит после локальной правки мы отправляли на единицу больше, чем у
-- сервера, и получали конфликт **на собственную правку**, всегда. Правка
-- уезжала не на сервер, а в «сохранённую проигравшую версию».
--
-- Поймано живым тестом двух устройств: правка первой машины не доехала до
-- второй. Ни один односторонний тест этого показать не мог — там сервера нет,
-- и сравнивать версию не с чем.
--
-- Теперь: `revision` — ровно то, что назвал сервер, а факт локальных правок
-- держит `dirty`. Именно его и читает отправка, как и было записано в схеме.

DROP TRIGGER IF EXISTS groups_sync_insert;
DROP TRIGGER IF EXISTS groups_sync_update;
DROP TRIGGER IF EXISTS hosts_sync_insert;
DROP TRIGGER IF EXISTS hosts_sync_update;
DROP TRIGGER IF EXISTS identities_sync_insert;
DROP TRIGGER IF EXISTS identities_sync_update;
DROP TRIGGER IF EXISTS snippets_sync_insert;
DROP TRIGGER IF EXISTS snippets_sync_update;
DROP TRIGGER IF EXISTS port_forwards_sync_insert;
DROP TRIGGER IF EXISTS port_forwards_sync_update;
DROP TRIGGER IF EXISTS known_hosts_sync_insert;
DROP TRIGGER IF EXISTS known_hosts_sync_update;

CREATE TRIGGER groups_sync_insert AFTER INSERT ON groups
BEGIN
    INSERT INTO sync_journal (kind, sync_id, local_id, revision, dirty, deleted, updated_at)
    VALUES ('groups', NEW.sync_id, NEW.id, 1, 1, 0, CURRENT_TIMESTAMP)
    ON CONFLICT(kind, sync_id) DO UPDATE SET
        local_id = NEW.id, dirty = 1, deleted = 0, updated_at = CURRENT_TIMESTAMP;
END;

CREATE TRIGGER groups_sync_update AFTER UPDATE ON groups
BEGIN
    INSERT INTO sync_journal (kind, sync_id, local_id, revision, dirty, deleted, updated_at)
    VALUES ('groups', NEW.sync_id, NEW.id, 1, 1, 0, CURRENT_TIMESTAMP)
    ON CONFLICT(kind, sync_id) DO UPDATE SET
        local_id = NEW.id, dirty = 1, deleted = 0, updated_at = CURRENT_TIMESTAMP;
END;

CREATE TRIGGER hosts_sync_insert AFTER INSERT ON hosts
BEGIN
    INSERT INTO sync_journal (kind, sync_id, local_id, revision, dirty, deleted, updated_at)
    VALUES ('hosts', NEW.sync_id, NEW.id, 1, 1, 0, CURRENT_TIMESTAMP)
    ON CONFLICT(kind, sync_id) DO UPDATE SET
        local_id = NEW.id, dirty = 1, deleted = 0, updated_at = CURRENT_TIMESTAMP;
END;

CREATE TRIGGER hosts_sync_update AFTER UPDATE ON hosts
BEGIN
    INSERT INTO sync_journal (kind, sync_id, local_id, revision, dirty, deleted, updated_at)
    VALUES ('hosts', NEW.sync_id, NEW.id, 1, 1, 0, CURRENT_TIMESTAMP)
    ON CONFLICT(kind, sync_id) DO UPDATE SET
        local_id = NEW.id, dirty = 1, deleted = 0, updated_at = CURRENT_TIMESTAMP;
END;

CREATE TRIGGER identities_sync_insert AFTER INSERT ON identities
BEGIN
    INSERT INTO sync_journal (kind, sync_id, local_id, revision, dirty, deleted, updated_at)
    VALUES ('identities', NEW.sync_id, NEW.id, 1, 1, 0, CURRENT_TIMESTAMP)
    ON CONFLICT(kind, sync_id) DO UPDATE SET
        local_id = NEW.id, dirty = 1, deleted = 0, updated_at = CURRENT_TIMESTAMP;
END;

CREATE TRIGGER identities_sync_update AFTER UPDATE ON identities
BEGIN
    INSERT INTO sync_journal (kind, sync_id, local_id, revision, dirty, deleted, updated_at)
    VALUES ('identities', NEW.sync_id, NEW.id, 1, 1, 0, CURRENT_TIMESTAMP)
    ON CONFLICT(kind, sync_id) DO UPDATE SET
        local_id = NEW.id, dirty = 1, deleted = 0, updated_at = CURRENT_TIMESTAMP;
END;

CREATE TRIGGER snippets_sync_insert AFTER INSERT ON snippets
BEGIN
    INSERT INTO sync_journal (kind, sync_id, local_id, revision, dirty, deleted, updated_at)
    VALUES ('snippets', NEW.sync_id, NEW.id, 1, 1, 0, CURRENT_TIMESTAMP)
    ON CONFLICT(kind, sync_id) DO UPDATE SET
        local_id = NEW.id, dirty = 1, deleted = 0, updated_at = CURRENT_TIMESTAMP;
END;

CREATE TRIGGER snippets_sync_update AFTER UPDATE ON snippets
BEGIN
    INSERT INTO sync_journal (kind, sync_id, local_id, revision, dirty, deleted, updated_at)
    VALUES ('snippets', NEW.sync_id, NEW.id, 1, 1, 0, CURRENT_TIMESTAMP)
    ON CONFLICT(kind, sync_id) DO UPDATE SET
        local_id = NEW.id, dirty = 1, deleted = 0, updated_at = CURRENT_TIMESTAMP;
END;

CREATE TRIGGER port_forwards_sync_insert AFTER INSERT ON port_forwards
BEGIN
    INSERT INTO sync_journal (kind, sync_id, local_id, revision, dirty, deleted, updated_at)
    VALUES ('port_forwards', NEW.sync_id, NEW.id, 1, 1, 0, CURRENT_TIMESTAMP)
    ON CONFLICT(kind, sync_id) DO UPDATE SET
        local_id = NEW.id, dirty = 1, deleted = 0, updated_at = CURRENT_TIMESTAMP;
END;

CREATE TRIGGER port_forwards_sync_update AFTER UPDATE ON port_forwards
BEGIN
    INSERT INTO sync_journal (kind, sync_id, local_id, revision, dirty, deleted, updated_at)
    VALUES ('port_forwards', NEW.sync_id, NEW.id, 1, 1, 0, CURRENT_TIMESTAMP)
    ON CONFLICT(kind, sync_id) DO UPDATE SET
        local_id = NEW.id, dirty = 1, deleted = 0, updated_at = CURRENT_TIMESTAMP;
END;

CREATE TRIGGER known_hosts_sync_insert AFTER INSERT ON known_hosts
BEGIN
    INSERT INTO sync_journal (kind, sync_id, local_id, revision, dirty, deleted, updated_at)
    VALUES ('known_hosts', NEW.sync_id, NEW.id, 1, 1, 0, CURRENT_TIMESTAMP)
    ON CONFLICT(kind, sync_id) DO UPDATE SET
        local_id = NEW.id, dirty = 1, deleted = 0, updated_at = CURRENT_TIMESTAMP;
END;

CREATE TRIGGER known_hosts_sync_update AFTER UPDATE ON known_hosts
BEGIN
    INSERT INTO sync_journal (kind, sync_id, local_id, revision, dirty, deleted, updated_at)
    VALUES ('known_hosts', NEW.sync_id, NEW.id, 1, 1, 0, CURRENT_TIMESTAMP)
    ON CONFLICT(kind, sync_id) DO UPDATE SET
        local_id = NEW.id, dirty = 1, deleted = 0, updated_at = CURRENT_TIMESTAMP;
END;
