use crate::db::models::{Host, Identity};
use crate::keyring_store::kinds;
use crate::known_hosts::{self, KnownHost};
use crate::master::MasterKeyring;
use crate::ssh::manager::{PendingHostKeyPrompt, SessionSnapshot};
use crate::ssh::session::{SessionCmd, SessionEvent, SessionHandle, SessionStatus};
use crate::AppState;
use russh::client::{self, Handle};
use russh::keys::ssh_key::HashAlg;
use russh::keys::ssh_key::PublicKey;
use russh::keys::{load_secret_key, PrivateKeyWithHashAlg};
use russh::ChannelMsg;
use serde::Serialize;
use sqlx::SqlitePool;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::{mpsc, oneshot};
use tracing::{error, info, warn};
use uuid::Uuid;

// Списки колонок перечислены руками, потому что `SELECT *` вернул бы и
// служебные поля синхронизации, которых в модели нет. Цена этого решения —
// список обязан ходить в ногу со структурой: забытая колонка не ломает
// сборку, она ломает подключение в рантайме. Ровно так и случилось с
// `agent_access` (см. тест ниже).
const HOST_FIELDS: &str = "id, group_id, name, hostname, port, username, identity_id, \
    auth_method, color, icon, jump_host_id, forward_agent, agent_access, agent_sudo_read, \
    agent_notes, keep_alive_interval, description, last_connected_at, created_at, updated_at";

const IDENTITY_FIELDS: &str = "id, label, kind, key_path, keyring_ref, has_passphrase, \
    has_totp, public_key, created_at, updated_at";

const SESSION_STATUS_EVENT: &str = "session:status";
const SESSION_DATA_EVENT: &str = "session:data";
const SESSION_CLOSED_EVENT: &str = "session:closed";
const SESSION_HOST_KEY_PROMPT_EVENT: &str = "session:host_key_prompt";
const SESSION_AUTH_PROMPT_EVENT: &str = "session:auth_prompt";

/// Ошибка наружу уходит кодом, причина — в журнал (#177).
fn map_err<E: std::fmt::Display>(e: E) -> String {
    crate::errors::opaque("ssh", e)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectResponse {
    pub session_id: Uuid,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyPromptPayload {
    pub session_id: Uuid,
    pub hostname: String,
    pub key_type: String,
    pub fingerprint: String,
    pub kind: HostKeyPromptKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HostKeyPromptKind {
    New,
    Changed,
}

/// russh client::Handler. Хранит контекст сессии для TOFU.
struct ClientHandler {
    session_id: Uuid,
    hostname: String,
    db: SqlitePool,
    app: AppHandle,
    manager: crate::ssh::SshManager,
    forwards: crate::forwarding::ForwardManager,
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    /// Соединение, пришедшее на проброшенный порт сервера (#130a).
    ///
    /// Здесь только передаём канал в рантайм пробросов: он знает, на какой
    /// локальный адрес его тянуть. Ошибку не поднимаем наверх — оборванный
    /// проброс не повод ронять всю SSH-сессию.
    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: russh::Channel<client::Msg>,
        connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        let forwards = self.forwards.clone();
        let session_id = self.session_id;
        let address = connected_address.to_string();
        tokio::spawn(async move {
            if let Err(e) = forwards
                .serve_remote_connection(session_id, connected_port, channel)
                .await
            {
                warn!(
                    target: "forward",
                    "session {session_id}: соединение с {address}:{connected_port} не обслужено: {e}"
                );
            }
        });
        Ok(())
    }

    /// Сервер просит доступ к нашему агенту по проброшенному каналу (#150a).
    ///
    /// Проксируем канал на локальный сокет агента. Ключи при этом остаются
    /// у агента: наружу уходят только подписи, как и при обычной работе.
    async fn server_channel_open_agent_forward(
        &mut self,
        channel: russh::Channel<client::Msg>,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        let session_id = self.session_id;
        // Канал уже подтверждён серверу, и молча бросить его нельзя: удалённый
        // `ssh` или `git` будут ждать ответа агента до тайм-аута. Если агента
        // у нас нет, канал закрываем сразу — они увидят обрыв и пойдут дальше.
        let Some(sock) = agent_socket_path() else {
            warn!(
                target: "ssh.agent",
                "session {session_id}: сервер запросил агента, но SSH_AUTH_SOCK не задан"
            );
            tokio::spawn(async move {
                let _ = channel.close().await;
            });
            return Ok(());
        };

        tokio::spawn(async move {
            match open_agent_stream(&sock).await {
                Ok(mut agent) => {
                    let mut stream = channel.into_stream();
                    if let Err(e) = tokio::io::copy_bidirectional(&mut agent, &mut stream).await {
                        warn!(target: "ssh.agent", "session {session_id}: обмен с агентом прерван: {e}");
                    }
                }
                Err(e) => {
                    warn!(target: "ssh.agent", "session {session_id}: не удалось открыть сокет агента: {e}");
                    let _ = channel.close().await;
                }
            }
        });
        Ok(())
    }

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKey,
    ) -> Result<bool, Self::Error> {
        let fingerprint = server_public_key.fingerprint(HashAlg::Sha256).to_string();
        let key_type = server_public_key.algorithm().as_str().to_string();

        // Сравниваем с записью в known_hosts.
        let existing = known_hosts::find(&self.db, &self.hostname, &key_type)
            .await
            .map_err(|e| {
                error!(target: "ssh.tofu", "DB lookup failed: {e}");
                russh::Error::Inconsistent
            })?;

        match existing {
            Some(known) if known.fingerprint == fingerprint => {
                info!(target: "ssh.tofu", "session {}: known host key matches", self.session_id);
                Ok(true)
            }
            other => {
                let (kind, previous) = match other {
                    Some(known) => (HostKeyPromptKind::Changed, Some(known.fingerprint)),
                    None => (HostKeyPromptKind::New, None),
                };
                info!(target: "ssh.tofu", "session {}: TOFU prompt needed, kind={:?}", self.session_id, kind);

                let (tx, rx) = oneshot::channel();
                let prompt = PendingHostKeyPrompt {
                    hostname: self.hostname.clone(),
                    key_type: key_type.clone(),
                    fingerprint: fingerprint.clone(),
                    responder: tx,
                };

                if let Err(e) = self.manager.register_prompt(self.session_id, prompt).await {
                    error!(target: "ssh.tofu", "register_prompt failed: {e}");
                    return Ok(false);
                }
                info!(target: "ssh.tofu", "session {}: prompt registered, emitting event", self.session_id);

                let payload = HostKeyPromptPayload {
                    session_id: self.session_id,
                    hostname: self.hostname.clone(),
                    key_type,
                    fingerprint,
                    kind,
                    previous_fingerprint: previous,
                };
                crate::attention::needs_decision(&self.app).await;
                if let Err(e) = self.app.emit(SESSION_HOST_KEY_PROMPT_EVENT, &payload) {
                    error!(target: "ssh.tofu", "emit host_key_prompt failed: {e}");
                    return Ok(false);
                }
                info!(target: "ssh.tofu", "session {}: event emitted, waiting for user response", self.session_id);

                match rx.await {
                    Ok(accepted) => {
                        info!(target: "ssh.tofu", "session {}: user responded accepted={}", self.session_id, accepted);
                        Ok(accepted)
                    }
                    Err(_) => {
                        warn!(target: "ssh.tofu", "session {}: prompt responder dropped", self.session_id);
                        Ok(false)
                    }
                }
            }
        }
    }
}

/// Хост, как его надо подключать: пробелы по краям адреса и имени
/// пользователя убраны. Запись могла быть сохранена старой версией или
/// импортирована с пробелом (« 1.1.1.1» давало `root@ 1.1.1.1`).
fn trimmed(mut host: Host) -> Host {
    host.hostname = host.hostname.trim().to_string();
    host.username = host.username.trim().to_string();
    host.name = host.name.trim().to_string();
    host
}

async fn load_host(pool: &SqlitePool, host_id: i64) -> Result<Host, String> {
    let sql = format!("SELECT {HOST_FIELDS} FROM hosts WHERE id = ?1");
    sqlx::query_as::<_, Host>(&sql)
        .bind(host_id)
        .fetch_optional(pool)
        .await
        .map_err(map_err)?
        .map(trimmed)
        .ok_or_else(|| crate::errors::not_found("ssh", "хост", host_id))
}

async fn load_identity(pool: &SqlitePool, identity_id: i64) -> Result<Identity, String> {
    let sql = format!("SELECT {IDENTITY_FIELDS} FROM identities WHERE id = ?1");
    sqlx::query_as::<_, Identity>(&sql)
        .bind(identity_id)
        .fetch_optional(pool)
        .await
        .map_err(map_err)?
        .ok_or_else(|| crate::errors::not_found("ssh", "идентификация", identity_id))
}

fn emit_status(app: &AppHandle, session_id: Uuid, status: SessionStatus, error: Option<String>) {
    let payload = SessionEvent::Status {
        session_id,
        status,
        error,
    };
    if let Err(e) = app.emit(SESSION_STATUS_EVENT, &payload) {
        error!(target: "ssh", "emit status failed: {e}");
    }
}

fn emit_data(app: &AppHandle, session_id: Uuid, chunk: Vec<u8>) {
    let payload = SessionEvent::Data { session_id, chunk };
    if let Err(e) = app.emit(SESSION_DATA_EVENT, &payload) {
        error!(target: "ssh", "emit data failed: {e}");
    }
}

fn emit_closed(app: &AppHandle, session_id: Uuid, exit_code: Option<u32>) {
    let payload = SessionEvent::Closed {
        session_id,
        exit_code,
    };
    if let Err(e) = app.emit(SESSION_CLOSED_EVENT, &payload) {
        error!(target: "ssh", "emit closed failed: {e}");
    }
}

enum AuthMaterial {
    Password(String),
    PrivateKey {
        path: String,
        passphrase: Option<String>,
    },
    /// Аутентификация через ssh-agent (#150).
    ///
    /// Секрета у нас нет и быть не должно: приватный ключ не покидает агента,
    /// подпись делает он сам. Если у identity записан публичный ключ — берём
    /// из агента только его, иначе перебираем все предложенные.
    Agent {
        public_key: Option<String>,
    },
}

async fn resolve_auth(
    host: &Host,
    pool: &SqlitePool,
    master: &MasterKeyring,
) -> Result<AuthMaterial, String> {
    match host.auth_method.as_str() {
        "password" => {
            let identity_id = host
                .identity_id
                .ok_or_else(|| "host has no identity for password auth".to_string())?;
            let password = crate::secrets::get(pool, master, kinds::PASSWORD, identity_id)
                .await?
                .ok_or_else(|| {
                    crate::errors::not_found_secret(
                        "ssh",
                        "пароль",
                        identity_id,
                        crate::errors::codes::SSH_NO_PASSWORD,
                    )
                })?;
            Ok(AuthMaterial::Password(password))
        }
        "publickey" => {
            let identity_id = host
                .identity_id
                .ok_or_else(|| "host has no identity for publickey auth".to_string())?;
            let identity = load_identity(pool, identity_id).await?;
            let path = identity.key_path.ok_or_else(|| {
                crate::errors::invalid(
                    "ssh",
                    format!("identity {identity_id} без key_path"),
                    crate::errors::codes::SSH_NO_KEY_PATH,
                )
            })?;
            let passphrase = if identity.has_passphrase {
                let stored =
                    crate::secrets::get(pool, master, kinds::PASSPHRASE, identity_id).await?;
                if stored.is_none() {
                    return Err(crate::errors::not_found_secret(
                        "ssh",
                        "ключевая фраза",
                        identity_id,
                        crate::errors::codes::SSH_NO_PASSPHRASE,
                    ));
                }
                stored
            } else {
                None
            };
            Ok(AuthMaterial::PrivateKey { path, passphrase })
        }
        "agent" => {
            // identity необязательна: без неё просто перебираем всё, что
            // предлагает агент.
            let public_key = match host.identity_id {
                Some(id) => load_identity(pool, id).await?.public_key,
                None => None,
            };
            Ok(AuthMaterial::Agent { public_key })
        }
        other => Err(crate::errors::opaque(
            "ssh",
            format!("неизвестный auth_method '{other}'"),
        )),
    }
}

fn host_address_label(host: &Host) -> String {
    format!("{}:{}", host.hostname, host.port)
}

/// Предел длины цепочки jump-хостов (#151).
///
/// Ограничение не техническое, а защитное: каждый лишний хоп — это ещё одно
/// TCP-соединение и ещё одно рукопожатие, а цепочки длиннее пары хостов на
/// практике означают ошибку в конфигурации, а не замысел.
const MAX_JUMP_DEPTH: usize = 8;

/// Собрать цепочку подключения по `jump_host_id`.
///
/// Возвращает хосты в порядке подключения: от самого дальнего jump-хоста
/// к целевому (целевой — всегда последний). Без jump-хостов вернёт цепочку
/// из одного элемента, то есть обычное прямое подключение.
async fn build_jump_chain(pool: &SqlitePool, target: Host) -> Result<Vec<Host>, String> {
    let mut visited = vec![target.id];
    let mut chain = vec![target];

    while let Some(jump_id) = chain.last().and_then(|h| h.jump_host_id) {
        // Цикл (A → B → A) иначе увёл бы обход в бесконечность.
        if visited.contains(&jump_id) {
            return Err(crate::errors::invalid(
                "ssh",
                format!("цикл в цепочке jump-хостов: host {jump_id} повторяется"),
                crate::errors::codes::SSH_JUMP_LOOP,
            ));
        }
        if chain.len() >= MAX_JUMP_DEPTH {
            return Err(crate::errors::invalid(
                "ssh",
                format!("цепочка jump-хостов длиннее {MAX_JUMP_DEPTH}"),
                crate::errors::codes::SSH_JUMP_TOO_DEEP,
            ));
        }
        let jump = load_host(pool, jump_id).await?;
        visited.push(jump_id);
        chain.push(jump);
    }

    chain.reverse();
    Ok(chain)
}

/// Канал агента OpenSSH для Windows (служба `ssh-agent`).
#[cfg(windows)]
const WINDOWS_AGENT_PIPE: &str = r"\\.\pipe\openssh-ssh-agent";

/// Где слушает ssh-agent.
///
/// На unix — только `SSH_AUTH_SOCK`. На Windows агент OpenSSH слушает
/// именованный канал, и переменную обычно никто не задаёт; а если её выставил
/// Git Bash, там unix-путь msys, который нам не открыть, — такой пропускаем.
fn agent_socket_path() -> Option<String> {
    let env = std::env::var("SSH_AUTH_SOCK").ok().filter(|v| !v.is_empty());
    #[cfg(windows)]
    {
        Some(
            env.filter(|v| v.starts_with(r"\\.\pipe\"))
                .unwrap_or_else(|| WINDOWS_AGENT_PIPE.to_string()),
        )
    }
    #[cfg(not(windows))]
    {
        env
    }
}

#[cfg(unix)]
type AgentIo = tokio::net::UnixStream;
#[cfg(windows)]
type AgentIo = tokio::net::windows::named_pipe::NamedPipeClient;

/// Открывает соединение с агентом: unix-сокет или именованный канал Windows.
///
/// Тип конкретный, не `Box<dyn AgentStream>`: с трейт-объектом rustc не может
/// доказать `Send` для future сессии целиком.
async fn open_agent_stream(path: &str) -> std::io::Result<AgentIo> {
    #[cfg(unix)]
    {
        tokio::net::UnixStream::connect(path).await
    }
    #[cfg(windows)]
    {
        tokio::net::windows::named_pipe::ClientOptions::new().open(path)
    }
}

/// Отвечает ли агент: можно ли открыть его сокет (канал) прямо сейчас.
///
/// «Файл есть» не годится: агент мог завершиться, оставив сокет и переменную.
/// Проброс такого сокета оставил бы удалённый `ssh` ждать до тайм-аута.
async fn agent_reachable(path: &str) -> bool {
    open_agent_stream(path).await.is_ok()
}

/// Сообщение о недоступном агенте. Без имён переменных и путей: на Linux и
/// macOS агент ищется через `SSH_AUTH_SOCK`, на Windows — в именованном
/// канале OpenSSH, и подсказка про одну из схем на другой системе путает.
fn agent_connect_error(e: &dyn std::fmt::Display) -> String {
    format!(
        "не удалось подключиться к ssh-agent: {e}. \
         Проверьте, что агент ssh вашей системы запущен"
    )
}

/// Аутентификация через ssh-agent (#150).
///
/// Приватный ключ агент не отдаёт — он сам подписывает challenge, поэтому
/// `AgentClient` выступает как `Signer`. Ключи перебираем по очереди: сервер
/// может принимать не первый из предложенных.
async fn authenticate_via_agent(
    session: &mut Handle<ClientHandler>,
    host: &Host,
    wanted_public_key: Option<&str>,
) -> Result<bool, String> {
    let sock = agent_socket_path().ok_or_else(|| {
        "ssh-agent не найден: проверьте, что агент ssh вашей системы запущен".to_string()
    })?;
    let stream = open_agent_stream(&sock)
        .await
        .map_err(|e| agent_connect_error(&e))?;
    let mut agent = russh::keys::agent::client::AgentClient::connect(stream);

    let identities = agent
        .request_identities()
        .await
        .map_err(|e| format!("ssh-agent не отдал список ключей: {e}"))?;

    if identities.is_empty() {
        return Err("в ssh-agent нет ни одного ключа (добавьте через `ssh-add`)".to_string());
    }

    // Если у identity записан конкретный публичный ключ — работаем только с ним.
    let candidates: Vec<_> = match wanted_public_key {
        Some(wanted) => {
            let parsed = russh::keys::ssh_key::PublicKey::from_openssh(wanted.trim())
                .map_err(|e| format!("не удалось разобрать публичный ключ identity: {e}"))?;
            let matched: Vec<_> = identities
                .into_iter()
                .filter(|k| k.key_data() == parsed.key_data())
                .collect();
            if matched.is_empty() {
                return Err(
                    "ключ, указанный в identity, не найден в ssh-agent (добавьте его через `ssh-add`)"
                        .to_string(),
                );
            }
            matched
        }
        None => identities,
    };

    let rsa_hash = session
        .best_supported_rsa_hash()
        .await
        .map_err(|e| format!("rsa hash negotiation failed: {e}"))?
        .flatten();

    for key in candidates {
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        let result = session
            .authenticate_publickey_with(host.username.clone(), key, rsa_hash, &mut agent)
            .await
            .map_err(|e| format!("ssh-agent auth failed: {e}"))?;
        if result.success() {
            info!(target: "ssh.agent", "принят ключ из агента {fingerprint}");
            return Ok(true);
        }
        info!(target: "ssh.agent", "сервер отклонил ключ {fingerprint}, пробуем следующий");
    }

    Ok(false)
}

/// Payload запроса кода, уходящий в UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthPromptPayload {
    pub session_id: Uuid,
    pub hostname: String,
    /// Заголовок от сервера (может быть пустым).
    pub name: String,
    /// Пояснение от сервера (может быть пустым).
    pub instructions: String,
    pub prompts: Vec<AuthPromptField>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthPromptField {
    pub prompt: String,
    /// `false` — ввод скрывать (пароль, а не код).
    pub echo: bool,
}

/// Интерактивная аутентификация: цикл «вопрос сервера → ответ человека» (#153).
///
/// Сервер может задать сколько угодно наборов вопросов, в том числе пустых
/// (так реализуют «нажмите подтверждение в приложении»), поэтому это цикл,
/// а не один обмен.
async fn authenticate_keyboard_interactive(
    session: &mut Handle<ClientHandler>,
    host: &Host,
    app: &AppHandle,
    manager: &crate::ssh::SshManager,
    session_id: Uuid,
) -> Result<bool, String> {
    use russh::client::KeyboardInteractiveAuthResponse as Resp;

    // Ограничение на число раундов: сервер, зациклившийся на запросах, иначе
    // держал бы пользователя в диалоге бесконечно.
    const MAX_ROUNDS: usize = 10;

    let mut response = session
        .authenticate_keyboard_interactive_start(host.username.clone(), None)
        .await
        .map_err(|e| format!("keyboard-interactive start failed: {e}"))?;

    for _ in 0..MAX_ROUNDS {
        match response {
            Resp::Success => return Ok(true),
            Resp::Failure { .. } => return Ok(false),
            Resp::InfoRequest {
                name,
                instructions,
                prompts,
            } => {
                // Пустой набор вопросов отвечаем пустым списком, не дёргая
                // пользователя: сервер так подтверждает промежуточный шаг.
                let answers = if prompts.is_empty() {
                    Vec::new()
                } else {
                    request_answers_from_user(
                        app,
                        manager,
                        session_id,
                        host,
                        name,
                        instructions,
                        &prompts,
                    )
                    .await?
                };

                response = session
                    .authenticate_keyboard_interactive_respond(answers)
                    .await
                    .map_err(|e| format!("keyboard-interactive respond failed: {e}"))?;
            }
        }
    }

    Err(crate::errors::invalid(
        "ssh",
        format!("сервер запросил больше {MAX_ROUNDS} раундов интерактивной аутентификации"),
        crate::errors::codes::SSH_TOO_MANY_ROUNDS,
    ))
}

/// Спросить у пользователя ответы и дождаться их.
async fn request_answers_from_user(
    app: &AppHandle,
    manager: &crate::ssh::SshManager,
    session_id: Uuid,
    host: &Host,
    name: String,
    instructions: String,
    prompts: &[russh::client::Prompt],
) -> Result<Vec<String>, String> {
    let (tx, rx) = oneshot::channel();
    manager
        .register_auth_prompt(
            session_id,
            crate::ssh::manager::PendingAuthPrompt { responder: tx },
        )
        .await?;

    let payload = AuthPromptPayload {
        session_id,
        hostname: host_address_label(host),
        name,
        instructions,
        prompts: prompts
            .iter()
            .map(|p| AuthPromptField {
                prompt: p.prompt.clone(),
                echo: p.echo,
            })
            .collect(),
    };
    crate::attention::needs_decision(app).await;
    app.emit(SESSION_AUTH_PROMPT_EVENT, &payload)
        .map_err(|e| format!("emit auth prompt failed: {e}"))?;

    match rx.await {
        Ok(Some(answers)) => Ok(answers),
        // Пользователь отменил ввод или сессия закрылась.
        Ok(None) => Err("ввод кода отменён".to_string()),
        Err(_) => Err("окно ввода кода закрылось раньше ответа".to_string()),
    }
}

/// Сколько байт вывода принимаем от одной команды.
///
/// Предел нужен не из экономии: команда вроде `cat` на большом файле или
/// `journalctl` без `-n` вернёт сотни мегабайт, и они пойдут в память, а
/// оттуда — в контекст агента. Полмегабайта хватает любому разумному выводу,
/// а обрезку мы честно отмечаем.
const EXEC_OUTPUT_LIMIT: usize = 512 * 1024;

/// Сколько ждать читающую команду.
///
/// Без предела `tail -f`, `journalctl -f` или зависшая на сетевой файловой
/// системе `ls` держат канал вечно: агент не получает ответа, а мы — не
/// освобождаем сессию. Ограничение по объёму вывода тут не спасает, потому
/// что молчащая команда ничего не печатает.
pub const EXEC_TIMEOUT_READ: std::time::Duration = std::time::Duration::from_secs(30);

/// Сколько ждать шаг подтверждённого плана (#213).
///
/// Те же 30 секунд обрывали `docker build`, миграцию базы и `apt upgrade` —
/// причём **посреди изменения**, оставляя сервер в наполовину изменённом
/// состоянии. Это худший исход из возможных: не «не сделали», а «сделали
/// половину и не сказали, какую».
///
/// Предел всё же есть: подтверждённый план не должен держать сессию вечно,
/// если шаг ушёл в бесконечный цикл или ждёт ввода, которого не будет.
pub const EXEC_TIMEOUT_LONG: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// Выполнить одну команду в отдельном канале.
/// Обработчик здесь не важен: команда выполняется одинаково, кто бы ни принял
/// ключ сервера. Обобщение сделано ради проверки — иначе тест обязан был бы
/// собрать `ClientHandler`, которому нужен `AppHandle` целого приложения.
async fn run_exec<H: client::Handler>(
    session: &Handle<H>,
    command: &str,
    limit: crate::ssh::session::ExecLimit,
) -> Result<crate::ssh::session::ExecResult, String> {
    let mut channel = session
        .channel_open_session()
        .await
        .map_err(|e| format!("не удалось открыть канал: {e}"))?;

    // Без PTY намеренно: с ним программы включают раскраску и переносы под
    // ширину терминала, и вывод, предназначенный человеку, приходится потом
    // очищать от управляющих последовательностей.
    channel
        .exec(true, command)
        .await
        .map_err(|e| format!("не удалось запустить команду: {e}"))?;

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_code = None;
    let mut truncated = false;
    let deadline = tokio::time::Instant::now() + limit.duration();

    loop {
        // Ждём следующего куска, но не дольше оставшегося времени. Проверять
        // срок только между сообщениями бесполезно: молчащая команда не
        // присылает ничего и цикл просто стоит.
        let Ok(next) = tokio::time::timeout_at(deadline, channel.wait()).await else {
            let _ = channel.close().await;
            return match limit {
                crate::ssh::session::ExecLimit::Fail(d) => Err(crate::errors::invalid(
                    "ssh",
                    format!("команда не уложилась в {} с", d.as_secs()),
                    crate::errors::codes::SSH_COMMAND_TIMEOUT,
                )),
                // Наблюдение за выводом (#218): срок истёк — это и есть конец
                // наблюдения, а собранное и было целью.
                crate::ssh::session::ExecLimit::Collect(_) => Ok(crate::ssh::session::ExecResult {
                    stdout: String::from_utf8_lossy(&stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&stderr).into_owned(),
                    exit_code: None,
                    truncated,
                    stopped_by_time: true,
                }),
            };
        };
        let Some(msg) = next else { break };

        match msg {
            ChannelMsg::Data { data } => {
                if stdout.len() + data.len() > EXEC_OUTPUT_LIMIT {
                    let room = EXEC_OUTPUT_LIMIT.saturating_sub(stdout.len());
                    stdout.extend_from_slice(&data[..room]);
                    truncated = true;
                } else {
                    stdout.extend_from_slice(&data);
                }
            }
            // ext == 1 — это stderr; других расширенных потоков в SSH нет.
            ChannelMsg::ExtendedData { data, ext: 1 } => {
                if stderr.len() + data.len() > EXEC_OUTPUT_LIMIT {
                    let room = EXEC_OUTPUT_LIMIT.saturating_sub(stderr.len());
                    stderr.extend_from_slice(&data[..room]);
                    truncated = true;
                } else {
                    stderr.extend_from_slice(&data);
                }
            }
            ChannelMsg::ExitStatus { exit_status } => exit_code = Some(exit_status),
            // Только Close, НЕ Eof. Порядок в протоколе такой: данные → Eof →
            // ExitStatus → Close. Выход по Eof теряет код возврата, и агент
            // не отличит успешную команду от провалившейся — то есть пойдёт
            // дальше по плану после неудачного шага.
            ChannelMsg::Close => break,
            _ => {}
        }
    }

    Ok(crate::ssh::session::ExecResult {
        // from_utf8_lossy, а не строгий разбор: вывод команд бывает каким
        // угодно, и падать из-за одного неудачного байта в логе нельзя.
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        exit_code,
        truncated,
        stopped_by_time: false,
    })
}

/// Аутентификация на одном хопе. Вынесена отдельно, потому что при ProxyJump
/// шагов несколько и у каждого своя identity.
async fn authenticate_hop(
    session: &mut Handle<ClientHandler>,
    host: &Host,
    material: AuthMaterial,
    app: &AppHandle,
    manager: &crate::ssh::SshManager,
    session_id: Uuid,
) -> Result<(), String> {
    // Первый метод — тот, что выбрал пользователь: пароль, ключ или агент.
    // Ровно как `ssh user@host`.
    let result = match material {
        AuthMaterial::Password(password) => session
            .authenticate_password(host.username.clone(), password)
            .await
            .map_err(|e| format!("auth send failed: {e}"))?,
        AuthMaterial::PrivateKey { path, passphrase } => {
            let key_pair = load_secret_key(&path, passphrase.as_deref())
                .map_err(|e| format!("load private key '{path}' failed: {e}"))?;
            let rsa_hash = session
                .best_supported_rsa_hash()
                .await
                .map_err(|e| format!("rsa hash negotiation failed: {e}"))?
                .flatten();
            session
                .authenticate_publickey(
                    host.username.clone(),
                    PrivateKeyWithHashAlg::new(Arc::new(key_pair), rsa_hash),
                )
                .await
                .map_err(|e| format!("auth send failed: {e}"))?
        }
        AuthMaterial::Agent { public_key } => {
            return authenticate_via_agent_checked(session, host, public_key.as_deref()).await;
        }
    };

    // Дальше — как в самом OpenSSH: если сервер говорит «этого мало, отвечай
    // на вопросы», отвечаем. Пользователь этот метод не выбирает и знать о нём
    // не обязан: так настроен сервер, а не наш клиент.
    //
    // Сюда попадают обе типовые ситуации: ключ принят частично и сервер ждёт
    // ещё чего-то, либо основной метод отклонён, но остался интерактивный.
    let ok = match result {
        russh::client::AuthResult::Success => true,
        russh::client::AuthResult::Failure {
            remaining_methods, ..
        } if remaining_methods
            .iter()
            .any(|m| matches!(m, russh::MethodKind::KeyboardInteractive)) =>
        {
            authenticate_keyboard_interactive(session, host, app, manager, session_id).await?
        }
        russh::client::AuthResult::Failure { .. } => false,
    };

    if !ok {
        return Err(crate::errors::invalid(
            "ssh",
            format!(
                "аутентификация отклонена для {}@{}",
                host.username,
                host_address_label(host)
            ),
            crate::errors::codes::SSH_AUTH_FAILED,
        ));
    }
    Ok(())
}

/// Аутентификация агентом с прежней проверкой результата.
async fn authenticate_via_agent_checked(
    session: &mut Handle<ClientHandler>,
    host: &Host,
    public_key: Option<&str>,
) -> Result<(), String> {
    if authenticate_via_agent(session, host, public_key).await? {
        Ok(())
    } else {
        Err(crate::errors::invalid(
            "ssh",
            format!(
                "аутентификация отклонена для {}@{}",
                host.username,
                host_address_label(host)
            ),
            crate::errors::codes::SSH_AUTH_FAILED,
        ))
    }
}

#[tauri::command]
pub async fn ssh_connect(
    state: State<'_, AppState>,
    app: AppHandle,
    host_id: i64,
) -> Result<ConnectResponse, String> {
    connect_session(state, app, host_id, false).await
}

/// Соединение только для файлов (экран SFTP): та же проверка ключа хоста и
/// вход, что у терминала, но без PTY и оболочки на сервере. Во вкладки не
/// попадает; закрывается обычным `ssh_disconnect`.
#[tauri::command]
pub async fn sftp_connect(
    state: State<'_, AppState>,
    app: AppHandle,
    host_id: i64,
) -> Result<ConnectResponse, String> {
    connect_session(state, app, host_id, true).await
}

async fn connect_session(
    state: State<'_, AppState>,
    app: AppHandle,
    host_id: i64,
    files_only: bool,
) -> Result<ConnectResponse, String> {
    let host = load_host(&state.db, host_id).await?;
    let auth_material = resolve_auth(&host, &state.db, &state.master).await?;
    let session_id = Uuid::new_v4();

    let (cmd_tx, cmd_rx) = mpsc::channel::<SessionCmd>(64);
    let handle = SessionHandle {
        host_id,
        files_only,
        status: SessionStatus::Connecting,
        cmd_tx,
    };
    state.ssh.insert(session_id, handle).await;

    let manager = state.ssh.clone();
    let forwards = state.forwards.clone();
    let master_clone = state.master.clone();
    let app_clone = app.clone();
    let db_clone = state.db.clone();

    // Connection log: record start
    let _ = crate::connection_log::start(&db_clone, host_id, session_id).await;

    tokio::spawn(async move {
        let result = run_session(
            SessionEnv {
                app: app_clone.clone(),
                manager: manager.clone(),
                forwards: forwards.clone(),
                db: db_clone.clone(),
                master: master_clone.clone(),
            },
            session_id,
            host,
            auth_material,
            cmd_rx,
            files_only,
        )
        .await;

        // Пробросы жили поверх этой сессии: без неё открывать каналы не через
        // что, а слушающий сокет остался бы висеть и занимать порт.
        forwards.stop_for_session(session_id).await;

        match &result {
            Ok(_) => {
                let _ = crate::connection_log::finish(&db_clone, session_id, "closed", None).await;
                emit_closed(&app_clone, session_id, result.unwrap());
            }
            Err(e) => {
                let _ =
                    crate::connection_log::finish(&db_clone, session_id, "failed", Some(e)).await;
                error!(target: "ssh", "session {session_id} failed: {e}");
                emit_status(
                    &app_clone,
                    session_id,
                    SessionStatus::Failed,
                    Some(e.clone()),
                );
                emit_closed(&app_clone, session_id, None);
            }
        }
        manager.remove(session_id).await;
    });

    Ok(ConnectResponse { session_id })
}

/// Начать запись сессии: собрать имя файла и открыть рекордер (#144).
async fn start_recording(
    host: &Host,
    session_id: Uuid,
    cols: u32,
    rows: u32,
) -> Result<crate::recording::Recorder, String> {
    let started = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = crate::recording::build_path(&host.name, started)
        .map_err(|e| format!("не удалось выбрать файл записи: {e}"))?;
    crate::recording::Recorder::start(
        path,
        cols,
        rows,
        Some(format!("{} ({})", host.name, host_address_label(host))),
    )
    .await
    .map_err(|e| {
        warn!(target: "recording", "session {session_id}: старт записи не удался: {e}");
        format!("не удалось начать запись: {e}")
    })
}

/// Записать кусок вывода, если запись включена.
///
/// Ошибка записи не должна ронять сессию: диск мог кончиться, а терминал
/// пользователю нужен. Поэтому только предупреждение в лог.
async fn record_chunk(
    recorder: &mut Option<crate::recording::Recorder>,
    session_id: Uuid,
    data: &[u8],
) {
    if let Some(rec) = recorder.as_mut() {
        if let Err(e) = rec.write_output(data).await {
            warn!(target: "recording", "session {session_id}: сбой записи: {e}");
        }
    }
}

/// Поднять пробросы, помеченные `auto_start`, сразу после подключения (#133).
async fn start_auto_forwards(
    forwards: &crate::forwarding::ForwardManager,
    manager: &crate::ssh::SshManager,
    db: &SqlitePool,
    session_id: Uuid,
    host_id: i64,
) {
    let sql = "SELECT id, host_id, kind, label, local_host, local_port, remote_host, \
               remote_port, auto_start, created_at, updated_at \
               FROM port_forwards WHERE host_id = ?1 AND auto_start = 1";
    let rules = match sqlx::query_as::<_, crate::db::models::PortForward>(sql)
        .bind(host_id)
        .fetch_all(db)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            warn!(target: "forward", "session {session_id}: не удалось прочитать правила: {e}");
            return;
        }
    };

    for rule in rules {
        if rule.kind == "remote" {
            warn!(target: "forward", "проброс {} пропущен: remote пока не поддерживается", rule.id);
            continue;
        }
        match forwards
            .start(
                manager.clone(),
                session_id,
                crate::forwarding::ForwardSpec {
                    forward_id: rule.id,
                    kind: rule.kind.clone(),
                    local_host: rule.local_host.clone(),
                    local_port: rule.local_port as u16,
                    remote_host: rule.remote_host.clone(),
                    remote_port: rule.remote_port.map(|p| p as u16),
                },
            )
            .await
        {
            Ok(bound) => info!(target: "forward", "автозапуск проброса {} на {bound}", rule.id),
            Err(e) => warn!(target: "forward", "автозапуск проброса {} не удался: {e}", rule.id),
        }
    }
}

/// Открыть дополнительный канал и запросить на нём подсистему `sftp` (#120).
///
/// Живёт рядом с `run_session`, потому что только там доступен `Handle`.
/// Само SFTP-рукопожатие делает вызывающая сторона — здесь только канал.
async fn open_sftp_channel(
    session: &Handle<ClientHandler>,
) -> Result<russh::Channel<client::Msg>, String> {
    let channel = session
        .channel_open_session()
        .await
        .map_err(|e| format!("sftp channel open failed: {e}"))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|e| format!("sftp subsystem request failed: {e}"))?;
    Ok(channel)
}

/// Окружение сессии: всё, что нужно `run_session` от приложения.
/// Отдельной структурой, чтобы список аргументов не разрастался.
struct SessionEnv {
    app: AppHandle,
    manager: crate::ssh::SshManager,
    forwards: crate::forwarding::ForwardManager,
    db: SqlitePool,
    /// Нужен для секретов jump-хостов: их пароли разрешаются уже внутри сессии.
    master: MasterKeyring,
}

async fn run_session(
    env: SessionEnv,
    session_id: Uuid,
    host: Host,
    auth_material: AuthMaterial,
    mut cmd_rx: mpsc::Receiver<SessionCmd>,
    files_only: bool,
) -> Result<Option<u32>, String> {
    let SessionEnv {
        app,
        manager,
        forwards,
        db,
        master: master_keys,
    } = env;
    emit_status(&app, session_id, SessionStatus::Connecting, None);
    manager
        .set_status(session_id, SessionStatus::Connecting)
        .await;

    // `take()` ниже отдаёт материал целевому хосту ровно один раз.
    let mut auth_material = Some(auth_material);

    let keep_alive_interval = if host.keep_alive_interval > 0 {
        Some(Duration::from_secs(host.keep_alive_interval as u64))
    } else {
        None
    };

    let config = Arc::new(client::Config {
        inactivity_timeout: None,
        keepalive_interval: keep_alive_interval,
        keepalive_max: 3,
        ..Default::default()
    });

    // Цепочка подключения: [дальний jump, ..., ближний jump, целевой хост] (#151).
    // Без jump-хостов в ней ровно один элемент — обычное прямое подключение.
    let chain = build_jump_chain(&db, host.clone()).await?;
    let hops = chain.len();
    if hops > 1 {
        info!(
            target: "ssh",
            "session {session_id}: подключение через {} jump-хост(ов)", hops - 1
        );
    }

    emit_status(&app, session_id, SessionStatus::Authenticating, None);
    manager
        .set_status(session_id, SessionStatus::Authenticating)
        .await;

    // Промежуточные хопы держим живыми до конца сессии: туннель до цели идёт
    // через их каналы, и если уронить хендл — оборвётся и целевая сессия.
    let mut jump_handles: Vec<Handle<ClientHandler>> = Vec::with_capacity(hops - 1);
    let mut target_session: Option<Handle<ClientHandler>> = None;

    for (index, hop) in chain.iter().enumerate() {
        let is_target = index + 1 == hops;
        let handler = ClientHandler {
            session_id,
            hostname: host_address_label(hop),
            db: db.clone(),
            app: app.clone(),
            manager: manager.clone(),
            forwards: forwards.clone(),
        };

        let mut hop_session: Handle<ClientHandler> = if index == 0 {
            // Первый хоп — прямое TCP-подключение.
            client::connect(
                config.clone(),
                (hop.hostname.as_str(), hop.port as u16),
                handler,
            )
            .await
            .map_err(|e| crate::errors::ssh_connect("ssh", &e))?
        } else {
            // Остальные — поверх direct-tcpip канала предыдущего хопа.
            let previous = jump_handles
                .last()
                .expect("предыдущий хоп обязан существовать при index > 0");
            let channel = previous
                .channel_open_direct_tcpip(hop.hostname.clone(), hop.port as u32, "127.0.0.1", 0)
                .await
                .map_err(|e| {
                    crate::errors::invalid(
                        "ssh",
                        format!("туннель до {} не открылся: {e}", host_address_label(hop)),
                        crate::errors::codes::SSH_TUNNEL_FAILED,
                    )
                })?;
            client::connect_stream(config.clone(), channel.into_stream(), handler)
                .await
                .map_err(|e| {
                    format!(
                        "handshake через туннель до {} не удался: {e}",
                        host_address_label(hop)
                    )
                })?
        };

        info!(
            target: "ssh",
            "session {session_id}: handshake с {} ok", host_address_label(hop)
        );

        // Целевой хост использует материал, разрешённый ещё в `ssh_connect`
        // (чтобы отсутствующий пароль всплывал сразу). Для jump-хостов
        // достаём их собственные секреты здесь.
        let material = if is_target {
            auth_material
                .take()
                .ok_or_else(|| "auth material for target host is missing".to_string())?
        } else {
            resolve_auth(hop, &db, &master_keys).await?
        };
        authenticate_hop(&mut hop_session, hop, material, &app, &manager, session_id).await?;
        info!(
            target: "ssh",
            "session {session_id}: auth на {} ok", host_address_label(hop)
        );

        if is_target {
            target_session = Some(hop_session);
        } else {
            jump_handles.push(hop_session);
        }
    }

    let session = target_session.expect("цепочка не может быть пустой");

    // Экран SFTP: терминал не нужен, оболочку на сервере не заводим.
    let mut channel: Option<russh::Channel<client::Msg>> = None;
    if files_only {
        emit_status(&app, session_id, SessionStatus::Connected, None);
        manager
            .set_status(session_id, SessionStatus::Connected)
            .await;
        info!(target: "ssh", "session {session_id}: соединение для файлов готово");
    } else {
        emit_status(&app, session_id, SessionStatus::PtyOpening, None);
        manager
            .set_status(session_id, SessionStatus::PtyOpening)
            .await;

        let chan = session
            .channel_open_session()
            .await
            .map_err(|e| format!("channel open failed: {e}"))?;

        chan
            .request_pty(false, "xterm-256color", 80, 24, 0, 0, &[])
            .await
            .map_err(|e| format!("request pty failed: {e}"))?;

        // Проброс агента запрашивается ДО shell: сервер должен успеть выставить
        // SSH_AUTH_SOCK в окружение запускаемой оболочки (#150a).
        //
        // Просим проброс, только если у нас есть что пробрасывать. Иначе сервер
        // выставит `SSH_AUTH_SOCK` на агента, которого нет, и `ssh`/`git` на нём
        // зависнут в ожидании ответа.
        let forward_agent = host.forward_agent
            && match agent_socket_path() {
                Some(path) => agent_reachable(&path).await,
                None => false,
            };
        if host.forward_agent && !forward_agent {
            warn!(
                target: "ssh.agent",
                "session {session_id}: проброс агента включён, но локального агента нет (SSH_AUTH_SOCK) — не запрашиваю"
            );
        }
        if forward_agent {
            match chan.agent_forward(false).await {
                Ok(()) => info!(target: "ssh.agent", "session {session_id}: запрошен проброс агента"),
                // Сервер может запрещать проброс (`AllowAgentForwarding no`) — это
                // не повод не открывать терминал.
                Err(e) => {
                    warn!(target: "ssh.agent", "session {session_id}: проброс агента отклонён: {e}")
                }
            }
        }

        chan
            .request_shell(false)
            .await
            .map_err(|e| format!("request shell failed: {e}"))?;

        emit_status(&app, session_id, SessionStatus::Connected, None);
        manager
            .set_status(session_id, SessionStatus::Connected)
            .await;
        info!(target: "ssh", "session {session_id}: shell ready");
        channel = Some(chan);
    }

    // Автозапуск пробросов (#133). Ошибки не роняют сессию: занятый локальный
    // порт — обычная ситуация, а терять из-за него терминал незачем.
    if !files_only {
        start_auto_forwards(&forwards, &manager, &db, session_id, host.id).await;
    }

    let mut exit_code: Option<u32> = None;
    let mut disconnect_requested = false;
    // Запись сессии (#144). Размер PTY держим актуальным: он попадает
    // в заголовок записи, и с неверным размером плеер нарисует кашу.
    let mut recorder: Option<crate::recording::Recorder> = None;
    let mut pty_cols: u32 = 80;
    let mut pty_rows: u32 = 24;

    // Запросы, которые ждут ответа сервера (замер задержки, SFTP, команды
    // агента, пробросы), идут отдельными задачами. Раньше цикл ждал их сам, и
    // на плохой сети каждый замер задержки на секунды останавливал всё
    // остальное: ввод, вывод и закрытие вкладки.
    //
    // RwLock только потому, что `tcpip_forward` берёт `&mut self`; остальным
    // хватает чтения, и они идут параллельно.
    let session = Arc::new(tokio::sync::RwLock::new(session));
    let mut requests = tokio::task::JoinSet::new();

    loop {
        tokio::select! {
            biased;

            maybe_cmd = cmd_rx.recv() => {
                match maybe_cmd {
                    Some(SessionCmd::Send(bytes)) => {
                        if let Some(channel) = channel.as_ref() {
                            if let Err(e) = channel.data(&bytes[..]).await {
                                warn!(target: "ssh", "session {session_id}: write failed: {e}");
                            }
                        }
                    }
                    Some(SessionCmd::Resize { cols, rows }) => {
                        pty_cols = cols;
                        pty_rows = rows;
                        if let Some(channel) = channel.as_ref() {
                            if let Err(e) = channel.window_change(cols, rows, 0, 0).await {
                                warn!(target: "ssh", "session {session_id}: resize failed: {e}");
                            }
                        }
                    }
                    Some(SessionCmd::StartRecording { responder }) => {
                        let result = if recorder.is_some() {
                            Err("запись уже идёт".to_string())
                        } else {
                            match start_recording(&host, session_id, pty_cols, pty_rows).await {
                                Ok(rec) => {
                                    let path = rec.path().to_string_lossy().to_string();
                                    recorder = Some(rec);
                                    info!(target: "recording", "session {session_id}: запись в {path}");
                                    Ok(path)
                                }
                                Err(e) => Err(e),
                            }
                        };
                        let _ = responder.send(result);
                    }
                    Some(SessionCmd::StopRecording { responder }) => {
                        let result = match recorder.take() {
                            Some(rec) => rec
                                .finish()
                                .await
                                .map(|p| p.to_string_lossy().to_string())
                                .map_err(|e| format!("не удалось закрыть запись: {e}")),
                            None => Err("запись не велась".to_string()),
                        };
                        if let Ok(path) = &result {
                            info!(target: "recording", "session {session_id}: запись закрыта: {path}");
                        }
                        let _ = responder.send(result);
                    }
                    Some(SessionCmd::Latency { responder }) => {
                        let session = session.clone();
                        requests.spawn(async move {
                            let session = session.read().await;
                            let _ = responder.send(measure_latency(&session).await);
                        });
                    }
                    Some(SessionCmd::OpenSftp { responder }) => {
                        let session = session.clone();
                        requests.spawn(async move {
                            let result = open_sftp_channel(&*session.read().await).await;
                            if let Err(e) = &result {
                                warn!(target: "sftp", "session {session_id}: open sftp channel failed: {e}");
                            }
                            // Ошибку получателя игнорируем: вызывающая команда могла
                            // отвалиться по таймауту — сессию это ронять не должно.
                            let _ = responder.send(result);
                        });
                    }
                    Some(SessionCmd::Exec {
                        command,
                        limit,
                        responder,
                    }) => {
                        let session = session.clone();
                        requests.spawn(async move {
                            let result = run_exec(&*session.read().await, &command, limit).await;
                            if let Err(e) = &result {
                                warn!(target: "ssh.exec", "session {session_id}: {e}");
                            }
                            let _ = responder.send(result);
                        });
                    }
                    Some(SessionCmd::OpenDirectTcpip { host, port, originator_host, originator_port, responder }) => {
                        let session = session.clone();
                        requests.spawn(async move {
                            let result = session
                                .read()
                                .await
                                .channel_open_direct_tcpip(host.clone(), port, originator_host, originator_port)
                                .await
                                .map_err(|e| format!("direct-tcpip до {host}:{port} не открылся: {e}"));
                            if let Err(e) = &result {
                                warn!(target: "forward", "session {session_id}: {e}");
                            }
                            let _ = responder.send(result);
                        });
                    }
                    Some(SessionCmd::RequestRemoteForward { address, port, responder }) => {
                        // Снять прослушку интерфейс может только после ответа на
                        // этот запрос, поэтому порядок двух задач не путается.
                        let session = session.clone();
                        requests.spawn(async move {
                            let result = session
                                .write()
                                .await
                                .tcpip_forward(address.clone(), port)
                                .await
                                .map_err(|e| format!("сервер отказал в прослушке {address}:{port}: {e}"));
                            if let Err(e) = &result {
                                warn!(target: "forward", "session {session_id}: {e}");
                            }
                            let _ = responder.send(result);
                        });
                    }
                    Some(SessionCmd::CancelRemoteForward { address, port }) => {
                        let session = session.clone();
                        requests.spawn(async move {
                            if let Err(e) = session.read().await.cancel_tcpip_forward(address.clone(), port).await {
                                warn!(target: "forward", "session {session_id}: снятие прослушки {address}:{port}: {e}");
                            }
                        });
                    }
                    Some(SessionCmd::Disconnect) => {
                        disconnect_requested = true;
                        break;
                    }
                    None => break,
                }
            }

            // Без терминала канала нет: ветка не срабатывает никогда, а сессия
            // живёт, пока её не закроют командой или не оборвётся связь.
            msg = async {
                match channel.as_mut() {
                    Some(channel) => channel.wait().await,
                    None => std::future::pending().await,
                }
            } => {
                match msg {
                    Some(ChannelMsg::Data { data }) => {
                        record_chunk(&mut recorder, session_id, &data).await;
                        emit_data(&app, session_id, data.to_vec());
                    }
                    Some(ChannelMsg::ExtendedData { data, ext: _ }) => {
                        record_chunk(&mut recorder, session_id, &data).await;
                        emit_data(&app, session_id, data.to_vec());
                    }
                    Some(ChannelMsg::ExitStatus { exit_status }) => {
                        exit_code = Some(exit_status);
                    }
                    Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) => {
                        break;
                    }
                    Some(_) => {}
                    None => break,
                }
            }

            // Убираем завершённые запросы, чтобы набор не рос.
            Some(_) = requests.join_next(), if !requests.is_empty() => {}
        }
    }

    // Недоделанные запросы сессии больше не нужны: их ответов никто не ждёт.
    requests.abort_all();

    // Сессия закончилась — закрываем запись, иначе на диске останется
    // файл без дописанного хвоста буфера.
    if let Some(rec) = recorder.take() {
        match rec.finish().await {
            Ok(path) => {
                info!(target: "recording", "session {session_id}: запись закрыта при выходе: {}", path.display())
            }
            Err(e) => {
                warn!(target: "recording", "session {session_id}: не удалось закрыть запись: {e}")
            }
        }
    }

    if disconnect_requested {
        emit_status(&app, session_id, SessionStatus::Disconnecting, None);
        manager
            .set_status(session_id, SessionStatus::Disconnecting)
            .await;
        // Вежливо попрощаться с сервером, но не дольше секунды: закрытие не
        // должно зависеть от сети. Не успели — соединение просто обрывается,
        // и сервер сам закроет сессию.
        let goodbye = async {
            if let Some(channel) = channel.as_ref() {
                let _ = channel.eof().await;
            }
            let _ = session
                .read()
                .await
                .disconnect(russh::Disconnect::ByApplication, "", "en")
                .await;
        };
        if tokio::time::timeout(DISCONNECT_GRACE, goodbye).await.is_err() {
            warn!(target: "ssh", "session {session_id}: сервер не ответил на прощание, соединение закрыто без него");
        }
    }

    emit_status(&app, session_id, SessionStatus::Closed, None);
    manager.set_status(session_id, SessionStatus::Closed).await;
    Ok(exit_code)
}

/// Сколько ждём сервер при закрытии вкладки.
const DISCONNECT_GRACE: Duration = Duration::from_secs(1);

/// Задержка до сервера: время открытия служебного канала. Больше трёх секунд
/// не ждём — такой ответ всё равно не нужен.
async fn measure_latency<H: client::Handler>(session: &Handle<H>) -> Result<u64, String> {
    let started = std::time::Instant::now();
    match tokio::time::timeout(Duration::from_secs(3), session.channel_open_session()).await {
        Ok(Ok(channel)) => {
            let millis = started.elapsed().as_millis() as u64;
            let _ = channel.close().await;
            Ok(millis)
        }
        Ok(Err(e)) => Err(crate::errors::opaque("ssh_latency", e)),
        Err(_) => Err(crate::errors::codes::SSH_TIMED_OUT.to_string()),
    }
}

#[tauri::command]
pub async fn ssh_send(
    state: State<'_, AppState>,
    session_id: Uuid,
    data: Vec<u8>,
) -> Result<(), String> {
    state.ssh.send_cmd(session_id, SessionCmd::Send(data)).await
}

#[tauri::command]
pub async fn ssh_resize(
    state: State<'_, AppState>,
    session_id: Uuid,
    cols: u32,
    rows: u32,
) -> Result<(), String> {
    state
        .ssh
        .send_cmd(session_id, SessionCmd::Resize { cols, rows })
        .await
}

/// Задержка до сервера в миллисекундах — для подписи «connected · 12 ms».
///
/// Окно спрашивает, пока вкладка на виду, раз в несколько секунд: событиями
/// от каждой сессии без нужды шумели бы и закрытые вкладки.
#[tauri::command]
pub async fn ssh_latency(state: State<'_, AppState>, session_id: Uuid) -> Result<u64, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    state
        .ssh
        .send_cmd(session_id, SessionCmd::Latency { responder: tx })
        .await?;
    rx.await
        .map_err(|e| crate::errors::opaque("ssh_latency", e))?
}

#[tauri::command]
pub async fn ssh_disconnect(state: State<'_, AppState>, session_id: Uuid) -> Result<(), String> {
    state.ssh.send_cmd(session_id, SessionCmd::Disconnect).await
}

#[tauri::command]
pub async fn ssh_active_sessions(
    state: State<'_, AppState>,
) -> Result<Vec<SessionSnapshot>, String> {
    Ok(state.ssh.snapshot().await)
}

// ────────────────────────────────────────────────────────────────────
// TOFU accept/reject (#046)
// ────────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn ssh_accept_host_key(
    state: State<'_, AppState>,
    session_id: Uuid,
) -> Result<(), String> {
    let prompt = state.ssh.take_prompt(session_id).await.ok_or_else(|| {
        crate::errors::opaque(
            "ssh",
            format!("нет ожидающего запроса ключа для сессии {session_id}"),
        )
    })?;

    // Сохраняем fingerprint в БД до отправки ответа — если accept'ом UI
    // подтверждает доверие, fingerprint должен оказаться в known_hosts.
    known_hosts::accept(
        &state.db,
        &prompt.hostname,
        &prompt.key_type,
        &prompt.fingerprint,
    )
    .await
    .map_err(map_err)?;

    if prompt.responder.send(true).is_err() {
        warn!(target: "ssh.tofu", "responder dropped before accept reached handler");
    }
    Ok(())
}

#[tauri::command]
pub async fn ssh_reject_host_key(
    state: State<'_, AppState>,
    session_id: Uuid,
) -> Result<(), String> {
    let prompt = state.ssh.take_prompt(session_id).await.ok_or_else(|| {
        crate::errors::opaque(
            "ssh",
            format!("нет ожидающего запроса ключа для сессии {session_id}"),
        )
    })?;
    if prompt.responder.send(false).is_err() {
        warn!(target: "ssh.tofu", "responder dropped before reject reached handler");
    }
    Ok(())
}

/// Ответить на запрос интерактивной аутентификации (#153).
#[tauri::command]
pub async fn ssh_submit_auth_prompt(
    state: State<'_, AppState>,
    session_id: Uuid,
    responses: Vec<String>,
) -> Result<(), String> {
    let pending = state
        .ssh
        .take_auth_prompt(session_id)
        .await
        .ok_or_else(|| {
            crate::errors::opaque(
                "ssh",
                format!("нет ожидающего запроса входа для сессии {session_id}"),
            )
        })?;
    if pending.responder.send(Some(responses)).is_err() {
        warn!(target: "ssh.auth", "responder dropped before answers reached handler");
    }
    Ok(())
}

/// Отменить ввод — сессия уйдёт в FAILED.
#[tauri::command]
pub async fn ssh_cancel_auth_prompt(
    state: State<'_, AppState>,
    session_id: Uuid,
) -> Result<(), String> {
    if let Some(pending) = state.ssh.take_auth_prompt(session_id).await {
        let _ = pending.responder.send(None);
    }
    Ok(())
}

// ────────────────────────────────────────────────────────────────────
// known_hosts CRUD (#049)
// ────────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn known_hosts_list(state: State<'_, AppState>) -> Result<Vec<KnownHost>, String> {
    known_hosts::list(&state.db).await.map_err(map_err)
}

#[tauri::command]
pub async fn known_hosts_remove(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    known_hosts::remove(&state.db, id).await.map_err(map_err)
}

#[tauri::command]
pub async fn known_hosts_clear(state: State<'_, AppState>) -> Result<(), String> {
    known_hosts::clear(&state.db).await.map_err(map_err)
}

// ────────────────────────────────────────────────────────────────────
// Импорт ~/.ssh/known_hosts (#175)
// ────────────────────────────────────────────────────────────────────

/// Запись из файла вместе с тем, что о ней уже известно нам.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownHostsImportItem {
    #[serde(flatten)]
    pub entry: crate::known_hosts::import::ImportEntry,
    /// Такая же запись уже есть и fingerprint совпадает — импортировать нечего.
    pub already_known: bool,
    /// Запись есть, но fingerprint **другой**.
    ///
    /// Это ровно тот случай, ради которого существует предупреждение о смене
    /// ключа: молча перезаписать его импортом значило бы обойти собственный
    /// TOFU. Показываем отдельно и по умолчанию не отмечаем.
    pub conflicts: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownHostsImportPreview {
    pub path: String,
    pub items: Vec<KnownHostsImportItem>,
    pub skipped_hashed: usize,
    pub skipped_wildcard: usize,
    pub skipped_marker: usize,
    pub skipped_unparsable: usize,
}

#[tauri::command]
pub async fn known_hosts_import_preview(
    state: State<'_, AppState>,
) -> Result<KnownHostsImportPreview, String> {
    let path = crate::known_hosts::import::default_path()
        .ok_or_else(|| "не удалось определить домашний каталог".to_string())?;
    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("не удалось прочитать {}: {e}", path.display()))?;

    let preview = crate::known_hosts::import::parse(&content, path.to_string_lossy().to_string());

    let mut items = Vec::with_capacity(preview.entries.len());
    for entry in preview.entries {
        let existing = known_hosts::find(&state.db, &entry.hostname, &entry.key_type)
            .await
            .map_err(map_err)?;
        let (already_known, conflicts, current) = match existing {
            Some(k) if k.fingerprint == entry.fingerprint => (true, false, Some(k.fingerprint)),
            Some(k) => (false, true, Some(k.fingerprint)),
            None => (false, false, None),
        };
        items.push(KnownHostsImportItem {
            entry,
            already_known,
            conflicts,
            current_fingerprint: current,
        });
    }

    Ok(KnownHostsImportPreview {
        path: preview.path,
        items,
        skipped_hashed: preview.skipped_hashed,
        skipped_wildcard: preview.skipped_wildcard,
        skipped_marker: preview.skipped_marker,
        skipped_unparsable: preview.skipped_unparsable,
    })
}

/// Импортировать выбранные записи. Возвращает число добавленных.
#[tauri::command]
pub async fn known_hosts_import(
    state: State<'_, AppState>,
    entries: Vec<crate::known_hosts::import::ImportEntry>,
) -> Result<usize, String> {
    let mut added = 0usize;
    for e in entries {
        known_hosts::accept(&state.db, &e.hostname, &e.key_type, &e.fingerprint)
            .await
            .map_err(map_err)?;
        added += 1;
    }
    info!(target: "known_hosts", "импортировано записей: {added}");
    Ok(added)
}

#[cfg(test)]
mod tests {
    #[test]
    fn agent_error_does_not_name_a_unix_only_variable() {
        let msg = super::agent_connect_error(&"connection refused");
        assert!(!msg.contains("SSH_AUTH_SOCK"), "{msg}");
        assert!(msg.contains("connection refused"));
    }

    use super::*;
    use crate::db::models::{Host, Identity};
    use sqlx::sqlite::SqlitePoolOptions;

    /// Свежая база со всеми миграциями — та же схема, что у пользователя.
    async fn fresh_db() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("подключение к памяти");
        crate::db::MIGRATOR.run(&pool).await.expect("миграции");
        pool
    }

    /// Список колонок хоста обязан покрывать структуру целиком.
    ///
    /// Это не придирка к стилю. `agent_access` появился 24 августа в модели и
    /// в командах хостов, а сюда его дописать забыли — и подключение к любому
    /// хосту падало с «no column found for name: agent_access» три дня.
    /// Сборка при этом была зелёной: список колонок — строка, компилятор её
    /// не читает. Значит проверять должен тест, и проверять на настоящей
    /// схеме, а не на выдуманной.
    /// Запись с пробелом (сохранена старой версией) подключается без него.
    #[tokio::test]
    async fn a_host_saved_with_spaces_is_connected_without_them() {
        let pool = fresh_db().await;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO hosts (name, hostname, port, username, auth_method, sync_id) \
             VALUES (' h ', ' 1.1.1.1', 22, 'root ', 'password', 'sync-sp') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .expect("вставка");
        let host = load_host(&pool, id).await.expect("загрузка");
        assert_eq!((host.hostname.as_str(), host.username.as_str()), ("1.1.1.1", "root"));
        assert_eq!(host_address_label(&host), "1.1.1.1:22");
    }

    #[tokio::test]
    async fn host_fields_cover_the_whole_model() {
        let pool = fresh_db().await;
        sqlx::query(
            // sync_id обязателен: его требует триггер журнала синхронизации.
            "INSERT INTO hosts (name, hostname, port, username, auth_method, sync_id) \
             VALUES ('t', 'example.org', 22, 'root', 'password', 'sync-host')",
        )
        .execute(&pool)
        .await
        .expect("вставка хоста");

        let sql = format!("SELECT {HOST_FIELDS} FROM hosts WHERE name = 't'");
        let host: Host = sqlx::query_as(&sql)
            .fetch_one(&pool)
            .await
            .expect("хост читается тем же списком колонок, каким его грузит подключение");

        assert_eq!(host.name, "t");
        // Значение неважно, важно что колонка вообще доехала до структуры.
        assert!(!host.agent_access);
    }

    /// Проброс агента без агента не запрашивается, а запрошенный канал не
    /// остаётся висеть: `ssh`/`git` на сервере иначе ждут ответа до тайм-аута.
    #[tokio::test]
    async fn a_missing_local_agent_is_not_offered_to_the_server() {
        assert!(!agent_reachable("/нет/такого/agent.sock").await);
        #[cfg(unix)]
        {
            let path = std::env::temp_dir().join(format!("darwaza-agent-{}.sock", std::process::id()));
            std::fs::remove_file(&path).ok();
            // Файл сокета есть, а слушателя нет — агент завершился.
            let dead = tokio::net::UnixListener::bind(&path).unwrap();
            drop(dead);
            assert!(path.exists());
            assert!(!agent_reachable(path.to_str().unwrap()).await);
            let alive = tokio::net::UnixListener::bind(path.with_extension("live")).unwrap();
            assert!(agent_reachable(path.with_extension("live").to_str().unwrap()).await);
            drop(alive);
            std::fs::remove_file(&path).ok();
            std::fs::remove_file(path.with_extension("live")).ok();
        }
    }

    /// То же для идентификаций: там точно так же не хватало `has_totp`.
    #[tokio::test]
    async fn identity_fields_cover_the_whole_model() {
        let pool = fresh_db().await;
        sqlx::query(
            "INSERT INTO identities (label, kind, sync_id) VALUES ('k', 'password', 'sync-id')",
        )
        .execute(&pool)
        .await
        .expect("вставка идентификации");

        let sql = format!("SELECT {IDENTITY_FIELDS} FROM identities WHERE label = 'k'");
        let identity: Identity = sqlx::query_as(&sql)
            .fetch_one(&pool)
            .await
            .expect("идентификация читается тем же списком колонок");

        assert_eq!(identity.label, "k");
        assert!(!identity.has_totp);
    }
}

/// Пределы времени проверяются на настоящем sshd.
///
/// Внутри процесса это не проверить: обрыв по времени — свойство разговора с
/// сервером, а не кода. Тест помечен `#[ignore]`, поднять сервер:
/// `scripts/live-sshd.sh start`.
#[cfg(test)]
mod exec_timeout_live_tests {
    use super::*;

    struct AcceptAnyKey;

    impl client::Handler for AcceptAnyKey {
        type Error = russh::Error;

        async fn check_server_key(&mut self, _key: &PublicKey) -> Result<bool, Self::Error> {
            // Отпечаток проверяется в другом месте (#046); здесь важно время.
            Ok(true)
        }
    }

    async fn connect_live() -> Handle<AcceptAnyKey> {
        let key_path = std::env::var("DARWAZA_LIVE_KEY")
            .unwrap_or_else(|_| "/tmp/darwaza-live-sshd/client_ed25519".to_string());
        let user = std::env::var("USER").unwrap_or_else(|_| "root".to_string());

        let config = Arc::new(client::Config::default());
        let mut session = client::connect(config, ("127.0.0.1", 2222u16), AcceptAnyKey)
            .await
            .expect("sshd на 2222 — scripts/live-sshd.sh start");
        let key = load_secret_key(&key_path, None).expect("ключ клиента");
        let ok = session
            .authenticate_publickey(
                user,
                PrivateKeyWithHashAlg::new(Arc::new(key), Some(HashAlg::Sha256)),
            )
            .await
            .expect("аутентификация");
        assert!(ok.success(), "сервер не принял ключ");
        session
    }

    use crate::ssh::session::ExecLimit;

    /// Долгая команда переживает свой предел, короткий — обрывает.
    ///
    /// Ради этого всё и делалось: общий предел в 30 секунд обрывал шаг плана
    /// посреди изменения (#213). Проверяем оба конца — что предел вообще
    /// работает и что он берётся из вызова, а не из общей константы.
    #[tokio::test]
    #[ignore = "требует локальный sshd на 2222 — scripts/live-sshd.sh"]
    async fn each_command_gets_its_own_time_limit() {
        let session = connect_live().await;

        let err = run_exec(
            &session,
            "sleep 3",
            ExecLimit::Fail(std::time::Duration::from_secs(1)),
        )
        .await
        .expect_err("короткий предел обязан оборвать команду");
        assert_eq!(err, crate::errors::codes::SSH_COMMAND_TIMEOUT);

        let result = run_exec(
            &session,
            "sleep 3",
            ExecLimit::Fail(std::time::Duration::from_secs(30)),
        )
        .await
        .expect("с длинным пределом та же команда обязана дойти до конца");
        assert_eq!(result.exit_code, Some(0));
    }

    /// Наблюдение за выводом отдаёт собранное, а не ошибку (#218).
    ///
    /// Проверяется оба свойства сразу: что вывод за отведённое время дошёл и
    /// что конец наблюдения помечен. Без пометки `exitCode: null` читается
    /// как «что-то сломалось», и агент решит, что команда не выполнялась.
    #[tokio::test]
    #[ignore = "требует локальный sshd на 2222 — scripts/live-sshd.sh"]
    async fn following_returns_what_it_collected() {
        let session = connect_live().await;

        let result = run_exec(
            &session,
            "echo первая; sleep 30",
            ExecLimit::Collect(std::time::Duration::from_secs(2)),
        )
        .await
        .expect("наблюдение обязано вернуть собранное, а не ошибку");

        assert!(result.stopped_by_time, "конец наблюдения не помечен");
        assert_eq!(result.exit_code, None, "команда не завершалась сама");
        assert!(
            result.stdout.contains("первая"),
            "вывод до конца наблюдения потерян: {:?}",
            result.stdout
        );
    }
}
