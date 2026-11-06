-- Идентификаторы синхронизации обязаны быть UUID (#179).
--
-- Миграция журнала (20260820000001) проставила существующим строкам
-- `lower(hex(randomblob(16)))` — 32 знака без дефисов. В её комментарии прямо
-- написано: «собственный формат UUID здесь не нужен, важна лишь глобальная
-- уникальность». Предположение оказалось неверным.
--
-- Контракт сервера требует `id` в формате UUID (`packages/contract/src/sync.ts`,
-- `SyncRecord.id = Uuid()`). Запись без дефисов сервер отвергает целиком, а
-- человек видит «Запрос не соответствует контракту» и не может ничего с этим
-- сделать: идентификаторы ему не видны и создавал их не он.
--
-- Коварство в том, что база при этом смешанная. Строки, созданные приложением
-- после августа, — настоящие UUID из Rust; строки старше — из этой миграции.
-- Поэтому синхронизация не падала всегда: она падала у тех, у кого база жила
-- до 20 августа. Тесты этого не видели никогда — они начинают с пустой базы,
-- где нечего переименовывать.
--
-- Значения не перегенерируем, а переписываем в тот же UUID: связи между
-- таблицами и записи журнала держатся на этих строках, и новый идентификатор
-- означал бы для сервера другую запись.

-- ── Порядок здесь имеет значение, и это не стиль ───────────────────────
--
-- Журнал правим ПЕРВЫМ. На таблицах висят триггеры: любая правка строки
-- добавляет в журнал запись с её `sync_id`. Если сначала переименовать
-- таблицы, триггер создаст в журнале строку с новым идентификатором — а
-- следом наша правка журнала попытается переименовать старую строку в тот же
-- и упрётся в UNIQUE (kind, sync_id). Миграция оборвётся на середине, и база
-- останется наполовину переименованной.
--
-- В обратном порядке всё сходится: журнал уже под новым идентификатором,
-- триггер попадает в существующую строку и просто отмечает её как изменённую
-- — что и требуется, эти записи сервер до сих пор не принял.

UPDATE sync_journal  SET sync_id = substr(sync_id,1,8)||'-'||substr(sync_id,9,4)||'-'||substr(sync_id,13,4)||'-'||substr(sync_id,17,4)||'-'||substr(sync_id,21,12) WHERE length(sync_id) = 32 AND sync_id NOT LIKE '%-%';

-- Теперь сами таблицы.
UPDATE groups        SET sync_id = substr(sync_id,1,8)||'-'||substr(sync_id,9,4)||'-'||substr(sync_id,13,4)||'-'||substr(sync_id,17,4)||'-'||substr(sync_id,21,12) WHERE length(sync_id) = 32 AND sync_id NOT LIKE '%-%';
UPDATE hosts         SET sync_id = substr(sync_id,1,8)||'-'||substr(sync_id,9,4)||'-'||substr(sync_id,13,4)||'-'||substr(sync_id,17,4)||'-'||substr(sync_id,21,12) WHERE length(sync_id) = 32 AND sync_id NOT LIKE '%-%';
UPDATE identities    SET sync_id = substr(sync_id,1,8)||'-'||substr(sync_id,9,4)||'-'||substr(sync_id,13,4)||'-'||substr(sync_id,17,4)||'-'||substr(sync_id,21,12) WHERE length(sync_id) = 32 AND sync_id NOT LIKE '%-%';
UPDATE snippets      SET sync_id = substr(sync_id,1,8)||'-'||substr(sync_id,9,4)||'-'||substr(sync_id,13,4)||'-'||substr(sync_id,17,4)||'-'||substr(sync_id,21,12) WHERE length(sync_id) = 32 AND sync_id NOT LIKE '%-%';
UPDATE port_forwards SET sync_id = substr(sync_id,1,8)||'-'||substr(sync_id,9,4)||'-'||substr(sync_id,13,4)||'-'||substr(sync_id,17,4)||'-'||substr(sync_id,21,12) WHERE length(sync_id) = 32 AND sync_id NOT LIKE '%-%';
UPDATE known_hosts   SET sync_id = substr(sync_id,1,8)||'-'||substr(sync_id,9,4)||'-'||substr(sync_id,13,4)||'-'||substr(sync_id,17,4)||'-'||substr(sync_id,21,12) WHERE length(sync_id) = 32 AND sync_id NOT LIKE '%-%';
