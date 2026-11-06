//! Клиентская часть синхронизации (#181).
//!
//! Запись превращается в зашифрованный блоб перед отправкой и обратно при
//! приёме. Сервер содержимого не видит и видеть не должен — см.
//! `docs/06-web/04-sync-protocol.md`.
//!
//! Главная тонкость здесь — **ссылки между записями**. Локально хост
//! ссылается на группу числовым ключом, но на другом устройстве у той же
//! группы будет другой номер: автоинкремент у каждой машины свой. Поэтому
//! внутри блоба ссылки хранятся по `sync_id`, а при приёме переводятся
//! обратно в локальные номера.

pub mod api;
pub mod engine;
pub mod keys;
pub mod schedule;
pub mod session;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// Запись в том виде, в каком она уезжает на сервер.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecordEnvelope {
    pub kind: String,
    pub sync_id: String,
    pub revision: i64,
    pub deleted: bool,
}

/// Перевести локальный номер записи в её `sync_id`.
///
/// `Ok(None)` — записи нет. Это не ошибка: на неё могли ссылаться, а её
/// успели удалить.
pub async fn local_to_sync(db: &SqlitePool, table: &str, local_id: i64) -> Result<Option<String>> {
    // Имя таблицы подставляется форматированием, а не параметром: SQLite не
    // умеет параметризовать имена. Значения приходят из нашего же списка
    // ниже, а не снаружи, так что подстановки извне здесь быть не может.
    let table = validated_table(table)?;
    let sql = format!("SELECT sync_id FROM {table} WHERE id = ?1");
    Ok(sqlx::query_scalar(&sql)
        .bind(local_id)
        .fetch_optional(db)
        .await?
        .flatten())
}

/// Обратный перевод: `sync_id` в локальный номер.
pub async fn sync_to_local(db: &SqlitePool, table: &str, sync_id: &str) -> Result<Option<i64>> {
    let table = validated_table(table)?;
    let sql = format!("SELECT id FROM {table} WHERE sync_id = ?1");
    Ok(sqlx::query_scalar(&sql)
        .bind(sync_id)
        .fetch_optional(db)
        .await?)
}

/// Таблицы, участвующие в синхронизации.
///
/// Список закрытый именно потому, что имя таблицы подставляется в запрос
/// текстом: так проверка становится единственным входом, и добавить туда
/// произвольную строку неоткуда.
pub(crate) const SYNCABLE: [&str; 6] = [
    "groups",
    "hosts",
    "identities",
    "snippets",
    "port_forwards",
    "known_hosts",
];

fn validated_table(table: &str) -> Result<&'static str> {
    SYNCABLE
        .iter()
        .find(|t| **t == table)
        .copied()
        .with_context(|| format!("таблица '{table}' не синхронизируется"))
}

/// Зашифровать полезную нагрузку записи.
pub fn seal(key: &[u8; 32], payload: &serde_json::Value) -> Result<(Vec<u8>, Vec<u8>)> {
    let text = serde_json::to_vec(payload)?;
    crate::crypto::encrypt(key, &text)
}

/// Расшифровать полезную нагрузку.
pub fn open(key: &[u8; 32], nonce: &[u8], ciphertext: &[u8]) -> Result<serde_json::Value> {
    let text = crate::crypto::decrypt(key, nonce, ciphertext)?;
    // Отдельное сообщение: «расшифровалось, но внутри не то» и «не
    // расшифровалось» — разные беды, и путать их при разборе дорого.
    serde_json::from_slice(&text).context("расшифрованная запись не разбирается как JSON")
}

/// Ключи записей: тот, которым пишем, и тот, которым ещё умеем читать.
///
/// Второй появился из-за исправления 2026-09-22 (#180c). До него приложение
/// шифровало записи ключом, выведенным **из пароля**, хотя по спецификации
/// (`docs/06-web/02-auth-and-crypto.md`) шифровать полагается случайным
/// `symmetricKey`, который лежит на сервере под обёрткой. Разница видна не
/// сразу, но она решающая: смена пароля меняет выведенный ключ, и все
/// записи, отправленные до смены, перестали бы читаться. По той же причине
/// невозможно было и восстановление по бумаге — там обёртку открывают ключом
/// с бумаги, а под обёрткой оказывался бы ключ, которым никто ничего не
/// шифровал.
///
/// Переход сделан без потерь: читаем обоими ключами, пишем только новым.
/// Записи, которых человек больше не касается, так и останутся под старым
/// ключом и будут читаться — а тронутые уедут под новый сами.
#[derive(Clone)]
pub struct RecordKeys {
    /// Которым шифруем всё новое.
    pub current: [u8; 32],
    /// Которым ещё читаем отправленное до перехода. `None` — нечего читать.
    pub legacy: Option<[u8; 32]>,
}

impl RecordKeys {
    /// Один ключ и никакого прошлого — для тестов и для аккаунтов, заведённых
    /// после перехода.
    pub fn only(current: [u8; 32]) -> Self {
        Self {
            current,
            legacy: None,
        }
    }

    /// Расшифровать тем ключом, каким получится.
    ///
    /// Порядок важен: сперва текущий. Иначе каждая свежая запись сначала
    /// безуспешно проверялась бы старым ключом — лишняя работа на каждой
    /// записи ради тех, которых со временем не останется.
    pub fn open(&self, nonce: &[u8], ciphertext: &[u8]) -> Result<serde_json::Value> {
        match open(&self.current, nonce, ciphertext) {
            Ok(payload) => Ok(payload),
            Err(e) => match self.legacy {
                Some(legacy) => open(&legacy, nonce, ciphertext).map_err(|_| e),
                None => Err(e),
            },
        }
    }
}

/// Выбрать записи, которые нужно отправить.
pub async fn pending(db: &SqlitePool, limit: i64) -> Result<Vec<RecordEnvelope>> {
    let rows = sqlx::query_as::<_, (String, String, i64, i64)>(
        "SELECT kind, sync_id, revision, deleted FROM sync_journal \
         WHERE dirty = 1 ORDER BY updated_at LIMIT ?1",
    )
    .bind(limit)
    .fetch_all(db)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(kind, sync_id, revision, deleted)| RecordEnvelope {
            kind,
            sync_id,
            revision,
            deleted: deleted != 0,
        })
        .collect())
}

/// Отметить записи отправленными и запомнить версию, которую принял сервер.
///
/// Версия берётся **от сервера**, а не увеличивается локально: расхождение
/// версий — это и есть признак конфликта, и вести свой счёт значило бы
/// объявлять конфликт там, где его нет.
pub async fn mark_synced(db: &SqlitePool, kind: &str, sync_id: &str, revision: i64) -> Result<()> {
    sqlx::query(
        "UPDATE sync_journal SET dirty = 0, revision = ?3 \
         WHERE kind = ?1 AND sync_id = ?2",
    )
    .bind(kind)
    .bind(sync_id)
    .bind(revision)
    .execute(db)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_tables_are_allowed_in_queries() {
        // Имя таблицы попадает в текст запроса, поэтому проверка обязана быть
        // единственным входом. Список закрытый — добавить произвольное имя
        // неоткуда.
        assert!(validated_table("hosts").is_ok());
        assert!(validated_table("settings").is_err(), "не синхронизируется");
        assert!(validated_table("hosts; DROP TABLE hosts").is_err());
        assert!(validated_table("").is_err());
    }

    #[test]
    fn payload_survives_sealing_and_opening() {
        let key = [9u8; 32];
        let payload = serde_json::json!({
            "name": "web-1.prod",
            "hostname": "10.0.0.1",
            // Ссылка на группу — по sync_id, а не по локальному номеру
            "groupSyncId": "11111111-1111-4111-8111-111111111111",
            "port": 22,
        });

        let (nonce, ct) = seal(&key, &payload).expect("шифрование");
        assert_eq!(open(&key, &nonce, &ct).expect("расшифровка"), payload);
    }

    #[test]
    fn payload_with_non_ascii_survives() {
        // Имена хостов и описания бывают на любом языке; урок из веб-части:
        // ровные данные прячут ровно то, что ломается на настоящих.
        let key = [10u8; 32];
        let payload = serde_json::json!({
            "name": "сервер «Прод» — основной",
            "description": "Ünlü sunucu, 日本語, emoji 🚀",
        });
        let (nonce, ct) = seal(&key, &payload).expect("шифрование");
        assert_eq!(open(&key, &nonce, &ct).expect("расшифровка"), payload);
    }

    #[test]
    fn foreign_key_cannot_open_the_payload() {
        let payload = serde_json::json!({"secret": "пароль"});
        let (nonce, ct) = seal(&[1u8; 32], &payload).expect("шифрование");
        assert!(open(&[2u8; 32], &nonce, &ct).is_err());
    }

    /// Переход на ключ из обёртки не теряет отправленного раньше (#180c).
    ///
    /// Ради этого свойства весь переход и устроен так: записи, отправленные
    /// старым ключом, продолжают читаться, а новые пишутся новым. Иначе
    /// исправление означало бы «синхронизация у всех начинается с нуля».
    #[test]
    fn records_sealed_with_the_old_key_are_still_readable() {
        let old = [1u8; 32];
        let new = [2u8; 32];
        let payload = serde_json::json!({ "name": "prod-01" });
        let (nonce, ct) = seal(&old, &payload).expect("шифрование старым");

        let keys = RecordKeys {
            current: new,
            legacy: Some(old),
        };
        assert_eq!(keys.open(&nonce, &ct).expect("чтение старого"), payload);

        // И новое читается тоже — иначе переход сломал бы свежие записи.
        let (nonce, ct) = seal(&new, &payload).expect("шифрование новым");
        assert_eq!(keys.open(&nonce, &ct).expect("чтение нового"), payload);
    }

    /// Без прежнего ключа чужая запись остаётся ошибкой, а не пустотой.
    #[test]
    fn a_record_from_another_key_still_fails() {
        let payload = serde_json::json!({ "name": "x" });
        let (nonce, ct) = seal(&[3u8; 32], &payload).expect("шифрование");

        assert!(RecordKeys::only([4u8; 32]).open(&nonce, &ct).is_err());
        // И с прежним ключом, который тоже не подходит.
        let keys = RecordKeys {
            current: [4u8; 32],
            legacy: Some([5u8; 32]),
        };
        assert!(keys.open(&nonce, &ct).is_err());
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Отправка изменений (#192)
// ─────────────────────────────────────────────────────────────────────────

/// Что сервер ответил по одной записи.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PushOutcome {
    pub sync_id: String,
    /// Версия, которую присвоил сервер.
    pub revision: i64,
}

/// Конфликт: сервер не принял изменение, потому что у него другая версия.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub sync_id: String,
    pub kind: String,
    pub server_revision: i64,
    /// Блоб серверной версии — её и надо взять за основу.
    pub blob: Vec<u8>,
    pub nonce: Vec<u8>,
}

/// Курсор синхронизации: что мы уже получили с сервера.
///
/// Хранится как есть, **ровно то значение, что прислал сервер**. Вычислять
/// его самостоятельно нельзя: при постраничной выдаче курсор указывает на
/// последнюю отданную запись, а не на текущий счётчик сервера, и свой счёт
/// заставил бы клиента перескочить через неполученное.
pub async fn cursor(db: &SqlitePool) -> Result<i64> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM sync_state WHERE key = 'cursor'")
            .fetch_optional(db)
            .await?;
    Ok(value.and_then(|v| v.parse().ok()).unwrap_or(0))
}

pub async fn set_cursor(db: &SqlitePool, value: i64) -> Result<()> {
    sqlx::query(
        "INSERT INTO sync_state (key, value) VALUES ('cursor', ?1) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(value.to_string())
    .execute(db)
    .await?;
    Ok(())
}

/// Разрешить конфликт: серверная версия становится основной, наша
/// сохраняется рядом отдельной записью.
///
/// Молча выбросить свою правку нельзя. Сервер не видит содержимого и слить
/// изменения не может, а исчезнувший без следа пароль от боевого сервера
/// человек обнаружит в худший момент. Лишняя запись раздражает, но она
/// **видима**.
pub async fn keep_losing_version(
    db: &SqlitePool,
    kind: &str,
    losing_sync_id: &str,
) -> Result<String> {
    let table = validated_table(kind)?;
    let new_id = crate::db::new_sync_id();

    // Копия делается средствами SQLite: перечислять колонки в коде значило бы
    // ломать копирование каждый раз, когда в таблицу добавят поле.
    let sql = format!("UPDATE {table} SET sync_id = ?1 WHERE sync_id = ?2");
    sqlx::query(&sql)
        .bind(&new_id)
        .bind(losing_sync_id)
        .execute(db)
        .await?;

    // Запись в журнале под новым идентификатором уже завёл триггер: смена
    // sync_id — это UPDATE, а он отмечается как всякое изменение.
    //
    // Остаётся убрать старую строку журнала. Переименовывать её нельзя —
    // строка с новым идентификатором уже есть, — а оставлять нельзя тем
    // более: она указывала бы на запись, которой под этим именем больше нет,
    // и отправка ушла бы под чужим идентификатором. Освобождённое имя займёт
    // серверная версия, которую мы вот-вот применим.
    sqlx::query("DELETE FROM sync_journal WHERE kind = ?1 AND sync_id = ?2")
        .bind(kind)
        .bind(losing_sync_id)
        .execute(db)
        .await?;

    Ok(new_id)
}

// ─────────────────────────────────────────────────────────────────────────
// Приём изменений (#193)
// ─────────────────────────────────────────────────────────────────────────

/// Что произошло при применении надгробия.
#[derive(Debug, Clone, PartialEq)]
pub enum TombstoneOutcome {
    /// Запись удалена.
    Applied,
    /// У нас была неотправленная правка — она сохранена под новым
    /// идентификатором, а удаление применено.
    LocalEditPreserved(String),
    /// Записи и не было: удалять нечего.
    Unknown,
}

/// Снять отметку «нужно отправить» после применения чужого изменения.
///
/// Обязательный шаг. Применение чужой записи — это локальный INSERT, UPDATE
/// или DELETE, а на них стоят триггеры, которые честно помечают запись к
/// отправке. Без снятия отметки клиент отправил бы серверу его же изменение
/// обратно, тот принял бы его новой версией, разослал остальным — и обмен
/// пошёл бы по кругу, не сходясь никогда.
async fn mark_applied_from_server(
    db: &SqlitePool,
    kind: &str,
    sync_id: &str,
    revision: i64,
) -> Result<()> {
    sqlx::query(
        "UPDATE sync_journal SET dirty = 0, revision = ?3 WHERE kind = ?1 AND sync_id = ?2",
    )
    .bind(kind)
    .bind(sync_id)
    .bind(revision)
    .execute(db)
    .await?;
    Ok(())
}

/// Есть ли у записи неотправленная местная правка.
///
/// Нужна перед применением пришедшего с сервера: затереть свою правку молча
/// нельзя. Для удалений такая проверка уже была (`apply_tombstone`), для
/// обычного применения её не было — и правка пропадала бесследно.
pub async fn is_dirty(db: &SqlitePool, kind: &str, sync_id: &str) -> Result<bool> {
    let dirty: Option<i64> =
        sqlx::query_scalar("SELECT dirty FROM sync_journal WHERE kind = ?1 AND sync_id = ?2")
            .bind(kind)
            .bind(sync_id)
            .fetch_optional(db)
            .await?;
    Ok(dirty.unwrap_or(0) != 0)
}

/// Применить удаление, пришедшее с сервера.
///
/// Если у нас есть **неотправленная** правка этой записи, она сохраняется
/// отдельной записью, а не пропадает. Удаление при этом всё равно
/// применяется: спорить с ним бессмысленно, но и работу человека терять
/// нельзя. Воскресшая под новым именем запись видима и удаляется повторно;
/// исчезнувшая правка не заметна, пока не понадобится.
pub async fn apply_tombstone(
    db: &SqlitePool,
    kind: &str,
    sync_id: &str,
    revision: i64,
) -> Result<TombstoneOutcome> {
    let table = validated_table(kind)?;

    let existing: Option<(i64, i64)> =
        sqlx::query_as("SELECT dirty, deleted FROM sync_journal WHERE kind = ?1 AND sync_id = ?2")
            .bind(kind)
            .bind(sync_id)
            .fetch_optional(db)
            .await?;

    let Some((dirty, deleted)) = existing else {
        return Ok(TombstoneOutcome::Unknown);
    };

    // Уже удалена у нас — просто соглашаемся с версией сервера.
    if deleted != 0 {
        mark_applied_from_server(db, kind, sync_id, revision).await?;
        return Ok(TombstoneOutcome::Applied);
    }

    if dirty != 0 {
        // Наша правка ещё не уехала. Переносим её под новый идентификатор:
        // после этого записи с прежним именем в таблице нет, и удалять уже
        // нечего — надгробие применено самим переносом.
        let preserved = keep_losing_version(db, kind, sync_id).await?;
        return Ok(TombstoneOutcome::LocalEditPreserved(preserved));
    }

    let sql = format!("DELETE FROM {table} WHERE sync_id = ?1");
    sqlx::query(&sql).bind(sync_id).execute(db).await?;

    // Триггер удаления пометил надгробие к отправке — снимаем: удаление
    // пришло от сервера, отправлять его обратно незачем.
    mark_applied_from_server(db, kind, sync_id, revision).await?;
    Ok(TombstoneOutcome::Applied)
}

// ─────────────────────────────────────────────────────────────────────────
// Материализация записей по типам (#193a)
// ─────────────────────────────────────────────────────────────────────────

/// Перевести ссылку из блоба в локальный номер.
///
/// `Ok(None)` и для «ссылки не было», и для «запись ещё не приехала».
/// Второе — обычное дело: порядок записей в выгрузке не гарантирован, и хост
/// может прийти раньше своей группы. Ссылка проставится при следующей
/// синхронизации, а пока запись просто без группы — это лучше, чем отказаться
/// её принимать.
async fn resolve_ref(
    db: &SqlitePool,
    table: &str,
    value: Option<&serde_json::Value>,
) -> Result<Option<i64>> {
    let Some(sync_id) = value.and_then(|v| v.as_str()) else {
        return Ok(None);
    };
    sync_to_local(db, table, sync_id).await
}

fn text(payload: &serde_json::Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn number(payload: &serde_json::Value, key: &str, default: i64) -> i64 {
    payload.get(key).and_then(|v| v.as_i64()).unwrap_or(default)
}

fn flag(payload: &serde_json::Value, key: &str) -> bool {
    payload.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

/// Применить пришедшую запись: расшифровать, разрешить ссылки, записать.
///
/// Вставка и обновление — одним запросом через `ON CONFLICT(sync_id)`:
/// проверять «есть ли уже такая» отдельным запросом значило бы оставить щель
/// между проверкой и записью, в которую поместится вторая синхронизация.
pub async fn apply_record(
    db: &SqlitePool,
    keys: &RecordKeys,
    kind: &str,
    sync_id: &str,
    revision: i64,
    nonce: &[u8],
    blob: &[u8],
) -> Result<()> {
    let table = validated_table(kind)?;
    let payload = keys.open(nonce, blob)?;

    match table {
        "groups" => {
            let parent = resolve_ref(db, "groups", payload.get("parentSyncId")).await?;
            sqlx::query(
                "INSERT INTO groups (name, parent_id, sort_order, sync_id) \
                 VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT(sync_id) DO UPDATE SET \
                    name = excluded.name, parent_id = excluded.parent_id, \
                    sort_order = excluded.sort_order, updated_at = CURRENT_TIMESTAMP",
            )
            .bind(text(&payload, "name").unwrap_or_default())
            .bind(parent)
            .bind(number(&payload, "sortOrder", 0))
            .bind(sync_id)
            .execute(db)
            .await?;
        }
        "hosts" => {
            let group = resolve_ref(db, "groups", payload.get("groupSyncId")).await?;
            let identity = resolve_ref(db, "identities", payload.get("identitySyncId")).await?;
            let jump = resolve_ref(db, "hosts", payload.get("jumpHostSyncId")).await?;
            sqlx::query(
                "INSERT INTO hosts (group_id, name, hostname, port, username, identity_id, \
                    auth_method, color, icon, jump_host_id, forward_agent, \
                    keep_alive_interval, description, sync_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14) \
                 ON CONFLICT(sync_id) DO UPDATE SET \
                    group_id = excluded.group_id, name = excluded.name, \
                    hostname = excluded.hostname, port = excluded.port, \
                    username = excluded.username, identity_id = excluded.identity_id, \
                    auth_method = excluded.auth_method, color = excluded.color, \
                    icon = excluded.icon, jump_host_id = excluded.jump_host_id, \
                    forward_agent = excluded.forward_agent, \
                    keep_alive_interval = excluded.keep_alive_interval, \
                    description = excluded.description, updated_at = CURRENT_TIMESTAMP",
            )
            .bind(group)
            .bind(text(&payload, "name").unwrap_or_default())
            .bind(text(&payload, "hostname").unwrap_or_default())
            .bind(number(&payload, "port", 22))
            .bind(text(&payload, "username").unwrap_or_default())
            .bind(identity)
            .bind(text(&payload, "authMethod").unwrap_or_else(|| "password".into()))
            .bind(text(&payload, "color"))
            .bind(text(&payload, "icon"))
            .bind(jump)
            .bind(flag(&payload, "forwardAgent"))
            .bind(number(&payload, "keepAliveInterval", 30))
            .bind(text(&payload, "description"))
            .bind(sync_id)
            .execute(db)
            .await?;
        }
        "identities" => {
            sqlx::query(
                "INSERT INTO identities (label, kind, key_path, keyring_ref, has_passphrase, \
                    public_key, sync_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
                 ON CONFLICT(sync_id) DO UPDATE SET \
                    label = excluded.label, kind = excluded.kind, \
                    key_path = excluded.key_path, has_passphrase = excluded.has_passphrase, \
                    public_key = excluded.public_key, updated_at = CURRENT_TIMESTAMP",
            )
            .bind(text(&payload, "label").unwrap_or_default())
            .bind(text(&payload, "kind").unwrap_or_else(|| "password".into()))
            .bind(text(&payload, "keyPath"))
            .bind(text(&payload, "keyringRef"))
            .bind(flag(&payload, "hasPassphrase"))
            .bind(text(&payload, "publicKey"))
            .bind(sync_id)
            .execute(db)
            .await?;
        }
        "snippets" => {
            sqlx::query(
                "INSERT INTO snippets (name, command, description, sort_order, sync_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5) \
                 ON CONFLICT(sync_id) DO UPDATE SET \
                    name = excluded.name, command = excluded.command, \
                    description = excluded.description, sort_order = excluded.sort_order, \
                    updated_at = CURRENT_TIMESTAMP",
            )
            .bind(text(&payload, "name").unwrap_or_default())
            .bind(text(&payload, "command").unwrap_or_default())
            .bind(text(&payload, "description"))
            .bind(number(&payload, "sortOrder", 0))
            .bind(sync_id)
            .execute(db)
            .await?;
        }
        "port_forwards" => {
            let host = resolve_ref(db, "hosts", payload.get("hostSyncId")).await?;
            // Правило проброса без хоста бессмысленно: применим, когда хост
            // приедет. Пропуск здесь честнее, чем запись, ведущая в никуда.
            let Some(host_id) = host else {
                return Ok(());
            };
            sqlx::query(
                "INSERT INTO port_forwards (host_id, kind, label, local_host, local_port, \
                    remote_host, remote_port, auto_start, sync_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
                 ON CONFLICT(sync_id) DO UPDATE SET \
                    host_id = excluded.host_id, kind = excluded.kind, label = excluded.label, \
                    local_host = excluded.local_host, local_port = excluded.local_port, \
                    remote_host = excluded.remote_host, remote_port = excluded.remote_port, \
                    auto_start = excluded.auto_start",
            )
            .bind(host_id)
            .bind(text(&payload, "kind").unwrap_or_else(|| "local".into()))
            .bind(text(&payload, "label"))
            .bind(text(&payload, "localHost").unwrap_or_else(|| "127.0.0.1".into()))
            .bind(number(&payload, "localPort", 0))
            .bind(text(&payload, "remoteHost"))
            .bind(payload.get("remotePort").and_then(|v| v.as_i64()))
            .bind(flag(&payload, "autoStart"))
            .bind(sync_id)
            .execute(db)
            .await?;
        }
        "known_hosts" => {
            sqlx::query(
                "INSERT INTO known_hosts (hostname, key_type, fingerprint, sync_id) \
                 VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT(sync_id) DO UPDATE SET fingerprint = excluded.fingerprint",
            )
            .bind(text(&payload, "hostname").unwrap_or_default())
            .bind(text(&payload, "keyType").unwrap_or_default())
            .bind(text(&payload, "fingerprint").unwrap_or_default())
            .bind(sync_id)
            .execute(db)
            .await?;
        }
        other => anyhow::bail!("нечем применить запись типа '{other}'"),
    }

    mark_applied_from_server(db, kind, sync_id, revision).await
}

// ─────────────────────────────────────────────────────────────────────────
// Первый запуск на устройстве с непустой базой (#194)
// ─────────────────────────────────────────────────────────────────────────

/// Что делать с уже накопленными локально записями при подключении аккаунта.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FirstSyncChoice {
    /// Отправить локальное на сервер и принять серверное — получится
    /// объединение. Одинаковые имена дадут две записи, и это честнее, чем
    /// пытаться угадать, какая из них «та же самая».
    Merge,
    /// Отдать предпочтение серверу: локальное удаляется целиком.
    ReplaceLocal,
    /// Отдать предпочтение устройству: серверное игнорируется, локальное
    /// уезжает как есть.
    KeepLocal,
}

/// Сколько записей уже есть локально.
///
/// Нужно ровно для одного: понять, надо ли вообще спрашивать. На чистом
/// устройстве вопрос неуместен и только пугает.
pub async fn local_record_count(db: &SqlitePool) -> Result<i64> {
    let mut total = 0i64;
    for table in SYNCABLE {
        let sql = format!("SELECT COUNT(*) FROM {table}");
        total += sqlx::query_scalar::<_, i64>(&sql).fetch_one(db).await?;
    }
    Ok(total)
}

/// Спрашивать ли пользователя перед первой синхронизацией.
///
/// Спрашиваем, только если синхронизации ещё не было **и** локально что-то
/// есть. Молча слить нельзя: у человека могут оказаться одинаковые имена с
/// разными адресами, и объединение даст мусор, в котором он не разберётся.
pub fn should_ask_before_first_sync(cursor: i64, local_records: i64) -> bool {
    cursor == 0 && local_records > 0
}

/// Применить выбор пользователя. Возвращает число затронутых записей.
pub async fn apply_first_sync_choice(db: &SqlitePool, choice: FirstSyncChoice) -> Result<i64> {
    match choice {
        // Ничего не делаем: локальные записи уже помечены к отправке
        // триггерами, серверные приедут обычным путём.
        FirstSyncChoice::Merge => local_record_count(db).await,
        FirstSyncChoice::ReplaceLocal => {
            let mut removed = 0i64;
            for table in SYNCABLE {
                let sql = format!("DELETE FROM {table}");
                removed += sqlx::query(&sql).execute(db).await?.rows_affected() as i64;
            }
            // Журнал тоже чистим: иначе надгробия от этой очистки уедут на
            // сервер и удалят там ровно то, что мы собирались принять.
            sqlx::query("DELETE FROM sync_journal").execute(db).await?;
            Ok(removed)
        }
        FirstSyncChoice::KeepLocal => {
            // Локальное остаётся и уезжает; серверное игнорируем, сдвинув
            // курсор вперёд при первой же выгрузке. Отдельного действия
            // здесь не нужно — достаточно не применять пришедшее.
            local_record_count(db).await
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Работа без сети (#195)
// ─────────────────────────────────────────────────────────────────────────

/// Состояние последней попытки синхронизации.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncHealth {
    /// Последняя ошибка, если была. Показывается ненавязчиво.
    pub last_error: Option<String>,
    /// Когда последний проход удался, Unix-секунды.
    pub last_sync_at: Option<i64>,
    /// Сколько записей ждёт отправки.
    pub pending: i64,
}

/// Запомнить неудачу синхронизации.
///
/// Ошибка сохраняется, но **никуда не всплывает сама**: невозможность
/// достучаться до сервера не должна мешать подключаться к своим машинам.
/// Приложение работало без аккаунта до появления сервера и обязано работать
/// так же после.
pub async fn record_sync_failure(db: &SqlitePool, error: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO sync_state (key, value) VALUES ('last_error', ?1) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(error)
    .execute(db)
    .await?;
    Ok(())
}

/// Удачный проход: убрать прежнюю ошибку и запомнить время.
pub async fn record_sync_success(db: &SqlitePool) -> Result<()> {
    sqlx::query("DELETE FROM sync_state WHERE key = 'last_error'")
        .execute(db)
        .await?;
    sqlx::query(
        "INSERT INTO sync_state (key, value) VALUES ('last_sync_at', strftime('%s','now')) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn health(db: &SqlitePool) -> Result<SyncHealth> {
    let last_error: Option<String> =
        sqlx::query_scalar("SELECT value FROM sync_state WHERE key = 'last_error'")
            .fetch_optional(db)
            .await?
            .flatten();
    let last_sync_at: Option<i64> = sqlx::query_scalar::<_, Option<String>>(
        "SELECT value FROM sync_state WHERE key = 'last_sync_at'",
    )
    .fetch_optional(db)
    .await?
    .flatten()
    .and_then(|v| v.parse().ok());
    let pending: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_journal WHERE dirty = 1")
        .fetch_one(db)
        .await?;
    Ok(SyncHealth {
        last_error,
        last_sync_at,
        pending,
    })
}

// ─────────────────────────────────────────────────────────────────────────
// Сборка записи для отправки
// ─────────────────────────────────────────────────────────────────────────

/// Собрать полезную нагрузку записи для отправки на сервер.
///
/// Зеркало `apply_record`. **Имена полей обязаны совпадать до буквы**: разойдись
/// они, запись уедет на сервер, вернётся на второе устройство и молча потеряет
/// половину значений — а выглядеть это будет как «синхронизация работает».
/// Поэтому обе стороны стоят рядом, и на каждую пару есть тест.
///
/// Ссылки на другие записи уходят **по `sync_id`, а не по локальному номеру**:
/// локальные номера на разных устройствах разные, и хост уехал бы в чужую
/// группу.
///
/// `None` означает, что записи уже нет локально — обычное дело для удалённой:
/// её надгробие отправляется без содержимого.
pub async fn record_payload(
    db: &SqlitePool,
    kind: &str,
    sync_id: &str,
) -> Result<Option<serde_json::Value>> {
    let table = validated_table(kind)?;

    let value = match table {
        "groups" => {
            let row: Option<(String, Option<i64>, i64)> =
                sqlx::query_as("SELECT name, parent_id, sort_order FROM groups WHERE sync_id = ?1")
                    .bind(sync_id)
                    .fetch_optional(db)
                    .await?;
            match row {
                None => None,
                Some((name, parent_id, sort_order)) => Some(serde_json::json!({
                    "name": name,
                    "parentSyncId": opt_ref(db, "groups", parent_id).await?,
                    "sortOrder": sort_order,
                })),
            }
        }
        "hosts" => {
            let row: Option<HostPayloadRow> = sqlx::query_as(
                "SELECT group_id, name, hostname, port, username, identity_id, auth_method, \
                    color, icon, jump_host_id, forward_agent, keep_alive_interval, description \
                 FROM hosts WHERE sync_id = ?1",
            )
            .bind(sync_id)
            .fetch_optional(db)
            .await?;
            match row {
                None => None,
                Some(r) => Some(serde_json::json!({
                    "groupSyncId": opt_ref(db, "groups", r.0).await?,
                    "name": r.1,
                    "hostname": r.2,
                    "port": r.3,
                    "username": r.4,
                    "identitySyncId": opt_ref(db, "identities", r.5).await?,
                    "authMethod": r.6,
                    "color": r.7,
                    "icon": r.8,
                    "jumpHostSyncId": opt_ref(db, "hosts", r.9).await?,
                    "forwardAgent": r.10 != 0,
                    "keepAliveInterval": r.11,
                    "description": r.12,
                })),
            }
        }
        "identities" => {
            let row: Option<IdentityPayloadRow> = sqlx::query_as(
                "SELECT label, kind, key_path, keyring_ref, has_passphrase, public_key \
                     FROM identities WHERE sync_id = ?1",
            )
            .bind(sync_id)
            .fetch_optional(db)
            .await?;
            row.map(
                |(label, kind, key_path, keyring_ref, has_passphrase, public_key)| {
                    serde_json::json!({
                        "label": label,
                        "kind": kind,
                        "keyPath": key_path,
                        // Указатель на секрет в keyring, а НЕ сам секрет: приватные
                        // ключи и пароли на сервер не уезжают вообще, даже
                        // зашифрованными. Хранилище секретов остаётся местным.
                        "keyringRef": keyring_ref,
                        "hasPassphrase": has_passphrase != 0,
                        "publicKey": public_key,
                    })
                },
            )
        }
        "snippets" => {
            let row: Option<(String, String, Option<String>, i64)> = sqlx::query_as(
                "SELECT name, command, description, sort_order FROM snippets WHERE sync_id = ?1",
            )
            .bind(sync_id)
            .fetch_optional(db)
            .await?;
            row.map(|(name, command, description, sort_order)| {
                serde_json::json!({
                    "name": name,
                    "command": command,
                    "description": description,
                    "sortOrder": sort_order,
                })
            })
        }
        "port_forwards" => {
            let row: Option<PortForwardPayloadRow> = sqlx::query_as(
                "SELECT host_id, kind, label, local_host, local_port, remote_host, \
                    remote_port, auto_start FROM port_forwards WHERE sync_id = ?1",
            )
            .bind(sync_id)
            .fetch_optional(db)
            .await?;
            match row {
                None => None,
                Some(r) => Some(serde_json::json!({
                    "hostSyncId": opt_ref(db, "hosts", Some(r.0)).await?,
                    "kind": r.1,
                    "label": r.2,
                    "localHost": r.3,
                    "localPort": r.4,
                    "remoteHost": r.5,
                    "remotePort": r.6,
                    "autoStart": r.7 != 0,
                })),
            }
        }
        "known_hosts" => {
            let row: Option<(String, String, String)> = sqlx::query_as(
                "SELECT hostname, key_type, fingerprint FROM known_hosts WHERE sync_id = ?1",
            )
            .bind(sync_id)
            .fetch_optional(db)
            .await?;
            row.map(|(hostname, key_type, fingerprint)| {
                serde_json::json!({
                    "hostname": hostname,
                    "keyType": key_type,
                    "fingerprint": fingerprint,
                })
            })
        }
        other => anyhow::bail!("нечем собрать запись типа '{other}'"),
    };

    Ok(value)
}

/// Строка выборки хоста для отправки.
type HostPayloadRow = (
    Option<i64>,
    String,
    String,
    i64,
    String,
    Option<i64>,
    String,
    Option<String>,
    Option<String>,
    Option<i64>,
    i64,
    i64,
    Option<String>,
);

/// Строка выборки учётки для отправки.
type IdentityPayloadRow = (
    String,
    String,
    Option<String>,
    Option<String>,
    i64,
    Option<String>,
);

/// Строка выборки правила проброса для отправки.
type PortForwardPayloadRow = (
    i64,
    String,
    Option<String>,
    String,
    i64,
    Option<String>,
    Option<i64>,
    i64,
);

/// Перевести локальный номер в `sync_id` для отправки.
///
/// Отсутствие цели — не ошибка: на неё могли сослаться и тут же её удалить.
async fn opt_ref(db: &SqlitePool, table: &str, local_id: Option<i64>) -> Result<Option<String>> {
    match local_id {
        None => Ok(None),
        Some(id) => local_to_sync(db, table, id).await,
    }
}
