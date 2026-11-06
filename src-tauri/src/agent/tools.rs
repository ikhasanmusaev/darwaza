//! Инструменты, которые видит агент (#203).
//!
//! Чтение идёт без вопросов: какие хосты разрешены, выполнить читающую
//! команду, прочитать файл, посмотреть каталог и сведения о файле.
//! Изменения идут **только** через подтверждение — `hosts.plan` для команд,
//! `files.write` для правки файла (#205, #206) и все файловые операции #212:
//! скачивание, загрузка, создание каталога, переименование, удаление.
//!
//! Скачивание тоже подтверждается, хотя с сервера оно только читает: оно
//! **пишет на компьютер человека**, по любому пути (решение владельца
//! 2026-09-17).
//!
//! **Агент нигде не получает ни адресов, ни логинов, ни ключей.** Он называет
//! хост по имени, а приложение само находит запись, само достаёт секреты и
//! само проверяет отпечаток сервера. Это и есть смысл всей затеи: доступ к
//! приложению, а не к учётным данным.

use super::{
    classify_steps, diff, runnable_prefix, safety, transfer, AgentHandler, AgentResponse,
    PendingPlan, PlanStep, PlanVerdict, CALLER,
};
use crate::ssh::session::{ExecLimit, SessionCmd};
use crate::AppState;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;
use tracing::info;
use uuid::Uuid;

/// Хост в том виде, в каком его видит агент.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentHost {
    /// Имя — единственное, чем агент адресует хост.
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

/// Записать действие агента в журнал.
///
/// Ошибку записи глушим намеренно: журнал не должен ломать работу. Но пишем
/// в общий лог — молча потерянная запись хуже отсутствующей, потому что
/// создаёт ложное впечатление полноты.
async fn log_action(
    db: &sqlx::SqlitePool,
    host: Option<&str>,
    action: &str,
    detail: Option<&str>,
    outcome: &str,
    exit_code: Option<i64>,
    note: Option<&str>,
) {
    let result = sqlx::query(
        "INSERT INTO agent_log (host, action, detail, outcome, exit_code, note) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(host)
    .bind(action)
    .bind(detail)
    .bind(outcome)
    .bind(exit_code)
    .bind(note)
    .execute(db)
    .await;

    if let Err(e) = result {
        tracing::warn!(target: "agent", "не удалось записать действие в журнал: {e}");
    }
}

/// Строка выборки хостов: имя, группа, описание.
type HostRow = (String, Option<String>, Option<String>);

pub struct Tools {
    app: AppHandle,
}

/// Записать файл по SFTP, создавая и обрезая его.
///
/// Не `SftpSession::write`: она открывает файл **только** флагом `WRITE` — без
/// `CREATE` и без `TRUNCATE`. Отсюда две беды, обе найдены на живом сервере
/// 2026-08-29.
///
/// Первая видимая: копии `*.darwaza-bak` ещё не существует, открыть её на
/// запись нельзя, и правка любого файла обрывалась на шаге «сохранить копию»
/// с невнятным «No such file».
///
/// Вторая тихая и опаснее: без `TRUNCATE` новый текст ложится **поверх**
/// старого, не укорачивая файл. Запиши конфиг короче прежнего — и в хвосте
/// останется мусор от старого. Никто этого не заметил только потому, что
/// первая беда срабатывала раньше и до самой записи дело не доходило.
async fn sftp_put(
    sftp: &russh_sftp::client::SftpSession,
    path: &str,
    data: &[u8],
) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;

    let mut file = sftp
        .create(path.to_string())
        .await
        .map_err(|e| format!("{path}: {e}"))?;
    file.write_all(data)
        .await
        .map_err(|e| format!("{path}: {e}"))?;
    // Без этого часть данных может остаться в буфере: закрытие по Drop
    // ошибку проглотит, и файл окажется обрезанным на середине.
    file.shutdown().await.map_err(|e| format!("{path}: {e}"))?;
    Ok(())
}

impl Tools {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }

    fn state(&self) -> tauri::State<'_, AppState> {
        self.app.state::<AppState>()
    }

    /// Найти хост по имени среди РАЗРЕШЁННЫХ агенту.
    ///
    /// Проверка доступа и поиск — один запрос намеренно. Разделённые, они
    /// приглашают найти хост сначала, а проверить потом, и однажды кто-нибудь
    /// поменяет порядок.
    async fn allowed_host(&self, name: &str) -> Result<(i64, String), String> {
        self.allowed_host_full(name)
            .await
            .map(|(id, name, _)| (id, name))
    }

    /// То же, но вместе со списком читающих команд с `sudo` (#214).
    ///
    /// Список приходит тем же запросом, что и проверка доступа: разделёнными,
    /// они приглашают прочитать список у одного хоста, а выполнить на другом.
    async fn allowed_host_full(&self, name: &str) -> Result<(i64, String, Vec<String>), String> {
        let state = self.state();
        let row: Option<(i64, String, Option<String>)> = sqlx::query_as(
            "SELECT id, name, agent_sudo_read FROM hosts WHERE name = ?1 AND agent_access = 1",
        )
        .bind(name)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| format!("{e}"))?;

        row.map(|(id, name, sudo)| (id, name, safety::sudo_allowlist(sudo.as_deref())))
            .ok_or_else(|| {
                // Одинаковый ответ и для «нет такого хоста», и для «есть, но
                // закрыт»: иначе агент перебором составит список всех хостов
                // человека, включая те, к которым его не пускали.
                format!("хост '{name}' недоступен")
            })
    }

    /// Взять действующую сессию хоста или открыть новую.
    async fn session_for(&self, host_id: i64) -> Result<Uuid, String> {
        let state = self.state();
        let existing = state.ssh.snapshot().await.into_iter().find(|s| {
            s.host_id == host_id
                && matches!(s.status, crate::ssh::session::SessionStatus::Connected)
        });

        if let Some(session) = existing {
            return Ok(session.session_id);
        }

        // Своей сессии нет — открываем. Подключение идёт обычным путём,
        // включая проверку отпечатка сервера: агент не должен получать
        // послаблений, которых нет у человека.
        let response =
            crate::commands::ssh_connect(self.state(), self.app.clone(), host_id).await?;
        Ok(response.session_id)
    }

    async fn hosts_list(&self) -> AgentResponse {
        let state = self.state();
        let rows: Result<Vec<HostRow>, _> = sqlx::query_as(
            "SELECT h.name, g.name, h.description FROM hosts h \
             LEFT JOIN groups g ON g.id = h.group_id \
             WHERE h.agent_access = 1 ORDER BY h.name",
        )
        .fetch_all(&state.db)
        .await;

        match rows {
            Ok(rows) => {
                let hosts: Vec<AgentHost> = rows
                    .into_iter()
                    .map(|(name, group, description)| AgentHost {
                        name,
                        group,
                        description,
                    })
                    .collect();
                AgentResponse::ok(serde_json::json!({ "hosts": hosts }))
            }
            Err(e) => AgentResponse::failed(format!("{e}")),
        }
    }

    async fn run(&self, params: serde_json::Value) -> AgentResponse {
        let host = match params.get("host").and_then(|v| v.as_str()) {
            Some(h) => h,
            None => return AgentResponse::failed("не указан хост"),
        };
        let command = match params.get("command").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return AgentResponse::failed("не указана команда"),
        };

        // Хост находим ДО проверки команды: список читающих команд с `sudo`
        // задан для конкретного сервера (#214). Подключение при этом ещё не
        // открывается — отказ не должен стоить человеку открытой сессии на
        // боевом сервере.
        let (host_id, host_name, sudo_allowed) = match self.allowed_host_full(host).await {
            Ok(found) => found,
            Err(e) => return AgentResponse::failed(e),
        };

        if let Some(reason) = safety::is_forbidden_outright(command) {
            log_action(
                &self.state().db,
                Some(host),
                "run",
                Some(command),
                "refused",
                None,
                Some(&reason),
            )
            .await;
            return AgentResponse::failed(reason);
        }
        if let Err(refusal) = safety::is_read_only_for_host(command, &sudo_allowed) {
            let reason = refusal.to_string();
            log_action(
                &self.state().db,
                Some(host),
                "run",
                Some(command),
                "refused",
                None,
                Some(&reason),
            )
            .await;
            return AgentResponse::failed(reason);
        }

        let session_id = match self.session_for(host_id).await {
            Ok(id) => id,
            Err(e) => return AgentResponse::failed(e),
        };

        let (tx, rx) = oneshot::channel();
        if let Err(e) = self
            .state()
            .ssh
            .send_cmd(
                session_id,
                SessionCmd::Exec {
                    command: command.to_string(),
                    limit: ExecLimit::Fail(crate::commands::ssh::EXEC_TIMEOUT_READ),
                    responder: tx,
                },
            )
            .await
        {
            return AgentResponse::failed(e);
        }

        match rx.await {
            Ok(Ok(result)) => {
                log_action(
                    &self.state().db,
                    Some(&host_name),
                    "run",
                    Some(command),
                    "ok",
                    result.exit_code.map(i64::from),
                    None,
                )
                .await;
                info!(
                    target: "agent",
                    "выполнено на {host_name}: {command} → код {:?}", result.exit_code
                );
                AgentResponse::ok(serde_json::to_value(result).unwrap_or_default())
            }
            Ok(Err(e)) => AgentResponse::failed(e),
            Err(_) => AgentResponse::failed("сессия закрылась до ответа"),
        }
    }

    /// Прочитать файл по SFTP.
    ///
    /// Отдельным инструментом, а не через `cat`: у чтения файла нет ни
    /// синтаксиса оболочки, ни способа сцепить с чем-то ещё. То, что можно
    /// сделать без разбора командной строки, лучше делать без неё.
    async fn read_file(&self, params: serde_json::Value) -> AgentResponse {
        let host = match params.get("host").and_then(|v| v.as_str()) {
            Some(h) => h,
            None => return AgentResponse::failed("не указан хост"),
        };
        let path = match params.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return AgentResponse::failed("не указан путь"),
        };

        let (host_id, host_name) = match self.allowed_host(host).await {
            Ok(found) => found,
            Err(e) => return AgentResponse::failed(e),
        };
        let session_id = match self.session_for(host_id).await {
            Ok(id) => id,
            Err(e) => return AgentResponse::failed(e),
        };

        let sftp = match crate::commands::sftp::sftp_for(&self.state(), session_id).await {
            Ok(sftp) => sftp,
            Err(e) => return AgentResponse::failed(e),
        };

        // Размер проверяем ДО чтения: файл на гигабайт иначе уедет в память,
        // а оттуда в контекст агента.
        if let Ok(meta) = sftp.metadata(path.to_string()).await {
            if let Some(size) = meta.size {
                if size > FILE_READ_LIMIT {
                    return AgentResponse::failed(format!(
                        "файл больше {} КБ ({size} байт): прочитайте нужную часть командой",
                        FILE_READ_LIMIT / 1024
                    ));
                }
            }
        }

        match sftp.read(path.to_string()).await {
            Ok(bytes) => {
                log_action(
                    &self.state().db,
                    Some(&host_name),
                    "read",
                    Some(path),
                    "ok",
                    None,
                    None,
                )
                .await;
                info!(target: "agent", "прочитан файл на {host_name}: {path}");
                AgentResponse::ok(serde_json::json!({
                    // Двоичный файл не должен рушить ответ: заменяем то, что
                    // не читается как текст, а не отказываемся отвечать.
                    "content": String::from_utf8_lossy(&bytes),
                    "bytes": bytes.len(),
                }))
            }
            Err(e) => AgentResponse::failed(format!("{e}")),
        }
    }
}

/// Строка из параметров запроса. `Err` — готовый отказ агенту.
fn text_param<'a>(
    params: &'a serde_json::Value,
    key: &str,
    what: &str,
) -> Result<&'a str, AgentResponse> {
    match params.get(key).and_then(|v| v.as_str()) {
        Some(v) if !v.trim().is_empty() => Ok(v),
        _ => Err(AgentResponse::failed(format!("не указан {what}"))),
    }
}

/// Сколько записей каталога отдаём агенту.
///
/// Каталог с сотней тысяч файлов (кэш, `node_modules`) иначе уехал бы в
/// контекст агента целиком. Сверх предела — отметка, а не молчание: агент
/// должен знать, что видит не всё.
const LIST_LIMIT: usize = 1000;

impl Tools {
    /// Разрешённый хост, его сессия и SFTP — всё, что нужно файловым операциям.
    async fn host_sftp(
        &self,
        host: &str,
    ) -> Result<(i64, String, std::sync::Arc<russh_sftp::client::SftpSession>), String> {
        let (host_id, host_name) = self.allowed_host(host).await?;
        let session_id = self.session_for(host_id).await?;
        let sftp = crate::commands::sftp::sftp_for(&self.state(), session_id).await?;
        Ok((host_id, host_name, sftp))
    }

    /// Спросить человека про файловую операцию и записать отказ, если он был.
    async fn confirm(
        &self,
        host_id: i64,
        host_name: &str,
        action: &str,
        detail: &str,
        intent: &str,
        steps: Vec<String>,
    ) -> Result<(), AgentResponse> {
        let plan = PendingPlan::new(
            host_id,
            host_name,
            intent,
            steps.into_iter().map(PlanStep::run).collect(),
        );
        if let Err(e) = self.ask(plan).await {
            log_action(
                &self.state().db,
                Some(host_name),
                action,
                Some(detail),
                "refused",
                None,
                Some(&e),
            )
            .await;
            return Err(AgentResponse::failed(e));
        }
        Ok(())
    }

    /// Проверка пути по списку того, что через агента не меняется никогда.
    ///
    /// Та же проверка, что у команд: путь подставляется в равнозначную команду
    /// оболочки. Иначе `rm /etc/sudoers` отклонялся бы, а удаление того же
    /// файла отдельным инструментом — нет.
    async fn refuse_forbidden(
        &self,
        host: &str,
        action: &str,
        as_command: &str,
    ) -> Option<AgentResponse> {
        let reason = safety::is_forbidden_outright(as_command)?;
        log_action(
            &self.state().db,
            Some(host),
            action,
            Some(as_command),
            "refused",
            None,
            Some(&reason),
        )
        .await;
        Some(AgentResponse::failed(reason))
    }

    /// Содержимое каталога на сервере. Без подтверждения: это чтение.
    async fn list_dir(&self, params: serde_json::Value) -> AgentResponse {
        let host = match text_param(&params, "host", "хост") {
            Ok(v) => v,
            Err(r) => return r,
        };
        let path = match text_param(&params, "path", "путь") {
            Ok(v) => v,
            Err(r) => return r,
        };
        let (_, host_name, sftp) = match self.host_sftp(host).await {
            Ok(v) => v,
            Err(e) => return AgentResponse::failed(e),
        };

        let dir = match sftp.read_dir(path.to_string()).await {
            Ok(dir) => dir,
            Err(e) => return AgentResponse::failed(format!("{path}: {e}")),
        };
        let mut entries: Vec<crate::sftp::SftpEntry> = dir
            .into_iter()
            .filter(|e| e.file_name() != "." && e.file_name() != "..")
            .map(|e| {
                let meta = e.metadata();
                crate::sftp::SftpEntry {
                    path: crate::sftp::join_path(path, &e.file_name()),
                    name: e.file_name(),
                    kind: meta.file_type().into(),
                    size: meta.size.unwrap_or(0),
                    modified: meta.mtime.map(i64::from),
                    permissions: meta.permissions,
                }
            })
            .collect();
        crate::sftp::sort_entries(&mut entries);

        let total = entries.len();
        entries.truncate(LIST_LIMIT);
        log_action(
            &self.state().db,
            Some(&host_name),
            "list",
            Some(path),
            "ok",
            None,
            None,
        )
        .await;
        AgentResponse::ok(serde_json::json!({
            "entries": entries,
            "total": total,
            "truncated": total > LIST_LIMIT,
        }))
    }

    /// Сведения о файле на сервере. Без подтверждения: это чтение.
    async fn stat(&self, params: serde_json::Value) -> AgentResponse {
        let host = match text_param(&params, "host", "хост") {
            Ok(v) => v,
            Err(r) => return r,
        };
        let path = match text_param(&params, "path", "путь") {
            Ok(v) => v,
            Err(r) => return r,
        };
        let (_, host_name, sftp) = match self.host_sftp(host).await {
            Ok(v) => v,
            Err(e) => return AgentResponse::failed(e),
        };

        let answer = match sftp.metadata(path.to_string()).await {
            Ok(meta) => serde_json::json!({
                "exists": true,
                "kind": crate::sftp::SftpEntryKind::from(meta.file_type()),
                "size": meta.size,
                "modified": meta.mtime,
                "permissions": meta.permissions.map(|p| format!("{:o}", p & 0o7777)),
                "uid": meta.uid,
                "gid": meta.gid,
            }),
            // «Нет файла» — ответ, а не ошибка: агент как раз это и выяснял.
            Err(russh_sftp::client::error::Error::Status(status))
                if status.status_code == russh_sftp::protocol::StatusCode::NoSuchFile =>
            {
                serde_json::json!({ "exists": false })
            }
            Err(e) => return AgentResponse::failed(format!("{path}: {e}")),
        };
        log_action(
            &self.state().db,
            Some(&host_name),
            "stat",
            Some(path),
            "ok",
            None,
            None,
        )
        .await;
        AgentResponse::ok(answer)
    }

    /// Скачать файл с сервера на компьютер. Только после подтверждения.
    async fn download(&self, params: serde_json::Value) -> AgentResponse {
        let host = match text_param(&params, "host", "хост") {
            Ok(v) => v,
            Err(r) => return r,
        };
        let remote = match text_param(&params, "remotePath", "путь на сервере (remotePath)")
        {
            Ok(v) => v,
            Err(r) => return r,
        };
        let raw_local = match text_param(&params, "localPath", "путь на компьютере (localPath)")
        {
            Ok(v) => v,
            Err(r) => return r,
        };
        let intent = params
            .get("intent")
            .and_then(|v| v.as_str())
            .unwrap_or("скачать файл");

        let home = directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf());
        let local = match transfer::local_path(raw_local, home.as_deref()) {
            Ok(p) => p,
            Err(e) => return AgentResponse::failed(e),
        };
        let (host_id, host_name, sftp) = match self.host_sftp(host).await {
            Ok(v) => v,
            Err(e) => return AgentResponse::failed(e),
        };

        let size = match transfer::remote_kind(&sftp, remote).await {
            Ok(Some((russh_sftp::protocol::FileType::File, size))) => size,
            Ok(Some(_)) => {
                return AgentResponse::failed(format!("на сервере это не файл: {remote}"))
            }
            Ok(None) => return AgentResponse::failed(format!("на сервере нет файла: {remote}")),
            Err(e) => return AgentResponse::failed(e),
        };

        let mut steps = vec![format!(
            "скачать {host_name}:{remote} ({}) → {}",
            transfer::human_size(size),
            local.display()
        )];
        // Замену существующего файла называем отдельной строкой: это то, что
        // человек должен увидеть, а не вычислить.
        if local.exists() {
            steps.push(format!("заменить существующий файл {}", local.display()));
        }
        let detail = format!("{remote} → {}", local.display());
        if let Err(r) = self
            .confirm(host_id, &host_name, "download", &detail, intent, steps)
            .await
        {
            return r;
        }

        match transfer::download(&sftp, remote, &local).await {
            Ok(bytes) => {
                log_action(
                    &self.state().db,
                    Some(&host_name),
                    "download",
                    Some(&detail),
                    "ok",
                    None,
                    None,
                )
                .await;
                info!(target: "agent", "скачан файл с {host_name}: {detail}");
                AgentResponse::ok(serde_json::json!({ "bytes": bytes, "localPath": local }))
            }
            Err(e) => {
                log_action(
                    &self.state().db,
                    Some(&host_name),
                    "download",
                    Some(&detail),
                    "failed",
                    None,
                    Some(&e),
                )
                .await;
                AgentResponse::failed(e)
            }
        }
    }

    /// Загрузить файл с компьютера на сервер. Только после подтверждения.
    async fn upload(&self, params: serde_json::Value) -> AgentResponse {
        let host = match text_param(&params, "host", "хост") {
            Ok(v) => v,
            Err(r) => return r,
        };
        let raw_local = match text_param(&params, "localPath", "путь на компьютере (localPath)")
        {
            Ok(v) => v,
            Err(r) => return r,
        };
        let remote = match text_param(&params, "remotePath", "путь на сервере (remotePath)")
        {
            Ok(v) => v,
            Err(r) => return r,
        };
        let intent = params
            .get("intent")
            .and_then(|v| v.as_str())
            .unwrap_or("загрузить файл");

        let home = directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf());
        let local = match transfer::local_path(raw_local, home.as_deref()) {
            Ok(p) => p,
            Err(e) => return AgentResponse::failed(e),
        };
        let size = match std::fs::metadata(&local) {
            Ok(meta) if meta.is_file() => meta.len(),
            Ok(_) => return AgentResponse::failed(format!("это не файл: {}", local.display())),
            Err(e) => return AgentResponse::failed(format!("{}: {e}", local.display())),
        };
        if let Some(r) = self
            .refuse_forbidden(host, "upload", &format!("tee {remote}"))
            .await
        {
            return r;
        }
        let (host_id, host_name, sftp) = match self.host_sftp(host).await {
            Ok(v) => v,
            Err(e) => return AgentResponse::failed(e),
        };

        let mut steps = vec![format!(
            "загрузить {} ({}) → {host_name}:{remote}",
            local.display(),
            transfer::human_size(size)
        )];
        match transfer::remote_kind(&sftp, remote).await {
            Ok(Some((russh_sftp::protocol::FileType::Dir, _))) => {
                return AgentResponse::failed(format!(
                    "на сервере по этому пути каталог: {remote}"
                ));
            }
            Ok(Some(_)) => steps.push(format!(
                "прежний файл сохранить как {remote}{}",
                transfer::BACKUP_SUFFIX
            )),
            Ok(None) => {}
            Err(e) => return AgentResponse::failed(e),
        }
        let detail = format!("{} → {remote}", local.display());
        if let Err(r) = self
            .confirm(host_id, &host_name, "upload", &detail, intent, steps)
            .await
        {
            return r;
        }

        match transfer::upload(&sftp, &local, remote).await {
            Ok(outcome) => {
                log_action(
                    &self.state().db,
                    Some(&host_name),
                    "upload",
                    Some(&detail),
                    "ok",
                    None,
                    None,
                )
                .await;
                info!(target: "agent", "загружен файл на {host_name}: {detail}");
                AgentResponse::ok(
                    serde_json::json!({ "bytes": outcome.bytes, "backup": outcome.backup }),
                )
            }
            Err(e) => {
                log_action(
                    &self.state().db,
                    Some(&host_name),
                    "upload",
                    Some(&detail),
                    "failed",
                    None,
                    Some(&e),
                )
                .await;
                AgentResponse::failed(e)
            }
        }
    }

    /// Создать каталог на сервере. Только после подтверждения.
    async fn mkdir(&self, params: serde_json::Value) -> AgentResponse {
        let host = match text_param(&params, "host", "хост") {
            Ok(v) => v,
            Err(r) => return r,
        };
        let path = match text_param(&params, "path", "путь") {
            Ok(v) => v,
            Err(r) => return r,
        };
        let intent = params
            .get("intent")
            .and_then(|v| v.as_str())
            .unwrap_or("создать каталог");

        if let Some(r) = self
            .refuse_forbidden(host, "mkdir", &format!("mkdir {path}"))
            .await
        {
            return r;
        }
        let (host_id, host_name, sftp) = match self.host_sftp(host).await {
            Ok(v) => v,
            Err(e) => return AgentResponse::failed(e),
        };
        if let Err(r) = self
            .confirm(
                host_id,
                &host_name,
                "mkdir",
                path,
                intent,
                vec![format!("создать каталог {host_name}:{path}")],
            )
            .await
        {
            return r;
        }

        match sftp.create_dir(path.to_string()).await {
            Ok(()) => {
                log_action(
                    &self.state().db,
                    Some(&host_name),
                    "mkdir",
                    Some(path),
                    "ok",
                    None,
                    None,
                )
                .await;
                AgentResponse::ok(serde_json::json!({ "created": path }))
            }
            Err(e) => {
                let e = format!("{path}: {e}");
                log_action(
                    &self.state().db,
                    Some(&host_name),
                    "mkdir",
                    Some(path),
                    "failed",
                    None,
                    Some(&e),
                )
                .await;
                AgentResponse::failed(e)
            }
        }
    }

    /// Переименовать или переместить на сервере. Только после подтверждения.
    async fn rename(&self, params: serde_json::Value) -> AgentResponse {
        let host = match text_param(&params, "host", "хост") {
            Ok(v) => v,
            Err(r) => return r,
        };
        let from = match text_param(&params, "from", "исходный путь (from)") {
            Ok(v) => v,
            Err(r) => return r,
        };
        let to = match text_param(&params, "to", "новый путь (to)") {
            Ok(v) => v,
            Err(r) => return r,
        };
        let intent = params
            .get("intent")
            .and_then(|v| v.as_str())
            .unwrap_or("переименовать");

        if let Some(r) = self
            .refuse_forbidden(host, "rename", &format!("mv {from} {to}"))
            .await
        {
            return r;
        }
        let (host_id, host_name, sftp) = match self.host_sftp(host).await {
            Ok(v) => v,
            Err(e) => return AgentResponse::failed(e),
        };
        let detail = format!("{from} → {to}");
        if let Err(r) = self
            .confirm(
                host_id,
                &host_name,
                "rename",
                &detail,
                intent,
                vec![format!("переименовать {host_name}:{from} → {to}")],
            )
            .await
        {
            return r;
        }

        // Поверх существующего SFTP не переименовывает — и хорошо: молча
        // затереть чужой файл при переименовании агент не должен.
        match sftp.rename(from.to_string(), to.to_string()).await {
            Ok(()) => {
                log_action(
                    &self.state().db,
                    Some(&host_name),
                    "rename",
                    Some(&detail),
                    "ok",
                    None,
                    None,
                )
                .await;
                AgentResponse::ok(serde_json::json!({ "from": from, "to": to }))
            }
            Err(e) => {
                let e =
                    format!("{detail}: {e} (если по новому пути уже есть файл — его не заменяют)");
                log_action(
                    &self.state().db,
                    Some(&host_name),
                    "rename",
                    Some(&detail),
                    "failed",
                    None,
                    Some(&e),
                )
                .await;
                AgentResponse::failed(e)
            }
        }
    }

    /// Удалить файл или пустой каталог на сервере. Только после подтверждения.
    ///
    /// Рекурсивного удаления нет — так же, как в файловом браузере: снести
    /// дерево одним подтверждением слишком легко. Для этого есть план с `rm`,
    /// где человек видит команду целиком.
    async fn delete(&self, params: serde_json::Value) -> AgentResponse {
        let host = match text_param(&params, "host", "хост") {
            Ok(v) => v,
            Err(r) => return r,
        };
        let path = match text_param(&params, "path", "путь") {
            Ok(v) => v,
            Err(r) => return r,
        };
        let intent = params
            .get("intent")
            .and_then(|v| v.as_str())
            .unwrap_or("удалить");

        if let Some(r) = self
            .refuse_forbidden(host, "delete", &format!("rm {path}"))
            .await
        {
            return r;
        }
        let (host_id, host_name, sftp) = match self.host_sftp(host).await {
            Ok(v) => v,
            Err(e) => return AgentResponse::failed(e),
        };

        let (is_dir, what) = match transfer::remote_kind(&sftp, path).await {
            Ok(Some((russh_sftp::protocol::FileType::Dir, _))) => {
                (true, "пустой каталог".to_string())
            }
            Ok(Some((_, size))) => (false, format!("файл ({})", transfer::human_size(size))),
            Ok(None) => {
                return AgentResponse::failed(format!("на сервере нет такого пути: {path}"))
            }
            Err(e) => return AgentResponse::failed(e),
        };
        if let Err(r) = self
            .confirm(
                host_id,
                &host_name,
                "delete",
                path,
                intent,
                vec![format!("удалить {what} {host_name}:{path}")],
            )
            .await
        {
            return r;
        }

        let result = if is_dir {
            sftp.remove_dir(path.to_string()).await
        } else {
            sftp.remove_file(path.to_string()).await
        };
        match result {
            Ok(()) => {
                log_action(
                    &self.state().db,
                    Some(&host_name),
                    "delete",
                    Some(path),
                    "ok",
                    None,
                    None,
                )
                .await;
                AgentResponse::ok(serde_json::json!({ "deleted": path }))
            }
            Err(e) => {
                let e = if is_dir {
                    format!("{path}: {e} (каталог должен быть пустым)")
                } else {
                    format!("{path}: {e}")
                };
                log_action(
                    &self.state().db,
                    Some(&host_name),
                    "delete",
                    Some(path),
                    "failed",
                    None,
                    Some(&e),
                )
                .await;
                AgentResponse::failed(e)
            }
        }
    }
}

/// Оказалась ли служба в том состоянии, ради которого делали действие (#215).
///
/// Отдельной функцией ради проверки: код возврата `systemctl` тут не помощник.
/// `restart` возвращает ноль, отправив службе сигнал, — а упала она через
/// секунду или нет, из кода не видно. Именно поэтому состояние спрашивается
/// отдельно.
fn service_is_healthy(action: &str, state: &str) -> bool {
    if action == "stop" {
        // После остановки `failed` — тоже остановлена: служба, упавшая при
        // выходе, всё равно больше не работает, и это не повод звать человека.
        matches!(state, "inactive" | "failed")
    } else {
        state == "active"
    }
}

/// Действия со службой, которые выполняет `darwaza_service` (#215).
///
/// `enable`, `disable` и `mask` сюда не входят намеренно: они меняют
/// поведение при загрузке, а не состояние сейчас, и проверить их результат
/// тем же способом нельзя. Для них есть план, где человек видит команду.
const SERVICE_ACTIONS: [&str; 5] = [
    "start",
    "stop",
    "restart",
    "reload",
    "try-reload-or-restart",
];

impl Tools {
    /// Действие со службой с проверкой до и после (#215).
    ///
    /// Собрано из того, что агенты и так делают планом, — но так, чтобы
    /// порядок нельзя было забыть. Сессия devops на проде каждый раз
    /// выстраивала его заново: проверить конфигурацию, перезапустить,
    /// убедиться, что поднялось. Однажды кто-нибудь пропустит проверку и
    /// перезапустит службу со сломанным конфигом.
    ///
    /// Отката здесь нет, и обещать его нельзя: перезапуск сам по себе ничего
    /// не менял, возвращать нечего. Откат есть там, где была правка файла, —
    /// в `files.write`. Здесь вместо отката последние строки журнала службы:
    /// человеку и агенту нужен ответ «почему не поднялась», а не «не
    /// поднялась».
    async fn service(&self, params: serde_json::Value) -> AgentResponse {
        let host = match text_param(&params, "host", "хост") {
            Ok(v) => v,
            Err(r) => return r,
        };
        let unit = match text_param(&params, "unit", "имя службы (unit)") {
            Ok(v) => v,
            Err(r) => return r,
        };
        let action = match text_param(&params, "action", "действие (action)") {
            Ok(v) => v,
            Err(r) => return r,
        };
        if !SERVICE_ACTIONS.contains(&action) {
            return AgentResponse::failed(format!(
                "действие '{action}' не из списка: {}. enable, disable и mask \
                 выполняются планом — там видно команду целиком",
                SERVICE_ACTIONS.join(", ")
            ));
        }
        let intent = params
            .get("intent")
            .and_then(|v| v.as_str())
            .unwrap_or("действие со службой");
        // Права: `systemctl restart` без root не выполнится ни на одном
        // сервере, где служба общая. Отдельным флагом, а не догадкой: человек
        // увидит в плане ровно ту команду, которая пойдёт.
        let sudo = params
            .get("sudo")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let prefix = if sudo { "sudo " } else { "" };
        let command = format!("{prefix}systemctl {action} {unit}");

        if let Some(r) = self.refuse_forbidden(host, "service", &command).await {
            return r;
        }
        let (host_id, host_name) = match self.allowed_host(host).await {
            Ok(found) => found,
            Err(e) => return AgentResponse::failed(e),
        };
        let session_id = match self.session_for(host_id).await {
            Ok(id) => id,
            Err(e) => return AgentResponse::failed(e),
        };

        // Проверка конфигурации — ДО подтверждения. Спрашивать человека про
        // перезапуск со сломанным конфигом значит спрашивать зря: ответ всё
        // равно будет «не делай», а узнает он об этом уже после «да».
        let check = crate::agent::diff::service_check(unit);
        if let Some(check) = check {
            let check_command = format!("{prefix}{check}");
            match self
                .exec(
                    session_id,
                    &check_command,
                    ExecLimit::Fail(crate::commands::ssh::EXEC_TIMEOUT_READ),
                )
                .await
            {
                Ok(result) if result.exit_code.unwrap_or(0) != 0 => {
                    log_action(
                        &self.state().db,
                        Some(&host_name),
                        "service",
                        Some(&command),
                        "refused",
                        result.exit_code.map(i64::from),
                        Some(&format!("{check_command}: {}", result.stderr.trim())),
                    )
                    .await;
                    return AgentResponse::failed(format!(
                        "проверка '{check_command}' не прошла, служба не тронута: {}",
                        result.stderr.trim()
                    ));
                }
                Ok(_) => {}
                Err(e) => return AgentResponse::failed(e),
            }
        }

        let mut steps = Vec::new();
        match check {
            Some(check) => steps.push(format!(
                "проверить конфигурацию: {prefix}{check} — пройдена"
            )),
            // Молчать об отсутствии проверки нельзя: человек решит, что она
            // была.
            None => steps.push(format!("проверки конфигурации для '{unit}' нет")),
        }
        steps.push(command.clone());
        steps.push(format!("убедиться, что служба {unit} в нужном состоянии"));

        if let Err(r) = self
            .confirm(host_id, &host_name, "service", &command, intent, steps)
            .await
        {
            return r;
        }

        let result = match self
            .exec(
                session_id,
                &command,
                ExecLimit::Fail(crate::commands::ssh::EXEC_TIMEOUT_LONG),
            )
            .await
        {
            Ok(r) => r,
            Err(e) => {
                log_action(
                    &self.state().db,
                    Some(&host_name),
                    "service",
                    Some(&command),
                    "failed",
                    None,
                    Some(&e),
                )
                .await;
                return AgentResponse::failed(e);
            }
        };

        // Проверка после — то, о чём забывают. `systemctl restart` возвращает
        // ноль, отправив службе сигнал; упала она через секунду или нет, из
        // кода возврата не видно.

        let state_result = self
            .exec(
                session_id,
                &format!("systemctl is-active {unit}"),
                ExecLimit::Fail(crate::commands::ssh::EXEC_TIMEOUT_READ),
            )
            .await;
        let state = state_result
            .as_ref()
            .map(|r| r.stdout.trim().to_string())
            .unwrap_or_else(|_| "неизвестно".to_string());
        let healthy = service_is_healthy(action, &state);

        if result.exit_code.unwrap_or(0) == 0 && healthy {
            log_action(
                &self.state().db,
                Some(&host_name),
                "service",
                Some(&command),
                "ok",
                result.exit_code.map(i64::from),
                None,
            )
            .await;
            info!(target: "agent", "{host_name}: {command} → {state}");
            return AgentResponse::ok(serde_json::json!({
                "state": state,
                "checked": check,
            }));
        }

        // Не поднялась. Последние строки журнала — это и есть ответ на
        // «почему», и добыть их должны мы: у агента может не быть права
        // читать журнал этой службы без sudo.
        let logs = self
            .exec(
                session_id,
                &format!("{prefix}journalctl -u {unit} -n 20 --no-pager"),
                ExecLimit::Fail(crate::commands::ssh::EXEC_TIMEOUT_READ),
            )
            .await
            .map(|r| r.stdout)
            .unwrap_or_default();

        log_action(
            &self.state().db,
            Some(&host_name),
            "service",
            Some(&command),
            "failed",
            result.exit_code.map(i64::from),
            Some(&format!("состояние: {state}")),
        )
        .await;
        AgentResponse::ok(serde_json::json!({
            "ok": false,
            "state": state,
            "exitCode": result.exit_code,
            "stderr": result.stderr,
            "logs": logs,
        }))
    }
}

/// Предел на длину заметки о хосте (#217).
///
/// Заметка — память о сервере, а не журнал работ: что за приложение, где
/// лежит, как выкатывается. Несколько абзацев. Без предела агент однажды
/// положит туда вывод команды целиком.
const NOTES_LIMIT: usize = 4 * 1024;

impl Tools {
    /// Заметки помощника о хосте (#217).
    ///
    /// Без `notes` — чтение, с `notes` — запись.
    ///
    /// **Запись без подтверждения, и это осознанно.** Заметка не трогает
    /// сервер, видна человеку в карточке хоста и стирается одной кнопкой.
    /// Спрашивать за неё — приучать нажимать «да», а тогда подтверждение
    /// перестанет что-либо значить там, где оно действительно нужно.
    ///
    /// Зачем: каждая сессия агента начинает с нуля и заново выясняет, что за
    /// приложение на сервере и как оно выкатывается. Это лишние команды,
    /// лишнее время и лишний риск ошибиться на разведке.
    async fn host_notes(&self, params: serde_json::Value) -> AgentResponse {
        let host = match text_param(&params, "host", "хост") {
            Ok(v) => v,
            Err(r) => return r,
        };
        let (host_id, host_name) = match self.allowed_host(host).await {
            Ok(found) => found,
            Err(e) => return AgentResponse::failed(e),
        };
        let state = self.state();

        let Some(notes) = params.get("notes").and_then(|v| v.as_str()) else {
            let current: Option<(Option<String>,)> =
                match sqlx::query_as("SELECT agent_notes FROM hosts WHERE id = ?1")
                    .bind(host_id)
                    .fetch_optional(&state.db)
                    .await
                {
                    Ok(row) => row,
                    Err(e) => return AgentResponse::failed(format!("{e}")),
                };
            return AgentResponse::ok(serde_json::json!({
                "notes": current.and_then(|(n,)| n),
            }));
        };

        let notes = notes.trim();
        if notes.chars().count() > NOTES_LIMIT {
            return AgentResponse::failed(format!(
                "заметка длиннее {NOTES_LIMIT} знаков: это память о сервере, а не журнал работ"
            ));
        }
        // Пустая заметка — стереть: иначе очистить написанное агент не сможет.
        let value = if notes.is_empty() { None } else { Some(notes) };

        if let Err(e) = sqlx::query(
            "UPDATE hosts SET agent_notes = ?2, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
        )
        .bind(host_id)
        .bind(value)
        .execute(&state.db)
        .await
        {
            return AgentResponse::failed(format!("{e}"));
        }

        log_action(
            &state.db,
            Some(&host_name),
            "notes",
            value,
            "ok",
            None,
            None,
        )
        .await;
        info!(target: "agent", "{host_name}: заметка обновлена");
        AgentResponse::ok(serde_json::json!({ "saved": true, "notes": value }))
    }
}

impl Tools {
    /// Копирование файла с одного сервера на другой (#222).
    ///
    /// Зачем. При переезде файл гоняли через диск человека: скачать, потом
    /// загрузить. Для дампа базы на несколько гигабайт это лишний проход по
    /// диску, лишнее время и требование места, которого может не быть.
    ///
    /// Почему не `scp` между серверами. Он требует, чтобы серверы видели друг
    /// друга по сети и чтобы у одного был ключ от другого. При переезде верно
    /// обычно ни то, ни другое — а приложение подключено к обоим уже сейчас.
    ///
    /// **Оба хоста проверяются на доступ отдельно.** Разрешённый исходный
    /// сервер не даёт права писать на закрытый — и наоборот.
    ///
    /// Подтверждается как всякая запись: копия прежнего файла, возврат копии
    /// при обрыве и сам показ плана — общие с загрузкой, буквально тот же код.
    async fn copy_file(&self, params: serde_json::Value) -> AgentResponse {
        let from_host = match text_param(&params, "fromHost", "исходный хост (fromHost)")
        {
            Ok(v) => v,
            Err(r) => return r,
        };
        let from_path = match text_param(&params, "fromPath", "путь на исходном (fromPath)")
        {
            Ok(v) => v,
            Err(r) => return r,
        };
        let to_host = match text_param(&params, "toHost", "хост назначения (toHost)")
        {
            Ok(v) => v,
            Err(r) => return r,
        };
        let to_path = match text_param(&params, "toPath", "путь назначения (toPath)")
        {
            Ok(v) => v,
            Err(r) => return r,
        };
        let intent = params
            .get("intent")
            .and_then(|v| v.as_str())
            .unwrap_or("скопировать файл между серверами");

        if let Some(r) = self
            .refuse_forbidden(to_host, "copy", &format!("tee {to_path}"))
            .await
        {
            return r;
        }

        let (_, from_name, from_sftp) = match self.host_sftp(from_host).await {
            Ok(v) => v,
            Err(e) => return AgentResponse::failed(e),
        };
        let (to_id, to_name, to_sftp) = match self.host_sftp(to_host).await {
            Ok(v) => v,
            Err(e) => return AgentResponse::failed(e),
        };

        // Копировать файл сам в себя — не работа, а способ его потерять:
        // прежний уедет в копию, а на его место ляжет он же наполовину.
        if from_name == to_name && from_path == to_path {
            return AgentResponse::failed(
                "источник и назначение — один и тот же файл на одном и том же хосте".to_string(),
            );
        }

        let size = match transfer::remote_kind(&from_sftp, from_path).await {
            Ok(Some((russh_sftp::protocol::FileType::Dir, _))) => {
                return AgentResponse::failed(format!(
                    "на исходном сервере по этому пути каталог: {from_path}"
                ));
            }
            Ok(Some((_, size))) => size,
            Ok(None) => {
                return AgentResponse::failed(format!("на исходном сервере нет файла: {from_path}"))
            }
            Err(e) => return AgentResponse::failed(e),
        };

        let mut steps = vec![format!(
            "скопировать {from_name}:{from_path} ({}) → {to_name}:{to_path}",
            transfer::human_size(size)
        )];
        match transfer::remote_kind(&to_sftp, to_path).await {
            Ok(Some((russh_sftp::protocol::FileType::Dir, _))) => {
                return AgentResponse::failed(format!(
                    "на сервере назначения по этому пути каталог: {to_path}"
                ));
            }
            Ok(Some(_)) => steps.push(format!(
                "прежний файл сохранить как {to_path}{}",
                transfer::BACKUP_SUFFIX
            )),
            Ok(None) => {}
            Err(e) => return AgentResponse::failed(e),
        }

        let detail = format!("{from_name}:{from_path} → {to_name}:{to_path}");
        // Спрашиваем у хоста НАЗНАЧЕНИЯ: меняется он, и подтверждение должно
        // называть тот сервер, на котором что-то станет иначе.
        if let Err(r) = self
            .confirm(to_id, &to_name, "copy", &detail, intent, steps)
            .await
        {
            return r;
        }

        match transfer::copy_between(&from_sftp, from_path, &to_sftp, to_path).await {
            Ok(outcome) => {
                log_action(
                    &self.state().db,
                    Some(&to_name),
                    "copy",
                    Some(&detail),
                    "ok",
                    None,
                    None,
                )
                .await;
                info!(target: "agent", "скопировано: {detail}");
                AgentResponse::ok(
                    serde_json::json!({ "bytes": outcome.bytes, "backup": outcome.backup }),
                )
            }
            Err(e) => {
                log_action(
                    &self.state().db,
                    Some(&to_name),
                    "copy",
                    Some(&detail),
                    "failed",
                    None,
                    Some(&e),
                )
                .await;
                AgentResponse::failed(e)
            }
        }
    }
}

/// Сколько держать туннель открытым по умолчанию и сколько максимум (#219).
///
/// Самозакрытие здесь не удобство, а условие существования инструмента.
/// Забытый туннель — открытый порт на машине человека, ведущий внутрь его
/// боевого сервера; он переживёт и разговор, и сам разговор с агентом, и
/// закрывать его будет некому.
const TUNNEL_SECONDS: u64 = 300;
const TUNNEL_SECONDS_MAX: u64 = 3600;

/// Откуда берутся номера для временных пробросов агента.
///
/// Отрицательные: ключ реестра общий с правилами из базы, а их `id` выдаёт
/// AUTOINCREMENT и он всегда положительный. Столкнуться эти два множества не
/// могут по устройству, а не по договорённости.
static TUNNEL_COUNTER: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-1);

/// Следующий номер для временного проброса.
fn next_tunnel_id() -> i64 {
    TUNNEL_COUNTER.fetch_sub(1, std::sync::atomic::Ordering::Relaxed)
}

/// Описание туннеля агента.
///
/// Отдельной функцией ради проверки: адрес прослушки и вид проброса здесь
/// заданы жёстко, и тест следит, чтобы они такими и остались. Сделать адрес
/// параметром — однострочная правка, а последствие у неё — открытый всей
/// местной сети порт внутрь чужого боевого сервера.
fn tunnel_spec(
    forward_id: i64,
    remote_host: &str,
    remote_port: u16,
) -> crate::forwarding::ForwardSpec {
    crate::forwarding::ForwardSpec {
        forward_id,
        // Только local: dynamic — это SOCKS, выход куда угодно через сервер.
        kind: "local".to_string(),
        local_host: "127.0.0.1".to_string(),
        // 0 — порт выдаёт система: занятый порт не должен стать отказом, а
        // какой именно достался, всё равно сообщается в ответе.
        local_port: 0,
        remote_host: Some(remote_host.to_string()),
        remote_port: Some(remote_port),
    }
}

impl Tools {
    /// Временный проброс порта для агента (#219).
    ///
    /// Зачем. Многое на сервере не слушает наружу и не должно: PostgreSQL,
    /// Redis, внутренний HTTP админки. Посмотреть на них агент мог только
    /// через команды в оболочке — `psql -c`, `redis-cli`, `curl` — то есть
    /// через строку, которую ещё надо собрать без ошибок.
    ///
    /// Что здесь важно.
    ///
    /// **Слушаем только `127.0.0.1`.** Адрес прослушки не параметр и не
    /// станет им: `0.0.0.0` открыл бы внутренности чужого сервера всей сети
    /// вокруг ноутбука, включая кафе и коворкинг.
    ///
    /// **Только `local`.** `dynamic` — это SOCKS, то есть выход куда угодно
    /// через сервер, а `remote` открывает порт на самом сервере. Ни то, ни
    /// другое не является «доступом к внутренней службе».
    ///
    /// **Закрывается сам.** Срок — часть подтверждения: человек видит не
    /// только куда, но и насколько.
    async fn tunnel(&self, params: serde_json::Value) -> AgentResponse {
        let host = match text_param(&params, "host", "хост") {
            Ok(v) => v,
            Err(r) => return r,
        };
        let remote_port = match params.get("remotePort").and_then(|v| v.as_u64()) {
            Some(p) if (1..=65535).contains(&p) => p as u16,
            _ => return AgentResponse::failed("не указан порт на сервере (remotePort, 1–65535)"),
        };
        // Умолчание — сама машина: внутренние службы слушают на ней. Другой
        // адрес допустим (сосед в локальной сети сервера), но называется явно.
        let remote_host = params
            .get("remoteHost")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("127.0.0.1")
            .to_string();
        let seconds = params
            .get("seconds")
            .and_then(|v| v.as_u64())
            .unwrap_or(TUNNEL_SECONDS)
            .clamp(1, TUNNEL_SECONDS_MAX);
        let intent = params
            .get("intent")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("доступ к внутренней службе");

        let (host_id, host_name) = match self.allowed_host(host).await {
            Ok(found) => found,
            Err(e) => return AgentResponse::failed(e),
        };

        let plan = PendingPlan::new(
            host_id,
            host_name.clone(),
            format!("{intent}: туннель до {remote_host}:{remote_port} на {seconds} с"),
            vec![PlanStep::run(format!(
                "открыть на 127.0.0.1 проброс до {remote_host}:{remote_port} \
                 через {host_name}, закрыть через {seconds} с"
            ))],
        );

        if let Err(e) = self.ask(plan).await {
            log_action(
                &self.state().db,
                Some(&host_name),
                "tunnel",
                Some(&format!("{remote_host}:{remote_port}")),
                "refused",
                None,
                Some(&e),
            )
            .await;
            return AgentResponse::failed(e);
        }

        let session_id = match self.session_for(host_id).await {
            Ok(id) => id,
            Err(e) => return AgentResponse::failed(e),
        };

        let forward_id = next_tunnel_id();
        let state = self.state();
        let bound = match state
            .forwards
            .start(
                state.ssh.clone(),
                session_id,
                tunnel_spec(forward_id, &remote_host, remote_port),
            )
            .await
        {
            Ok(bound) => bound,
            Err(e) => return AgentResponse::failed(e),
        };

        // Закрытие по сроку. Задача отдельная и живёт сама: ответ агенту
        // уходит сразу, иначе он ждал бы весь срок вместо того, чтобы
        // пользоваться туннелем.
        let forwards = state.forwards.clone();
        let closing_host = host_name.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(seconds)).await;
            if forwards.stop(forward_id).await {
                info!(target: "agent", "{closing_host}: туннель закрыт по сроку ({seconds} с)");
            }
        });

        log_action(
            &state.db,
            Some(&host_name),
            "tunnel",
            Some(&format!("{bound} → {remote_host}:{remote_port}")),
            "ok",
            None,
            Some(&format!("{seconds} с")),
        )
        .await;
        info!(target: "agent", "{host_name}: туннель {bound} → {remote_host}:{remote_port}");

        AgentResponse::ok(serde_json::json!({
            "address": bound,
            "target": format!("{remote_host}:{remote_port}"),
            "seconds": seconds,
            "note": "туннель закроется сам по истечении срока",
        }))
    }
}

/// Сниппет в том виде, в каком его видит агент (#220).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentSnippet {
    name: String,
    /// Команда показывается целиком, и это не упущение: агент должен знать,
    /// что предлагает выполнить. Скрывать её было бы хуже — он предлагал бы
    /// человеку подтвердить то, чего сам не видел.
    command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

impl Tools {
    /// Сниппеты, открытые агенту (#220).
    ///
    /// Открытые — не все. В сниппеты пишут команды целиком, как их набирают
    /// руками, вместе с паролями и токенами прямо в строке. Поэтому у каждого
    /// свой флаг, выключенный по умолчанию, и выборка идёт только по
    /// включённым.
    async fn snippets_list(&self) -> AgentResponse {
        let rows: Result<Vec<(String, String, Option<String>)>, _> = sqlx::query_as(
            "SELECT name, command, description FROM snippets \
             WHERE agent_access = 1 ORDER BY sort_order, name",
        )
        .fetch_all(&self.state().db)
        .await;

        match rows {
            Ok(rows) => {
                let snippets: Vec<AgentSnippet> = rows
                    .into_iter()
                    .map(|(name, command, description)| AgentSnippet {
                        name,
                        command,
                        description,
                    })
                    .collect();
                AgentResponse::ok(serde_json::json!({ "snippets": snippets }))
            }
            Err(e) => AgentResponse::failed(format!("{e}")),
        }
    }

    /// Выполнить сниппет на хосте (#220).
    ///
    /// Сниппет — это команда владельца, написанная им самим и проверенная на
    /// его же серверах. Отсюда весь смысл: агент не придумывает, как что-то
    /// делается здесь, а берёт уже принятый способ.
    ///
    /// Выполняется **планом с подтверждением**, тем же самым, что и обычные
    /// изменения. Ровно через `plan`, а не похожим кодом рядом: подтверждение,
    /// проверка запрещённого, журнал и остановка на первой ошибке должны
    /// работать одинаково, а два похожих пути расходятся через полгода.
    ///
    /// То, что команду написал владелец, подтверждения не отменяет: написал он
    /// её для себя и своей минуты, а не для того, чтобы её запускали без
    /// спроса.
    async fn run_snippet(&self, params: serde_json::Value) -> AgentResponse {
        let host = match text_param(&params, "host", "хост") {
            Ok(v) => v,
            Err(r) => return r,
        };
        let name = match text_param(&params, "snippet", "сниппет") {
            Ok(v) => v,
            Err(r) => return r,
        };

        let found: Option<(String, Option<String>)> = match sqlx::query_as(
            "SELECT command, description FROM snippets WHERE name = ?1 AND agent_access = 1",
        )
        .bind(name)
        .fetch_optional(&self.state().db)
        .await
        {
            Ok(row) => row,
            Err(e) => return AgentResponse::failed(format!("{e}")),
        };

        let Some((command, description)) = found else {
            // Один ответ и для «нет такого», и для «есть, но закрыт»: иначе
            // перебором имён составляется список всех сниппетов владельца.
            return AgentResponse::failed(format!("сниппет '{name}' недоступен"));
        };

        let intent = match description {
            Some(d) if !d.trim().is_empty() => format!("сниппет «{name}»: {d}"),
            _ => format!("сниппет «{name}»"),
        };

        self.plan(serde_json::json!({
            "host": host,
            "intent": intent,
            "steps": [command],
        }))
        .await
    }
}

/// Сколько секунд наблюдать за выводом по умолчанию и сколько максимум
/// (#218).
///
/// Предел жёсткий, и он тут главное. Команда с `-f` не заканчивается никогда;
/// всё, что делает этот инструмент, — ограничивает её временем. Убери предел,
/// и канал занят навсегда, а агент ждёт ответа, которого не будет.
///
/// Две минуты сверху — потому что дольше держать сессию занятой ради
/// наблюдения незачем: если за две минуты в журнале ничего не появилось,
/// смотреть надо не туда.
const FOLLOW_SECONDS: u64 = 15;
const FOLLOW_SECONDS_MAX: u64 = 120;

impl Tools {
    /// Смотреть на вывод команды отведённое время (#218).
    ///
    /// Зачем. Агент видел только готовый результат: команда либо завершилась,
    /// либо не уложилась в предел и считалась провалившейся. Про выкатку это
    /// означало «не знаю, идёт она или встала» — самое неприятное состояние,
    /// потому что делать с ним нечего.
    ///
    /// Теперь `journalctl -f` во время перезапуска или `tail -f` по логу
    /// приложения можно посмотреть столько, сколько нужно, и получить
    /// собранное за это время.
    ///
    /// Подтверждения нет: это чтение, и проверка команды та же, что у
    /// `darwaza_run`, — с единственным послаблением на флаг слежения.
    async fn follow(&self, params: serde_json::Value) -> AgentResponse {
        let host = match text_param(&params, "host", "хост") {
            Ok(v) => v,
            Err(r) => return r,
        };
        let command = match text_param(&params, "command", "команда") {
            Ok(v) => v,
            Err(r) => return r,
        };
        let seconds = params
            .get("seconds")
            .and_then(|v| v.as_u64())
            .unwrap_or(FOLLOW_SECONDS)
            .clamp(1, FOLLOW_SECONDS_MAX);

        let (host_id, host_name, sudo_allowed) = match self.allowed_host_full(host).await {
            Ok(found) => found,
            Err(e) => return AgentResponse::failed(e),
        };

        if let Some(reason) = safety::is_forbidden_outright(command) {
            log_action(
                &self.state().db,
                Some(&host_name),
                "follow",
                Some(command),
                "refused",
                None,
                Some(&reason),
            )
            .await;
            return AgentResponse::failed(reason);
        }
        if let Err(refusal) = safety::is_read_only_for_follow(command, &sudo_allowed) {
            let reason = refusal.to_string();
            log_action(
                &self.state().db,
                Some(&host_name),
                "follow",
                Some(command),
                "refused",
                None,
                Some(&reason),
            )
            .await;
            return AgentResponse::failed(reason);
        }

        let session_id = match self.session_for(host_id).await {
            Ok(id) => id,
            Err(e) => return AgentResponse::failed(e),
        };

        let limit = ExecLimit::Collect(std::time::Duration::from_secs(seconds));
        match self.exec(session_id, command, limit).await {
            Ok(result) => {
                log_action(
                    &self.state().db,
                    Some(&host_name),
                    "follow",
                    Some(command),
                    "ok",
                    result.exit_code.map(i64::from),
                    Some(&format!("{seconds} с")),
                )
                .await;
                info!(target: "agent", "наблюдение на {host_name} ({seconds} с): {command}");
                let mut value = serde_json::to_value(result).unwrap_or_default();
                if let Some(map) = value.as_object_mut() {
                    map.insert("seconds".into(), seconds.into());
                }
                AgentResponse::ok(value)
            }
            Err(e) => AgentResponse::failed(e),
        }
    }
}

/// Из чего складывается сводка о сервере (#224).
///
/// Набор закрытый и задан здесь, а не приходит от агента. Это не удобство
/// реализации: «выполни мне вот эти команды одним вызовом» было бы обходом
/// проверки команд целиком. Сводка — заранее известные вопросы, на которые
/// приложение знает, как ответить безопасно.
///
/// Все команды — только чтение и без `sudo`: сводку должно быть можно взять на
/// любом разрешённом хосте, ничего не настраивая. Поэтому в слушающих портах
/// не будет имён процессов — для них нужен `sudo`, и это отдельный разговор
/// (#214).
///
/// Сторожевой тест ниже проверяет каждую строку этого списка обычной проверкой
/// читающих команд. Без него сюда однажды допишут удобную команду, которую
/// агенту через `darwaza_run` выполнить нельзя.
const SUMMARY_PARTS: [(&str, &str); 7] = [
    ("os", "cat /etc/os-release"),
    ("kernel", "uname -sr"),
    ("uptime", "uptime"),
    ("disk", "df -h"),
    ("memory", "free -h"),
    // Без `-p`: имена процессов требуют прав, а порты видны и так.
    ("listening", "ss -tuln"),
    (
        "failed_services",
        "systemctl list-units --failed --no-pager",
    ),
];

/// Сколько текста берём от каждой части сводки.
///
/// `df -h` на машине с полусотней точек монтирования — это экран, а семь
/// таких частей сразу уедут в контекст агента. Обрезанное помечается, а не
/// молчит.
const SUMMARY_PART_LIMIT: usize = 4 * 1024;

/// Одна часть сводки.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SummaryPart {
    command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    truncated: bool,
    /// Почему части нет. Команда могла не найтись — `ss` есть не везде.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Обрезать вывод до предела, сообщив об этом.
fn clip(text: &str, limit: usize) -> (String, bool) {
    let trimmed = text.trim_end();
    if trimmed.len() <= limit {
        return (trimmed.to_string(), false);
    }
    // Режем по границе символа: строка по байтам рвёт UTF-8, а в выводе
    // команд бывают и русские буквы, и рамки псевдографики.
    let mut end = limit;
    while end > 0 && !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    (trimmed[..end].to_string(), true)
}

impl Tools {
    /// Сводка о сервере одним вызовом (#224).
    ///
    /// Удобство, а не новая возможность: всё это агент мог узнать и раньше,
    /// но пятью вызовами `darwaza_run` — то есть пятью кругами «спросить,
    /// дождаться, разобрать». Первый разговор о незнакомом сервере почти
    /// всегда начинается именно с этих вопросов.
    ///
    /// Неудача одной части не отменяет остальных: `ss` установлен не везде, и
    /// отсутствие портов — не повод не показать диск и память.
    async fn summary(&self, params: serde_json::Value) -> AgentResponse {
        let host = match text_param(&params, "host", "хост") {
            Ok(v) => v,
            Err(r) => return r,
        };
        let (host_id, host_name) = match self.allowed_host(host).await {
            Ok(found) => found,
            Err(e) => return AgentResponse::failed(e),
        };
        let session_id = match self.session_for(host_id).await {
            Ok(id) => id,
            Err(e) => return AgentResponse::failed(e),
        };

        let mut parts = serde_json::Map::new();
        for (name, command) in SUMMARY_PARTS {
            let part = match self
                .exec(
                    session_id,
                    command,
                    ExecLimit::Fail(crate::commands::ssh::EXEC_TIMEOUT_READ),
                )
                .await
            {
                Ok(result) if result.exit_code == Some(0) => {
                    let (output, truncated) = clip(&result.stdout, SUMMARY_PART_LIMIT);
                    SummaryPart {
                        command: command.to_string(),
                        output: Some(output),
                        truncated,
                        error: None,
                    }
                }
                // Ненулевой код — тоже неудача части, но разборчивая: чаще
                // всего это «команда не найдена».
                Ok(result) => SummaryPart {
                    command: command.to_string(),
                    output: None,
                    truncated: false,
                    error: Some(clip(&result.stderr, SUMMARY_PART_LIMIT).0),
                },
                Err(e) => SummaryPart {
                    command: command.to_string(),
                    output: None,
                    truncated: false,
                    error: Some(e),
                },
            };
            parts.insert(
                name.to_string(),
                serde_json::to_value(part).unwrap_or_default(),
            );
        }

        log_action(
            &self.state().db,
            Some(&host_name),
            "summary",
            None,
            "ok",
            None,
            None,
        )
        .await;
        info!(target: "agent", "{host_name}: сводка о сервере");
        AgentResponse::ok(serde_json::json!({ "host": host_name, "parts": parts }))
    }
}

/// Сколько записей журнала отдаём за раз и сколько максимум можно попросить
/// (#221).
///
/// Журнал ведётся с первого дня, и на живом сервере он длинный. Без предела
/// первый же вызов увёз бы в контекст агента всю историю хоста — а нужна ему
/// последняя работа, а не вся.
const JOURNAL_LIMIT: i64 = 50;
const JOURNAL_LIMIT_MAX: i64 = 200;

/// Запись журнала в том виде, в каком её видит агент.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JournalEntry {
    at: String,
    action: String,
    outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

/// Строка журнала как она лежит в базе: время, действие, что именно, итог,
/// код возврата, примечание.
type JournalRow = (
    String,
    String,
    Option<String>,
    String,
    Option<i64>,
    Option<String>,
);

/// Последние записи журнала по хосту, от старой к новой.
///
/// Вынесено из инструмента, чтобы запрос выполнял тест. Запрос, который
/// исполняется только у пользователя, — это не проверенный запрос: ровно так
/// в августе до окна доехало «14 values for 15 columns».
///
/// Выбираем с конца (`ORDER BY id DESC`), а отдаём с начала: `LIMIT` иначе
/// отрезал бы свежее вместо старого. Порядок по `id`, а не по `at`: время
/// пишется с точностью до секунды, и несколько действий одной секунды
/// расположились бы как попало.
async fn journal_entries(
    db: &sqlx::SqlitePool,
    host: &str,
    limit: i64,
) -> Result<Vec<JournalEntry>, sqlx::Error> {
    let rows: Vec<JournalRow> = sqlx::query_as(
        "SELECT at, action, detail, outcome, exit_code, note FROM agent_log \
         WHERE host = ?1 ORDER BY id DESC LIMIT ?2",
    )
    .bind(host)
    .bind(limit)
    .fetch_all(db)
    .await?;

    Ok(rows
        .into_iter()
        .rev()
        .map(
            |(at, action, detail, outcome, exit_code, note)| JournalEntry {
                at,
                action,
                outcome,
                detail,
                exit_code,
                note,
            },
        )
        .collect())
}

impl Tools {
    /// Журнал собственных действий агента по хосту (#221).
    ///
    /// Зачем. Каждая сессия агента начинается с нуля: он не помнит, что делал
    /// вчера, и повторяет разведку, а иногда и ошибку. Заметка о сервере
    /// (#217) отвечает «что здесь устроено», журнал — «что здесь уже делали».
    ///
    /// Отказы показываются наравне с успехами, и это важнее, чем кажется:
    /// «эту команду вчера не дали выполнить» избавляет от второй попытки в
    /// том же тупике.
    ///
    /// **Хост обязателен и проверяется на доступ.** Журнал ведётся по именам
    /// хостов, в том числе закрытых для агента; без проверки чтение журнала
    /// стало бы способом узнать, что человек делает на серверах, куда агента
    /// не пускали.
    ///
    /// **Само чтение журнала в журнал не пишется.** Иначе каждая сессия
    /// начинала бы с записи о том, что она прочитала журнал, и через неделю
    /// журнал состоял бы в основном из чтений журнала.
    async fn journal(&self, params: serde_json::Value) -> AgentResponse {
        let host = match text_param(&params, "host", "хост") {
            Ok(v) => v,
            Err(r) => return r,
        };
        let (_, host_name) = match self.allowed_host(host).await {
            Ok(found) => found,
            Err(e) => return AgentResponse::failed(e),
        };

        let limit = params
            .get("limit")
            .and_then(|v| v.as_i64())
            .unwrap_or(JOURNAL_LIMIT)
            .clamp(1, JOURNAL_LIMIT_MAX);

        let entries = match journal_entries(&self.state().db, &host_name, limit).await {
            Ok(entries) => entries,
            Err(e) => return AgentResponse::failed(format!("{e}")),
        };

        AgentResponse::ok(serde_json::json!({
            "host": host_name,
            "count": entries.len(),
            "entries": entries,
        }))
    }
}

/// Событие для окна: агент просит открыть человеку вкладку (#216).
pub const TERMINAL_EVENT: &str = "agent://terminal";

/// Что открыть человеку в терминале.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalRequest {
    pub host_id: i64,
    pub host: String,
    /// Команда, которая будет **набрана**, но не выполнена.
    pub command: Option<String>,
}

impl Tools {
    /// Открыть человеку вкладку с подготовленной командой (#216).
    ///
    /// Команда набирается в приглашении и **не выполняется**: Enter нажимает
    /// человек. Поэтому подтверждения здесь нет — выполнять нечего.
    ///
    /// Зачем это нужно. Часть работы агенту запрещена навсегда: разметка
    /// диска, смена ключа сервера, всё из списка `safety::NEVER`. Раньше он
    /// мог только написать «сделайте сами», и человек набирал команду заново,
    /// рискуя опечататься там, где опечатка дорога. Теперь команда уже на
    /// экране — остаётся прочитать её и решить.
    ///
    /// Запрещённое остаётся запрещённым **для агента**: он по-прежнему не
    /// может её выполнить. Но подать её человеку — не выполнение, а
    /// предложение, и решает по-прежнему человек.
    async fn open_terminal(&self, params: serde_json::Value) -> AgentResponse {
        let host = match text_param(&params, "host", "хост") {
            Ok(v) => v,
            Err(r) => return r,
        };
        let command = params
            .get("command")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|c| !c.is_empty());

        // Перевод строки в набранной команде означал бы выполнение — а обещано
        // обратное. Отклоняем явно, а не вырезаем молча: агент должен знать,
        // что его замысел не исполнен, иначе он решит, что команда пошла.
        if command.is_some_and(|c| c.contains('\n') || c.contains('\r')) {
            return AgentResponse::failed(
                "команда не должна содержать перевод строки: она набирается, но не \
                 выполняется — Enter нажимает человек",
            );
        }

        let (host_id, host_name) = match self.allowed_host(host).await {
            Ok(found) => found,
            Err(e) => return AgentResponse::failed(e),
        };

        let request = TerminalRequest {
            host_id,
            host: host_name.clone(),
            command: command.map(str::to_string),
        };
        if let Err(e) = self.app.emit(TERMINAL_EVENT, &request) {
            return AgentResponse::failed(format!("не удалось открыть вкладку: {e}"));
        }

        log_action(
            &self.state().db,
            Some(&host_name),
            "open_terminal",
            command,
            "ok",
            None,
            params.get("reason").and_then(|v| v.as_str()),
        )
        .await;
        info!(target: "agent", "{host_name}: открыта вкладка человеку");
        AgentResponse::ok(serde_json::json!({
            "opened": true,
            "note": "вкладка открыта, команда набрана и ждёт решения человека",
        }))
    }
}

/// Шаги плана из запроса агента.
///
/// Любой элемент не строкой — отказ всего плана. Молчаливо выбросить его
/// значило бы показать и выполнить меньше, чем описал агент, а агент считал бы
/// остальное выполненным.
fn parse_steps(params: &serde_json::Value) -> Result<Vec<String>, String> {
    let list = match params.get("steps").and_then(|v| v.as_array()) {
        Some(list) if !list.is_empty() => list,
        _ => return Err("не указаны шаги".to_string()),
    };
    list.iter()
        .enumerate()
        .map(|(index, value)| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("шаг {} не строка: ожидается команда текстом", index + 1))
        })
        .collect()
}

/// Адрес и пользователь хоста для окна подтверждения.
async fn host_address(
    db: &sqlx::SqlitePool,
    host_id: i64,
) -> Result<Option<(String, String)>, String> {
    sqlx::query_as("SELECT hostname, username FROM hosts WHERE id = ?1")
        .bind(host_id)
        .fetch_optional(db)
        .await
        .map_err(|e| format!("{e}"))
}

/// Событие для окна: пришёл план, требующий решения.
pub const PLAN_EVENT: &str = "agent://plan";

/// Сообщить окну, что карточка плана больше не действует.
pub fn emit_plan_gone(app: &AppHandle, id: &str, reason: super::PlanGoneReason) {
    let payload = super::PlanGone { id: id.to_string(), reason };
    if let Err(e) = app.emit(super::PLAN_GONE_EVENT, &payload) {
        tracing::warn!(target: "agent", "plan_gone не доставлено окну: {e}");
    }
}

/// Событие для окна: подтверждённый план выполнен или остановлен (#255).
pub const PLAN_RESULT_EVENT: &str = "agent://plan_result";

/// Исход одного шага для окна.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepReport {
    pub title: String,
    /// `done` — код 0; `failed` — ненулевой код или сбой связи; `blocked` —
    /// запрещённый шаг, до SSH не дошёл; `notRun` — не выполнялся.
    pub outcome: &'static str,
    pub exit_code: Option<i64>,
}

/// Итог плана для окна. **Без вывода команд**: stdout/stderr могут нести
/// секреты (конфиги, переменные окружения), а окно показывает итог на виду.
/// Вывод остаётся у агента и в журнале действий.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanReport {
    pub plan_id: String,
    pub host: String,
    /// `completed` | `stopped`.
    pub status: &'static str,
    pub steps: Vec<StepReport>,
    /// Почему остановились; усечено, чтобы не нести длинный текст ошибки.
    pub reason: Option<String>,
    pub done: usize,
    pub total: usize,
}

const REASON_MAX_CHARS: usize = 300;

impl PlanReport {
    /// Собрать итог из ответа `run_plan`. Берутся только исходы и коды.
    pub fn from_result(
        plan_id: &str,
        host: &str,
        steps: &[String],
        result: &serde_json::Value,
    ) -> Self {
        let mut reports: Vec<StepReport> = Vec::with_capacity(steps.len());
        for entry in result["completed"].as_array().into_iter().flatten() {
            let code = entry["exitCode"].as_i64();
            reports.push(StepReport {
                title: entry["step"].as_str().unwrap_or_default().to_string(),
                outcome: if code.unwrap_or(0) == 0 { "done" } else { "failed" },
                exit_code: code,
            });
        }
        let stopped = result["status"] == "stopped";
        let blocked = result["blocked"] == true;
        let stopped_at = result["stoppedAt"].as_str();
        // Сбой связи на шаге: шаг не попал ни в completed, ни в notRun.
        if stopped && !blocked {
            if let Some(title) = stopped_at {
                if reports.last().map(|r| r.title.as_str()) != Some(title) {
                    reports.push(StepReport {
                        title: title.to_string(),
                        outcome: "failed",
                        exit_code: None,
                    });
                }
            }
        }
        for (i, entry) in result["notRun"].as_array().into_iter().flatten().enumerate() {
            reports.push(StepReport {
                title: entry.as_str().unwrap_or_default().to_string(),
                outcome: if blocked && i == 0 { "blocked" } else { "notRun" },
                exit_code: None,
            });
        }
        let done = reports.iter().filter(|r| r.outcome == "done").count();
        PlanReport {
            plan_id: plan_id.to_string(),
            host: host.to_string(),
            status: if stopped { "stopped" } else { "completed" },
            reason: result["reason"]
                .as_str()
                .map(|r| r.chars().take(REASON_MAX_CHARS).collect()),
            steps: reports,
            done,
            total: steps.len(),
        }
    }
}

impl Tools {
    /// Спросить человека и дождаться ответа.
    ///
    /// Ожидание ограничено: агент, повисший навсегда из-за закрытого окна,
    /// выглядит как поломка приложения, хотя решение просто некому принять.
    async fn ask(&self, mut plan: PendingPlan) -> Result<(), String> {
        let state = self.state();

        // Адрес — того хоста, на котором план выполнится: по `id`, а не по
        // имени. Имя не уникально, и при двух хостах «db» окно показало бы
        // один адрес, а команды ушли бы на другой.
        if let Some((address, user)) = host_address(&state.db, plan.host_id).await? {
            plan.host_address = address;
            plan.host_user = user;
        }

        let caller = CALLER.try_with(Clone::clone).unwrap_or_default();
        plan.client = caller.client;
        plan.session = caller.session;

        // Срок — тот же, что у ожидания ниже, и считается один раз: окно
        // показывает обратный отсчёт, который не должен расходиться с делом.
        plan.expires_at =
            chrono::Utc::now().timestamp_millis() + super::PLAN_DECISION_TIMEOUT.as_millis() as i64;

        let waiting = state.plans.enqueue(plan.clone()).await;
        let mut gone = super::GoneGuard::new(state.plans.clone(), plan.id.clone(), {
            let app = self.app.clone();
            let id = plan.id.clone();
            move |reason| emit_plan_gone(&app, &id, reason)
        });

        crate::attention::needs_decision(&self.app).await;
        if let Err(e) = self.app.emit(PLAN_EVENT, &plan) {
            // Некому показать план — значит некому и подтвердить. Убираем
            // его из очереди сразу: иначе человек пять минут ждал бы окна,
            // которого не будет.
            state.plans.resolve(&plan.id, PlanVerdict::Rejected).await;
            return Err(format!("не удалось показать план: {e}"));
        }

        match tokio::time::timeout(super::PLAN_DECISION_TIMEOUT, waiting).await {
            Ok(Ok(PlanVerdict::Approved)) => {
                gone.disarm();
                Ok(())
            }
            Ok(Ok(PlanVerdict::Rejected)) => {
                // Об отказе окну сообщает та команда, которой человек отказал.
                gone.disarm();
                Err("план отклонён".to_string())
            }
            Ok(Err(_)) => {
                gone.finish(super::PlanGoneReason::Cancelled);
                Err("окно подтверждения закрылось".to_string())
            }
            Err(_) => {
                state.plans.resolve(&plan.id, PlanVerdict::Rejected).await;
                gone.finish(super::PlanGoneReason::Expired);
                Err("не дождались решения: подтвердите план в окне Darwaza".to_string())
            }
        }
    }

    /// Выполнить одну команду и вернуть её результат.
    /// Выполнить одну команду с явным пределом времени.
    ///
    /// Предел параметром, а не общей константой: шагу подтверждённого плана
    /// нужны минуты, а проверке конфигурации после записи файла — секунды
    /// (#213).
    async fn exec(
        &self,
        session_id: Uuid,
        command: &str,
        limit: ExecLimit,
    ) -> Result<crate::ssh::session::ExecResult, String> {
        let (tx, rx) = oneshot::channel();
        self.state()
            .ssh
            .send_cmd(
                session_id,
                SessionCmd::Exec {
                    command: command.to_string(),
                    limit,
                    responder: tx,
                },
            )
            .await?;
        rx.await
            .map_err(|_| "сессия закрылась до ответа".to_string())?
    }

    /// План изменений: агент описывает замысел и шаги, человек подтверждает
    /// **один раз** и целиком.
    ///
    /// Подтверждать каждую из тридцати команд деплоя невыносимо, и через три
    /// раза начнут жать «да» не глядя — что хуже, чем не спрашивать вовсе.
    async fn plan(&self, params: serde_json::Value) -> AgentResponse {
        let host = match params.get("host").and_then(|v| v.as_str()) {
            Some(h) => h,
            None => return AgentResponse::failed("не указан хост"),
        };
        let intent = match params.get("intent").and_then(|v| v.as_str()) {
            Some(i) if !i.trim().is_empty() => i,
            // Без объяснения человек подтверждает вслепую, а значит и
            // подтверждение ничего не стоит.
            _ => return AgentResponse::failed("не указано, зачем это делается (intent)"),
        };
        let steps = match parse_steps(&params) {
            Ok(steps) => steps,
            Err(e) => return AgentResponse::failed(e),
        };

        // Запрещённый шаг человеку показывается, но не выполняется никогда:
        // исполнение останавливается перед ним. Показывать его нужно — иначе
        // человек не узнает, что именно агент пытался сделать. Заранее же
        // отказываем, когда нечего исполнять вовсе: подтверждать нечего.
        let classified = classify_steps(&steps);
        for blocked in classified.iter().filter(|step| !step.will_run()) {
            if let Some(reason) = &blocked.blocked_reason {
                log_action(
                    &self.state().db,
                    Some(host),
                    "plan",
                    Some(&blocked.title),
                    "refused",
                    None,
                    Some(reason),
                )
                .await;
            }
        }
        if let Some(first) = classified.first() {
            if let Some(reason) = &first.blocked_reason {
                return AgentResponse::failed(reason.clone());
            }
        }

        let (host_id, host_name) = match self.allowed_host(host).await {
            Ok(found) => found,
            Err(e) => return AgentResponse::failed(e),
        };

        let plan = PendingPlan::new(host_id, host_name.clone(), intent, classified.clone());
        let plan_id = plan.id.clone();

        if let Err(e) = self.ask(plan).await {
            log_action(
                &self.state().db,
                Some(&host_name),
                "plan",
                Some(intent),
                "refused",
                None,
                Some(&e),
            )
            .await;
            return AgentResponse::failed(e);
        }

        let session_id = match self.session_for(host_id).await {
            Ok(id) => id,
            Err(e) => return AgentResponse::failed(e),
        };

        let runner = LivePlan {
            tools: self,
            session_id,
            host_name: &host_name,
        };
        let result = run_plan(&runner, &host_name, intent, &steps, &classified).await;
        // Окно узнаёт итог отдельным событием: подтверждение вернулось сразу,
        // а выполнение идёт минутами в вызове агента.
        let report = PlanReport::from_result(&plan_id, &host_name, &steps, &result);
        if let Err(e) = self.app.emit(PLAN_RESULT_EVENT, &report) {
            tracing::warn!(target: "agent", "итог плана не доставлен окну: {e}");
        }
        AgentResponse::ok(result)
    }

    /// Правка файла: агент присылает новое содержимое, человек видит разницу.
    ///
    /// Именно содержимое, а не команду правки: `sed -i 's/80/443/g'`
    /// выглядит безобидно, пока не окажется, что в файле было ещё три
    /// восьмидесятых.
    async fn write_file(&self, params: serde_json::Value) -> AgentResponse {
        let host = match params.get("host").and_then(|v| v.as_str()) {
            Some(h) => h,
            None => return AgentResponse::failed("не указан хост"),
        };
        let path = match params.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return AgentResponse::failed("не указан путь"),
        };
        let content = match params.get("content").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return AgentResponse::failed("не указано новое содержимое"),
        };
        let intent = params
            .get("intent")
            .and_then(|v| v.as_str())
            .unwrap_or("правка файла");

        let prepared = match self.prepare_write(host, path).await {
            Ok(p) => p,
            Err(e) => return AgentResponse::failed(e),
        };
        self.confirm_and_write(&prepared, path, content, intent)
            .await
    }

    /// Правка файла заплаткой (#223).
    ///
    /// Вместо нового содержимого целиком агент присылает список замен: что
    /// найти и чем заменить. Для `nginx.conf` на полсотни килобайт это разница
    /// между «переслать весь файл» и «переслать две строки», причём в обе
    /// стороны — читать его целиком тоже приходилось.
    ///
    /// **Человек при этом видит ровно то же, что и раньше.** Подтверждение
    /// показывает готовую разницу двух содержимых, а не текст заплатки:
    /// заплатка — способ её описать, а не способ её показать.
    ///
    /// Формат замен намеренно простейший — найти и заменить, без контекста и
    /// номеров строк. Единый diff пришлось бы разбирать со всеми его
    /// смещениями и приблизительными совпадениями, а ошибка такого разбора
    /// молча правит не ту строку.
    ///
    /// Каждый фрагмент обязан встречаться в файле **ровно один раз**. Два
    /// совпадения — отказ, а не «возьмём первое»: агент почти наверняка имел в
    /// виду конкретное место, и угадывать его за него нельзя.
    async fn patch_file(&self, params: serde_json::Value) -> AgentResponse {
        let host = match params.get("host").and_then(|v| v.as_str()) {
            Some(h) => h,
            None => return AgentResponse::failed("не указан хост"),
        };
        let path = match params.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return AgentResponse::failed("не указан путь"),
        };
        let edits = match parse_edits(&params) {
            Ok(edits) => edits,
            Err(e) => return AgentResponse::failed(e),
        };
        let intent = params
            .get("intent")
            .and_then(|v| v.as_str())
            .unwrap_or("правка файла");

        let prepared = match self.prepare_write(host, path).await {
            Ok(p) => p,
            Err(e) => return AgentResponse::failed(e),
        };

        // Пустой файл заплаткой не правят: искать в нём нечего, и «замена не
        // нашлась» объясняет происходящее хуже, чем прямой ответ.
        if prepared.before.is_empty() {
            return AgentResponse::failed(format!(
                "файла {path} нет или он пуст — заплатка ищет в существующем тексте; создать файл можно через files.write"
            ));
        }

        let content = match apply_edits(&prepared.before, &edits) {
            Ok(content) => content,
            Err(e) => return AgentResponse::failed(e),
        };

        self.confirm_and_write(&prepared, path, &content, intent)
            .await
    }

    /// Хост, сессия, SFTP и текущее содержимое файла — всё, что нужно любой
    /// записи.
    async fn prepare_write(&self, host: &str, path: &str) -> Result<PreparedWrite, String> {
        if let Some(reason) = safety::is_forbidden_outright(&format!("tee {path}")) {
            return Err(reason);
        }

        let (host_id, host_name) = self.allowed_host(host).await?;
        let session_id = self.session_for(host_id).await?;
        let sftp = crate::commands::sftp::sftp_for(&self.state(), session_id).await?;

        // Отсутствие файла — не ошибка: создание конфига с нуля тоже правка.
        let before = match sftp.read(path.to_string()).await {
            Ok(bytes) => String::from_utf8_lossy(&bytes).to_string(),
            Err(_) => String::new(),
        };

        Ok(PreparedWrite {
            host_id,
            host_name,
            session_id,
            sftp,
            before,
        })
    }

    /// Показать разницу, дождаться подтверждения, записать и проверить.
    ///
    /// Общая часть обычной записи и заплатки. Разделять её было бы опасно:
    /// копия перед записью, откат при непрошедшей проверке и журнал должны
    /// работать одинаково независимо от того, как агент описал правку.
    async fn confirm_and_write(
        &self,
        prepared: &PreparedWrite,
        path: &str,
        content: &str,
        intent: &str,
    ) -> AgentResponse {
        let PreparedWrite {
            host_id,
            host_name,
            session_id,
            sftp,
            before,
        } = prepared;
        let session_id = *session_id;

        let changes = diff::diff(before, content);
        if !diff::has_changes(&changes) {
            return AgentResponse::ok(serde_json::json!({
                "changed": false,
                "note": "файл уже в нужном виде",
            }));
        }

        let plan = PendingPlan::new(
            *host_id,
            host_name.clone(),
            format!("{intent} ({path})"),
            vec![PlanStep::edit(format!("записать {path}"), changes)],
        );

        if let Err(e) = self.ask(plan).await {
            log_action(
                &self.state().db,
                Some(host_name),
                "write",
                Some(path),
                "refused",
                None,
                Some(&e),
            )
            .await;
            return AgentResponse::failed(e);
        }

        // Копия ДО записи. Без неё откат невозможен ровно тогда, когда он
        // нужен: файл уже перезаписан, а прежнего содержимого ни у кого нет.
        let backup = format!("{path}.darwaza-bak");
        if !before.is_empty() {
            if let Err(e) = sftp_put(sftp, &backup, before.as_bytes()).await {
                return AgentResponse::failed(format!("не удалось сохранить копию: {e}"));
            }
        }

        if let Err(e) = sftp_put(sftp, path, content.as_bytes()).await {
            return AgentResponse::failed(format!("запись не удалась: {e}"));
        }

        // Проверка конфигурации — то, о чём агент забудет. Сломанный конфиг
        // не должен дожить до перезапуска службы.
        let mut validation = serde_json::Value::Null;
        if let Some(check) = diff::validation_command(path) {
            match self
                .exec(
                    session_id,
                    check,
                    ExecLimit::Fail(crate::commands::ssh::EXEC_TIMEOUT_READ),
                )
                .await
            {
                Ok(result) if result.exit_code.unwrap_or(0) != 0 => {
                    // Откатываем сами: оставить сломанный конфиг и сообщить
                    // об этом агенту — значит переложить починку на того, у
                    // кого может не хватить прав или желания.
                    let restored = if before.is_empty() {
                        sftp.remove_file(path.to_string()).await.is_ok()
                    } else {
                        sftp_put(sftp, path, before.as_bytes()).await.is_ok()
                    };
                    log_action(
                        &self.state().db,
                        Some(host_name),
                        "write",
                        Some(path),
                        "failed",
                        result.exit_code.map(i64::from),
                        Some(&format!("{check}: {}", result.stderr.trim())),
                    )
                    .await;
                    return AgentResponse::failed(format!(
                        "проверка '{check}' не прошла, изменения {}: {}",
                        if restored {
                            "откачены"
                        } else {
                            "ОСТАЛИСЬ — откат не удался"
                        },
                        result.stderr.trim()
                    ));
                }
                Ok(result) => {
                    validation = serde_json::json!({ "command": check, "ok": true, "output": result.stdout });
                }
                Err(e) => {
                    validation = serde_json::json!({ "command": check, "ok": false, "error": e });
                }
            }
        }

        log_action(
            &self.state().db,
            Some(host_name),
            "write",
            Some(path),
            "ok",
            None,
            None,
        )
        .await;
        info!(target: "agent", "записан файл на {host_name}: {path}");

        AgentResponse::ok(serde_json::json!({
            "changed": true,
            "backup": if before.is_empty() { serde_json::Value::Null } else { backup.into() },
            "validation": validation,
        }))
    }
}

/// Всё, что нужно записи файла: куда, чем и что там сейчас.
struct PreparedWrite {
    host_id: i64,
    host_name: String,
    session_id: Uuid,
    sftp: std::sync::Arc<russh_sftp::client::SftpSession>,
    before: String,
}

/// Одна замена: что найти и чем заменить.
#[derive(Debug, Clone)]
struct Edit {
    find: String,
    replace: String,
}

/// Разобрать список замен из параметров вызова.
fn parse_edits(params: &serde_json::Value) -> Result<Vec<Edit>, String> {
    let items = params
        .get("edits")
        .and_then(|v| v.as_array())
        .filter(|a| !a.is_empty())
        .ok_or("не указаны замены (edits)")?;

    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let find = item
                .get("find")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| format!("в замене {} не указано, что искать (find)", i + 1))?;
            // Пустая замена — законное удаление фрагмента, поэтому проверяем
            // только наличие поля, а не его непустоту.
            let replace = item
                .get("replace")
                .and_then(|v| v.as_str())
                .ok_or_else(|| format!("в замене {} не указано, чем заменить (replace)", i + 1))?;
            Ok(Edit {
                find: find.to_string(),
                replace: replace.to_string(),
            })
        })
        .collect()
}

/// Применить замены по порядку.
///
/// Каждый фрагмент обязан встречаться **ровно один раз** в том тексте, каким
/// он стал после предыдущих замен. Не «хотя бы раз»: два совпадения означают,
/// что агент не описал место однозначно, и выбирать за него первое — молча
/// править не ту строку.
///
/// Считаем совпадения заново перед каждой заменой именно поэтому: первая
/// замена может создать второе вхождение того, что ищет вторая.
fn apply_edits(before: &str, edits: &[Edit]) -> Result<String, String> {
    let mut text = before.to_string();
    for (i, edit) in edits.iter().enumerate() {
        let count = text.matches(&edit.find).count();
        let shown = short(&edit.find);
        match count {
            0 => {
                return Err(format!(
                    "замена {}: в файле нет такого фрагмента — '{shown}'. Искомое должно совпадать дословно, включая отступы",
                    i + 1
                ))
            }
            1 => {}
            n => {
                return Err(format!(
                    "замена {}: фрагмент '{shown}' встречается {n} раз. Добавьте окружающие строки, чтобы место было одно",
                    i + 1
                ))
            }
        }
        text = text.replace(&edit.find, &edit.replace);
    }
    Ok(text)
}

/// Короткая выдержка фрагмента для сообщения об ошибке.
///
/// Без неё отказ по конфигу на сорок строк был бы отказом на сорок строк.
fn short(text: &str) -> String {
    const LIMIT: usize = 60;
    let one_line = text.replace('\n', "⏎");
    if one_line.chars().count() <= LIMIT {
        return one_line;
    }
    let cut: String = one_line.chars().take(LIMIT).collect();
    format!("{cut}…")
}

/// Предел на чтение файла целиком.
///
/// Столько же, сколько на вывод команды: конфиг или журнал столько не
/// занимают, а всё большее агенту незачем передавать целиком.
const FILE_READ_LIMIT: u64 = 512 * 1024;

#[async_trait::async_trait]
impl AgentHandler for Tools {
    async fn handle(&self, method: &str, params: serde_json::Value) -> AgentResponse {
        match method {
            "hosts.list" => self.hosts_list().await,
            "hosts.run" => self.run(params).await,
            "files.read" => self.read_file(params).await,
            "hosts.plan" => self.plan(params).await,
            "files.write" => self.write_file(params).await,
            "files.patch" => self.patch_file(params).await,
            "files.list" => self.list_dir(params).await,
            "files.stat" => self.stat(params).await,
            "files.download" => self.download(params).await,
            "files.upload" => self.upload(params).await,
            "files.mkdir" => self.mkdir(params).await,
            "files.rename" => self.rename(params).await,
            "files.delete" => self.delete(params).await,
            "hosts.service" => self.service(params).await,
            "hosts.open_terminal" => self.open_terminal(params).await,
            "hosts.notes" => self.host_notes(params).await,
            "hosts.journal" => self.journal(params).await,
            "hosts.summary" => self.summary(params).await,
            "hosts.follow" => self.follow(params).await,
            "hosts.tunnel" => self.tunnel(params).await,
            "files.copy" => self.copy_file(params).await,
            "snippets.list" => self.snippets_list().await,
            "snippets.run" => self.run_snippet(params).await,
            other => AgentResponse::failed(format!("неизвестный метод '{other}'")),
        }
    }
}

/// Как план исполняется на деле. Отдельный интерфейс — чтобы цикл ниже можно
/// было проверить подставным исполнителем: настоящей SSH-сессии в тесте нет, а
/// без теста на самом цикле замена `return` на `continue` проходит мимо всех.
#[async_trait::async_trait]
trait PlanRunner: Sync {
    /// Выполнить одну команду.
    async fn run(&self, step: &str) -> Result<crate::ssh::session::ExecResult, String>;
    /// Записать итог шага в журнал агента.
    async fn record(&self, step: &str, outcome: &str, exit_code: Option<i64>, note: Option<&str>);
}

struct LivePlan<'a> {
    tools: &'a Tools,
    session_id: Uuid,
    host_name: &'a str,
}

#[async_trait::async_trait]
impl PlanRunner for LivePlan<'_> {
    async fn run(&self, step: &str) -> Result<crate::ssh::session::ExecResult, String> {
        self.tools
            .exec(
                self.session_id,
                step,
                ExecLimit::Fail(crate::commands::ssh::EXEC_TIMEOUT_LONG),
            )
            .await
    }

    async fn record(&self, step: &str, outcome: &str, exit_code: Option<i64>, note: Option<&str>) {
        log_action(
            &self.tools.state().db,
            Some(self.host_name),
            "plan",
            Some(step),
            outcome,
            exit_code,
            note,
        )
        .await;
    }
}

/// Выполнить подтверждённый план по шагам.
///
/// Останавливаемся на первой же ошибке: иначе дойдём до перезапуска службы со
/// сломанной сборкой — а это ровно тот случай, ради которого всё подтверждение
/// и затевалось. Дальше первого запрещённого шага не идём никогда.
async fn run_plan<R: PlanRunner>(
    runner: &R,
    host_name: &str,
    intent: &str,
    steps: &[String],
    classified: &[PlanStep],
) -> serde_json::Value {
    let mut done = Vec::new();
    let total = steps.len();
    let allowed = runnable_prefix(classified);
    for (index, step) in steps.iter().enumerate() {
        if index >= allowed {
            return serde_json::json!({
                "status": "stopped",
                "stoppedAt": step,
                "reason": classified[index].blocked_reason.clone().unwrap_or_else(
                    || "стоит после запрещённого шага".to_string()
                ),
                "blocked": true,
                "completed": done,
                "notRun": steps[index..],
            });
        }
        // Запрет проверяется ещё раз непосредственно перед шагом, а не только
        // при составлении плана: подтверждённый план — это список строк, и
        // «наверняка уже проверено» здесь не аргумент.
        if let Some(reason) = safety::is_forbidden_outright(step) {
            runner.record(step, "refused", None, Some(&reason)).await;
            return serde_json::json!({
                "status": "stopped",
                "stoppedAt": step,
                "reason": reason,
                "blocked": true,
                "completed": done,
                "notRun": steps[index..],
            });
        }

        // Отметка ДО выполнения, а не только после. Без неё в журнале видны
        // лишь завершённые шаги, и на вопрос «где оно стоит сейчас» ответить
        // нечем — а шаг может идти минутами (#213).
        info!(target: "agent", "{host_name}: шаг {}/{total} — {step}", index + 1);
        let result = match runner.run(step).await {
            Ok(r) => r,
            Err(e) => {
                runner.record(step, "failed", None, Some(&e)).await;
                return serde_json::json!({
                    "status": "stopped",
                    "stoppedAt": step,
                    "reason": e,
                    "completed": done,
                    "notRun": steps[index + 1..],
                });
            }
        };

        let failed = result.exit_code.unwrap_or(0) != 0;
        runner
            .record(
                step,
                if failed { "failed" } else { "ok" },
                result.exit_code.map(i64::from),
                None,
            )
            .await;

        done.push(serde_json::json!({
            "step": step,
            "exitCode": result.exit_code,
            "stdout": result.stdout,
            "stderr": result.stderr,
        }));

        if failed {
            info!(target: "agent", "план остановлен на шаге '{step}' на {host_name}");
            return serde_json::json!({
                "status": "stopped",
                "stoppedAt": step,
                "reason": "шаг завершился с ненулевым кодом",
                "completed": done,
                "notRun": steps[index + 1..],
            });
        }
    }

    info!(target: "agent", "план выполнен на {host_name}: {intent}");
    serde_json::json!({ "status": "completed", "completed": done })
}

#[cfg(test)]
mod plan_report_tests {
    use super::*;

    fn report(steps: &[&str], result: serde_json::Value) -> PlanReport {
        let steps: Vec<String> = steps.iter().map(|s| s.to_string()).collect();
        PlanReport::from_result("p1", "web", &steps, &result)
    }

    #[test]
    fn blocked_plan_shows_done_blocked_and_not_run_without_output() {
        let r = report(
            &["ls /a", "mkfs.ext4 /dev/sdb", "ls /b"],
            serde_json::json!({
                "status": "stopped", "stoppedAt": "mkfs.ext4 /dev/sdb",
                "reason": "запрещено", "blocked": true,
                "completed": [{"step": "ls /a", "exitCode": 0, "stdout": "СЕКРЕТ", "stderr": ""}],
                "notRun": ["mkfs.ext4 /dev/sdb", "ls /b"],
            }),
        );
        let outcomes: Vec<_> = r.steps.iter().map(|s| s.outcome).collect();
        assert_eq!(outcomes, ["done", "blocked", "notRun"]);
        assert_eq!((r.status, r.done, r.total), ("stopped", 1, 3));
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("СЕКРЕТ"), "вывод команд в итог не попадает");
        assert!(json.contains("\"planId\"") && json.contains("\"exitCode\""));
    }

    #[test]
    fn nonzero_exit_is_failed_and_the_rest_not_run() {
        let r = report(
            &["a", "b"],
            serde_json::json!({
                "status": "stopped", "stoppedAt": "a", "reason": "ненулевой",
                "completed": [{"step": "a", "exitCode": 2, "stdout": "", "stderr": ""}],
                "notRun": ["b"],
            }),
        );
        let outcomes: Vec<_> = r.steps.iter().map(|s| (s.outcome, s.exit_code)).collect();
        assert_eq!(outcomes, [("failed", Some(2)), ("notRun", None)]);
    }

    #[test]
    fn connection_failure_marks_the_step_failed_and_cuts_long_reason() {
        let long = "ж".repeat(1000);
        let r = report(
            &["a", "b"],
            serde_json::json!({
                "status": "stopped", "stoppedAt": "a", "reason": long,
                "completed": [], "notRun": ["b"],
            }),
        );
        assert_eq!(r.steps[0].outcome, "failed");
        assert_eq!(r.steps[0].exit_code, None);
        assert_eq!(r.reason.unwrap().chars().count(), REASON_MAX_CHARS);
    }

    #[test]
    fn completed_plan_is_all_done() {
        let r = report(
            &["a", "b"],
            serde_json::json!({"status": "completed", "completed": [
                {"step": "a", "exitCode": 0, "stdout": "", "stderr": ""},
                {"step": "b", "exitCode": 0, "stdout": "", "stderr": ""}]}),
        );
        assert_eq!((r.status, r.done, r.total), ("completed", 2, 2));
        assert!(r.reason.is_none());
    }
}

#[cfg(test)]
mod parse_steps_tests {
    use super::*;

    #[test]
    fn a_non_string_step_rejects_the_whole_plan() {
        let params = serde_json::json!({ "steps": ["ls", 42, { "cmd": "rm -rf x" }] });
        let error = parse_steps(&params).expect_err("часть шагов не строки");
        assert!(error.contains("шаг 2"), "{error}");
    }

    #[test]
    fn plain_strings_pass_in_order_and_empty_is_refused() {
        let ok = parse_steps(&serde_json::json!({ "steps": ["a", "b"] })).unwrap();
        assert_eq!(ok, vec!["a", "b"]);
        assert!(parse_steps(&serde_json::json!({ "steps": [] })).is_err());
        assert!(parse_steps(&serde_json::json!({})).is_err());
    }
}

#[cfg(test)]
mod host_address_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    /// Адрес в окне подтверждения — того хоста, что выбран, а не любого с тем
    /// же именем. Имя в таблице не уникально.
    #[tokio::test]
    async fn the_address_belongs_to_the_chosen_host_not_to_its_namesake() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("память");
        crate::db::MIGRATOR.run(&pool).await.expect("миграции");

        let mut ids = Vec::new();
        for (address, sync_id) in [
            ("10.0.0.5", "aaaa0000-0000-4000-8000-000000000001"),
            ("10.0.0.9", "aaaa0000-0000-4000-8000-000000000002"),
        ] {
            let id: i64 = sqlx::query_scalar(
                "INSERT INTO hosts (name, hostname, port, username, auth_method, sync_id) \
                 VALUES ('db', ?1, 22, 'root', 'password', ?2) RETURNING id",
            )
            .bind(address)
            .bind(sync_id)
            .fetch_one(&pool)
            .await
            .expect("хост");
            ids.push(id);
        }

        let first = host_address(&pool, ids[0]).await.unwrap().unwrap();
        let second = host_address(&pool, ids[1]).await.unwrap().unwrap();

        assert_eq!(first, ("10.0.0.5".to_string(), "root".to_string()));
        assert_eq!(second, ("10.0.0.9".to_string(), "root".to_string()));
    }
}

#[cfg(test)]
mod plan_runner_tests {
    use super::*;
    use std::sync::Mutex;

    /// Подставной исполнитель: запоминает, что до него дошло.
    struct Recorder {
        ran: Mutex<Vec<String>>,
        exit_code: u32,
    }

    #[async_trait::async_trait]
    impl PlanRunner for Recorder {
        async fn run(&self, step: &str) -> Result<crate::ssh::session::ExecResult, String> {
            self.ran.lock().unwrap().push(step.to_string());
            Ok(crate::ssh::session::ExecResult {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: Some(self.exit_code),
                truncated: false,
                stopped_by_time: false,
            })
        }
        async fn record(&self, _: &str, _: &str, _: Option<i64>, _: Option<&str>) {}
    }

    async fn run(steps: &[&str], exit_code: u32) -> (serde_json::Value, Vec<String>) {
        let steps: Vec<String> = steps.iter().map(|s| s.to_string()).collect();
        let classified = classify_steps(&steps);
        let runner = Recorder {
            ran: Mutex::new(Vec::new()),
            exit_code,
        };
        let answer = run_plan(&runner, "web", "проверка", &steps, &classified).await;
        let ran = runner.ran.into_inner().unwrap();
        (answer, ran)
    }

    /// Ничего начиная с запрещённого шага не доходит до SSH.
    ///
    /// Это тест самого исполнителя, а не `runnable_prefix`: промежуточное
    /// решение «пропустить запрещённый и идти дальше» (`continue` вместо
    /// `return`) ломает ровно здесь, и функции-классификаторы этого не видят.
    #[tokio::test]
    async fn nothing_from_the_blocked_step_onward_reaches_ssh() {
        let (answer, ran) = run(&["ls /etc/nginx", "mkfs.ext4 /dev/sdb", "ls /var"], 0).await;

        assert_eq!(ran, vec!["ls /etc/nginx"], "до SSH дошёл только первый шаг");
        assert_eq!(answer["stoppedAt"], "mkfs.ext4 /dev/sdb");
        assert_eq!(answer["blocked"], true);
        assert_eq!(
            answer["status"], "stopped",
            "остановка на запретном шаге — не успех, агент не должен считать план выполненным"
        );
        assert_eq!(answer["notRun"].as_array().unwrap().len(), 2);
    }

    /// Ошибка шага тоже останавливает план: следующие не выполняются.
    #[tokio::test]
    async fn a_failing_step_stops_the_rest() {
        let (answer, ran) = run(&["ls /etc/nginx", "ls /var"], 1).await;

        assert_eq!(ran, vec!["ls /etc/nginx"]);
        assert_eq!(answer["stoppedAt"], "ls /etc/nginx");
        assert_eq!(answer["status"], "stopped");
        assert_eq!(answer["notRun"], serde_json::json!(["ls /var"]));
    }

    /// Чистый план проходит целиком.
    #[tokio::test]
    async fn a_clean_plan_runs_every_step() {
        let (answer, ran) = run(&["ls /etc/nginx", "ls /var"], 0).await;

        assert_eq!(ran, vec!["ls /etc/nginx", "ls /var"]);
        assert_eq!(answer["completed"].as_array().unwrap().len(), 2);
        assert_eq!(answer["status"], "completed");
    }

    /// Второй барьер: запрет проверяется перед шагом, даже если классификатор
    /// пропустил. Подставляем «классификацию», в которой всё разрешено.
    #[tokio::test]
    async fn the_forbidden_check_runs_again_before_each_step() {
        let steps = vec![
            "ls /etc/nginx".to_string(),
            "mkfs.ext4 /dev/sdb".to_string(),
        ];
        let lenient: Vec<PlanStep> = classify_steps(&["ls /a".to_string(), "ls /b".to_string()]);
        let runner = Recorder {
            ran: Mutex::new(Vec::new()),
            exit_code: 0,
        };

        let answer = run_plan(&runner, "web", "проверка", &steps, &lenient).await;

        assert_eq!(runner.ran.into_inner().unwrap(), vec!["ls /etc/nginx"]);
        assert_eq!(answer["blocked"], true);
    }
}

#[cfg(test)]
mod tunnel_tests {
    use super::*;

    /// Туннель слушает только себя и только как local.
    ///
    /// Тест не о коде, а о границе. `0.0.0.0` вместо `127.0.0.1` — правка в
    /// один символ, которую легко сделать «чтобы проверить с телефона», а
    /// означает она открытый всей местной сети — кафе, коворкингу — порт
    /// внутрь чужого боевого сервера.
    #[test]
    fn a_tunnel_listens_only_on_loopback() {
        let spec = tunnel_spec(-1, "10.0.0.5", 5432);

        assert_eq!(spec.local_host, "127.0.0.1");
        assert_eq!(
            spec.kind, "local",
            "dynamic — это SOCKS, а не доступ к службе"
        );
        assert_eq!(spec.local_port, 0, "порт выбирает система");
        assert_eq!(spec.remote_host.as_deref(), Some("10.0.0.5"));
        assert_eq!(spec.remote_port, Some(5432));
    }

    /// Номера временных пробросов не могут совпасть с правилами из базы.
    ///
    /// Реестр активных пробросов у них общий, а `id` правила выдаёт
    /// AUTOINCREMENT и он положительный. Совпадение означало бы, что туннель
    /// агента останавливает чужой проброс владельца — или наоборот.
    #[test]
    fn tunnel_ids_are_negative_and_never_repeat() {
        let first = next_tunnel_id();
        let second = next_tunnel_id();

        assert!(first < 0 && second < 0, "{first}, {second}");
        assert_ne!(first, second);
    }
}

#[cfg(test)]
mod snippet_tests {
    use sqlx::sqlite::SqlitePoolOptions;

    async fn db_with_snippets() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("память");
        crate::db::MIGRATOR.run(&pool).await.expect("миграции");

        for (name, command, allowed) in [
            ("открытый", "systemctl status web", 1),
            ("закрытый", "mysql -u root -pSECRET < dump.sql", 0),
        ] {
            sqlx::query(
                "INSERT INTO snippets (name, command, sort_order, agent_access, sync_id) \
                 VALUES (?1, ?2, 0, ?3, ?4)",
            )
            .bind(name)
            .bind(command)
            .bind(allowed)
            .bind(format!("bbbb0000-0000-4000-8000-00000000000{allowed}"))
            .execute(&pool)
            .await
            .expect("вставка");
        }
        pool
    }

    /// Новый сниппет закрыт для агента, пока владелец не откроет.
    ///
    /// Умолчание здесь решает всё: в сниппетах лежат пароли прямо в командах,
    /// и «видно по умолчанию» означало бы, что после обновления приложения
    /// агент получил их все разом.
    #[tokio::test]
    async fn a_new_snippet_is_closed_to_the_agent() {
        let pool = db_with_snippets().await;
        sqlx::query("INSERT INTO snippets (name, command, sync_id) VALUES ('новый', 'ls', 'c1')")
            .execute(&pool)
            .await
            .expect("вставка");

        let open: Vec<(String,)> =
            sqlx::query_as("SELECT name FROM snippets WHERE agent_access = 1")
                .fetch_all(&pool)
                .await
                .expect("выборка");

        assert_eq!(open.len(), 1);
        assert_eq!(open[0].0, "открытый");
    }

    /// Закрытый сниппет не виден агенту и по имени тоже.
    ///
    /// Список и запуск отбирают по одному признаку, но отдельными запросами:
    /// забыть условие в одном из них — значит выдать по имени то, что не
    /// показано в списке.
    #[tokio::test]
    async fn a_closed_snippet_is_not_reachable_by_name() {
        let pool = db_with_snippets().await;

        let found: Option<(String, Option<String>)> = sqlx::query_as(
            "SELECT command, description FROM snippets WHERE name = ?1 AND agent_access = 1",
        )
        .bind("закрытый")
        .fetch_optional(&pool)
        .await
        .expect("выборка");

        assert!(found.is_none(), "закрытый сниппет достали по имени");
    }
}

#[cfg(test)]
mod patch_tests {
    use super::*;

    fn edit(find: &str, replace: &str) -> Edit {
        Edit {
            find: find.to_string(),
            replace: replace.to_string(),
        }
    }

    const CONF: &str = "server {\n    listen 80;\n    root /srv/app;\n}\n";

    #[test]
    fn a_single_match_is_replaced() {
        let after = apply_edits(CONF, &[edit("listen 80;", "listen 8080;")]).expect("замена");
        assert_eq!(after, "server {\n    listen 8080;\n    root /srv/app;\n}\n");
    }

    /// Главная проверка всего инструмента.
    ///
    /// «Возьмём первое совпадение» — самое естественное поведение и самое
    /// вредное: в конфиге с двумя одинаковыми строками агент почти наверняка
    /// имел в виду вторую, и правка молча ушла бы не туда.
    #[test]
    fn two_matches_are_refused_rather_than_guessed() {
        let text = "listen 80;\nlisten 80;\n";
        let err = apply_edits(text, &[edit("listen 80;", "listen 8080;")]).expect_err("отказ");
        assert!(err.contains("2 раз"), "{err}");
        // И текст остался нетронутым: отказ — это отказ целиком.
        assert_eq!(text, "listen 80;\nlisten 80;\n");
    }

    #[test]
    fn a_missing_fragment_is_refused() {
        let err = apply_edits(CONF, &[edit("listen 443;", "listen 8443;")]).expect_err("отказ");
        assert!(err.contains("нет такого фрагмента"), "{err}");
    }

    /// Замены идут по порядку, и каждая считает совпадения заново: первая
    /// может создать второе вхождение того, что ищет вторая.
    #[test]
    fn edits_are_counted_against_the_text_as_it_becomes() {
        let err = apply_edits("a\nb\n", &[edit("b", "a"), edit("a", "c")]).expect_err("отказ");
        assert!(err.contains("замена 2"), "{err}");
        assert!(err.contains("2 раз"), "{err}");
    }

    /// Пустая замена — удаление фрагмента, а не ошибка в запросе.
    #[test]
    fn an_empty_replacement_deletes() {
        let after = apply_edits(CONF, &[edit("    root /srv/app;\n", "")]).expect("замена");
        assert_eq!(after, "server {\n    listen 80;\n}\n");
    }

    #[test]
    fn edits_must_say_what_to_find_and_what_to_put() {
        assert!(parse_edits(&serde_json::json!({})).is_err());
        assert!(parse_edits(&serde_json::json!({ "edits": [] })).is_err());
        assert!(parse_edits(&serde_json::json!({ "edits": [{ "replace": "x" }] })).is_err());
        // Отсутствие replace — ошибка, пустая строка — нет.
        assert!(parse_edits(&serde_json::json!({ "edits": [{ "find": "x" }] })).is_err());
        let ok = parse_edits(&serde_json::json!({ "edits": [{ "find": "x", "replace": "" }] }))
            .expect("разбор");
        assert_eq!(ok.len(), 1);
        assert_eq!(ok[0].replace, "");
    }

    /// Отказ по конфигу на сорок строк не должен быть отказом на сорок строк.
    #[test]
    fn the_refusal_shows_a_short_excerpt() {
        let long = "x".repeat(200);
        let err = apply_edits("пусто", &[edit(&long, "y")]).expect_err("отказ");
        assert!(err.chars().count() < 200, "выдержка не сократилась: {err}");
        assert!(err.contains('…'), "{err}");
        // Перевод строки не рвёт сообщение на части.
        assert_eq!(short("a\nb"), "a⏎b");
    }
}

#[cfg(test)]
mod summary_tests {
    use super::*;

    /// Каждая команда сводки должна проходить обычную проверку читающих
    /// команд.
    ///
    /// Смысл теста: сводка не может быть обходом проверки. Если однажды сюда
    /// допишут удобную команду, которую агент через `darwaza_run` выполнить
    /// не вправе, тест упадёт — а не откроет её задним ходом.
    #[test]
    fn every_summary_command_is_a_reading_command() {
        for (name, command) in SUMMARY_PARTS {
            assert!(
                safety::is_read_only(command).is_ok(),
                "часть '{name}' выполняет то, что агенту нельзя: {command}"
            );
        }
    }

    /// Сводка не должна требовать прав: иначе она работает не везде и молча
    /// показывает меньше, чем обещает.
    #[test]
    fn the_summary_never_asks_for_sudo() {
        for (name, command) in SUMMARY_PARTS {
            assert!(
                safety::strip_sudo(command).is_none(),
                "часть '{name}' идёт через sudo: {command}"
            );
        }
    }

    /// Обрезка не должна рвать многобайтный символ пополам.
    #[test]
    fn clipping_keeps_characters_whole() {
        // Каждая буква по два байта, предел приходится на середину третьей.
        let (text, truncated) = clip("абвгд", 5);
        assert!(truncated);
        assert_eq!(text, "аб");

        let (text, truncated) = clip("  короткое  ", 100);
        assert!(!truncated);
        assert_eq!(text, "  короткое");
    }
}

#[cfg(test)]
mod journal_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn db_with_log() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("память");
        crate::db::MIGRATOR.run(&pool).await.expect("миграции");

        // Пять записей по одному хосту и одна по чужому — последняя не должна
        // попасть в выборку ни при каких условиях.
        for i in 1..=5 {
            log_action(
                &pool,
                Some("наш"),
                "run",
                Some(&format!("ls -la /srv/{i}")),
                if i == 3 { "refused" } else { "ok" },
                Some(0),
                None,
            )
            .await;
        }
        log_action(
            &pool,
            Some("чужой"),
            "run",
            Some("secret"),
            "ok",
            None,
            None,
        )
        .await;
        pool
    }

    /// Предел отрезает СТАРОЕ, а не свежее.
    ///
    /// Легко написать наоборот — `ORDER BY id LIMIT 2` выглядит так же
    /// безобидно и вернёт первые действия недельной давности вместо
    /// вчерашних. Агенту нужны последние.
    #[tokio::test]
    async fn the_limit_keeps_the_newest_entries() {
        let pool = db_with_log().await;
        let entries = journal_entries(&pool, "наш", 2).await.expect("выборка");

        assert_eq!(entries.len(), 2);
        // От старой к новой, и обе — из последних.
        assert_eq!(entries[0].detail.as_deref(), Some("ls -la /srv/4"));
        assert_eq!(entries[1].detail.as_deref(), Some("ls -la /srv/5"));
    }

    /// Журнал чужого хоста не виден: инструмент проверяет доступ по имени, и
    /// выборка обязана держаться того же имени.
    #[tokio::test]
    async fn entries_of_another_host_are_not_returned() {
        let pool = db_with_log().await;
        let entries = journal_entries(&pool, "наш", 50).await.expect("выборка");

        assert_eq!(entries.len(), 5);
        assert!(
            entries
                .iter()
                .all(|e| e.detail.as_deref() != Some("secret")),
            "в журнале хоста оказалась запись чужого"
        );
    }

    /// Отказы показываются наравне с успехами — ради них половина смысла.
    #[tokio::test]
    async fn refusals_are_kept() {
        let pool = db_with_log().await;
        let entries = journal_entries(&pool, "наш", 50).await.expect("выборка");

        assert_eq!(
            entries.iter().filter(|e| e.outcome == "refused").count(),
            1,
            "отказ пропал из журнала"
        );
    }
}

#[cfg(test)]
mod service_tests {
    use super::*;

    /// Запуск и перезапуск считаются удачными, только если служба работает.
    ///
    /// `systemctl restart` возвращает ноль, отправив сигнал. Служба, упавшая
    /// через секунду, даёт тот же ноль — и без этой проверки агент отчитался
    /// бы об успешной выкатке на лежащем сервере.
    #[test]
    fn a_started_service_must_actually_be_running() {
        for action in ["start", "restart", "reload", "try-reload-or-restart"] {
            assert!(service_is_healthy(action, "active"), "{action}");
            assert!(!service_is_healthy(action, "failed"), "{action}");
            assert!(!service_is_healthy(action, "inactive"), "{action}");
            assert!(!service_is_healthy(action, "activating"), "{action}");
            // Состояние не удалось узнать — это не успех.
            assert!(!service_is_healthy(action, "неизвестно"), "{action}");
        }
    }

    /// После остановки «упала» — тоже остановлена.
    #[test]
    fn a_stopped_service_may_be_inactive_or_failed() {
        assert!(service_is_healthy("stop", "inactive"));
        assert!(service_is_healthy("stop", "failed"));
        assert!(!service_is_healthy("stop", "active"));
    }

    /// Список действий закрытый: `enable` и `mask` меняют поведение при
    /// загрузке, а проверить их тем же способом нельзя.
    #[test]
    fn boot_time_actions_are_not_in_the_list() {
        for action in ["enable", "disable", "mask", "unmask", "edit"] {
            assert!(!SERVICE_ACTIONS.contains(&action), "{action}");
        }
        for action in ["start", "stop", "restart", "reload"] {
            assert!(SERVICE_ACTIONS.contains(&action), "{action}");
        }
    }
}
