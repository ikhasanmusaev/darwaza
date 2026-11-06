//! Один проход синхронизации: забрать, применить, отправить.
//!
//! Порядок именно такой и он не случаен. **Сначала забираем, потом
//! отправляем**: отправив первыми, мы получили бы конфликт по каждой записи,
//! которую сосед изменил и которую мы могли бы спокойно принять до своей
//! правки. Конфликт стоит человеку решения, а лишних решений быть не должно.
//!
//! Проход **не бросает всё при первой ошибке одной записи**. Одна битая
//! запись не повод оставить остальные несинхронизированными: она попадает в
//! отчёт, а работа продолжается. Иначе один испорченный блоб замораживал бы
//! синхронизацию навсегда, и человек не понимал бы, почему ничего не
//! происходит.

use super::api::{table_kind, wire_kind, ApiClient, WireRecord, MAX_BLOB_BYTES};
use anyhow::Result;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde::Serialize;
use sqlx::SqlitePool;
use tracing::{info, warn};

/// Сколько записей отправляем за раз. Сервер принимает не более 500.
const PUSH_BATCH: i64 = 200;

/// Сколько получаем за раз.
const PULL_LIMIT: i64 = 500;

/// Что вышло за проход. Показывается человеку, поэтому счётчики раздельные:
/// «синхронизировано 12» не отвечает на вопрос, что произошло.
#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    /// Принято с сервера.
    pub received: usize,
    /// Отправлено и принято сервером.
    pub sent: usize,
    /// Конфликтов, где наша версия сохранена отдельной записью.
    pub conflicts: usize,
    /// Записи, которые не удалось обработать. Не роняют проход.
    pub failed: Vec<String>,
    /// Курсор после прохода.
    pub cursor: i64,
}

/// Выполнить один проход.
pub async fn run(
    db: &SqlitePool,
    keys: &super::RecordKeys,
    api: &ApiClient,
    access_token: &str,
) -> Result<SyncReport> {
    let mut report = SyncReport::default();

    // Перевоз — ДО pull, по состоянию на начало прохода. После pull в
    // журнале лежало бы и то, что приехало сейчас с другого устройства: чужие
    // записи перевезёт их же устройство, а лишняя отправка отсюда только
    // сдвинула бы версии и могла бы сделать конфликт там, где его не было.
    rekey_once(db, keys).await?;
    pull(db, keys, api, access_token, &mut report).await?;
    push(db, keys, api, access_token, &mut report).await?;

    report.cursor = super::cursor(db).await?;
    info!(
        target: "sync",
        "проход завершён: принято {}, отправлено {}, конфликтов {}, ошибок {}",
        report.received, report.sent, report.conflicts, report.failed.len()
    );
    Ok(report)
}

/// Ключ настройки: перевоз записей под новый ключ уже сделан.
const REKEY_DONE: &str = "sync.rekey_v2";

/// Один раз перевезти все свои записи под новый ключ (#180c).
///
/// **Почему это нельзя откладывать.** Запасной ключ — тот, что выведен из
/// пароля, — живёт ровно до смены пароля. Сменил человек пароль в кабинете, и
/// старого ключа больше нет ни у кого, включая его самого: он выводится из
/// старого пароля. Записи, оставшиеся под ним, в этот момент умирают
/// окончательно — их не вернёт ни восстановление по бумаге, ни что-либо ещё.
///
/// Поэтому «читаем обоими, пишем новым» само по себе — гонка: оно спасает
/// только того, кто не менял пароль. Перевоз превращает гонку в разовую
/// работу: все записи помечаются к отправке и уезжают уже новым ключом.
///
/// Делается один раз и только там, где есть что перевозить: без прежнего
/// ключа перевозить нечего. Цена — одна полная отправка своих записей.
pub async fn rekey_once(db: &SqlitePool, keys: &super::RecordKeys) -> Result<()> {
    if keys.legacy.is_none() {
        return Ok(());
    }
    // Устройство, которое ещё ни разу не синхронизировалось, перевозить
    // нечего: всё, что оно отправит, уйдёт уже новым ключом. Курсор здесь
    // единственный надёжный признак «я работал со старым кодом» — и смотреть
    // на него надо до pull, пока он не сдвинулся в этом же проходе.
    if super::cursor(db).await? == 0 {
        return Ok(());
    }
    let done: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = ?1")
        .bind(REKEY_DONE)
        .fetch_optional(db)
        .await?;
    if done.is_some() {
        return Ok(());
    }

    // Помечаем к отправке всё, что есть. Удалённое тоже: надгробие не
    // шифруется, но и вреда от лишней отметки нет, а разбирать исключения —
    // лишний повод ошибиться.
    let marked = sqlx::query("UPDATE sync_journal SET dirty = 1")
        .execute(db)
        .await?
        .rows_affected();

    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?1, '1') \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(REKEY_DONE)
    .execute(db)
    .await?;

    info!(
        target: "sync",
        "записи переводятся под новый ключ: помечено {marked}"
    );
    Ok(())
}

/// Забрать изменения и применить.
async fn pull(
    db: &SqlitePool,
    keys: &super::RecordKeys,
    api: &ApiClient,
    access: &str,
    report: &mut SyncReport,
) -> Result<()> {
    loop {
        let since = super::cursor(db).await?;
        let changes = api.changes(access, since, PULL_LIMIT).await?;

        for record in &changes.changes {
            // Своя неотправленная правка не должна исчезнуть под серверной
            // версией. Это тот же конфликт, только обнаруженный при получении,
            // а не при отправке, — и разрешается он так же.
            match preserve_if_dirty(db, record).await {
                Ok(true) => report.conflicts += 1,
                Ok(false) => {}
                Err(e) => {
                    warn!(target: "sync", "не удалось сохранить свою версию {}: {e}", record.id);
                    report.failed.push(format!("{}: {e}", record.id));
                    continue;
                }
            }

            if let Err(e) = apply_one(db, keys, record).await {
                // Запись называем, но проход продолжаем: одна битая запись не
                // повод заморозить остальные.
                warn!(target: "sync", "не удалось применить {}: {e}", record.id);
                report
                    .failed
                    .push(format!("{} ({}): {e}", record.id, record.kind));
                continue;
            }
            report.received += 1;
        }

        // Курсор двигаем **после** применения пачки и сохраняем ровно то, что
        // прислал сервер. Сохрани мы его заранее, упавшая посередине пачка
        // была бы потеряна навсегда: следующий запрос начал бы после неё.
        super::set_cursor(db, changes.cursor).await?;

        if !changes.has_more {
            return Ok(());
        }
        // Защита от кружения на месте: сервер обещает двигать курсор, но если
        // он этого не сделает, цикл станет вечным.
        if changes.cursor <= since {
            warn!(target: "sync", "курсор не сдвинулся ({since}), прекращаем получение");
            return Ok(());
        }
    }
}

/// Увести свою неотправленную правку под новым именем, если она есть.
///
/// Возвращает `true`, если правка была и её сохранили. Сервер не видит
/// содержимого и слить изменения не может, а исчезнувший без следа пароль от
/// боевого сервера человек обнаружит в худший момент. Лишняя запись
/// раздражает, но она **видима**.
async fn preserve_if_dirty(db: &SqlitePool, record: &WireRecord) -> Result<bool> {
    let Some(table) = table_kind(&record.kind) else {
        return Ok(false);
    };
    if !super::is_dirty(db, table, &record.id).await? {
        return Ok(false);
    }
    let preserved = super::keep_losing_version(db, table, &record.id).await?;
    info!(
        target: "sync",
        "своя версия {} сохранена как {preserved}: с сервера пришла другая", record.id
    );
    Ok(true)
}

async fn apply_one(db: &SqlitePool, keys: &super::RecordKeys, record: &WireRecord) -> Result<()> {
    // Неизвестный вид пропускаем, а не роняем проход: сервер вправе завести
    // новый вид записи раньше, чем клиент научится его понимать, и старая
    // версия приложения не должна из-за этого переставать работать.
    let Some(table) = table_kind(&record.kind) else {
        anyhow::bail!("вид '{}' этой версии приложения неизвестен", record.kind);
    };

    if record.deleted {
        super::apply_tombstone(db, table, &record.id, record.revision).await?;
        return Ok(());
    }

    let blob = record
        .blob
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("запись не удалена, но блоба нет"))?;
    // Блоб приходит одной строкой: nonce впереди, внутри неё. Локально они
    // хранятся раздельно, поэтому здесь разбираем.
    let raw = B64.decode(blob)?;
    if raw.len() <= 12 {
        anyhow::bail!("блоб короче nonce");
    }
    let (nonce, ciphertext) = raw.split_at(12);
    super::apply_record(
        db,
        keys,
        table,
        &record.id,
        record.revision,
        nonce,
        ciphertext,
    )
    .await
}

/// Отправить накопленное.
async fn push(
    db: &SqlitePool,
    keys: &super::RecordKeys,
    api: &ApiClient,
    access: &str,
    report: &mut SyncReport,
) -> Result<()> {
    loop {
        let waiting = super::pending(db, PUSH_BATCH).await?;
        if waiting.is_empty() {
            return Ok(());
        }

        let mut batch = Vec::with_capacity(waiting.len());
        for envelope in &waiting {
            match build(db, keys, envelope).await {
                Ok(Some(record)) => batch.push(record),
                // Собрать нечего — обычное дело для записи, удалённой уже
                // после того, как она попала в очередь.
                Ok(None) => {}
                Err(e) => {
                    warn!(target: "sync", "не удалось собрать {}: {e}", envelope.sync_id);
                    report
                        .failed
                        .push(format!("{} ({}): {e}", envelope.sync_id, envelope.kind));
                }
            }
        }

        if batch.is_empty() {
            return Ok(());
        }

        let base = super::cursor(db).await?;
        let result = api.push(access, base, &batch).await?;

        for accepted in &result.accepted {
            // В журнале запись зовётся по таблице, а сервер вернул вид с
            // провода — переводим обратно, иначе отметка «отправлено» не
            // найдёт свою строку и запись поедет второй раз.
            let kind = batch
                .iter()
                .find(|r| r.id == accepted.id)
                .and_then(|r| table_kind(&r.kind))
                .unwrap_or_default();
            // Версию записываем ту, что назвал сервер, а не вычисляем: правило
            // «прислал N — стало N+1» неверно для новой записи, ей сервер
            // присваивает 1.
            super::mark_synced(db, kind, &accepted.id, accepted.revision).await?;
            report.sent += 1;
        }

        for conflict in &result.conflicts {
            resolve(db, keys, conflict, report).await?;
        }

        // Курсор двигаем и после отправки: свои же записи иначе вернутся
        // следующим получением, а вместе с ними — затирание правок, которые
        // мы сделали после отправки.
        if result.cursor > super::cursor(db).await? {
            super::set_cursor(db, result.cursor).await?;
        }

        // Ни одна запись не сдвинулась — дальше пойдём по кругу.
        if result.accepted.is_empty() && result.conflicts.is_empty() {
            warn!(target: "sync", "сервер не принял и не отверг ни одной записи");
            return Ok(());
        }
    }
}

/// Собрать запись для отправки.
async fn build(
    db: &SqlitePool,
    keys: &super::RecordKeys,
    envelope: &super::RecordEnvelope,
) -> Result<Option<WireRecord>> {
    let Some(wire) = wire_kind(&envelope.kind) else {
        anyhow::bail!("для '{}' нет вида на проводе", envelope.kind);
    };

    // Надгробие уходит без содержимого.
    if envelope.deleted {
        return Ok(Some(WireRecord {
            id: envelope.sync_id.clone(),
            kind: wire.to_string(),
            blob: None,
            nonce: None,
            revision: envelope.revision.max(1),
            deleted: true,
        }));
    }

    let Some(payload) = super::record_payload(db, &envelope.kind, &envelope.sync_id).await? else {
        return Ok(None);
    };

    let text = serde_json::to_vec(&payload)?;
    let sealed = super::keys::seal_for_wire(&keys.current, &text)?;

    // Предел проверяем у себя: узнать о переполнении из отказа сервера можно,
    // но тогда впустую уедет вся пачка, а виноватую запись придётся вылавливать
    // из ответа.
    if sealed.len() > MAX_BLOB_BYTES {
        anyhow::bail!(
            "запись не влезает: {} байт при пределе {}",
            sealed.len(),
            MAX_BLOB_BYTES
        );
    }

    Ok(Some(WireRecord {
        id: envelope.sync_id.clone(),
        kind: wire.to_string(),
        blob: Some(sealed),
        // Nonce едет первыми байтами внутри блоба, отдельным полем он не
        // нужен. Шлём null осознанно: пустая строка всё равно превращалась
        // сервером в null, только молча. Удалённую запись отличает `deleted`.
        nonce: None,
        // Версия не меньше единицы: нуля не бывает ни у одной записи.
        revision: envelope.revision.max(1),
        deleted: false,
    }))
}

/// Разрешить конфликт: серверная версия становится основной, наша сохраняется
/// отдельной записью.
///
/// Молча выбросить свою правку нельзя. Сервер не видит содержимого и слить
/// изменения не может, а исчезнувший без следа пароль от боевого сервера
/// человек обнаружит в худший момент.
async fn resolve(
    db: &SqlitePool,
    keys: &super::RecordKeys,
    conflict: &super::api::Conflict,
    report: &mut SyncReport,
) -> Result<()> {
    let Some(kind) = table_kind(&conflict.server.kind) else {
        anyhow::bail!("конфликт по неизвестному виду '{}'", conflict.server.kind);
    };

    // Сначала уводим свою версию под новым именем, потом кладём серверную на
    // прежнее место. Обратный порядок затёр бы нашу правку.
    let preserved = super::keep_losing_version(db, kind, &conflict.id).await?;
    info!(
        target: "sync",
        "конфликт по {}: наша версия сохранена как {preserved}", conflict.id
    );

    if let Err(e) = apply_one(db, keys, &conflict.server).await {
        warn!(target: "sync", "серверная версия {} не применилась: {e}", conflict.id);
        report.failed.push(format!("{}: {e}", conflict.id));
    }

    report.conflicts += 1;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_says_what_happened_not_just_how_many() {
        // «Синхронизировано 12» не отвечает на вопрос, что произошло.
        let report = SyncReport {
            received: 3,
            sent: 2,
            conflicts: 1,
            failed: vec!["abc (hosts): битый блоб".into()],
            cursor: 42,
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"received\":3"));
        assert!(json.contains("\"conflicts\":1"));
        assert!(
            json.contains("битый блоб"),
            "виноватая запись должна быть названа"
        );
    }

    #[test]
    fn the_batch_size_stays_within_what_the_server_accepts() {
        // Сравнение постоянных величин clippy справедливо считает бессмыслицей
        // в проверке, поэтому сверяем через переменные: смысл в том, чтобы
        // правка предела здесь не разошлась с тем, что принимает сервер.
        let limits = [PUSH_BATCH, PULL_LIMIT];
        assert!(
            limits.iter().all(|n| *n <= 500),
            "сервер принимает не более 500 за раз: {limits:?}"
        );
    }
}
