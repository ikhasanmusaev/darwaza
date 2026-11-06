//! Живой прогон **клиента приложения** против сервера.
//!
//! Отличие от `sync_live.rs` принципиальное: тот проверял, что спецификация
//! реализуема, повторяя её у себя. Этот дёргает `darwaza_lib::sync::api` —
//! тот самый код, который выполнится у пользователя. Если клиент разойдётся
//! с сервером, упадёт здесь, а не у человека.
//!
//! ```bash
//! # в другой сессии: SMTP_HOST= pnpm --filter @darwaza/api dev
//! cargo test --test sync_client_live -- --ignored --nocapture
//! ```

use darwaza_lib::sync::api::{ApiClient, ApiError, Session, WireRecord};
use darwaza_lib::sync::keys;

const API: &str = "http://127.0.0.1:8080/api/v1";

/// Регистрация так, как её делает приложение (#180c).
///
/// Ключ восстановления заводится **вместе с аккаунтом**: у сервера эти поля
/// обязательные, а регистрация без них — не та регистрация, которая
/// выполнится у человека. Помощником, а не строкой на каждом месте: три
/// копии разойдутся, и разойдутся молча.
async fn register_account(
    api: &ApiClient,
    email: &str,
    auth: &str,
    protected: &str,
    symmetric: &[u8; 32],
) {
    let rk = darwaza_lib::recovery_key::generate().expect("случайность");
    let recovery = keys::recovery_keys(&rk).expect("ключи восстановления");
    let protected_rk = keys::seal_for_wire(&recovery.enc, symmetric).expect("обёртка для бумаги");

    api.register(email, auth, protected, &recovery.auth_hash, &protected_rk)
        .await
        .expect("регистрация");
}

/// Данные намеренно неровные: смешанный регистр, пробелы, не-ASCII в пароле.
/// Ровные данные прячут ровно то, что ломается у первого живого пользователя.
fn credentials() -> (String, String) {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).expect("случайность");
    let tag: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    (
        format!("  Client.{tag}@Example.COM "),
        "пароль-Ünïcode".to_string(),
    )
}

/// Подтвердить адрес: сервер в разработке кладёт письмо файлом на диск.
async fn verification_token(email: &str) -> String {
    // Тот же каталог, что у сервера: `MAIL_OUTBOX`, иначе `server/.mail-outbox`.
    // Умолчание берём от сервера, а не своё — разойдись они, тест ждал бы
    // письма там, куда его никто не кладёт. Путь от `src-tauri`, откуда
    // запускается cargo: пакет API лежит в `server/server`, а не в `server`.
    let dir = std::env::var("MAIL_OUTBOX")
        .unwrap_or_else(|_| "../../server/server/.mail-outbox".to_string());
    let normalized = keys::normalize_email(email);

    for _ in 0..50 {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let Ok(text) = std::fs::read_to_string(entry.path()) else {
                    continue;
                };
                if !text.contains(&normalized) {
                    continue;
                }
                if let Some(rest) = text.split("token=").nth(1) {
                    return rest.split_whitespace().next().unwrap_or("").to_string();
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("письмо для {normalized} не найдено в {dir}");
}

#[tokio::test]
#[ignore = "нужен поднятый сервер"]
async fn the_application_client_speaks_to_the_live_server() {
    let api = ApiClient::new(API).expect("клиент");
    let (email, password) = credentials();

    // Ключи выводит приложение — те же функции, что и у пользователя.
    let master = keys::master_key(&email, &password).expect("мастер-ключ");
    let auth = keys::auth_hash(&master, &password).expect("auth_hash");
    let enc = keys::encryption_key(&master).expect("ключ шифрования");

    let mut symmetric = [0u8; 32];
    getrandom::fill(&mut symmetric).expect("случайность");
    let protected = keys::seal_for_wire(&enc, &symmetric).expect("защита ключа");

    register_account(&api, &email, &auth, &protected, &symmetric).await;

    let token = verification_token(&email).await;
    api.verify_email(&token)
        .await
        .expect("подтверждение адреса");

    let session = api.login(&email, &auth, None).await.expect("вход");
    assert!(!session.access_token.is_empty());

    // Первый раз: курсора нет, значит полная выгрузка, и она пуста.
    let empty = api
        .changes(&session.access_token, 0, 500)
        .await
        .expect("первая выгрузка");
    assert!(empty.changes.is_empty(), "у нового аккаунта записей нет");

    // Новая запись уходит с revision = 1 и получает от сервера тоже 1.
    let blob = keys::seal_for_wire(&enc, "хост prod-01 🔐".as_bytes()).expect("шифрование");
    let record = WireRecord {
        id: uuid::Uuid::new_v4().to_string(),
        kind: "host".into(),
        blob: Some(blob),
        nonce: Some(String::new()),
        revision: 1,
        deleted: false,
    };

    let pushed = api
        .push(
            &session.access_token,
            empty.cursor,
            std::slice::from_ref(&record),
        )
        .await
        .expect("отправка");
    assert!(pushed.conflicts.is_empty(), "конфликта быть не должно");
    assert_eq!(pushed.accepted.len(), 1, "запись должна быть принята");
    assert_eq!(
        pushed.accepted[0].revision, 1,
        "новой записи сервер присваивает 1, а не 2"
    );

    // Запись возвращается и расшифровывается ключом приложения.
    let back = api
        .changes(&session.access_token, 0, 500)
        .await
        .expect("выгрузка");
    assert_eq!(back.changes.len(), 1);
    let stored = back.changes[0].blob.as_ref().expect("блоб на месте");
    let text = keys::open_from_wire(&enc, stored).expect("расшифровка");
    assert_eq!(String::from_utf8_lossy(&text), "хост prod-01 🔐");

    let status = api
        .status(&session.access_token)
        .await
        .expect("состояние синхронизации");
    assert_eq!(status.records, 1);

    // Обновление токена: старый refresh после этого недействителен.
    let refreshed = api
        .refresh(&session.refresh_token)
        .await
        .expect("обновление токена");
    assert_ne!(refreshed.refresh_token, session.refresh_token);

    api.logout(&refreshed.refresh_token).await.expect("выход");
}

#[tokio::test]
#[ignore = "нужен поднятый сервер"]
async fn a_stale_revision_comes_back_as_a_conflict_inside_a_successful_answer() {
    // Главное расхождение контракта с сервером: кода 409 нет, конфликт лежит
    // в теле ответа 200. Обработчик 409 не сработал бы ни разу.
    let api = ApiClient::new(API).expect("клиент");
    let (email, password) = credentials();
    let master = keys::master_key(&email, &password).expect("мастер-ключ");
    let auth = keys::auth_hash(&master, &password).expect("auth_hash");
    let enc = keys::encryption_key(&master).expect("ключ");

    // Случайный ключ записей, как при настоящей регистрации. Прежде здесь
    // лежали нули, и тест не заметил бы, что приложение эту обёртку вообще
    // не открывает (#180c).
    let mut symmetric = [0u8; 32];
    getrandom::fill(&mut symmetric).expect("случайность");
    let protected = keys::seal_for_wire(&enc, &symmetric).expect("защита ключа");

    register_account(&api, &email, &auth, &protected, &symmetric).await;
    let token = verification_token(&email).await;
    api.verify_email(&token).await.expect("подтверждение");
    let session = api.login(&email, &auth, None).await.expect("вход");

    let id = uuid::Uuid::new_v4().to_string();
    let make = |revision: i64| WireRecord {
        id: id.clone(),
        kind: "host".into(),
        blob: Some(keys::seal_for_wire(&enc, b"x").expect("шифрование")),
        nonce: Some(String::new()),
        revision,
        deleted: false,
    };

    // Заводим запись, затем поднимаем её версию — сервер уходит вперёд.
    api.push(&session.access_token, 0, &[make(1)])
        .await
        .expect("создание");
    let second = api
        .push(&session.access_token, 0, &[make(1)])
        .await
        .expect("вторая версия");
    let server_revision = second.accepted[0].revision;
    assert_eq!(server_revision, 2, "существующая запись растёт на единицу");

    // Теперь шлём заведомо устаревшую версию.
    let conflicted = api
        .push(&session.access_token, 0, &[make(1)])
        .await
        .expect("сервер отвечает 200, а не ошибкой");

    assert!(
        conflicted.accepted.is_empty(),
        "отвергнутая запись в accepted попадать не должна"
    );
    assert_eq!(conflicted.conflicts.len(), 1, "конфликт должен быть назван");
    assert_eq!(conflicted.conflicts[0].id, id);
    assert!(
        conflicted.conflicts[0].server_revision >= server_revision,
        "сервер сообщает свою версию"
    );
}

#[tokio::test]
#[ignore = "нужен поднятый сервер"]
async fn an_unverified_account_is_refused_sync() {
    // Требование подтверждённой почты. Проверяем именно клиентом: отказ
    // должен приходить разобранным, а не «что-то пошло не так».
    let api = ApiClient::new(API).expect("клиент");
    let (email, password) = credentials();
    let master = keys::master_key(&email, &password).expect("мастер-ключ");
    let auth = keys::auth_hash(&master, &password).expect("auth_hash");
    let enc = keys::encryption_key(&master).expect("ключ");
    let symmetric = [0u8; 32];
    let protected = keys::seal_for_wire(&enc, &symmetric).expect("защита ключа");

    register_account(&api, &email, &auth, &protected, &symmetric).await;
    let session = api.login(&email, &auth, None).await.expect("вход");

    let err = api
        .status(&session.access_token)
        .await
        .expect_err("без подтверждённой почты синхронизация закрыта");
    let api_err = err.downcast_ref::<ApiError>().expect("разобранная ошибка");
    assert_eq!(api_err.status, 403, "получено: {api_err}");
}

#[tokio::test]
#[ignore = "нужен поднятый сервер"]
async fn a_garbage_token_is_reported_as_an_expired_one() {
    // Один код на все беды с токеном — отсутствующий, просроченный,
    // подделанный, мусорный. Клиент обязан понять это как «нужен вход».
    let api = ApiClient::new(API).expect("клиент");
    let err = api
        .status("явно-не-токен")
        .await
        .expect_err("мусорный токен должен быть отвергнут");
    let api_err = err.downcast_ref::<ApiError>().expect("разобранная ошибка");
    assert_eq!(api_err.status, 401);
    assert!(api_err.needs_refresh(), "получено: {api_err}");
}

// ─────────────────────────────────────────────────────────────────────────
// Движок целиком: два устройства и один аккаунт
// ─────────────────────────────────────────────────────────────────────────

/// Завести локальную базу с миграциями — изображаем отдельное устройство.
async fn device_db() -> sqlx::SqlitePool {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:")
        .await
        .expect("база устройства");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("миграции");
    pool
}

async fn verified_account(api: &ApiClient) -> ([u8; 32], String, String) {
    let (email, password) = credentials();
    let master = keys::master_key(&email, &password).expect("мастер-ключ");
    let auth = keys::auth_hash(&master, &password).expect("auth_hash");
    let enc = keys::encryption_key(&master).expect("ключ");
    let symmetric = [0u8; 32];
    let protected = keys::seal_for_wire(&enc, &symmetric).expect("защита ключа");

    register_account(api, &email, &auth, &protected, &symmetric).await;
    let token = verification_token(&email).await;
    api.verify_email(&token).await.expect("подтверждение");
    (enc, email, auth)
}

/// Ключи записей так, как их получает приложение: снимая обёртку с сервера.
///
/// Именно **тем же** вызовом, что и `sync_login`. Повтори тест вывод ключа у
/// себя — и он снова доказывал бы сервер, а не клиента: ровно так пропустили
/// то, что приложение обёртку не открывает вовсе.
fn record_keys(enc: &[u8; 32], session: &Session) -> darwaza_lib::sync::RecordKeys {
    let (current, legacy) = keys::record_keys(enc, session.protected_symmetric_key.as_deref())
        .expect("ключ записей из обёртки");
    darwaza_lib::sync::RecordKeys { current, legacy }
}

/// Главная проверка всей затеи: хост, заведённый на одной машине, появляется
/// на другой — со всеми значениями и с правильной группой.
#[tokio::test]
#[ignore = "нужен поднятый сервер"]
async fn a_host_created_on_one_device_appears_on_the_other() {
    let api = ApiClient::new(API).expect("клиент");
    let (enc, email, auth) = verified_account(&api).await;

    let first = device_db().await;
    let second = device_db().await;

    // Первое устройство: заводим группу и хост.
    sqlx::query("INSERT INTO groups (name, sort_order, sync_id) VALUES ('Prod', 0, ?1)")
        .bind(darwaza_lib::db::new_sync_id())
        .execute(&first)
        .await
        .expect("группа");
    let group_id: i64 = sqlx::query_scalar("SELECT id FROM groups WHERE name = 'Prod'")
        .fetch_one(&first)
        .await
        .expect("номер группы");
    sqlx::query(
        "INSERT INTO hosts (group_id, name, hostname, port, username, auth_method, \
            keep_alive_interval, sync_id) \
         VALUES (?1, 'prod-01', 'prod-01.internal', 2222, 'deploy', 'publickey', 45, ?2)",
    )
    .bind(group_id)
    .bind(darwaza_lib::db::new_sync_id())
    .execute(&first)
    .await
    .expect("хост");

    let session = api.login(&email, &auth, None).await.expect("вход первого");
    let sent = darwaza_lib::sync::engine::run(
        &first,
        &record_keys(&enc, &session),
        &api,
        &session.access_token,
    )
    .await
    .expect("проход на первом устройстве");
    assert_eq!(sent.sent, 2, "должны уехать группа и хост: {sent:?}");
    assert!(sent.failed.is_empty(), "ошибок быть не должно: {sent:?}");

    // Второе устройство: тот же аккаунт, пустая база.
    let session2 = api.login(&email, &auth, None).await.expect("вход второго");
    let got = darwaza_lib::sync::engine::run(
        &second,
        &record_keys(&enc, &session2),
        &api,
        &session2.access_token,
    )
    .await
    .expect("проход на втором устройстве");
    assert_eq!(got.received, 2, "должны приехать группа и хост: {got:?}");

    let row: (String, String, i64, String, i64, Option<i64>) = sqlx::query_as(
        "SELECT name, hostname, port, username, keep_alive_interval, group_id \
         FROM hosts WHERE name = 'prod-01'",
    )
    .fetch_one(&second)
    .await
    .expect("хост приехал");

    assert_eq!(row.1, "prod-01.internal");
    assert_eq!(row.2, 2222, "порт не должен подмениться умолчанием");
    assert_eq!(row.3, "deploy");
    assert_eq!(row.4, 45, "интервал не должен стать умолчанием");
    assert!(row.5.is_some(), "хост должен попасть в свою группу");

    let group_name: String = sqlx::query_scalar("SELECT name FROM groups WHERE id = ?1")
        .bind(row.5.unwrap())
        .fetch_one(&second)
        .await
        .expect("группа приехала");
    assert_eq!(group_name, "Prod", "и именно в свою, а не в чужую");
}

/// W037: правка на двух машинах, ни одна из которых не видела чужой, даёт
/// **две записи, а не потерю**.
///
/// Это единственный случай, где теряется не версия, а сама вторая запись —
/// и он ближе всего к тому, как люди работают в дороге.
#[tokio::test]
#[ignore = "нужен поднятый сервер"]
async fn two_devices_editing_offline_end_up_with_two_records_not_a_loss() {
    let api = ApiClient::new(API).expect("клиент");
    let (enc, email, auth) = verified_account(&api).await;

    let first = device_db().await;
    let second = device_db().await;
    let s1 = api.login(&email, &auth, None).await.expect("вход первого");
    let s2 = api.login(&email, &auth, None).await.expect("вход второго");

    // Общая отправная точка: запись создана и разошлась на оба устройства.
    let sync_id = darwaza_lib::db::new_sync_id();
    sqlx::query(
        "INSERT INTO snippets (name, command, sort_order, sync_id) \
         VALUES ('перезапуск', 'systemctl restart app', 0, ?1)",
    )
    .bind(&sync_id)
    .execute(&first)
    .await
    .expect("сниппет");
    darwaza_lib::sync::engine::run(&first, &record_keys(&enc, &s1), &api, &s1.access_token)
        .await
        .expect("первая отправка");
    darwaza_lib::sync::engine::run(&second, &record_keys(&enc, &s2), &api, &s2.access_token)
        .await
        .expect("получение на втором");

    // Теперь оба правят её, не видя друг друга.
    sqlx::query("UPDATE snippets SET command = 'systemctl reload app' WHERE sync_id = ?1")
        .bind(&sync_id)
        .execute(&first)
        .await
        .expect("правка на первом");
    sqlx::query("UPDATE snippets SET command = 'systemctl restart app --now' WHERE sync_id = ?1")
        .bind(&sync_id)
        .execute(&second)
        .await
        .expect("правка на втором");

    // Первый успел раньше.
    let early =
        darwaza_lib::sync::engine::run(&first, &record_keys(&enc, &s1), &api, &s1.access_token)
            .await
            .expect("отправка первого");
    assert_eq!(early.sent, 1, "правка первого должна уехать: {early:?}");

    // Второй приходит с устаревшей версией.
    let late =
        darwaza_lib::sync::engine::run(&second, &record_keys(&enc, &s2), &api, &s2.access_token)
            .await
            .expect("проход второго");
    assert_eq!(
        late.conflicts, 1,
        "должен быть ровно один конфликт: {late:?}"
    );

    // И вот главное: на втором устройстве ДВЕ записи, а не одна.
    let commands: Vec<String> = sqlx::query_scalar("SELECT command FROM snippets ORDER BY command")
        .fetch_all(&second)
        .await
        .expect("сниппеты");

    assert_eq!(
        commands.len(),
        2,
        "правка второго должна сохраниться отдельной записью, а не пропасть: {commands:?}"
    );
    assert!(
        commands.iter().any(|c| c == "systemctl reload app"),
        "серверная версия должна приехать: {commands:?}"
    );
    assert!(
        commands.iter().any(|c| c == "systemctl restart app --now"),
        "своя версия должна остаться видимой: {commands:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Восстановление доступа ключом на бумаге (#180c)
// ─────────────────────────────────────────────────────────────────────────

/// Полный круг: записи уехали, пароль забыт, доступ возвращён бумагой.
///
/// Ради этого всё и делалось, и проверять это можно только вживую: обе
/// стороны выводят ключи по одной спецификации на разных языках, и
/// расхождение хотя бы в байте не видно ничем, кроме «ключ не подошёл».
///
/// Главное здесь — последняя проверка: **запись читается после
/// восстановления**. Всё остальное можно сделать правильно и всё равно
/// вернуть человеку пустую базу, если под бумагу положили не тот ключ.
#[tokio::test]
#[ignore = "нужен поднятый сервер"]
async fn a_paper_key_brings_back_access_and_the_records() {
    let api = ApiClient::new(API).expect("клиент");
    let (email, password) = credentials();

    // ── Регистрация так, как её делает приложение ────────────────────────
    let master = keys::master_key(&email, &password).expect("мастер-ключ");
    let auth = keys::auth_hash(&master, &password).expect("auth_hash");
    let enc = keys::encryption_key(&master).expect("ключ из пароля");

    let mut symmetric = [0u8; 32];
    getrandom::fill(&mut symmetric).expect("случайность");
    let protected = keys::seal_for_wire(&enc, &symmetric).expect("обёртка паролем");

    let rk = darwaza_lib::recovery_key::generate().expect("случайность");
    let paper = darwaza_lib::recovery_key::format(&rk);
    let recovery = keys::recovery_keys(&rk).expect("ключи восстановления");
    let protected_rk = keys::seal_for_wire(&recovery.enc, &symmetric).expect("обёртка бумагой");

    api.register(
        &email,
        &auth,
        &protected,
        &recovery.auth_hash,
        &protected_rk,
    )
    .await
    .expect("регистрация");
    let token = verification_token(&email).await;
    api.verify_email(&token).await.expect("подтверждение");

    // ── Запись уезжает на сервер под ключом записей ──────────────────────
    let device = device_db().await;
    sqlx::query(
        "INSERT INTO snippets (name, command, sort_order, sync_id) \
         VALUES ('выкатка', 'deploy.sh --prod', 0, ?1)",
    )
    .bind(darwaza_lib::db::new_sync_id())
    .execute(&device)
    .await
    .expect("сниппет");

    let session = api.login(&email, &auth, None).await.expect("вход");
    let keys_now = record_keys(&enc, &session);
    let sent = darwaza_lib::sync::engine::run(&device, &keys_now, &api, &session.access_token)
        .await
        .expect("отправка");
    assert_eq!(sent.sent, 1, "запись должна уехать: {sent:?}");

    // ── Пароль забыт. В руках только бумага ──────────────────────────────
    let begun = api
        .recover_begin(&email, &recovery.auth_hash)
        .await
        .expect("начало восстановления");

    let opened = keys::open_from_wire(&recovery.enc, &begun.protected_symmetric_key_rk)
        .expect("обёртка обязана открыться ключом с бумаги");
    let recovered: [u8; 32] = opened.try_into().expect("32 байта");
    assert_eq!(
        recovered, symmetric,
        "под бумагой обязан лежать ТОТ ЖЕ ключ записей"
    );

    // Бумага, набранная руками с дефисами и в нижнем регистре, — тот же ключ.
    let retyped = darwaza_lib::recovery_key::parse(&paper.to_lowercase()).expect("разбор с бумаги");
    let retyped_keys = keys::recovery_keys(&retyped).expect("ключи из набранного");
    assert_eq!(retyped_keys.auth_hash, recovery.auth_hash);

    // ── Новый пароль и новая бумага — одним запросом ─────────────────────
    let new_password = format!("{password}-новый");
    let new_master = keys::master_key(&email, &new_password).expect("новый мастер-ключ");
    let new_auth = keys::auth_hash(&new_master, &new_password).expect("новый auth_hash");
    let new_enc = keys::encryption_key(&new_master).expect("новый ключ из пароля");
    let new_protected = keys::seal_for_wire(&new_enc, &recovered).expect("новая обёртка паролем");

    let next_rk = darwaza_lib::recovery_key::generate().expect("случайность");
    let next = keys::recovery_keys(&next_rk).expect("следующие ключи");
    let next_protected = keys::seal_for_wire(&next.enc, &recovered).expect("новая обёртка бумагой");

    api.recover_complete(
        &begun.recovery_token,
        &new_auth,
        &new_protected,
        &next.auth_hash,
        &next_protected,
    )
    .await
    .expect("завершение восстановления");

    // ── Старый пароль больше не пускает, новый пускает ───────────────────
    assert!(
        api.login(&email, &auth, None).await.is_err(),
        "старый пароль обязан перестать работать"
    );
    let after = api
        .login(&email, &new_auth, None)
        .await
        .expect("вход новым паролем");

    // ── И главное: запись на месте и читается ────────────────────────────
    let fresh = device_db().await;
    let keys_after = record_keys(&new_enc, &after);
    let got = darwaza_lib::sync::engine::run(&fresh, &keys_after, &api, &after.access_token)
        .await
        .expect("проход после восстановления");
    assert_eq!(got.received, 1, "запись должна приехать: {got:?}");
    assert!(got.failed.is_empty(), "ничего не должно ломаться: {got:?}");

    let command: String = sqlx::query_scalar("SELECT command FROM snippets WHERE name = 'выкатка'")
        .fetch_one(&fresh)
        .await
        .expect("сниппет после восстановления");
    assert_eq!(
        command, "deploy.sh --prod",
        "восстановление вернуло доступ, но не данные"
    );

    // ── Использованная бумага больше не работает ─────────────────────────
    assert!(
        api.recover_begin(&email, &recovery.auth_hash)
            .await
            .is_err(),
        "прежний ключ обязан перестать действовать"
    );
    api.recover_begin(&email, &next.auth_hash)
        .await
        .expect("новый ключ обязан работать");
}

/// Неверный ключ и несуществующий адрес отвечают одинаково.
///
/// Иначе ручка становится перечислителем аккаунтов: по разнице ответов
/// узнают, что адрес заведён, а заодно и что ключ у него ещё не использован.
#[tokio::test]
#[ignore = "нужен поднятый сервер"]
async fn a_wrong_key_and_an_unknown_address_look_the_same() {
    let api = ApiClient::new(API).expect("клиент");
    let (email, password) = credentials();
    let master = keys::master_key(&email, &password).expect("мастер-ключ");
    let auth = keys::auth_hash(&master, &password).expect("auth_hash");
    let enc = keys::encryption_key(&master).expect("ключ");
    let symmetric = [3u8; 32];
    let protected = keys::seal_for_wire(&enc, &symmetric).expect("обёртка");
    register_account(&api, &email, &auth, &protected, &symmetric).await;

    let wrong = keys::recovery_keys(&darwaza_lib::recovery_key::generate().unwrap()).unwrap();
    let known = api
        .recover_begin(&email, &wrong.auth_hash)
        .await
        .expect_err("неверный ключ обязан отказать");
    // Адрес обязан быть правильным по форме, но несуществующим: адрес с
    // кириллицей отвергается проверкой формата и до восстановления не
    // доходит вовсе — сравнивать было бы нечего.
    let unknown = api
        .recover_begin("nobody-here@example.com", &wrong.auth_hash)
        .await
        .expect_err("несуществующий адрес обязан отказать");

    let a = known
        .downcast_ref::<ApiError>()
        .expect("разобранная ошибка");
    let b = unknown
        .downcast_ref::<ApiError>()
        .expect("разобранная ошибка");
    assert_eq!(a.status, b.status, "разные коды выдают, что аккаунт есть");
    assert_eq!(a.code, b.code, "разные причины выдают то же самое");
}
