//! Доступ агента через MCP: подлинность и канал (#202).
//!
//! Агент разговаривает не с сервером напрямую, а с **работающим
//! приложением** — через локальный сокет. Такое устройство выбрано не для
//! красоты: секреты остаются внутри приложения, а подтверждения показываются
//! в окне, где человек уже ввёл мастер-пароль, а не в невидимом фоновом
//! процессе.
//!
//! Отсюда главное требование к этому модулю: **любой процесс на машине не
//! должен получить доступ к боевым серверам просто потому, что он тут
//! запущен.** Поэтому канал закрыт токеном, а сокет лежит в каталоге,
//! доступном только владельцу.

pub mod diff;
pub mod safety;
pub mod tools;
pub mod transfer;

use anyhow::{Context, Result};
use std::path::PathBuf;

/// Длина токена. Тридцать два случайных байта — столько же, сколько у ключа
/// шифрования: подбирать бессмысленно при любой скорости.
const TOKEN_BYTES: usize = 32;

/// Где лежит токен. Права `0600`, как у приватного ключа.
pub fn token_path() -> Result<PathBuf> {
    Ok(crate::paths::local_state_dir()?.join("agent-token"))
}

/// Где токен лежал до 0.1.1 на Windows: каталог данных (Roaming). Нужен
/// только затем, чтобы убрать оттуда старый токен.
#[cfg(windows)]
fn roaming_token_path() -> Result<PathBuf> {
    Ok(crate::paths::state_dir()?.join("agent-token"))
}

/// Где слушаем. Предпочитаем каталог времени выполнения: он создаётся с
/// правами только для владельца и очищается при выходе из системы, то есть
/// забытый сокет не переживёт перезагрузку.
#[cfg(unix)]
pub fn socket_path() -> Result<PathBuf> {
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        let dir = PathBuf::from(runtime).join("darwaza");
        std::fs::create_dir_all(&dir).with_context(|| format!("создание {}", dir.display()))?;
        return Ok(dir.join("agent.sock"));
    }
    Ok(crate::paths::state_dir()?.join("agent.sock"))
}

/// Прочитать токен или создать новый.
///
/// Права выставляются **до** записи содержимого: создать файл, положить в
/// него секрет и только потом закрыть доступ — значит оставить окно, в
/// котором его успеет прочитать посторонний.
pub fn ensure_token() -> Result<String> {
    let path = token_path()?;
    // Старый токен из Roaming не оставляем лежать: его могли скопировать
    // вместе с профилем на другую машину, и он не должен остаться рабочим.
    #[cfg(windows)]
    {
        if let Ok(old) = roaming_token_path() {
            if old != path {
                std::fs::remove_file(old).ok();
            }
        }
    }
    ensure_token_at(&path)
}

/// То же для заданного файла. Отдельно от `ensure_token` затем, чтобы тест
/// мог работать во временной папке и не трогать настоящий токен разработчика
/// (и не удалять его старую копию в Roaming на Windows).
pub fn ensure_token_at(path: &std::path::Path) -> Result<String> {
    let path = path.to_path_buf();
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim().to_string();
        if !trimmed.is_empty() {
            // Токен мог быть создан прежней версией с правами каталога.
            #[cfg(windows)]
            {
                crate::winsec::restrict_to_current_user(&path)
                    .with_context(|| format!("права на {}", path.display()))?;
            }
            return Ok(trimmed);
        }
    }

    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("нет случайности: {e}"))?;
    let token = hex(&bytes);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_private(&path, &token)?;
    Ok(token)
}

/// Записать файл, доступный только владельцу.
#[cfg(unix)]
fn write_private(path: &std::path::Path, contents: &str) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("создание {}", path.display()))?;
    file.write_all(contents.as_bytes())?;
    Ok(())
}

#[cfg(not(unix))]
fn write_private(path: &std::path::Path, contents: &str) -> Result<()> {
    // Сначала пустой файл и замок на него, потом секрет: в обратном порядке
    // осталось бы окно, в котором токен лежит с правами каталога.
    std::fs::write(path, "").with_context(|| format!("создание {}", path.display()))?;
    #[cfg(windows)]
    {
        crate::winsec::restrict_to_current_user(path)
            .with_context(|| format!("права на {}", path.display()))?;
    }
    std::fs::write(path, contents).with_context(|| format!("запись {}", path.display()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Сравнение токенов за постоянное время.
///
/// Обычное `==` прекращает сравнение на первом несовпавшем байте, и по
/// времени ответа токен подбирается по одному символу. Здесь дешевле пройти
/// строку целиком, чем объяснять потом, как утекли ключи от боевых серверов.
pub fn token_matches(expected: &str, provided: &str) -> bool {
    let a = expected.as_bytes();
    let b = provided.as_bytes();
    // Разная длина — уже несовпадение, но сравнение всё равно доводим до
    // конца по длине ожидаемого, чтобы не подсказать её замером.
    let mut diff = (a.len() ^ b.len()) as u8;
    for (i, expected_byte) in a.iter().enumerate() {
        // Недостающие байты заменяем нулём, а не прерываемся: выход раньше
        // времени и есть та самая утечка длины через замер.
        let other = b.get(i).copied().unwrap_or(0);
        diff |= expected_byte ^ other;
    }
    diff == 0
}

#[cfg(test)]
mod gone_tests {
    use super::*;

    fn plan(id: &str) -> PendingPlan {
        let mut p = PendingPlan::new(1, "web", "проверка", Vec::new());
        p.id = id.into();
        p
    }

    /// Вызов агента отброшен посреди ожидания: план уходит из очереди, окно
    /// получает «cancelled».
    #[tokio::test]
    async fn dropped_wait_removes_the_plan_and_reports_cancelled() {
        let queue = PlanQueue::new();
        let _rx = queue.enqueue(plan("p1")).await;
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        let guard = GoneGuard::new(queue.clone(), "p1".into(), move |r| sink.lock().unwrap().push(r));

        drop(guard);
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        assert_eq!(queue.count().await, 0);
        assert_eq!(*seen.lock().unwrap(), vec![PlanGoneReason::Cancelled]);
    }

    #[tokio::test]
    async fn answered_plan_stays_silent_and_expiry_reports_expired() {
        let queue = PlanQueue::new();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        let mut guard = GoneGuard::new(queue.clone(), "p2".into(), move |r| sink.lock().unwrap().push(r));
        guard.disarm();
        drop(guard);
        tokio::task::yield_now().await;
        assert!(seen.lock().unwrap().is_empty());

        let sink = seen.clone();
        let mut guard = GoneGuard::new(queue, "p3".into(), move |r| sink.lock().unwrap().push(r));
        guard.finish(PlanGoneReason::Expired);
        drop(guard);
        tokio::task::yield_now().await;
        assert_eq!(*seen.lock().unwrap(), vec![PlanGoneReason::Expired]);
    }

    #[test]
    fn event_payload_has_lowercase_reason() {
        let json = serde_json::to_string(&PlanGone { id: "x".into(), reason: PlanGoneReason::Expired }).unwrap();
        assert_eq!(json, r#"{"id":"x","reason":"expired"}"#);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo;

    #[async_trait::async_trait]
    impl AgentHandler for Echo {
        async fn handle(&self, method: &str, _params: serde_json::Value) -> AgentResponse {
            AgentResponse::ok(serde_json::json!({ "echo": method }))
        }
    }

    /// Обмен по любому потоку байтов, а не только по unix-сокету: на нём же
    /// держится именованный канал Windows (#160), который на Linux не поднять.
    #[tokio::test]
    async fn connection_logic_works_over_any_byte_stream() {
        let (client, server) = tokio::io::duplex(4096);
        let task = tokio::spawn(serve_connection(server, Arc::new(Echo), "tok".into()));

        let (read, mut write) = tokio::io::split(client);
        let mut lines = BufReader::new(read).lines();
        write
            .write_all(b"{\"token\":\"tok\",\"method\":\"hosts.list\"}\n{\"token\":\"bad\",\"method\":\"x\"}\n")
            .await
            .unwrap();
        let ok: serde_json::Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(ok["result"]["echo"].as_str(), Some("hosts.list"));
        let refused: serde_json::Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(refused["error"].as_str(), Some("доступ не разрешён"));

        drop(write);
        drop(lines);
        task.await.unwrap().unwrap();
    }

    #[test]
    fn pipe_name_hides_the_token() {
        let name = pipe_name("secret-token");
        assert!(name.starts_with(r"\\.\pipe\darwaza-agent-"));
        assert!(!name.contains("secret"));
        assert_ne!(name, pipe_name("another-token"));
    }

    #[test]
    fn token_is_long_enough_to_be_pointless_to_guess() {
        let token = hex(&[0xabu8; TOKEN_BYTES]);
        assert_eq!(token.len(), TOKEN_BYTES * 2, "32 байта в hex — 64 символа");
    }

    #[test]
    fn matching_accepts_only_the_exact_token() {
        let token = "a".repeat(64);
        assert!(token_matches(&token, &token));
        assert!(!token_matches(&token, &"a".repeat(63)), "короче");
        assert!(!token_matches(&token, &format!("{token}x")), "длиннее");
        assert!(
            !token_matches(&token, &("b".to_string() + &"a".repeat(63))),
            "первый байт"
        );
        assert!(
            !token_matches(&token, &("a".repeat(63) + "b")),
            "последний байт — именно его пропустило бы раннее прекращение сравнения"
        );
        assert!(!token_matches(&token, ""), "пустой");
    }

    #[test]
    fn hex_encoding_is_lowercase_and_fixed_width() {
        // Ведущий ноль обязан сохраниться: иначе токены разной длины и
        // сравнение по длине начнёт подсказывать содержимое.
        assert_eq!(hex(&[0x00, 0x0f, 0xff]), "000fff");
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Канал: локальный сокет
// ─────────────────────────────────────────────────────────────────────────

use serde::{Deserialize, Serialize};

/// Запрос от посредника. Одна строка JSON — один запрос.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRequest {
    /// Токен присылается в **каждом** запросе, а не один раз при
    /// подключении: соединение может пережить смену токена, и «однажды
    /// впустили» не должно значить «впускаем навсегда».
    pub token: String,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
    /// Как назвался агент (`clientInfo.name` из MCP). Со слов самого агента,
    /// проверить нечем — поэтому только для показа, решений на нём не строим.
    #[serde(default)]
    pub client: Option<String>,
    /// Идентификатор сессии агента, одинаковый у всех её запросов.
    #[serde(default)]
    pub session: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl AgentResponse {
    pub fn ok(value: serde_json::Value) -> Self {
        Self {
            result: Some(value),
            error: None,
        }
    }

    pub fn failed(message: impl Into<String>) -> Self {
        Self {
            result: None,
            error: Some(message.into()),
        }
    }
}

/// Разобрать строку запроса и проверить токен.
///
/// Отдельная функция, потому что это единственное место, где решается
/// «пускать или нет», и его надо уметь проверить тестом без сокета.
pub fn authorize(expected_token: &str, line: &str) -> Result<AgentRequest, AgentResponse> {
    let request: AgentRequest =
        serde_json::from_str(line).map_err(|_| AgentResponse::failed("запрос не разбирается"))?;

    if !token_matches(expected_token, &request.token) {
        // Сообщение одинаковое и скупое: подсказывать, что именно не так —
        // длина, формат или содержимое, — значит помогать подбирать.
        return Err(AgentResponse::failed("доступ не разрешён"));
    }
    Ok(request)
}

#[cfg(test)]
mod channel_tests {
    use super::*;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn valid_request_passes() {
        let line = format!(r#"{{"token":"{TOKEN}","method":"hosts.list"}}"#);
        let request = authorize(TOKEN, &line).expect("должен пройти");
        assert_eq!(request.method, "hosts.list");
    }

    #[test]
    fn wrong_token_is_refused_with_the_same_message_as_garbage() {
        // Разные сообщения на «неверный токен» и «кривой запрос» рассказали бы
        // перебирающему, что он хотя бы попал в формат.
        let wrong = format!(r#"{{"token":"{}","method":"hosts.list"}}"#, "f".repeat(64));
        let refused = authorize(TOKEN, &wrong).expect_err("не должен пройти");
        assert_eq!(refused.error.as_deref(), Some("доступ не разрешён"));
        assert!(
            refused.result.is_none(),
            "при отказе результата быть не должно"
        );
    }

    #[test]
    fn missing_token_is_refused() {
        let line = r#"{"method":"hosts.list"}"#;
        assert!(authorize(TOKEN, line).is_err(), "без токена — отказ");
    }

    #[test]
    fn garbage_does_not_panic() {
        // На вход приходит что угодно: посредник может быть чужой программой.
        for line in ["", "не json", "{", "[]", "null", r#"{"token":123}"#] {
            assert!(
                authorize(TOKEN, line).is_err(),
                "должно быть отказано: {line:?}"
            );
        }
    }

    #[test]
    fn params_default_to_null_when_absent() {
        let line = format!(r#"{{"token":"{TOKEN}","method":"hosts.list"}}"#);
        let request = authorize(TOKEN, &line).expect("проходит");
        assert!(
            request.params.is_null(),
            "отсутствующие параметры — не ошибка"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Слушатель
// ─────────────────────────────────────────────────────────────────────────

use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};
use tracing::{info, warn};

/// Имя канала агента на Windows (#160).
///
/// Именованные каналы Windows живут в одном общем пространстве имён на всю
/// машину, и создать канал с любым именем может кто угодно. Если бы имя было
/// постоянным, посторонний пользователь мог бы занять его раньше нас, и
/// `darwaza-mcp` отдал бы ему токен вместе с первым же запросом. Поэтому имя
/// выводится из токена: хеш, по которому токен не восстановить, а имя без
/// токена не угадать. Токен при этом всё равно проверяется в каждом запросе.
///
/// Функция чистая и доступна на всех системах — её проверяет обычный тест.
/// Посредник `darwaza-mcp` повторяет её намеренно (см. там).
pub fn pipe_name(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("darwaza-agent-pipe:{token}").as_bytes());
    format!(r"\\.\pipe\darwaza-agent-{}", &hex(&digest)[..32])
}

/// Поднять слушатель на именованном канале Windows.
#[cfg(windows)]
pub async fn serve(handler: Arc<dyn AgentHandler>, token: String) -> Result<PathBuf> {
    let name = pipe_name(&token);
    serve_pipe(name, handler, token).await
}

/// Поднять слушатель на заданном имени канала. Имя параметром — по той же
/// причине, что и путь в `serve_at`: тесты не должны драться за один канал.
#[cfg(windows)]
pub async fn serve_pipe(
    name: String,
    handler: Arc<dyn AgentHandler>,
    token: String,
) -> Result<PathBuf> {
    use crate::winsec::create_owner_only_pipe;
    use tokio::net::windows::named_pipe::NamedPipeServer;

    // Первый экземпляр создаётся с `first_pipe_instance`: если канал с этим
    // именем уже есть, создание не удастся — не станем слушать поверх чужого.
    // Удалённые клиенты (по SMB) не допускаются, а список доступа — только
    // текущему пользователю: имена каналов видны всем, и умолчание системы
    // (чтение для всех) здесь не годится. Список не построился — канал не
    // поднимается вовсе.
    let mut server = create_owner_only_pipe(&name, true)
        .with_context(|| format!("не удалось занять {name}: приложение запущено дважды?"))?;
    info!(target: "agent", "слушаю {name}");

    let pipe = name.clone();
    tokio::spawn(async move {
        let next_instance = || -> std::io::Result<NamedPipeServer> {
            create_owner_only_pipe(&pipe, false)
        };
        loop {
            if let Err(e) = server.connect().await {
                // Одна неудачная попытка не повод переставать слушать.
                warn!(target: "agent", "не принял соединение на {pipe}: {e}");
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                if let Ok(fresh) = next_instance() {
                    server = fresh;
                }
                continue;
            }
            // Следующий экземпляр поднимаем ДО передачи этого в работу: иначе
            // между двумя клиентами канал на мгновение исчезает, и второй
            // получает «не запущено».
            let connected = server;
            server = loop {
                match next_instance() {
                    Ok(fresh) => break fresh,
                    Err(e) => {
                        warn!(target: "agent", "не создал следующий экземпляр {pipe}: {e}");
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    }
                }
            };
            let handler = handler.clone();
            let token = token.clone();
            tokio::spawn(async move {
                if let Err(e) = serve_connection(connected, handler, token).await {
                    warn!(target: "agent", "соединение оборвалось: {e}");
                }
            });
        }
    });

    Ok(PathBuf::from(name))
}

/// Что умеет обработчик запросов агента.
///
/// Отдельная черта, а не прямой вызов: слушатель ничего не знает ни про SSH,
/// ни про базу. Так его можно проверить тестом с подставным обработчиком, не
/// поднимая половину приложения.
#[async_trait::async_trait]
pub trait AgentHandler: Send + Sync + 'static {
    async fn handle(&self, method: &str, params: serde_json::Value) -> AgentResponse;
}

/// Поднять слушатель на стандартном пути.
#[cfg(unix)]
pub async fn serve(handler: Arc<dyn AgentHandler>, token: String) -> Result<PathBuf> {
    serve_at(socket_path()?, handler, token).await
}

/// Поднять слушатель на заданном пути.
///
/// Путь параметром, а не внутри: слушатель не должен решать, где ему жить.
/// Иначе два одновременных теста дерутся за один сокет — и падает не тот, кто
/// виноват.
#[cfg(unix)]
pub async fn serve_at(
    path: PathBuf,
    handler: Arc<dyn AgentHandler>,
    token: String,
) -> Result<PathBuf> {
    // Прежний файл сокета мог остаться от аварийно завершившегося запуска.
    // Слушать поверх него нельзя, а молча удалять чужой файл — тем более.
    if path.exists() {
        if UnixStream::connect(&path).await.is_ok() {
            anyhow::bail!(
                "сокет {} уже занят: приложение запущено дважды?",
                path.display()
            );
        }
        std::fs::remove_file(&path).ok();
    }

    let listener = UnixListener::bind(&path)
        .with_context(|| format!("не удалось занять {}", path.display()))?;
    restrict_socket(&path)?;
    info!(target: "agent", "слушаю {}", path.display());

    let accept_path = path.clone();
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let handler = handler.clone();
                    let token = token.clone();
                    tokio::spawn(async move {
                        if let Err(e) = serve_connection(stream, handler, token).await {
                            warn!(target: "agent", "соединение оборвалось: {e}");
                        }
                    });
                }
                Err(e) => {
                    // Одна неудачная попытка не повод переставать слушать:
                    // иначе первый же сбой тихо отключит агента навсегда.
                    warn!(target: "agent", "не принял соединение на {}: {e}", accept_path.display());
                }
            }
        }
    });

    Ok(path)
}

/// Ограничить доступ к сокету владельцем.
#[cfg(unix)]
fn restrict_socket(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("права на {}", path.display()))
}

#[cfg(not(unix))]
fn restrict_socket(_path: &std::path::Path) -> Result<()> {
    Ok(())
}

/// Обслужить одно соединение. Не знает, что под ним — сокет unix или канал
/// Windows: нужен только поток байтов.
async fn serve_connection<S>(
    stream: S,
    handler: Arc<dyn AgentHandler>,
    token: String,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (read, mut write) = tokio::io::split(stream);
    let mut lines = BufReader::new(read).lines();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let response = match authorize(&token, &line) {
            Ok(request) => {
                let caller = Caller::from_request(&request);
                CALLER
                    .scope(caller, handler.handle(&request.method, request.params))
                    .await
            }
            Err(refusal) => refusal,
        };
        let mut out = serde_json::to_vec(&response)?;
        out.push(b'\n');
        write.write_all(&out).await?;
        write.flush().await?;
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────
// Подтверждение изменений (#205)
// ─────────────────────────────────────────────────────────────────────────

use std::collections::HashMap;
use tokio::sync::{oneshot, Mutex};

/// Кто прислал запрос. Нужен окну подтверждения: «агент просит» без имени и
/// сессии не отличает двух агентов, работающих одновременно.
#[derive(Debug, Clone, Default)]
pub struct Caller {
    pub client: String,
    pub session: String,
}

impl Caller {
    pub fn from_request(request: &AgentRequest) -> Self {
        Self {
            client: shown(request.client.as_deref(), "неизвестный клиент"),
            session: shown(request.session.as_deref(), "—"),
        }
    }
}

/// Невидимые и меняющие направление символы: категории `Cf` (форматирующие —
/// в том числе U+202E «справа налево», из-за которого подпись читается
/// зеркально), `Zl` и `Zp` (разделители строк и абзацев).
///
/// `char::is_control` — это только `Cc`, и всё перечисленное проходит мимо него.
fn is_invisible(c: char) -> bool {
    matches!(
        c as u32,
        0x00AD
            | 0x0600..=0x0605
            | 0x061C
            | 0x06DD
            | 0x070F
            | 0x0890..=0x0891
            | 0x08E2
            | 0x180E
            | 0x200B..=0x200F
            | 0x2028..=0x202E
            | 0x2060..=0x2064
            | 0x2066..=0x206F
            | 0xFEFF
            | 0xFFF9..=0xFFFB
            | 0x110BD
            | 0x110CD
            | 0xE0001
            | 0xE0020..=0xE007F
    )
}

/// Строка от агента — в окно человеку. Режем длину и убираем управляющие
/// символы: имя клиента выводится рядом с кнопкой «подтвердить», и длинная
/// или многострочная подпись могла бы вытеснить из вида то, что подтверждается.
fn shown(raw: Option<&str>, fallback: &str) -> String {
    let cleaned: String = raw
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control() && !is_invisible(*c))
        .take(64)
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        fallback.to_string()
    } else {
        cleaned.to_string()
    }
}

tokio::task_local! {
    /// Вызывающий в рамках одного запроса. Через локальную переменную задачи, а
    /// не параметром: иначе его пришлось бы протаскивать через два десятка
    /// инструментов ради одной строки в окне подтверждения.
    pub static CALLER: Caller;
}

/// Вид шага плана. Определяет Rust, интерфейс его не переопределяет.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StepKind {
    /// Запись файла.
    Edit,
    /// Команда проходит проверку «только читает».
    Check,
    /// Действие над службой: `systemctl`, `service`.
    Service,
    /// Всё остальное, что меняет.
    Run,
    /// Запрещено правилами насовсем. **Не выполняется ни при каких условиях.**
    Blocked,
    /// Стоит после заблокированного шага и поэтому не выполнится.
    Skipped,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStep {
    pub kind: StepKind,
    /// Команда или путь — целиком, интерфейс усекать не должен.
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    /// Машинный вид причины запрета: окно показывает её на языке интерфейса, а
    /// `blocked_reason` (по-русски) остаётся запасным текстом.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_arg: Option<String>,
    /// Разница, если шаг — правка файла. Строки «было/стало» человек читает
    /// глазами, а команду записи ему пришлось бы выполнять в уме.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<Vec<diff::DiffLine>>,
}

impl PlanStep {
    fn plain(kind: StepKind, title: impl Into<String>) -> Self {
        Self {
            kind,
            title: title.into(),
            blocked_reason: None,
            blocked_code: None,
            blocked_arg: None,
            diff: None,
        }
    }

    /// Шаг, меняющий что-то, без более точного вида.
    pub fn run(title: impl Into<String>) -> Self {
        Self::plain(StepKind::Run, title)
    }

    /// Правка файла с разницей.
    pub fn edit(title: impl Into<String>, diff: Vec<diff::DiffLine>) -> Self {
        Self {
            diff: Some(diff),
            ..Self::plain(StepKind::Edit, title)
        }
    }

    /// Выполнится ли шаг, если план подтвердят.
    pub fn will_run(&self) -> bool {
        !matches!(self.kind, StepKind::Blocked | StepKind::Skipped)
    }
}

/// Разложить команды плана по видам.
///
/// Всё, что стоит после первого запрещённого шага, помечается `Skipped`:
/// исполнение остановится перед ним, и показывать такие шаги как обычные
/// значило бы обещать человеку то, чего не будет. Пропускать запрещённый
/// шаг и идти дальше нельзя — следующие могут рассчитывать на его результат.
pub fn classify_steps(commands: &[String]) -> Vec<PlanStep> {
    let mut stopped = false;
    commands
        .iter()
        .map(|command| {
            if stopped {
                return PlanStep::plain(StepKind::Skipped, command);
            }
            if let Some(forbidden) = safety::forbidden_detail(command) {
                stopped = true;
                return PlanStep {
                    blocked_reason: Some(forbidden.reason),
                    blocked_code: Some(forbidden.code.to_string()),
                    blocked_arg: forbidden.arg,
                    ..PlanStep::plain(StepKind::Blocked, command)
                };
            }
            PlanStep::plain(safety::step_kind(command), command)
        })
        .collect()
}

/// Сколько шагов с начала плана разрешено выполнять: все до первого
/// запрещённого. Исполнитель идёт именно по этому числу, а не по собственному
/// представлению о запретном: «что выполняется» и «что показано человеку»
/// решает одно и то же место.
pub fn runnable_prefix(steps: &[PlanStep]) -> usize {
    steps.iter().take_while(|step| step.will_run()).count()
}

/// План изменений, ожидающий решения человека.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingPlan {
    pub id: String,
    /// Хост плана по `id` — имя не уникально, и адрес в окне должен быть того
    /// хоста, на котором план выполнится.
    pub host_id: i64,
    pub host: String,
    /// Адрес и пользователь хоста, чтобы окно не искало их по имени само.
    pub host_address: String,
    pub host_user: String,
    /// Как назвался агент и из какой он сессии — со слов агента.
    pub client: String,
    pub session: String,
    /// Зачем это делается — словами агента. Человек подтверждает замысел, а
    /// не набор строк, и без объяснения решение принимается вслепую.
    pub intent: String,
    pub steps: Vec<PlanStep>,
    /// Когда план будет отклонён сам, Unix-миллисекунды. Ставится в момент
    /// постановки в очередь и совпадает с настоящим сроком ожидания.
    pub expires_at: i64,
}

impl PendingPlan {
    /// Остальное — адрес, клиента, срок — подставит очередь в момент показа.
    pub fn new(
        host_id: i64,
        host: impl Into<String>,
        intent: impl Into<String>,
        steps: Vec<PlanStep>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            host_id,
            host: host.into(),
            host_address: String::new(),
            host_user: String::new(),
            client: String::new(),
            session: String::new(),
            intent: intent.into(),
            steps,
            expires_at: 0,
        }
    }
}

/// Почему план исчез из очереди без подтверждения (#257).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlanGoneReason {
    /// Не дождались решения за `PLAN_DECISION_TIMEOUT`.
    Expired,
    /// Человек отклонил (кнопкой или «отклонить всё»).
    Rejected,
    /// Агент ушёл, не дождавшись: сессия закрыта, вызов оборван.
    Cancelled,
}

/// Событие `agent://plan_gone`: карточка плана больше не действует.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlanGone {
    pub id: String,
    pub reason: PlanGoneReason,
}

pub const PLAN_GONE_EVENT: &str = "agent://plan_gone";

/// Страж плана в очереди. Если ожидание оборвалось (вызов агента отброшен),
/// план убирается из очереди и окно узнаёт об этом: без стража карточка висела
/// бы с живой кнопкой «Подтвердить», нажатие на которую никому не нужно.
pub struct GoneGuard {
    queue: PlanQueue,
    id: String,
    notify: Option<Box<dyn FnOnce(PlanGoneReason) + Send>>,
}

impl GoneGuard {
    pub fn new(
        queue: PlanQueue,
        id: String,
        notify: impl FnOnce(PlanGoneReason) + Send + 'static,
    ) -> Self {
        Self { queue, id, notify: Some(Box::new(notify)) }
    }

    /// Ответ получен — сообщать нечего.
    pub fn disarm(&mut self) {
        self.notify = None;
    }

    /// Закончили сами с известной причиной.
    pub fn finish(&mut self, reason: PlanGoneReason) {
        if let Some(notify) = self.notify.take() {
            notify(reason);
        }
    }
}

impl Drop for GoneGuard {
    fn drop(&mut self) {
        if let Some(notify) = self.notify.take() {
            let queue = self.queue.clone();
            let id = std::mem::take(&mut self.id);
            tokio::spawn(async move {
                queue.forget(&id).await;
                notify(PlanGoneReason::Cancelled);
            });
        }
    }
}

/// Решение человека.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlanVerdict {
    Approved,
    Rejected,
}

/// Планы, ожидающие ответа.
#[derive(Clone, Default)]
pub struct PlanQueue {
    pending: std::sync::Arc<Mutex<HashMap<String, Waiting>>>,
}

/// План вместе с тем, кто ждёт по нему ответа.
///
/// Хранить один только канал ответа было бы дешевле, но тогда окну нечего
/// показывать: подтверждать план, которого не видно, — то же самое, что
/// подтверждать вслепую.
struct Waiting {
    plan: PendingPlan,
    responder: oneshot::Sender<PlanVerdict>,
}

impl PlanQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Поставить план в очередь и получить ожидание ответа.
    pub async fn enqueue(&self, plan: PendingPlan) -> oneshot::Receiver<PlanVerdict> {
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(
            plan.id.clone(),
            Waiting {
                plan,
                responder: tx,
            },
        );
        rx
    }

    /// Что сейчас ждёт ответа — для окна подтверждения.
    ///
    /// Порядок по времени постановки: человек отвечает на то, что пришло
    /// раньше, а не на то, что случайно оказалось первым в отображении.
    pub async fn list(&self) -> Vec<PendingPlan> {
        let mut plans: Vec<PendingPlan> = self
            .pending
            .lock()
            .await
            .values()
            .map(|w| w.plan.clone())
            .collect();
        plans.sort_by(|a, b| a.id.cmp(&b.id));
        plans
    }

    /// Ответить на план. `false` — такого плана уже нет.
    pub async fn resolve(&self, id: &str, verdict: PlanVerdict) -> bool {
        match self.pending.lock().await.remove(id) {
            Some(waiting) => waiting.responder.send(verdict).is_ok(),
            None => false,
        }
    }

    /// Убрать план без ответа (агент ушёл). `true` — он ещё был в очереди.
    pub async fn forget(&self, id: &str) -> bool {
        self.pending.lock().await.remove(id).is_some()
    }

    /// Отклонить всё разом — кнопка «отключить агента от всего».
    ///
    /// Именно отклонить, а не забыть: агент, оставшийся ждать ответа
    /// навсегда, выглядит как зависший, и человек не поймёт, что произошло.
    ///
    /// Возвращает номера отклонённых планов: окну нужно знать, какие карточки
    /// исчезли.
    pub async fn reject_all(&self) -> Vec<String> {
        let mut guard = self.pending.lock().await;
        let mut ids = Vec::with_capacity(guard.len());
        for (id, waiting) in guard.drain() {
            let _ = waiting.responder.send(PlanVerdict::Rejected);
            ids.push(id);
        }
        ids
    }

    pub async fn count(&self) -> usize {
        self.pending.lock().await.len()
    }
}

/// Сколько ждать решения человека.
///
/// Без предела агент висит бесконечно, если окно закрыли или человек ушёл.
/// Пять минут — достаточно, чтобы прочитать план, и мало, чтобы забыть о
/// висящем запросе.
pub const PLAN_DECISION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

#[cfg(test)]
mod plan_tests {
    fn commands(list: &[&str]) -> Vec<String> {
        list.iter().map(|c| c.to_string()).collect()
    }

    #[test]
    fn steps_are_labelled_by_what_they_do() {
        let steps = classify_steps(&commands(&[
            "ls /etc/nginx",
            "systemctl reload nginx",
            "make deploy",
        ]));
        let kinds: Vec<StepKind> = steps.iter().map(|s| s.kind).collect();
        assert_eq!(kinds, [StepKind::Check, StepKind::Service, StepKind::Run]);
    }

    #[test]
    fn nothing_after_a_blocked_step_will_run() {
        // Пропустить запрещённый шаг и идти дальше нельзя: следующие могут
        // рассчитывать на его результат. Поэтому всё после него — Skipped.
        let steps = classify_steps(&commands(&[
            "ls /etc/nginx",
            "mkfs.ext4 /dev/sdb",
            "systemctl reload nginx",
        ]));
        assert!(steps[0].will_run());
        assert_eq!(steps[1].kind, StepKind::Blocked);
        assert!(steps[1].blocked_reason.is_some());
        assert!(!steps[1].will_run());
        assert_eq!(steps[2].kind, StepKind::Skipped);
        assert!(!steps[2].will_run());
        assert!(steps[2].blocked_reason.is_none());
    }

    #[test]
    fn nothing_from_the_blocked_step_onward_is_runnable() {
        // Исполнитель выполняет ровно `runnable_prefix` шагов. Если запрещённый
        // шаг пропускают и идут дальше, число становится 2, а не 1 — и тест
        // падает: шаг после запрещённого мог рассчитывать на его результат.
        let steps = classify_steps(&commands(&[
            "ls /etc/nginx",
            "mkfs.ext4 /dev/sdb",
            "systemctl reload nginx",
        ]));
        assert_eq!(runnable_prefix(&steps), 1);

        let blocked_first = classify_steps(&commands(&["mkfs.ext4 /dev/sdb", "ls /etc"]));
        assert_eq!(runnable_prefix(&blocked_first), 0);

        let clean = classify_steps(&commands(&["ls /etc", "systemctl reload nginx"]));
        assert_eq!(runnable_prefix(&clean), 2);
    }

    #[test]
    fn a_blocked_command_is_blocked_whatever_its_label_would_be() {
        // Вид — подпись, запрет — правило: проверяем, что `mkfs` в хвосте
        // длинной строки не прячется за безобидным началом.
        let steps = classify_steps(&commands(&["echo ok && mkfs.ext4 /dev/sdb"]));
        assert_eq!(steps[0].kind, StepKind::Blocked);
    }

    #[test]
    fn the_plan_reaches_the_window_in_the_agreed_shape() {
        // Контракт с интерфейсом записан в `backend-gaps.md`: имена полей и
        // вид шага. Расхождение здесь не ломает сборку ни с одной стороны.
        let plan = PendingPlan::new(
            1,
            "web-1.prod",
            "обновить",
            classify_steps(&commands(&["ls /etc/nginx", "mkfs.ext4 /dev/sdb"])),
        );
        let json = serde_json::to_value(&plan).expect("сериализуется");
        for key in [
            "id",
            "hostId",
            "host",
            "hostAddress",
            "hostUser",
            "client",
            "session",
            "intent",
            "steps",
            "expiresAt",
        ] {
            assert!(json.get(key).is_some(), "нет поля {key}");
        }
        assert!(json.get("diff").is_none(), "дифф живёт в шаге, не в плане");
        assert_eq!(json["steps"][0]["kind"], "check");
        assert_eq!(json["steps"][1]["kind"], "blocked");
        assert!(json["steps"][1]["blockedReason"].is_string());
        // Окно переводит причину по коду, а не по русскому тексту.
        assert_eq!(json["steps"][1]["blockedCode"], "never");
        assert_eq!(json["steps"][1]["blockedArg"], "mkfs");
        assert!(json["steps"][0].get("blockedCode").is_none());
        assert!(json["steps"][0].get("blockedReason").is_none());
    }

    #[test]
    fn the_callers_name_cannot_crowd_out_the_plan() {
        assert_eq!(shown(None, "неизвестный клиент"), "неизвестный клиент");
        assert_eq!(shown(Some("  \n "), "—"), "—");
        let long = "x".repeat(500);
        assert_eq!(shown(Some(&long), "—").chars().count(), 64);
        assert_eq!(shown(Some("claude\nmcp"), "—"), "claudemcp");
        // Направление текста и разделители строк не проходят: подпись рядом с
        // кнопкой «подтвердить» нельзя отразить зеркально или разорвать.
        assert_eq!(
            shown(Some("\u{202E}claude\u{2028}mcp\u{200B}"), "—"),
            "claudemcp"
        );
    }

    use super::*;

    /// План для тестов — содержание неважно, важна работа очереди.
    fn plan(id: &str) -> PendingPlan {
        let mut plan = PendingPlan::new(
            1,
            "prod",
            "обновить сборку",
            classify_steps(&["systemctl restart app".to_string()]),
        );
        plan.id = id.to_string();
        plan
    }

    #[tokio::test]
    async fn the_queue_shows_what_it_asks_about() {
        // Подтверждать план, которого не видно, — то же самое, что
        // подтверждать вслепую.
        let queue = PlanQueue::new();
        let _rx = queue.enqueue(plan("plan-1")).await;

        let shown = queue.list().await;
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].intent, "обновить сборку");
        assert_eq!(shown[0].steps[0].title, "systemctl restart app");
    }

    #[tokio::test]
    async fn approval_reaches_the_waiting_side() {
        let queue = PlanQueue::new();
        let rx = queue.enqueue(plan("plan-1")).await;
        assert_eq!(queue.count().await, 1);

        assert!(queue.resolve("plan-1", PlanVerdict::Approved).await);
        assert_eq!(rx.await.expect("ответ"), PlanVerdict::Approved);
        assert_eq!(
            queue.count().await,
            0,
            "отвеченный план не остаётся в очереди"
        );
    }

    #[tokio::test]
    async fn answering_an_unknown_plan_reports_false() {
        // Иначе повторное нажатие «подтвердить» выглядело бы успешным, а
        // второй раз выполнять уже нечего.
        let queue = PlanQueue::new();
        assert!(!queue.resolve("нет такого", PlanVerdict::Approved).await);
    }

    #[tokio::test]
    async fn rejecting_everything_answers_rather_than_forgets() {
        // Агент, оставшийся ждать навсегда, выглядит зависшим, и человек не
        // поймёт, что произошло. Поэтому именно отклоняем.
        let queue = PlanQueue::new();
        let first = queue.enqueue(plan("a")).await;
        let second = queue.enqueue(plan("b")).await;

        assert_eq!(queue.reject_all().await.len(), 2);
        assert_eq!(first.await.expect("ответ"), PlanVerdict::Rejected);
        assert_eq!(second.await.expect("ответ"), PlanVerdict::Rejected);
        assert_eq!(queue.count().await, 0);
    }

    #[tokio::test]
    async fn a_plan_answered_twice_only_counts_once() {
        let queue = PlanQueue::new();
        let _rx = queue.enqueue(plan("plan")).await;
        assert!(queue.resolve("plan", PlanVerdict::Approved).await);
        assert!(
            !queue.resolve("plan", PlanVerdict::Rejected).await,
            "второй ответ на тот же план не должен проходить"
        );
    }
}
