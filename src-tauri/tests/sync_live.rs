//! Сквозной прогон синхронизации против живого сервера Darwaza.
//!
//! Проверяет не наш код в отдельности, а **стык двух реализаций**: клиент на
//! Rust и сервер на TypeScript обязаны понимать вывод ключей, форматы блобов
//! и семантику курсора байт в байт. Модульные тесты этого не покажут — обе
//! стороны по отдельности «работают».
//!
//! **Ходит через код приложения, а не через свою копию.** Раньше вывод ключей,
//! `auth_hash` и шифрование были написаны прямо здесь — и тест сходился с
//! сервером, ничего не говоря о клиенте: он и был вторым клиентом. Зелёный
//! тест, проверяющий не то, что кажется, дороже отсутствующего — отсутствующий
//! виден, а этот успокаивает. Теперь всё берётся из `darwaza_lib::sync::keys`,
//! и расхождение приложения со спецификацией валит этот тест.
//!
//! Помечен `#[ignore]`: нужен поднятый сервер.
//! ```bash
//! # в другой сессии: SMTP_HOST= pnpm --filter @darwaza/api dev
//! cargo test --test sync_live -- --ignored --nocapture
//! ```
//!
//! Данные намеренно НЕРОВНЫЕ: почта в смешанном регистре с пробелами, пароль
//! с не-ASCII символами, содержимое с кириллицей и эмодзи. Ровные данные
//! прячут ровно то, что ломается у первого живого пользователя.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;

// Вывод ключей и шифрование берутся у ПРИЛОЖЕНИЯ. Своей копии здесь больше
// нет: копия проверяла бы саму себя.
use darwaza_lib::sync::keys::{
    auth_hash as lib_auth_hash, encryption_key, master_key as lib_master_key, normalize_email,
    open_from_wire, seal_for_wire, KDF_ITERATIONS, KDF_MEMORY_KIB, KDF_PARALLELISM,
};

const API: &str = "http://127.0.0.1:8080/api/v1";

fn master_key(email: &str, password: &str) -> [u8; 32] {
    lib_master_key(email, password).expect("вывод мастер-ключа")
}

fn auth_hash(master: &[u8; 32], password: &str) -> String {
    lib_auth_hash(master, password).expect("вывод auth_hash")
}

fn stretched_enc_key(master: &[u8; 32]) -> [u8; 32] {
    encryption_key(master).expect("вывод ключа шифрования")
}

fn seal(key: &[u8; 32], plaintext: &[u8]) -> String {
    seal_for_wire(key, plaintext).expect("шифрование")
}

fn open(key: &[u8; 32], blob_b64: &str) -> Vec<u8> {
    open_from_wire(key, blob_b64).expect("расшифровка")
}

#[tokio::test]
#[ignore = "нужен поднятый сервер: SMTP_HOST= pnpm --filter @darwaza/api dev"]
async fn key_derivation_matches_the_specification() {
    // Не про сервер, а про то, что наши три шага дают разные и устойчивые
    // значения. Если authHash совпадёт с masterKey — на сервер уедет ключ
    // шифрования, и всё остальное потеряет смысл.
    let email = "  Test.User@Example.COM  ";
    let password = "пароль-с-ümlaut-🔑";

    let mk = master_key(email, password);
    let ah = auth_hash(&mk, password);
    let ek = stretched_enc_key(&mk);

    assert_ne!(
        B64.encode(mk),
        ah,
        "authHash не должен совпадать с masterKey — иначе ключ уедет на сервер"
    );
    assert_ne!(mk, ek, "ключ шифрования выводится отдельно");

    // Тот же вход — тот же выход. Иначе человек не войдёт со второго устройства.
    assert_eq!(mk, master_key(email, password));
    // Регистр и пробелы в почте не должны менять ключ.
    assert_eq!(mk, master_key("test.user@example.com", password));
    // Другая почта — другой ключ.
    assert_ne!(mk, master_key("other@example.com", password));
}

// ─────────────────────────────────────────────────────────────────────────
// Полный путь: регистрация → подтверждение → вход → обмен → конфликт
// ─────────────────────────────────────────────────────────────────────────

use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_email() -> String {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("часы")
        .as_millis();
    // Регистр и пробелы намеренно: сервер обязан их нормализовать.
    format!("  Sync.Probe.{stamp}@Example.COM  ")
}

/// Куда `FileMailer` кладёт письма, когда `SMTP_HOST` пуст.
///
/// Путь считается от каталога этого пакета, а не от текущего каталога: тест
/// запускают и из `src-tauri`, и из корня, и относительный путь ломался бы
/// молча. Раскладка — два репозитория рядом, как описано в `START-HERE.md`:
/// `darwaza/desktop` и `darwaza/server`, а внутри второго пакет API лежит в
/// подкаталоге `server/`.
///
/// Переопределяется `DARWAZA_MAIL_OUTBOX`, если репозитории лежат иначе.
fn mail_outbox() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("DARWAZA_MAIL_OUTBOX") {
        return dir.into();
    }
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../server/server/.mail-outbox")
}

/// Достать токен подтверждения из письма, положенного файлом.
///
/// Файл называется по адресу и **перезаписывается** каждым новым письмом:
/// `FileMailer` намеренно не добавляет время, чтобы при разработке под рукой
/// было последнее письмо. Значит, читать его надо сразу после нужного шага —
/// после `resend-verification` прежний токен из файла уже не достать.
fn verification_token(email: &str) -> String {
    let file = mail_outbox().join(format!("{}.txt", normalize_email(email)));
    let text = std::fs::read_to_string(&file).unwrap_or_else(|e| {
        panic!(
            "письмо {} не прочиталось: {e}\n\
             Проверьте, что API поднят с пустым SMTP_HOST (`SMTP_HOST= pnpm dev:api`) \
             и что репозиторий server лежит рядом с desktop.",
            file.display()
        )
    });
    // Токен — 32 байта в base64url, то есть без '+', '/' и '='.
    let start = text.find("token=").expect("в письме нет токена") + "token=".len();
    text[start..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

async fn post(client: &reqwest::Client, path: &str, body: Value) -> reqwest::Response {
    client
        .post(format!("{API}{path}"))
        .json(&body)
        .send()
        .await
        .unwrap_or_else(|e| panic!("POST {path}: {e}"))
}

/// Идентификатор той же формы, что и в базе приложения.
fn uuid_like() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("случайность");
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

#[tokio::test]
#[ignore = "нужен поднятый сервер: SMTP_HOST= pnpm --filter @darwaza/api dev"]
async fn full_sync_path_against_live_server() {
    let client = reqwest::Client::new();
    let email = unique_email();
    // Пароль с не-ASCII: ровные данные прячут ровно то, что ломается у первого
    // живого пользователя.
    let password = "правильная-лошадь-ünlü-🔑";

    let mk = master_key(&email, password);
    let ah = auth_hash(&mk, password);
    let enc = stretched_enc_key(&mk);

    // Ключ данных случайный: смена пароля должна перешифровывать его одного,
    // а не все записи.
    let mut symmetric = [0u8; 32];
    getrandom::fill(&mut symmetric).expect("случайность");
    let protected = seal(&enc, &symmetric);

    // ── 1. Регистрация ───────────────────────────────────────────────────
    let res = post(
        &client,
        "/auth/register",
        json!({
            "email": email,
            "authHash": ah,
            "protectedSymmetricKey": protected,
            "kdf": { "memoryKib": KDF_MEMORY_KIB, "iterations": KDF_ITERATIONS,
                     "parallelism": KDF_PARALLELISM },
        }),
    )
    .await;
    let status = res.status();
    let reg_body: Value = res.json().await.unwrap_or(Value::Null);
    assert!(
        status.is_success(),
        "регистрация: {status}, тело: {reg_body}"
    );

    // ── 2. До подтверждения синхронизация закрыта ────────────────────────
    // Проверяем именно код: из общего «неверный запрос» клиент не поймёт, что
    // надо показать кнопку «отправить письмо заново».
    let res = client
        .get(format!("{API}/sync/changes?since=0"))
        .send()
        .await
        .expect("changes без токена");
    assert_eq!(res.status(), 401, "без входа — не пускать");

    // ── 3. Подтверждение почты ───────────────────────────────────────────
    let token = verification_token(&email);
    let res = post(&client, "/auth/verify-email", json!({ "token": token })).await;
    assert_eq!(res.status(), 204, "подтверждение должно пройти");

    let res = post(&client, "/auth/verify-email", json!({ "token": token })).await;
    assert_eq!(res.status(), 400, "повторное подтверждение тем же токеном");

    // ── 4. Вход ──────────────────────────────────────────────────────────
    let res = post(
        &client,
        "/auth/login",
        json!({
            // Грязное значение намеренно: весь путь проходится ОДНИМ И ТЕМ
            // ЖЕ вводом. Проверка каждого эндпоинта по отдельности этот класс
            // ошибок не ловит — он живёт на стыке, что и подтвердилось.
            "email": email, "authHash": ah,
            "device": { "name": "probe", "platform": "linux" },
        }),
    )
    .await;
    let status = res.status();
    let body: Value = res.json().await.expect("тело входа");
    assert!(status.is_success(), "вход: {status}, тело: {body}");
    let access = body["accessToken"]
        .as_str()
        .expect("accessToken")
        .to_string();

    // ── 5. Ключ данных расшифровывается тем же путём ─────────────────────
    let returned = body["protectedSymmetricKey"].as_str().expect("ключ");
    assert_eq!(
        open(&enc, returned),
        symmetric.to_vec(),
        "сервер обязан вернуть ключ байт в байт — иначе данные не расшифровать"
    );

    println!("вход прошёл, ключ данных сошёлся");

    let auth = |req: reqwest::RequestBuilder| req.bearer_auth(&access);

    // ── 6. Отправка записи ───────────────────────────────────────────────
    // Содержимое с кириллицей и эмодзи: сервер его не читает, но обязан
    // вернуть байт в байт.
    let payload = json!({
        "name": "сервер «Прод» 🚀",
        "hostname": "prod-01.example.internal",
        "description": "Ünlü sunucu, 日本語",
    });
    let plain = serde_json::to_vec(&payload).expect("сериализация");
    let blob_b64 = seal(&symmetric, &plain);
    let blob = B64.decode(&blob_b64).expect("base64");
    let (nonce_part, cipher_part) = blob.split_at(12);

    let record_id = "11111111-2222-4333-8444-555555555555";
    let res = auth(client.post(format!("{API}/sync/push")))
        .json(&json!({
            "baseCursor": 0,
            "changes": [{
                "id": record_id, "type": "host",
                "blob": B64.encode(cipher_part), "nonce": B64.encode(nonce_part),
                "revision": 1, "deleted": false,
            }],
        }))
        .send()
        .await
        .expect("push");
    let status = res.status();
    let body: Value = res.json().await.expect("тело push");
    assert!(status.is_success(), "push: {status}, тело: {body}");
    assert!(
        body["conflicts"]
            .as_array()
            .map(|c| c.is_empty())
            .unwrap_or(true),
        "первая отправка не должна конфликтовать: {body}"
    );
    let server_cursor = body["cursor"].as_i64().expect("курсор");

    // Сервер обязан сообщить присвоенную версию. Без этого клиенту неоткуда
    // узнать, что записывать себе, и правило «прислал N — стало N+1»
    // осталось бы договорённостью на словах.
    let accepted = body["accepted"].as_array().expect("список принятых");
    assert_eq!(
        accepted.len(),
        1,
        "принятая запись должна быть в accepted: {body}"
    );
    assert_eq!(accepted[0]["id"].as_str(), Some(record_id));
    let assigned = accepted[0]["revision"].as_i64().expect("версия");
    assert!(assigned >= 1, "версия должна быть осмысленной: {body}");
    println!("push прошёл, курсор {server_cursor}, присвоена версия {assigned}");

    // ── 7. Приём «вторым устройством» ────────────────────────────────────
    let res = auth(client.get(format!("{API}/sync/changes?since=0")))
        .send()
        .await
        .expect("changes");
    assert!(res.status().is_success(), "changes: {}", res.status());
    let body: Value = res.json().await.expect("тело changes");
    let changes = body["changes"].as_array().expect("массив изменений");
    assert_eq!(changes.len(), 1, "должна вернуться одна запись: {body}");

    let got = &changes[0];
    assert_eq!(got["id"].as_str(), Some(record_id));
    let mut back = B64
        .decode(got["nonce"].as_str().expect("nonce"))
        .expect("b64");
    back.extend_from_slice(
        &B64.decode(got["blob"].as_str().expect("blob"))
            .expect("b64"),
    );
    let decrypted: Value =
        serde_json::from_slice(&open(&symmetric, &B64.encode(back))).expect("разбор");
    assert_eq!(
        decrypted, payload,
        "содержимое обязано вернуться байт в байт, включая кириллицу и эмодзи"
    );
    println!("приём прошёл, содержимое совпало");

    // ── 8. Конфликт по версии ────────────────────────────────────────────
    // Отправляем с устаревшей версией: сервер обязан вернуть свою, а не
    // молча перезаписать.
    let res = auth(client.post(format!("{API}/sync/push")))
        .json(&json!({
            "baseCursor": server_cursor,
            "changes": [{
                "id": record_id, "type": "host",
                "blob": B64.encode(cipher_part), "nonce": B64.encode(nonce_part),
                "revision": 1, "deleted": false,
            }],
        }))
        .send()
        .await
        .expect("push с устаревшей версией");
    let body: Value = res.json().await.expect("тело");

    // Вторая отправка той же версии принимается: правило сервера — принять
    // совпадающую и сохранить следующую. Конфликта здесь нет и быть не должно.
    assert!(
        body["conflicts"]
            .as_array()
            .map(|c| c.is_empty())
            .unwrap_or(false),
        "совпадающая версия не конфликт: {body}"
    );

    // Правило сервера: принимает совпадающую версию и сохраняет СЛЕДУЮЩУЮ.
    // Значит после двух отправок у него 2, а устаревшая для нас — 1.
    // Ноль послать нельзя: схема требует минимум единицу.
    let res = auth(client.post(format!("{API}/sync/push")))
        .json(&json!({
            "baseCursor": 0,
            "changes": [{
                "id": record_id, "type": "host",
                "blob": B64.encode(cipher_part), "nonce": B64.encode(nonce_part),
                "revision": 1, "deleted": false,
            }],
        }))
        .send()
        .await
        .expect("push с устаревшей версией");
    let stale: Value = res.json().await.expect("тело");
    let conflicts = stale["conflicts"].as_array().expect("массив конфликтов");
    assert_eq!(
        conflicts.len(),
        1,
        "заведомо устаревшая версия обязана дать конфликт: {stale}"
    );
    println!("конфликт по версии определяется правильно");

    // ── 9. Удаление ──────────────────────────────────────────────────────
    let current_revision = conflicts[0]["serverRevision"]
        .as_i64()
        .expect("сервер обязан сообщить свою версию, иначе клиенту нечем разрешать конфликт");
    let res = auth(client.post(format!("{API}/sync/push")))
        .json(&json!({
            "baseCursor": server_cursor,
            "changes": [{
                "id": record_id, "type": "host", "blob": null, "nonce": null,
                "revision": current_revision, "deleted": true,
            }],
        }))
        .send()
        .await
        .expect("удаление");
    let status = res.status();
    let body: Value = res.json().await.expect("тело");
    assert!(status.is_success(), "удаление: {status}, тело: {body}");
    // У надгробия версия растёт так же — её тоже надо записать, иначе
    // следующее изменение уйдёт с устаревшей и даст конфликт из ниоткуда.
    let accepted = body["accepted"].as_array().expect("принятые при удалении");
    assert_eq!(
        accepted.len(),
        1,
        "удаление тоже принято и версировано: {body}"
    );

    let res = auth(client.get(format!("{API}/sync/changes?since=0")))
        .send()
        .await
        .expect("changes после удаления");
    let body: Value = res.json().await.expect("тело");
    let changes = body["changes"].as_array().expect("массив");
    assert_eq!(
        changes.len(),
        1,
        "надгробие должно приехать, а не исчезнуть"
    );
    assert_eq!(
        changes[0]["deleted"].as_bool(),
        Some(true),
        "запись обязана прийти помеченной удалённой: {body}"
    );
    println!("удаление доезжает надгробием");

    // ── 10. Постраничность ───────────────────────────────────────────────
    // Курсор в ответе — позиция ПОСЛЕДНЕЙ ОТДАННОЙ записи. Отдай сервер
    // текущий счётчик, клиент перескочил бы через непрочитанное.
    for i in 0..3 {
        let p = json!({ "n": i });
        let b = seal(&symmetric, &serde_json::to_vec(&p).unwrap());
        let raw = B64.decode(&b).unwrap();
        let (n, c) = raw.split_at(12);
        let res = auth(client.post(format!("{API}/sync/push")))
            .json(&json!({
                "baseCursor": 0,
                "changes": [{
                    "id": format!("99999999-0000-4000-8000-00000000000{i}"),
                    "type": "snippet", "blob": B64.encode(c), "nonce": B64.encode(n),
                    "revision": 1, "deleted": false,
                }],
            }))
            .send()
            .await
            .expect("push страницы");
        assert!(res.status().is_success(), "push {i}");
    }

    let res = auth(client.get(format!("{API}/sync/changes?since=0&limit=2")))
        .send()
        .await
        .expect("первая страница");
    let page1: Value = res.json().await.expect("тело");
    assert_eq!(page1["changes"].as_array().unwrap().len(), 2);
    assert_eq!(
        page1["hasMore"].as_bool(),
        Some(true),
        "должно быть ещё: {page1}"
    );

    let cursor1 = page1["cursor"].as_i64().expect("курсор страницы");
    let res = auth(client.get(format!("{API}/sync/changes?since={cursor1}&limit=10")))
        .send()
        .await
        .expect("вторая страница");
    let page2: Value = res.json().await.expect("тело");
    let rest = page2["changes"].as_array().unwrap();

    // Ключевая проверка: сумма страниц равна общему числу записей. Если бы
    // курсор указывал на счётчик, а не на последнюю отданную запись, часть
    // записей потерялась бы молча.
    let res = auth(client.get(format!("{API}/sync/changes?since=0&limit=500")))
        .send()
        .await
        .expect("всё разом");
    let all: Value = res.json().await.expect("тело");
    let total = all["changes"].as_array().unwrap().len();
    assert_eq!(
        2 + rest.len(),
        total,
        "постранично должно приехать столько же, сколько разом: {} + {} против {total}",
        2,
        rest.len()
    );
    println!("постраничность не теряет записей: {total} всего");

    // ── Проверка НАШИМ клиентом, а не самодельным запросом ───────────────
    //
    // Идёт последней: она добавляет запись в аккаунт, и стоя выше сбивала бы
    // счёт в проверках постраничности.
    //
    // Всё, что ниже в этом тесте, посылается вручную через `json!` — и потому
    // проверяет договор с сервером, но не то, что отправляет приложение.
    // Ровно в эту щель прошла ошибка: `#[serde(skip_serializing_if)]`
    // выбрасывал `blob` и `nonce` у надгробия, сервер отвечал
    // `missingProperty: "blob"`, и одно удаление роняло весь проход. Тест был
    // зелёный, потому что надгробие в нём собиралось руками.
    //
    // Поэтому надгробие отправляем настоящим `ApiClient` приложения.
    {
        use darwaza_lib::sync::api::{ApiClient, WireRecord};

        // Клиент ждёт базу вместе с `/api/v1` — ровно то, что человек вводит в
        // настройках приложения.
        let app_client = ApiClient::new(API).expect("клиент");
        let tombstone = WireRecord {
            id: uuid_like(),
            kind: "host".into(),
            blob: None,
            nonce: None,
            revision: 1,
            deleted: true,
        };
        let result = app_client
            .push(&access, 0, std::slice::from_ref(&tombstone))
            .await
            .expect("надгробие, отправленное клиентом приложения, должно приниматься");
        assert_eq!(
            result.accepted.len(),
            1,
            "сервер обязан принять надгробие с blob: null, а не пропущенным полем"
        );
        println!("надгробие уехало клиентом приложения и принято");
    }
}

/// Завести аккаунт с заданными данными — инструмент, а не проверка.
///
/// Нужен, потому что `authHash` считается на устройстве: через `curl` аккаунт
/// не заведёшь, не повторив весь вывод ключей вручную.
///
/// ```bash
/// DEMO_EMAIL=… DEMO_PASSWORD=… cargo test --test sync_live register_demo -- --ignored --nocapture
/// ```
#[tokio::test]
#[ignore = "инструмент: заводит аккаунт на живом сервере"]
async fn register_demo_account() {
    // Выходим тихо, а не падаем: это инструмент, и он попадает под общий
    // прогон `-- --ignored` вместе с настоящими проверками. Падение здесь
    // выглядит как сломанная синхронизация, хотя означает лишь «переменные не
    // заданы, потому что аккаунт заводить не просили».
    let (Ok(email), Ok(password)) = (std::env::var("DEMO_EMAIL"), std::env::var("DEMO_PASSWORD"))
    else {
        eprintln!(
            "register_demo_account пропущен: задайте DEMO_EMAIL и DEMO_PASSWORD, \
             если хотите завести аккаунт"
        );
        return;
    };

    let mk = master_key(&email, &password);
    let enc = stretched_enc_key(&mk);
    let mut symmetric = [0u8; 32];
    getrandom::fill(&mut symmetric).expect("случайность");

    let client = reqwest::Client::new();
    let res = post(
        &client,
        "/auth/register",
        json!({
            "email": email,
            "authHash": auth_hash(&mk, &password),
            "protectedSymmetricKey": seal(&enc, &symmetric),
            "kdf": { "memoryKib": KDF_MEMORY_KIB, "iterations": KDF_ITERATIONS,
                     "parallelism": KDF_PARALLELISM },
        }),
    )
    .await;
    let status = res.status();
    let body: Value = res.json().await.unwrap_or(Value::Null);
    println!("регистрация {status}: {body}");
    // 409 — аккаунт уже есть, это не повод останавливаться: инструмент
    // должен быть повторяемым, иначе им пользуются один раз.
    assert!(
        status.is_success() || status.as_u16() == 409,
        "не удалось завести аккаунт"
    );

    // Аккаунт, которым нельзя войти, бесполезен — узнать об этом лучше сейчас.
    let res = post(
        &client,
        "/auth/login",
        json!({
            "email": email,
            "authHash": auth_hash(&mk, &password),
            "device": { "name": "demo", "platform": "linux" },
        }),
    )
    .await;
    let login_status = res.status();
    let login_body: Value = res.json().await.unwrap_or(Value::Null);
    println!("вход {login_status}");
    assert!(
        login_status.is_success(),
        "вход не прошёл: {login_status}, {login_body}"
    );
}
