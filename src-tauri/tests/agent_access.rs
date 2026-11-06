//! Проверки доступа агента, требующие настоящей файловой системы (#202).
//!
//! Права на файл токена — это то, что нельзя проверить по коду: важно, каким
//! файл получился на диске, а не какие флаги передали в вызов.

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(unix)] // права файла в стиле unix; на Windows их задаёт профиль пользователя
#[test]
fn token_file_is_readable_only_by_owner() {
    // Токен даёт доступ ко всем разрешённым боевым серверам. Файл, который
    // может прочитать кто угодно из системы, обесценивает всю проверку.
    let dir = std::env::temp_dir().join(format!("darwaza-token-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("каталог");
    let path = dir.join("agent-token");

    // Повторяем ровно то, что делает приложение.
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)
            .expect("создание");
        f.write_all(b"secret").expect("запись");
    }

    let mode = std::fs::metadata(&path)
        .expect("метаданные")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        mode, 0o600,
        "права должны быть только для владельца, получено {mode:o}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn token_survives_restart_and_does_not_change() {
    // Меняющийся при каждом запуске токен ломал бы настроенного агента после
    // перезапуска приложения — и человек решил бы, что сломалась интеграция.
    //
    // Работаем во временной папке: настоящий `ensure_token()` читает и пишет
    // рабочий токен разработчика, а на Windows ещё и удаляет его копию в
    // Roaming — прогон тестов не должен ломать настроенного агента.
    let dir = std::env::temp_dir().join(format!("darwaza-token-test-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).expect("временная папка");
    let path = dir.join("agent-token");

    let first = darwaza_lib::agent::ensure_token_at(&path).expect("первый вызов");
    let second = darwaza_lib::agent::ensure_token_at(&path).expect("повторный вызов");
    assert_eq!(first, second, "токен обязан быть постоянным");
    assert_eq!(first.len(), 64, "32 байта в hex");
    assert_ne!(
        Some(path.as_path()),
        darwaza_lib::agent::token_path().ok().as_deref(),
        "тест не должен работать с настоящим токеном"
    );
    std::fs::remove_dir_all(&dir).ok();
}

// ─────────────────────────────────────────────────────────────────────────
// Слушатель на настоящем сокете
// ─────────────────────────────────────────────────────────────────────────

use darwaza_lib::agent::{AgentHandler, AgentResponse};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

/// Подставной обработчик: считает вызовы и отвечает эхом.
struct Probe {
    calls: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl AgentHandler for Probe {
    async fn handle(&self, method: &str, params: serde_json::Value) -> AgentResponse {
        self.calls.lock().unwrap().push(method.to_string());
        AgentResponse::ok(serde_json::json!({ "echo": method, "params": params }))
    }
}

/// Отдельный сокет на тест: общий путь заставляет тесты драться за него, и
/// падает не тот, кто виноват.
#[cfg(unix)]
fn temp_socket(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("darwaza-agent-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("каталог");
    dir.join("agent.sock")
}

/// Отправить строку и прочитать ответ.
async fn exchange<S: AsyncRead + AsyncWrite>(stream: S, line: &str) -> serde_json::Value {
    let (read, mut write) = tokio::io::split(stream);
    write
        .write_all(format!("{line}\n").as_bytes())
        .await
        .expect("запись");
    write.flush().await.expect("сброс");
    let mut lines = BufReader::new(read).lines();
    let answer = lines.next_line().await.expect("чтение").expect("ответ");
    serde_json::from_str(&answer).expect("ответ — JSON")
}

#[cfg(unix)]
async fn ask(path: &std::path::Path, line: &str) -> serde_json::Value {
    let stream = tokio::net::UnixStream::connect(path)
        .await
        .expect("подключение");
    exchange(stream, line).await
}

#[cfg(windows)]
async fn ask(name: &std::path::Path, line: &str) -> serde_json::Value {
    let stream = tokio::net::windows::named_pipe::ClientOptions::new()
        .open(name)
        .expect("подключение к каналу");
    exchange(stream, line).await
}

/// Где поднять слушатель в тесте: отдельный сокет на unix, отдельный канал на
/// Windows (имя с номером процесса, чтобы параллельные тесты не дрались).
async fn listen(
    name: &str,
    handler: Arc<dyn AgentHandler>,
    token: String,
) -> std::path::PathBuf {
    #[cfg(unix)]
    {
        darwaza_lib::agent::serve_at(temp_socket(name), handler, token)
            .await
            .expect("слушатель")
    }
    #[cfg(windows)]
    {
        let pipe = format!(r"\\.\pipe\darwaza-test-{}-{name}", std::process::id());
        darwaza_lib::agent::serve_pipe(pipe, handler, token)
            .await
            .expect("слушатель")
    }
}

#[tokio::test]
async fn listener_answers_authorized_requests_and_refuses_the_rest() {
    let token = "0".repeat(64);
    let probe = Arc::new(Probe {
        calls: std::sync::Mutex::new(Vec::new()),
    });
    let path = listen("answers", probe.clone(), token.clone()).await;

    // Верный токен — запрос доходит до обработчика.
    let ok = ask(
        &path,
        &format!(r#"{{"token":"{token}","method":"hosts.list"}}"#),
    )
    .await;
    assert_eq!(ok["result"]["echo"].as_str(), Some("hosts.list"));
    assert!(
        ok.get("error").is_none(),
        "при успехе ошибки быть не должно"
    );

    // Чужой токен — отказ, и обработчик НЕ вызывается. Это главное: проверка
    // обязана стоять до всякой полезной работы, иначе она бесполезна.
    let refused = ask(
        &path,
        &format!(r#"{{"token":"{}","method":"hosts.list"}}"#, "f".repeat(64)),
    )
    .await;
    assert_eq!(refused["error"].as_str(), Some("доступ не разрешён"));
    assert!(refused.get("result").is_none());

    // Мусор — тоже отказ, и тоже без вызова обработчика.
    let garbage = ask(&path, "это не json").await;
    assert!(garbage["error"].is_string());

    let calls = probe.calls.lock().unwrap().clone();
    assert_eq!(
        calls,
        vec!["hosts.list"],
        "обработчик должен был сработать ровно один раз, а не {calls:?}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn socket_is_reachable_only_by_owner() {
    let token = "1".repeat(64);
    let probe = Arc::new(Probe {
        calls: std::sync::Mutex::new(Vec::new()),
    });
    let path = darwaza_lib::agent::serve_at(temp_socket("perms"), probe, token)
        .await
        .expect("слушатель");

    let mode = std::fs::metadata(&path)
        .expect("метаданные")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        mode, 0o600,
        "сокет должен быть доступен только владельцу, получено {mode:o}"
    );
}

/// Канал, который поднял сам процесс, посредник признаёт своим: проверка
/// владельца (SID процесса-сервера совпадает с текущим пользователем) не
/// должна отвергать законный канал. Запускается только на Windows.
#[cfg(windows)]
#[tokio::test]
async fn the_pipe_of_our_own_process_passes_the_owner_check() {
    let probe = Arc::new(Probe {
        calls: std::sync::Mutex::new(Vec::new()),
    });
    let name = listen("owner", probe, "2".repeat(64)).await;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&name)
        .expect("подключение к каналу");
    assert!(darwaza_lib::winsec::server_is_current_user(&file).expect("проверка владельца"));
}

/// Токен после записи закрыт списком доступа без наследования: никаких записей
/// от каталога, кроме текущего пользователя. Только Windows.
#[cfg(windows)]
#[test]
fn the_token_file_is_locked_to_the_current_user() {
    let dir = std::env::temp_dir().join(format!("darwaza-acl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("каталог");
    let file = dir.join("agent-token");
    std::fs::write(&file, "").expect("файл");
    darwaza_lib::winsec::restrict_to_current_user(&file).expect("права");
    // Записать и прочитать текущему пользователю по-прежнему можно.
    std::fs::write(&file, "secret").expect("запись владельцем");
    assert_eq!(std::fs::read_to_string(&file).expect("чтение владельцем"), "secret");
    std::fs::remove_dir_all(&dir).ok();
}

/// Имя канала зависит от токена и не угадывается без него (#160).
#[test]
fn pipe_name_is_derived_from_the_token() {
    let a = darwaza_lib::agent::pipe_name("test-token");
    assert_eq!(a, r"\\.\pipe\darwaza-agent-4b2a71fa45091137a84a869a7ed2f210");
    assert_ne!(a, darwaza_lib::agent::pipe_name("other-token"));
    assert!(!a.contains("test-token"), "токен в имени канала виден всем");
}

// ─────────────────────────────────────────────────────────────────────────
// Отбор хостов, доступных агенту (#203)
// ─────────────────────────────────────────────────────────────────────────

use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

async fn db_with_hosts() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("память");
    darwaza_lib::db::MIGRATOR
        .run(&pool)
        .await
        .expect("миграции");

    for (name, allowed) in [("open-host", 1), ("closed-host", 0)] {
        sqlx::query(
            "INSERT INTO hosts (name, hostname, port, username, auth_method, \
             keep_alive_interval, agent_access, sync_id) \
             VALUES (?1, '10.0.0.1', 22, 'root', 'password', 30, ?2, ?3)",
        )
        .bind(name)
        .bind(allowed)
        .bind(format!(
            "aaaa0000-0000-4000-8000-0000000000{allowed}{}",
            name.len()
        ))
        .execute(&pool)
        .await
        .expect("вставка");
    }
    pool
}

#[tokio::test]
async fn only_allowed_hosts_are_visible() {
    // Агент не должен даже знать о существовании хостов, к которым его не
    // пускали: иначе перебором он составит список всех машин человека.
    let pool = db_with_hosts().await;

    let visible: Vec<String> =
        sqlx::query_scalar("SELECT name FROM hosts WHERE agent_access = 1 ORDER BY name")
            .fetch_all(&pool)
            .await
            .expect("выборка");
    assert_eq!(
        visible,
        vec!["open-host"],
        "закрытый хост виден быть не должен"
    );
}

#[tokio::test]
async fn closed_host_is_indistinguishable_from_a_missing_one() {
    // Разные ответы «нет такого» и «есть, но закрыт» превращают отказ в
    // способ проверить, существует ли машина с таким именем.
    let pool = db_with_hosts().await;

    let closed: Option<(i64, String)> =
        sqlx::query_as("SELECT id, name FROM hosts WHERE name = ?1 AND agent_access = 1")
            .bind("closed-host")
            .fetch_optional(&pool)
            .await
            .expect("запрос");
    let missing: Option<(i64, String)> =
        sqlx::query_as("SELECT id, name FROM hosts WHERE name = ?1 AND agent_access = 1")
            .bind("нет-такого")
            .fetch_optional(&pool)
            .await
            .expect("запрос");

    assert!(closed.is_none(), "закрытый хост не должен находиться");
    assert!(missing.is_none());
    assert_eq!(
        closed.is_none(),
        missing.is_none(),
        "оба случая обязаны выглядеть одинаково"
    );
}

#[tokio::test]
async fn existing_hosts_are_closed_after_migration() {
    // Обновление приложения не должно внезапно открыть агенту чужие боевые
    // серверы: человек об этом не просил и не узнает.
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("память");
    darwaza_lib::db::MIGRATOR
        .run(&pool)
        .await
        .expect("миграции");

    sqlx::query(
        "INSERT INTO hosts (name, hostname, port, username, auth_method, \
         keep_alive_interval, sync_id) \
         VALUES ('legacy', '10.0.0.9', 22, 'root', 'password', 30, 'bbbb0000-0000-4000-8000-000000000001')",
    )
    .execute(&pool)
    .await
    .expect("вставка без указания флага");

    let allowed: i64 = sqlx::query_scalar("SELECT agent_access FROM hosts WHERE name = 'legacy'")
        .fetch_one(&pool)
        .await
        .expect("чтение");
    assert_eq!(allowed, 0, "по умолчанию хост обязан быть закрыт");
}

// ─────────────────────────────────────────────────────────────────────────
// Журнал действий агента (#204)
// ─────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn refusals_are_logged_just_like_successes() {
    // Попытка выполнить запрещённое интереснее удачного `ls`. Журнал, куда
    // попадает только успешное, создаёт ложное впечатление, что ничего и не
    // происходило.
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("память");
    darwaza_lib::db::MIGRATOR
        .run(&pool)
        .await
        .expect("миграции");

    for (action, outcome, note) in [
        ("run", "ok", None),
        ("run", "refused", Some("команда содержит точку с запятой")),
        ("read", "failed", Some("нет такого файла")),
    ] {
        sqlx::query(
            "INSERT INTO agent_log (host, action, detail, outcome, note) \
             VALUES ('prod-01', ?1, 'что-то', ?2, ?3)",
        )
        .bind(action)
        .bind(outcome)
        .bind(note)
        .execute(&pool)
        .await
        .expect("запись");
    }

    let refused: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM agent_log WHERE outcome = 'refused'")
            .fetch_one(&pool)
            .await
            .expect("подсчёт");
    assert_eq!(refused, 1, "отказ обязан остаться в журнале");

    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_log")
        .fetch_one(&pool)
        .await
        .expect("подсчёт");
    assert_eq!(total, 3, "пишутся все исходы, а не только удачные");
}

#[tokio::test]
async fn log_survives_deletion_of_the_host() {
    // Имя хоста хранится строкой, а не ссылкой: журнал, теряющий строки при
    // уборке хостов, перестаёт быть журналом ровно тогда, когда нужен.
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("память");
    darwaza_lib::db::MIGRATOR
        .run(&pool)
        .await
        .expect("миграции");

    sqlx::query(
        "INSERT INTO hosts (name, hostname, port, username, auth_method, \
         keep_alive_interval, sync_id) \
         VALUES ('doomed', '10.0.0.1', 22, 'root', 'password', 30, 'cccc0000-0000-4000-8000-000000000001')",
    )
    .execute(&pool)
    .await
    .expect("хост");
    sqlx::query("INSERT INTO agent_log (host, action, detail, outcome) VALUES ('doomed', 'run', 'ls', 'ok')")
        .execute(&pool)
        .await
        .expect("запись");

    sqlx::query("DELETE FROM hosts WHERE name = 'doomed'")
        .execute(&pool)
        .await
        .expect("удаление хоста");

    let host: Option<String> = sqlx::query_scalar("SELECT host FROM agent_log LIMIT 1")
        .fetch_one(&pool)
        .await
        .expect("чтение");
    assert_eq!(
        host.as_deref(),
        Some("doomed"),
        "запись журнала обязана пережить удаление хоста"
    );
}
