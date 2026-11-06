//! Журнал изменений для синхронизации (#180).
//!
//! Проверяется именно то, ради чего журнал сделан на триггерах, а не в коде:
//! приложение о нём не знает, а запись всё равно попадает в журнал. Забыть
//! отметить изменение невозможно, потому что отмечает его сама база.

use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Row, SqlitePool};

/// Свежая база в памяти со всеми миграциями.
async fn fresh_db() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("подключение к памяти");
    darwaza_lib::db::MIGRATOR
        .run(&pool)
        .await
        .expect("миграции");
    pool
}

async fn journal_row(pool: &SqlitePool, kind: &str, sync_id: &str) -> Option<(i64, i64, i64)> {
    sqlx::query(
        "SELECT revision, dirty, deleted FROM sync_journal WHERE kind = ?1 AND sync_id = ?2",
    )
    .bind(kind)
    .bind(sync_id)
    .fetch_optional(pool)
    .await
    .expect("запрос журнала")
    .map(|r| (r.get(0), r.get(1), r.get(2)))
}

#[tokio::test]
async fn insert_is_recorded_without_application_help() {
    let pool = fresh_db().await;
    let sync_id = "11111111-1111-4111-8111-111111111111";

    sqlx::query("INSERT INTO groups (name, sort_order, sync_id) VALUES ('Prod', 0, ?1)")
        .bind(sync_id)
        .execute(&pool)
        .await
        .expect("вставка");

    let row = journal_row(&pool, "groups", sync_id).await;
    assert_eq!(
        row,
        Some((1, 1, 0)),
        "вставка должна попасть в журнал как изменённая и не удалённая"
    );
}

#[tokio::test]
async fn a_local_edit_marks_dirty_but_leaves_the_server_version_alone() {
    // Раньше правка увеличивала `revision`, и это было ошибкой: сервер
    // принимает изменение, только если присланная версия СОВПАДАЕТ с его.
    // Отправив на единицу больше, мы получали конфликт на собственную правку
    // — каждый раз, — и она уезжала не на сервер, а в «сохранённую
    // проигравшую версию». Поймано живым тестом двух устройств.
    //
    // Теперь `revision` — ровно то, что назвал сервер, а о наличии локальных
    // правок говорит `dirty`. Именно его читает отправка.
    let pool = fresh_db().await;
    let sync_id = "22222222-2222-4222-8222-222222222222";

    sqlx::query("INSERT INTO groups (name, sort_order, sync_id) VALUES ('Dev', 0, ?1)")
        .bind(sync_id)
        .execute(&pool)
        .await
        .expect("вставка");

    // Отмечаем отправленным, как это сделает синхронизация.
    sqlx::query("UPDATE sync_journal SET dirty = 0 WHERE sync_id = ?1")
        .bind(sync_id)
        .execute(&pool)
        .await
        .expect("сброс dirty");

    sqlx::query("UPDATE groups SET name = 'Development' WHERE sync_id = ?1")
        .bind(sync_id)
        .execute(&pool)
        .await
        .expect("изменение");

    let (revision, dirty, deleted) = journal_row(&pool, "groups", sync_id)
        .await
        .expect("запись журнала");
    assert_eq!(
        revision, 1,
        "версия остаётся серверной: локальная правка её не меняет"
    );
    assert_eq!(
        dirty, 1,
        "изменение после синхронизации снова требует отправки"
    );
    assert_eq!(deleted, 0);
}

#[tokio::test]
async fn delete_leaves_a_tombstone_instead_of_forgetting() {
    // Жёстко забыть удалённую запись нельзя: устройство, не выходившее в сеть,
    // не узнало бы об удалении и залило бы её обратно.
    let pool = fresh_db().await;
    let sync_id = "33333333-3333-4333-8333-333333333333";

    sqlx::query("INSERT INTO groups (name, sort_order, sync_id) VALUES ('Temp', 0, ?1)")
        .bind(sync_id)
        .execute(&pool)
        .await
        .expect("вставка");
    sqlx::query("DELETE FROM groups WHERE sync_id = ?1")
        .bind(sync_id)
        .execute(&pool)
        .await
        .expect("удаление");

    let (_, dirty, deleted) = journal_row(&pool, "groups", sync_id)
        .await
        .expect("надгробие должно остаться");
    assert_eq!(deleted, 1, "запись должна быть помечена удалённой");
    assert_eq!(dirty, 1, "удаление тоже нужно отправить");

    let local_id: Option<i64> =
        sqlx::query_scalar("SELECT local_id FROM sync_journal WHERE sync_id = ?1")
            .bind(sync_id)
            .fetch_one(&pool)
            .await
            .expect("local_id");
    assert!(
        local_id.is_none(),
        "строки больше нет, ссылка должна быть пустой"
    );
}

#[tokio::test]
async fn every_syncable_table_has_working_triggers() {
    // Триггеры заводятся по одному шаблону на шесть таблиц, и пропустить одну
    // при правке слишком легко. Проверяем все разом.
    let pool = fresh_db().await;

    let cases: [(&str, &str); 6] = [
        ("groups", "INSERT INTO groups (name, sort_order, sync_id) VALUES ('g', 0, ?1)"),
        (
            "hosts",
            "INSERT INTO hosts (name, hostname, port, username, auth_method, keep_alive_interval, sync_id) \
             VALUES ('h', '10.0.0.1', 22, 'root', 'password', 30, ?1)",
        ),
        (
            "identities",
            "INSERT INTO identities (label, kind, has_passphrase, sync_id) VALUES ('i', 'password', 0, ?1)",
        ),
        (
            "snippets",
            "INSERT INTO snippets (name, command, sort_order, sync_id) VALUES ('s', 'ls', 0, ?1)",
        ),
        (
            "known_hosts",
            "INSERT INTO known_hosts (hostname, key_type, fingerprint, sync_id) \
             VALUES ('a:22', 'ssh-ed25519', 'SHA256:x', ?1)",
        ),
        (
            "port_forwards",
            "INSERT INTO port_forwards (host_id, kind, local_port, sync_id) \
             VALUES ((SELECT id FROM hosts LIMIT 1), 'local', 8080, ?1)",
        ),
    ];

    for (i, (kind, sql)) in cases.iter().enumerate() {
        let sync_id = format!("aaaaaaaa-0000-4000-8000-00000000000{i}");
        sqlx::query(sql)
            .bind(&sync_id)
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("вставка в {kind}: {e}"));

        assert!(
            journal_row(&pool, kind, &sync_id).await.is_some(),
            "у таблицы {kind} не сработал триггер вставки"
        );
    }
}

#[tokio::test]
async fn existing_rows_get_identifiers_on_migration() {
    // Пользователь, у которого база заведена до появления синхронизации, не
    // должен остаться с записями без идентификаторов: их нечем было бы
    // отправить, и они молча выпали бы из синхронизации.
    let pool = fresh_db().await;

    let missing: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM (
             SELECT sync_id FROM groups WHERE sync_id IS NULL
             UNION ALL SELECT sync_id FROM hosts WHERE sync_id IS NULL
             UNION ALL SELECT sync_id FROM identities WHERE sync_id IS NULL
         )",
    )
    .fetch_one(&pool)
    .await
    .expect("подсчёт");

    assert_eq!(missing, 0);
}

// ─────────────────────────────────────────────────────────────────────────
// Перевод ссылок между локальными номерами и sync_id (#181)
// ─────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn references_translate_both_ways() {
    let pool = fresh_db().await;
    let sync_id = "44444444-4444-4444-8444-444444444444";

    let local_id: i64 = sqlx::query_scalar(
        "INSERT INTO groups (name, sort_order, sync_id) VALUES ('Prod', 0, ?1) RETURNING id",
    )
    .bind(sync_id)
    .fetch_one(&pool)
    .await
    .expect("вставка");

    let forward = darwaza_lib::sync::local_to_sync(&pool, "groups", local_id)
        .await
        .expect("перевод вперёд");
    assert_eq!(forward.as_deref(), Some(sync_id));

    let back = darwaza_lib::sync::sync_to_local(&pool, "groups", sync_id)
        .await
        .expect("перевод назад");
    assert_eq!(back, Some(local_id));
}

#[tokio::test]
async fn reference_to_a_deleted_record_is_absent_not_an_error() {
    // На запись могли ссылаться, а её успели удалить. Это обычная жизнь, а не
    // сбой: отсутствие ссылки должно возвращаться значением, а не паникой.
    let pool = fresh_db().await;

    let missing = darwaza_lib::sync::local_to_sync(&pool, "groups", 424_242)
        .await
        .expect("запрос не должен падать");
    assert_eq!(missing, None);

    let unknown = darwaza_lib::sync::sync_to_local(&pool, "hosts", "нет такого")
        .await
        .expect("запрос не должен падать");
    assert_eq!(unknown, None);
}

#[tokio::test]
async fn pending_returns_only_unsent_records() {
    let pool = fresh_db().await;

    for (i, name) in ["a", "b", "c"].iter().enumerate() {
        sqlx::query("INSERT INTO groups (name, sort_order, sync_id) VALUES (?1, 0, ?2)")
            .bind(name)
            .bind(format!("55555555-0000-4000-8000-00000000000{i}"))
            .execute(&pool)
            .await
            .expect("вставка");
    }

    // Одну отмечаем отправленной — она больше не должна попадать в выборку.
    darwaza_lib::sync::mark_synced(&pool, "groups", "55555555-0000-4000-8000-000000000001", 7)
        .await
        .expect("отметка");

    let pending = darwaza_lib::sync::pending(&pool, 100)
        .await
        .expect("выборка");
    assert_eq!(pending.len(), 2, "отправленная не должна возвращаться");
    assert!(pending
        .iter()
        .all(|r| r.sync_id != "55555555-0000-4000-8000-000000000001"));
}

#[tokio::test]
async fn server_revision_is_stored_as_is_not_incremented_locally() {
    // Расхождение версий — признак конфликта. Вести свой счёт значило бы
    // объявлять конфликт там, где его нет.
    let pool = fresh_db().await;
    let sync_id = "66666666-6666-4666-8666-666666666666";

    sqlx::query("INSERT INTO groups (name, sort_order, sync_id) VALUES ('x', 0, ?1)")
        .bind(sync_id)
        .execute(&pool)
        .await
        .expect("вставка");

    darwaza_lib::sync::mark_synced(&pool, "groups", sync_id, 42)
        .await
        .expect("отметка");

    let (revision, dirty, _) = journal_row(&pool, "groups", sync_id)
        .await
        .expect("запись журнала");
    assert_eq!(revision, 42, "версия должна быть ровно серверная");
    assert_eq!(dirty, 0);
}

// ─────────────────────────────────────────────────────────────────────────
// Курсор и разрешение конфликтов (#192)
// ─────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn cursor_starts_at_zero_and_survives_writing() {
    let pool = fresh_db().await;

    // Ноль означает «ничего не получено» — первый запуск запросит всё.
    assert_eq!(darwaza_lib::sync::cursor(&pool).await.expect("чтение"), 0);

    darwaza_lib::sync::set_cursor(&pool, 137)
        .await
        .expect("запись");
    assert_eq!(darwaza_lib::sync::cursor(&pool).await.expect("чтение"), 137);

    // Повторная запись заменяет, а не добавляет вторую строку.
    darwaza_lib::sync::set_cursor(&pool, 200)
        .await
        .expect("перезапись");
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_state WHERE key = 'cursor'")
        .fetch_one(&pool)
        .await
        .expect("подсчёт");
    assert_eq!(rows, 1);
    assert_eq!(darwaza_lib::sync::cursor(&pool).await.expect("чтение"), 200);
}

#[tokio::test]
async fn losing_version_is_kept_under_a_new_identifier() {
    // Молча выбросить свою правку нельзя: сервер не видит содержимого и слить
    // изменения не может, а исчезнувший пароль от боевого сервера человек
    // обнаружит в худший момент.
    let pool = fresh_db().await;
    let contested = "77777777-7777-4777-8777-777777777777";

    sqlx::query("INSERT INTO groups (name, sort_order, sync_id) VALUES ('моя версия', 0, ?1)")
        .bind(contested)
        .execute(&pool)
        .await
        .expect("вставка");

    let new_id = darwaza_lib::sync::keep_losing_version(&pool, "groups", contested)
        .await
        .expect("сохранение проигравшей");

    assert_ne!(new_id, contested, "идентификатор должен смениться");

    // Данные на месте, просто под другим идентификатором.
    let name: String = sqlx::query_scalar("SELECT name FROM groups WHERE sync_id = ?1")
        .bind(&new_id)
        .fetch_one(&pool)
        .await
        .expect("запись должна остаться");
    assert_eq!(name, "моя версия");

    // Прежний идентификатор освободился — его займёт серверная версия.
    let taken: Option<i64> = sqlx::query_scalar("SELECT id FROM groups WHERE sync_id = ?1")
        .bind(contested)
        .fetch_optional(&pool)
        .await
        .expect("запрос");
    assert!(taken.is_none(), "старый идентификатор должен освободиться");

    // Журнал знает о новом идентификаторе, иначе запись потерялась бы для
    // синхронизации: отправлять её было бы под именем, которого уже нет.
    assert!(
        journal_row(&pool, "groups", &new_id).await.is_some(),
        "журнал должен знать новый идентификатор"
    );
}

#[tokio::test]
async fn conflict_resolution_refuses_unknown_tables() {
    let pool = fresh_db().await;
    assert!(
        darwaza_lib::sync::keep_losing_version(&pool, "settings", "x")
            .await
            .is_err(),
        "таблица не синхронизируется — операция не должна проходить"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Применение чужих изменений (#193)
// ─────────────────────────────────────────────────────────────────────────

use darwaza_lib::sync::TombstoneOutcome;

/// Завести запись и отметить её отправленной — обычное состояние
/// синхронизированной записи.
async fn synced_group(pool: &SqlitePool, name: &str, sync_id: &str) {
    sqlx::query("INSERT INTO groups (name, sort_order, sync_id) VALUES (?1, 0, ?2)")
        .bind(name)
        .bind(sync_id)
        .execute(pool)
        .await
        .expect("вставка");
    darwaza_lib::sync::mark_synced(pool, "groups", sync_id, 1)
        .await
        .expect("отметка");
}

#[tokio::test]
async fn tombstone_deletes_and_is_not_echoed_back() {
    // Применение чужого изменения — это локальный DELETE, а на нём триггер,
    // который честно помечает запись к отправке. Не сняв отметку, клиент
    // отправил бы серверу его же удаление обратно, и обмен пошёл бы по кругу.
    let pool = fresh_db().await;
    let sync_id = "88888888-8888-4888-8888-888888888888";
    synced_group(&pool, "Prod", sync_id).await;

    let outcome = darwaza_lib::sync::apply_tombstone(&pool, "groups", sync_id, 5)
        .await
        .expect("применение");
    assert_eq!(outcome, TombstoneOutcome::Applied);

    let alive: Option<i64> = sqlx::query_scalar("SELECT id FROM groups WHERE sync_id = ?1")
        .bind(sync_id)
        .fetch_optional(&pool)
        .await
        .expect("запрос");
    assert!(alive.is_none(), "запись должна быть удалена");

    let (revision, dirty, deleted) = journal_row(&pool, "groups", sync_id)
        .await
        .expect("надгробие остаётся");
    assert_eq!(dirty, 0, "чужое удаление не должно уехать обратно");
    assert_eq!(deleted, 1);
    assert_eq!(revision, 5, "версия должна быть серверной");
}

#[tokio::test]
async fn tombstone_preserves_an_unsent_local_edit() {
    // Спорить с удалением бессмысленно, но терять работу человека нельзя.
    let pool = fresh_db().await;
    let sync_id = "99999999-9999-4999-8999-999999999999";
    synced_group(&pool, "Prod", sync_id).await;

    // Локальная правка, ещё не отправленная.
    sqlx::query("UPDATE groups SET name = 'Prod (правка)' WHERE sync_id = ?1")
        .bind(sync_id)
        .execute(&pool)
        .await
        .expect("правка");

    let outcome = darwaza_lib::sync::apply_tombstone(&pool, "groups", sync_id, 9)
        .await
        .expect("применение");

    let preserved_id = match outcome {
        TombstoneOutcome::LocalEditPreserved(id) => id,
        other => panic!("правку должны были сохранить, получено {other:?}"),
    };

    let name: String = sqlx::query_scalar("SELECT name FROM groups WHERE sync_id = ?1")
        .bind(&preserved_id)
        .fetch_one(&pool)
        .await
        .expect("сохранённая правка должна остаться");
    assert_eq!(name, "Prod (правка)");

    // Под прежним именем записи больше нет — удаление применено.
    let old: Option<i64> = sqlx::query_scalar("SELECT id FROM groups WHERE sync_id = ?1")
        .bind(sync_id)
        .fetch_optional(&pool)
        .await
        .expect("запрос");
    assert!(old.is_none());
}

#[tokio::test]
async fn tombstone_for_unknown_record_is_a_no_op() {
    // Запись могли создать и удалить, пока нас не было в сети: удалять нечего,
    // и это не ошибка.
    let pool = fresh_db().await;
    let outcome = darwaza_lib::sync::apply_tombstone(&pool, "groups", "нет такой", 3)
        .await
        .expect("не должно падать");
    assert_eq!(outcome, TombstoneOutcome::Unknown);
}

#[tokio::test]
async fn tombstone_on_already_deleted_record_just_accepts_server_revision() {
    let pool = fresh_db().await;
    let sync_id = "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa";
    synced_group(&pool, "Temp", sync_id).await;

    sqlx::query("DELETE FROM groups WHERE sync_id = ?1")
        .bind(sync_id)
        .execute(&pool)
        .await
        .expect("наше удаление");

    let outcome = darwaza_lib::sync::apply_tombstone(&pool, "groups", sync_id, 12)
        .await
        .expect("применение");
    assert_eq!(outcome, TombstoneOutcome::Applied);

    let (revision, dirty, _) = journal_row(&pool, "groups", sync_id)
        .await
        .expect("надгробие");
    assert_eq!(revision, 12);
    assert_eq!(dirty, 0, "обе стороны удалили — отправлять нечего");
}

// ─────────────────────────────────────────────────────────────────────────
// Материализация записей (#193a)
// ─────────────────────────────────────────────────────────────────────────

const KEY: [u8; 32] = [42u8; 32];

/// Зашифровать полезную нагрузку так, как это сделал бы сервер.
fn sealed(payload: serde_json::Value) -> (Vec<u8>, Vec<u8>) {
    darwaza_lib::sync::seal(&KEY, &payload).expect("шифрование")
}

#[tokio::test]
async fn incoming_group_is_created_and_not_echoed_back() {
    let pool = fresh_db().await;
    let sync_id = "bbbbbbbb-1111-4111-8111-bbbbbbbbbbbb";
    let (nonce, blob) = sealed(serde_json::json!({ "name": "Prod", "sortOrder": 2 }));

    darwaza_lib::sync::apply_record(
        &pool,
        &darwaza_lib::sync::RecordKeys::only(KEY),
        "groups",
        sync_id,
        7,
        &nonce,
        &blob,
    )
    .await
    .expect("применение");

    let (name, order): (String, i64) =
        sqlx::query_as("SELECT name, sort_order FROM groups WHERE sync_id = ?1")
            .bind(sync_id)
            .fetch_one(&pool)
            .await
            .expect("запись должна появиться");
    assert_eq!(name, "Prod");
    assert_eq!(order, 2);

    let (revision, dirty, _) = journal_row(&pool, "groups", sync_id).await.expect("журнал");
    assert_eq!(revision, 7, "версия должна быть серверной");
    assert_eq!(dirty, 0, "чужая запись не должна уехать обратно");
}

#[tokio::test]
async fn incoming_record_updates_instead_of_duplicating() {
    let pool = fresh_db().await;
    let sync_id = "cccccccc-2222-4222-8222-cccccccccccc";

    for (name, revision) in [("Старое", 1), ("Новое", 2)] {
        let (nonce, blob) = sealed(serde_json::json!({ "name": name, "sortOrder": 0 }));
        darwaza_lib::sync::apply_record(
            &pool,
            &darwaza_lib::sync::RecordKeys::only(KEY),
            "groups",
            sync_id,
            revision,
            &nonce,
            &blob,
        )
        .await
        .expect("применение");
    }

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM groups WHERE sync_id = ?1")
        .bind(sync_id)
        .fetch_one(&pool)
        .await
        .expect("подсчёт");
    assert_eq!(count, 1, "повторное применение не должно плодить записи");

    let name: String = sqlx::query_scalar("SELECT name FROM groups WHERE sync_id = ?1")
        .bind(sync_id)
        .fetch_one(&pool)
        .await
        .expect("чтение");
    assert_eq!(name, "Новое");
}

#[tokio::test]
async fn host_reference_to_group_resolves_to_local_id() {
    let pool = fresh_db().await;
    let group_sync = "dddddddd-3333-4333-8333-dddddddddddd";
    let host_sync = "eeeeeeee-4444-4444-8444-eeeeeeeeeeee";

    let (n, b) = sealed(serde_json::json!({ "name": "Prod", "sortOrder": 0 }));
    darwaza_lib::sync::apply_record(
        &pool,
        &darwaza_lib::sync::RecordKeys::only(KEY),
        "groups",
        group_sync,
        1,
        &n,
        &b,
    )
    .await
    .expect("группа");

    let (n, b) = sealed(serde_json::json!({
        "name": "web-1", "hostname": "10.0.0.1", "port": 22,
        "username": "deploy", "authMethod": "publickey",
        "groupSyncId": group_sync,
    }));
    darwaza_lib::sync::apply_record(
        &pool,
        &darwaza_lib::sync::RecordKeys::only(KEY),
        "hosts",
        host_sync,
        1,
        &n,
        &b,
    )
    .await
    .expect("хост");

    let group_local: i64 = sqlx::query_scalar("SELECT id FROM groups WHERE sync_id = ?1")
        .bind(group_sync)
        .fetch_one(&pool)
        .await
        .expect("группа");
    let host_group: Option<i64> =
        sqlx::query_scalar("SELECT group_id FROM hosts WHERE sync_id = ?1")
            .bind(host_sync)
            .fetch_one(&pool)
            .await
            .expect("хост");
    assert_eq!(
        host_group,
        Some(group_local),
        "ссылка обязана указывать на локальный номер группы, а не на чужой"
    );
}

#[tokio::test]
async fn host_arriving_before_its_group_is_still_accepted() {
    // Порядок записей в выгрузке не гарантирован. Отказаться принять хост,
    // потому что группа ещё не приехала, значит потерять его совсем: второй
    // раз сервер его не пришлёт — он уже отдан.
    let pool = fresh_db().await;
    let host_sync = "ffffffff-5555-4555-8555-ffffffffffff";

    let (n, b) = sealed(serde_json::json!({
        "name": "web-2", "hostname": "10.0.0.2", "port": 22,
        "username": "deploy", "authMethod": "password",
        "groupSyncId": "группы-ещё-нет",
    }));
    darwaza_lib::sync::apply_record(
        &pool,
        &darwaza_lib::sync::RecordKeys::only(KEY),
        "hosts",
        host_sync,
        1,
        &n,
        &b,
    )
    .await
    .expect("хост должен приняться и без группы");

    let group: Option<i64> = sqlx::query_scalar("SELECT group_id FROM hosts WHERE sync_id = ?1")
        .bind(host_sync)
        .fetch_one(&pool)
        .await
        .expect("чтение");
    assert!(group.is_none(), "ссылка пока пустая — это допустимо");
}

#[tokio::test]
async fn wrong_key_does_not_write_anything() {
    // Если ключ не тот, запись не должна появиться даже частично: половина
    // применённой синхронизации хуже, чем ни одной.
    let pool = fresh_db().await;
    let sync_id = "aaaaaaaa-6666-4666-8666-aaaaaaaaaaaa";
    let (nonce, blob) = sealed(serde_json::json!({ "name": "Секрет" }));

    let result = darwaza_lib::sync::apply_record(
        &pool,
        &darwaza_lib::sync::RecordKeys::only([9u8; 32]),
        "groups",
        sync_id,
        1,
        &nonce,
        &blob,
    )
    .await;
    assert!(result.is_err(), "чужой ключ не должен расшифровать");

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM groups")
        .fetch_one(&pool)
        .await
        .expect("подсчёт");
    assert_eq!(count, 0, "ничего не должно было записаться");
}

// ─────────────────────────────────────────────────────────────────────────
// Первый запуск и работа без сети (#194, #195)
// ─────────────────────────────────────────────────────────────────────────

use darwaza_lib::sync::FirstSyncChoice;

#[tokio::test]
async fn asks_only_when_there_is_something_to_lose() {
    // На чистом устройстве вопрос неуместен и только пугает.
    assert!(
        !darwaza_lib::sync::should_ask_before_first_sync(0, 0),
        "нечего терять — спрашивать не о чем"
    );
    assert!(
        darwaza_lib::sync::should_ask_before_first_sync(0, 5),
        "есть локальные записи и не было синхронизации — надо спросить"
    );
    assert!(
        !darwaza_lib::sync::should_ask_before_first_sync(137, 5),
        "синхронизация уже шла — вопрос задан однажды и больше не задаётся"
    );
}

#[tokio::test]
async fn replacing_local_also_clears_the_journal() {
    // Иначе надгробия от этой очистки уедут на сервер и удалят там ровно то,
    // что мы собирались принять.
    let pool = fresh_db().await;
    for i in 0..3 {
        sqlx::query("INSERT INTO groups (name, sort_order, sync_id) VALUES (?1, 0, ?2)")
            .bind(format!("g{i}"))
            .bind(format!("11110000-0000-4000-8000-00000000000{i}"))
            .execute(&pool)
            .await
            .expect("вставка");
    }

    let removed = darwaza_lib::sync::apply_first_sync_choice(&pool, FirstSyncChoice::ReplaceLocal)
        .await
        .expect("замена");
    assert_eq!(removed, 3);

    let journal: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_journal")
        .fetch_one(&pool)
        .await
        .expect("подсчёт");
    assert_eq!(journal, 0, "журнал должен быть пуст, иначе удаления уедут");
}

#[tokio::test]
async fn merging_keeps_everything_pending() {
    let pool = fresh_db().await;
    sqlx::query("INSERT INTO groups (name, sort_order, sync_id) VALUES ('Prod', 0, ?1)")
        .bind("22220000-0000-4000-8000-000000000000")
        .execute(&pool)
        .await
        .expect("вставка");

    let count = darwaza_lib::sync::apply_first_sync_choice(&pool, FirstSyncChoice::Merge)
        .await
        .expect("слияние");
    assert_eq!(count, 1);

    // Запись на месте и по-прежнему ждёт отправки.
    let pending = darwaza_lib::sync::pending(&pool, 100)
        .await
        .expect("выборка");
    assert_eq!(pending.len(), 1);
}

#[tokio::test]
async fn sync_failure_is_remembered_but_does_not_block_anything() {
    // Невозможность достучаться до сервера не должна мешать подключаться к
    // своим машинам: приложение работало без аккаунта и обязано работать так
    // же после его появления.
    let pool = fresh_db().await;

    darwaza_lib::sync::record_sync_failure(&pool, "сеть недоступна")
        .await
        .expect("запись ошибки");

    let health = darwaza_lib::sync::health(&pool).await.expect("состояние");
    assert_eq!(health.last_error.as_deref(), Some("сеть недоступна"));

    // Локальная работа продолжается: хост заводится, попадает в очередь.
    sqlx::query("INSERT INTO groups (name, sort_order, sync_id) VALUES ('Офлайн', 0, ?1)")
        .bind("33330000-0000-4000-8000-000000000000")
        .execute(&pool)
        .await
        .expect("вставка должна пройти при неработающей синхронизации");

    let health = darwaza_lib::sync::health(&pool).await.expect("состояние");
    assert_eq!(health.pending, 1, "накопленное дождётся сети");

    assert!(
        health.last_sync_at.is_none(),
        "удачных проходов ещё не было"
    );

    darwaza_lib::sync::record_sync_success(&pool)
        .await
        .expect("успех");
    let health = darwaza_lib::sync::health(&pool).await.expect("состояние");
    assert!(health.last_error.is_none(), "успех должен убрать ошибку");
    let first = health
        .last_sync_at
        .expect("время удачного прохода записано");
    assert!(
        first > 1_700_000_000,
        "это Unix-секунды, а не счётчик: {first}"
    );

    // Неудача время удачи не стирает: подвал должен показывать, когда
    // получилось в последний раз, а не когда пробовали.
    darwaza_lib::sync::record_sync_failure(&pool, "сеть недоступна")
        .await
        .expect("запись ошибки");
    let health = darwaza_lib::sync::health(&pool).await.expect("состояние");
    assert_eq!(health.last_sync_at, Some(first));
    assert_eq!(health.pending, 1, "очередь при этом не трогается");
}

// ─────────────────────────────────────────────────────────────────────────
// Сборка записи для отправки — зеркало применения
// ─────────────────────────────────────────────────────────────────────────

/// Круговой прогон: собрать запись → зашифровать → применить как пришедшую с
/// сервера → сверить значения.
///
/// Ловит расхождение имён полей между `record_payload` и `apply_record`.
/// Разойдись они — запись уехала бы на сервер, вернулась на второе устройство
/// и молча потеряла половину значений, а выглядело бы это как работающая
/// синхронизация. Ни один односторонний тест такого не покажет.
#[tokio::test]
async fn a_host_survives_the_round_trip_through_the_server() {
    let db = fresh_db().await;
    let key = [9u8; 32];

    // sync_id ставит приложение (`db::new_sync_id`), умолчания в схеме нет.
    // Сырой INSERT мимо этого правила роняет триггер журнала.
    sqlx::query("INSERT INTO groups (name, sort_order, sync_id) VALUES ('Production', 3, ?1)")
        .bind(darwaza_lib::db::new_sync_id())
        .execute(&db)
        .await
        .unwrap();
    let group_id: i64 = sqlx::query_scalar("SELECT id FROM groups WHERE name = 'Production'")
        .fetch_one(&db)
        .await
        .unwrap();

    sqlx::query(
        "INSERT INTO hosts (group_id, name, hostname, port, username, auth_method, \
            forward_agent, keep_alive_interval, description, sync_id) \
         VALUES (?1, 'prod-01', 'prod-01.example.internal', 2222, 'deploy', 'publickey', \
            1, 45, 'боевой сервер', ?2)",
    )
    .bind(group_id)
    .bind(darwaza_lib::db::new_sync_id())
    .execute(&db)
    .await
    .unwrap();

    let sync_id: String = sqlx::query_scalar("SELECT sync_id FROM hosts WHERE name = 'prod-01'")
        .fetch_one(&db)
        .await
        .unwrap();

    // Отправляем.
    let payload = darwaza_lib::sync::record_payload(&db, "hosts", &sync_id)
        .await
        .unwrap()
        .expect("запись есть");
    let (nonce, blob) = darwaza_lib::sync::seal(&key, &payload).unwrap();

    // Стираем локально всё, кроме группы: изображаем второе устройство,
    // которое хост ещё не видело.
    sqlx::query("DELETE FROM hosts").execute(&db).await.unwrap();

    // Принимаем как пришедшее с сервера.
    darwaza_lib::sync::apply_record(
        &db,
        &darwaza_lib::sync::RecordKeys::only(key),
        "hosts",
        &sync_id,
        4,
        &nonce,
        &blob,
    )
    .await
    .unwrap();

    let row: (
        Option<i64>,
        String,
        String,
        i64,
        String,
        String,
        i64,
        i64,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT group_id, name, hostname, port, username, auth_method, forward_agent, \
                keep_alive_interval, description FROM hosts WHERE sync_id = ?1",
    )
    .bind(&sync_id)
    .fetch_one(&db)
    .await
    .unwrap();

    assert_eq!(
        row.0,
        Some(group_id),
        "ссылка на группу должна восстановиться"
    );
    assert_eq!(row.1, "prod-01");
    assert_eq!(row.2, "prod-01.example.internal");
    assert_eq!(row.3, 2222, "порт не должен подмениться умолчанием 22");
    assert_eq!(row.4, "deploy");
    assert_eq!(row.5, "publickey");
    assert_eq!(row.6, 1, "флаг проброса агента должен сохраниться");
    assert_eq!(
        row.7, 45,
        "интервал keep-alive не должен стать умолчанием 30"
    );
    assert_eq!(row.8.as_deref(), Some("боевой сервер"));
}

#[tokio::test]
async fn references_travel_as_sync_ids_not_local_numbers() {
    // Локальные номера на разных устройствах разные. Уйди ссылка числом —
    // хост приехал бы в чужую группу, и заметили бы это не сразу.
    let db = fresh_db().await;

    sqlx::query("INSERT INTO groups (name, sort_order, sync_id) VALUES ('Databases', 0, ?1)")
        .bind(darwaza_lib::db::new_sync_id())
        .execute(&db)
        .await
        .unwrap();
    let group_id: i64 = sqlx::query_scalar("SELECT id FROM groups WHERE name = 'Databases'")
        .fetch_one(&db)
        .await
        .unwrap();
    let group_sync: String =
        sqlx::query_scalar("SELECT sync_id FROM groups WHERE name = 'Databases'")
            .fetch_one(&db)
            .await
            .unwrap();

    sqlx::query(
        "INSERT INTO hosts (group_id, name, hostname, port, username, auth_method, sync_id) \
         VALUES (?1, 'db-01', 'db-01.internal', 22, 'root', 'password', ?2)",
    )
    .bind(group_id)
    .bind(darwaza_lib::db::new_sync_id())
    .execute(&db)
    .await
    .unwrap();
    let sync_id: String = sqlx::query_scalar("SELECT sync_id FROM hosts WHERE name = 'db-01'")
        .fetch_one(&db)
        .await
        .unwrap();

    let payload = darwaza_lib::sync::record_payload(&db, "hosts", &sync_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        payload.get("groupSyncId").and_then(|v| v.as_str()),
        Some(group_sync.as_str()),
        "ссылка должна уходить по sync_id: {payload}"
    );
    assert!(
        payload.get("groupId").is_none(),
        "локального номера в отправляемой записи быть не должно"
    );
}

#[tokio::test]
async fn a_deleted_record_has_nothing_to_send() {
    // Надгробие уходит без содержимого, и собирать его нечего. Отсутствие —
    // обычный случай, а не ошибка.
    let db = fresh_db().await;
    let missing = darwaza_lib::sync::record_payload(&db, "hosts", "нет-такой-записи")
        .await
        .unwrap();
    assert!(missing.is_none());
}

#[tokio::test]
async fn a_secret_never_leaves_in_the_payload() {
    // Указатель на keyring уехать может, сам секрет — никогда. Это то самое
    // обещание, ради которого сервер слепой.
    let db = fresh_db().await;
    sqlx::query(
        "INSERT INTO identities (label, kind, key_path, keyring_ref, has_passphrase, sync_id) \
         VALUES ('рабочий ключ', 'private_key', '/home/u/.ssh/id_ed25519', 'identity:7', 1, ?1)",
    )
    .bind(darwaza_lib::db::new_sync_id())
    .execute(&db)
    .await
    .unwrap();
    let sync_id: String =
        sqlx::query_scalar("SELECT sync_id FROM identities WHERE label = 'рабочий ключ'")
            .fetch_one(&db)
            .await
            .unwrap();

    let payload = darwaza_lib::sync::record_payload(&db, "identities", &sync_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        payload.get("keyringRef").and_then(|v| v.as_str()),
        Some("identity:7"),
        "указатель на секрет уезжает"
    );
    let text = payload.to_string();
    assert!(
        !text.contains("password") && !text.contains("passphrase\":\""),
        "сам секрет уехать не должен: {text}"
    );
}

/// Идентификаторы синхронизации обязаны быть UUID (#179).
///
/// Контракт сервера принимает `id` только в формате UUID. Миграция журнала
/// проставляла старым строкам `hex(randomblob(16))` — 32 знака без дефисов, — и
/// сервер отвергал такую запись целиком: человек видел «Запрос не соответствует
/// контракту» и не мог сделать ничего, потому что идентификаторы ему не видны.
///
/// Проверка идёт по **всем** таблицам, участвующим в синхронизации, а не по
/// одной: ошибка была одинаковой в шести местах сразу, и следующая таблица
/// повторит её ровно так же.
#[tokio::test]
async fn every_sync_id_is_a_uuid() {
    let pool = fresh_db().await;

    // Строки в старом формате — ровно то, что миграция обязана исправить.
    let legacy = "d79cd9f1dd8bdb5079f9a1d67d6e05ae";
    sqlx::query("INSERT INTO groups (name, sync_id) VALUES ('старая', ?1)")
        .bind(legacy)
        .execute(&pool)
        .await
        .expect("вставка группы со старым идентификатором");

    // Прогоняем НАСТОЯЩУЮ миграцию, а не её пересказ. Пересказ проверял бы
    // только сам себя: первая же версия этого теста чинила таблицы и забывала
    // журнал — ровно ту половину, на которой всё и ломается.
    migrate_sync_ids(&pool).await;

    for table in [
        "groups",
        "hosts",
        "identities",
        "snippets",
        "port_forwards",
        "known_hosts",
        "sync_journal",
    ] {
        let bad: Vec<(String,)> = sqlx::query_as(&format!(
            "SELECT sync_id FROM {table} WHERE sync_id IS NOT NULL AND sync_id NOT GLOB \
             '[0-9a-f]*-[0-9a-f]*-[0-9a-f]*-[0-9a-f]*-[0-9a-f]*'"
        ))
        .fetch_all(&pool)
        .await
        .unwrap_or_default();

        assert!(
            bad.is_empty(),
            "в таблице {table} идентификаторы не в формате UUID: {bad:?}. \
             Сервер отвергнет такие записи с VALIDATION_FAILED."
        );
    }
}

/// Прогнать настоящую миграцию приведения `sync_id` к UUID.
///
/// Именно файл, а не его пересказ: первая версия проверки чинила таблицы и
/// забывала журнал — ровно ту половину, на которой всё и ломалось.
async fn migrate_sync_ids(pool: &SqlitePool) {
    sqlx::raw_sql(include_str!(
        "../migrations/20260829000001_sync_id_uuid_format.sql"
    ))
    .execute(pool)
    .await
    .expect("миграция приведения sync_id к UUID");
}

/// Смешанная база: часть строк уже UUID, часть — старого вида (#179).
///
/// Именно так выглядит база живого человека: записи до 20 августа получили
/// `hex(randomblob(16))`, созданные позже — настоящие UUID из Rust. Миграция
/// обязана тронуть только первые и не испортить вторые: переписанный UUID
/// означал бы для сервера другую запись, то есть дубликат вместо правки.
#[tokio::test]
async fn the_migration_touches_only_the_broken_ids() {
    let pool = fresh_db().await;

    let legacy = "d79cd9f1dd8bdb5079f9a1d67d6e05ae";
    let proper = "5aa3b4e0-cc3f-4ced-a327-a38358684f89";

    for (name, id) in [("старая", legacy), ("новая", proper)] {
        sqlx::query("INSERT INTO groups (name, sync_id) VALUES (?1, ?2)")
            .bind(name)
            .bind(id)
            .execute(&pool)
            .await
            .expect("вставка группы");
    }

    migrate_sync_ids(&pool).await;

    let after: Vec<(String, String)> =
        sqlx::query_as("SELECT name, sync_id FROM groups ORDER BY name")
            .fetch_all(&pool)
            .await
            .expect("чтение групп");

    let new_id = &after.iter().find(|(n, _)| n == "новая").expect("новая").1;
    let old_id = &after.iter().find(|(n, _)| n == "старая").expect("старая").1;

    assert_eq!(new_id, proper, "готовый UUID трогать нельзя");
    assert_eq!(
        old_id, "d79cd9f1-dd8b-db50-79f9-a1d67d6e05ae",
        "старому идентификатору расставляются дефисы, байты те же"
    );
}

/// Повторный прогон ничего не портит.
///
/// Миграции применяются один раз, но у этой есть свойство поважнее
/// формального: она обязана быть безвредной при повторе. Иначе любой ручной
/// прогон — а он случается при разборе жалобы — превратил бы уже исправленные
/// идентификаторы в мусор.
#[tokio::test]
async fn running_the_migration_twice_changes_nothing() {
    let pool = fresh_db().await;

    sqlx::query("INSERT INTO groups (name, sync_id) VALUES ('одна', ?1)")
        .bind("d79cd9f1dd8bdb5079f9a1d67d6e05ae")
        .execute(&pool)
        .await
        .expect("вставка");

    migrate_sync_ids(&pool).await;
    let once: String = sqlx::query_scalar("SELECT sync_id FROM groups WHERE name = 'одна'")
        .fetch_one(&pool)
        .await
        .expect("после первого прогона");

    migrate_sync_ids(&pool).await;
    let twice: String = sqlx::query_scalar("SELECT sync_id FROM groups WHERE name = 'одна'")
        .fetch_one(&pool)
        .await
        .expect("после второго прогона");

    assert_eq!(once, twice, "повторный прогон обязан быть безвредным");
}

/// Строку без `sync_id` создать нельзя — и это то, на что опирается миграция.
///
/// Колонка объявлена без NOT NULL: её добавляли `ALTER TABLE`, и по схеме
/// пустое значение выглядит возможным. На деле его не бывает: триггер журнала
/// вставляет `sync_id` в таблицу с NOT NULL и обрывает всю операцию.
///
/// Проверка нужна именно поэтому. Миграция приведения к UUID не обрабатывает
/// NULL отдельно, и это не упущение, а опора на данное правило. Если правило
/// однажды ослабят, упадёт этот тест, а не человек с испорченной базой.
#[tokio::test]
async fn a_row_without_sync_id_cannot_exist() {
    let pool = fresh_db().await;

    let e = sqlx::query("INSERT INTO groups (name, sync_id) VALUES ('без-идентификатора', NULL)")
        .execute(&pool)
        .await
        .expect_err("строка без sync_id не должна создаваться");

    assert!(
        e.to_string().contains("sync_journal.sync_id"),
        "не даёт именно журнал: {e}"
    );

    // Раз таких строк не бывает, миграции нечего с ними делать — и она не падает.
    migrate_sync_ids(&pool).await;
}

// ─────────────────────────────────────────────────────────────────────────
// Перевоз записей под новый ключ (#180c)
// ─────────────────────────────────────────────────────────────────────────

/// Перевоз помечает к отправке ВСЁ, и делается ровно один раз.
///
/// Почему это важнее, чем «читаем обоими»: запасной ключ выведен из пароля и
/// живёт до его смены. После смены старого ключа нет ни у кого, включая
/// человека, и оставшиеся под ним записи умирают окончательно. Перевоз
/// превращает гонку в разовую работу.
#[tokio::test]
async fn rekey_marks_everything_once() {
    let pool = fresh_db().await;

    sqlx::query("INSERT INTO groups (name, sort_order, sync_id) VALUES ('Prod', 0, ?1)")
        .bind(darwaza_lib::db::new_sync_id())
        .execute(&pool)
        .await
        .expect("группа");
    // Отправлено: грязным не считается.
    sqlx::query("UPDATE sync_journal SET dirty = 0")
        .execute(&pool)
        .await
        .expect("отметка отправленного");

    let dirty_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_journal WHERE dirty = 1")
        .fetch_one(&pool)
        .await
        .expect("счёт");
    assert_eq!(dirty_before, 0, "перед перевозом отправлять нечего");

    // Устройство уже синхронизировалось: иначе перевозить было бы нечего.
    sqlx::query(
        "INSERT INTO sync_state (key, value) VALUES ('cursor', '7') \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .execute(&pool)
    .await
    .expect("курсор");

    let keys = darwaza_lib::sync::RecordKeys {
        current: [2u8; 32],
        legacy: Some([1u8; 32]),
    };
    darwaza_lib::sync::engine::rekey_once(&pool, &keys)
        .await
        .expect("перевоз");

    let dirty: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_journal WHERE dirty = 1")
        .fetch_one(&pool)
        .await
        .expect("счёт");
    assert_eq!(dirty, 1, "запись обязана уехать под новым ключом");

    // Второй раз — ничего: иначе каждый проход слал бы всё заново.
    sqlx::query("UPDATE sync_journal SET dirty = 0")
        .execute(&pool)
        .await
        .expect("сброс");
    darwaza_lib::sync::engine::rekey_once(&pool, &keys)
        .await
        .expect("повторный перевоз");

    let dirty: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_journal WHERE dirty = 1")
        .fetch_one(&pool)
        .await
        .expect("счёт");
    assert_eq!(dirty, 0, "перевоз делается один раз");
}

/// Без прежнего ключа перевозить нечего — и ничего не помечается.
#[tokio::test]
async fn rekey_does_nothing_without_a_legacy_key() {
    let pool = fresh_db().await;
    sqlx::query("INSERT INTO groups (name, sort_order, sync_id) VALUES ('Prod', 0, ?1)")
        .bind(darwaza_lib::db::new_sync_id())
        .execute(&pool)
        .await
        .expect("группа");
    sqlx::query("UPDATE sync_journal SET dirty = 0")
        .execute(&pool)
        .await
        .expect("сброс");

    darwaza_lib::sync::engine::rekey_once(&pool, &darwaza_lib::sync::RecordKeys::only([2u8; 32]))
        .await
        .expect("перевоз");

    let dirty: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_journal WHERE dirty = 1")
        .fetch_one(&pool)
        .await
        .expect("счёт");
    assert_eq!(dirty, 0);
}

/// Устройству, которое ещё не синхронизировалось, перевозить нечего.
///
/// Всё, что оно отправит впервые, уйдёт уже новым ключом. Лишняя отправка
/// отсюда сдвинула бы версии на сервере и могла бы сделать конфликт там, где
/// его не было, — а конфликт у нас разводит запись надвое.
#[tokio::test]
async fn rekey_skips_a_device_that_never_synced() {
    let pool = fresh_db().await;
    sqlx::query("INSERT INTO groups (name, sort_order, sync_id) VALUES ('Prod', 0, ?1)")
        .bind(darwaza_lib::db::new_sync_id())
        .execute(&pool)
        .await
        .expect("группа");
    sqlx::query("UPDATE sync_journal SET dirty = 0")
        .execute(&pool)
        .await
        .expect("сброс");

    let keys = darwaza_lib::sync::RecordKeys {
        current: [2u8; 32],
        legacy: Some([1u8; 32]),
    };
    darwaza_lib::sync::engine::rekey_once(&pool, &keys)
        .await
        .expect("перевоз");

    let dirty: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_journal WHERE dirty = 1")
        .fetch_one(&pool)
        .await
        .expect("счёт");
    assert_eq!(dirty, 0, "курсор нулевой — переезжать нечему");
}
