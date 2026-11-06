use serde::Serialize;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

/// Команды, которые UI шлёт активной сессии (через mpsc).
#[derive(Debug)]
pub enum SessionCmd {
    /// Послать данные в stdin SSH-канала.
    Send(Vec<u8>),
    /// Изменить размер PTY.
    Resize { cols: u32, rows: u32 },
    /// Корректно закрыть сессию.
    Disconnect,
    /// Открыть дополнительный канал с подсистемой `sftp` (#120).
    ///
    /// Идёт через тот же mpsc, потому что `russh::client::Handle` не `Clone` и
    /// живёт внутри задачи сессии — снаружи к нему не достучаться. Задача
    /// открывает канал и отдаёт его обратно по oneshot; SFTP-рукопожатие и
    /// дальнейшие операции происходят уже вне цикла терминала, чтобы не
    /// блокировать ввод-вывод шелла.
    OpenSftp {
        responder: oneshot::Sender<Result<russh::Channel<russh::client::Msg>, String>>,
    },
    /// Выполнить одну команду и вернуть результат (#200, доступ агента).
    ///
    /// Отдельно от терминала намеренно. Интерактивный шелл отдаёт поток
    /// вперемешку с приглашениями и управляющими последовательностями —
    /// разобрать оттуда «где закончился вывод и каков код возврата» можно
    /// только угадыванием. Отдельный канал с `exec` даёт и то, и другое
    /// точно, потому что так устроен сам протокол.
    Exec {
        command: String,
        /// Сколько ждать эту команду и что делать, если не дождались.
        ///
        /// Своё значение на каждый вызов, а не общая константа: читающей
        /// команде хватает секунд, а шагу плана — `docker build`, миграции,
        /// `apt upgrade` — нужны минуты. Общий предел в 30 секунд обрывал
        /// такой шаг **посреди изменения** (#213).
        limit: ExecLimit,
        responder: oneshot::Sender<Result<ExecResult, String>>,
    },
    /// Открыть `direct-tcpip` канал до `host:port` со стороны сервера (#130).
    ///
    /// Как и `OpenSftp`, идёт через mpsc, потому что `Handle` живёт внутри
    /// задачи сессии. Каждое новое соединение к проброшенному порту просит
    /// отдельный канал.
    OpenDirectTcpip {
        host: String,
        port: u32,
        originator_host: String,
        originator_port: u32,
        responder: oneshot::Sender<Result<russh::Channel<russh::client::Msg>, String>>,
    },
    /// Попросить сервер слушать порт у себя — remote-проброс, `ssh -R` (#130a).
    ///
    /// Возвращает фактический порт: при запросе нулевого его выбирает сервер.
    RequestRemoteForward {
        address: String,
        port: u32,
        responder: oneshot::Sender<Result<u32, String>>,
    },
    /// Снять remote-проброс.
    CancelRemoteForward { address: String, port: u32 },
    /// Начать запись сессии в файл (#144). Возвращает путь к файлу.
    ///
    /// Идёт через ту же очередь, что и остальное: писать умеет только задача
    /// сессии — именно она видит поток вывода.
    StartRecording {
        responder: oneshot::Sender<Result<String, String>>,
    },
    /// Остановить запись. Возвращает путь к закрытому файлу.
    StopRecording {
        responder: oneshot::Sender<Result<String, String>>,
    },
    /// Измерить задержку до сервера, миллисекунды (#229).
    ///
    /// Мерим открытием и закрытием пустого канала: подтверждение приходит
    /// от сервера, то есть это настоящий круг туда и обратно. `send_keepalive`
    /// для этого не годится — он только отправляет запрос и ответа не ждёт.
    Latency {
        responder: oneshot::Sender<Result<u64, String>>,
    },
}

/// Статус, который мы отдаём UI событием `session:status`.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SessionStatus {
    Connecting,
    Authenticating,
    PtyOpening,
    Connected,
    Disconnecting,
    Closed,
    Failed,
}

/// События в сторону UI. Сериализуются как Tauri-payload по тегу `event`.
#[derive(Debug, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum SessionEvent {
    #[serde(rename_all = "camelCase")]
    Status {
        session_id: Uuid,
        status: SessionStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Data { session_id: Uuid, chunk: Vec<u8> },
    #[serde(rename_all = "camelCase")]
    Closed {
        session_id: Uuid,
        #[serde(skip_serializing_if = "Option::is_none")]
        exit_code: Option<u32>,
    },
}

/// Хэндл сессии, хранится в `SshManager`.
pub struct SessionHandle {
    pub host_id: i64,
    /// Соединение только для файлов (экран SFTP): без терминала и оболочки.
    /// Во вкладки и в список «активных сессий» не попадает.
    pub files_only: bool,
    pub status: SessionStatus,
    pub cmd_tx: mpsc::Sender<SessionCmd>,
}

/// Результат одиночного выполнения команды.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecResult {
    pub stdout: String,
    pub stderr: String,
    /// Код возврата. `None` — команду прервал сигнал, а не завершение.
    pub exit_code: Option<u32>,
    /// Вывод обрезан по пределу: команда напечатала больше, чем разумно
    /// передавать целиком.
    pub truncated: bool,
    /// Команду остановили по времени, а не она завершилась сама (#218).
    ///
    /// Для наблюдения за выводом это нормальный исход, и отличать его надо:
    /// без этой отметки `exit_code: null` читается как «что-то пошло не так».
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub stopped_by_time: bool,
}

/// Что делать, если команда не уложилась в отведённое время.
///
/// Разница принципиальная. Обычную команду, которая не закончилась, нельзя
/// считать выполненной: её результат неизвестен, и отдавать половину вывода
/// как ответ — значит соврать. А наблюдение за выводом (#218) и **не должно**
/// заканчиваться само: `journalctl -f` смотрят сколько нужно и отпускают, и
/// собранное за это время — как раз то, за чем смотрели.
#[derive(Debug, Clone, Copy)]
pub enum ExecLimit {
    /// Не уложилась — ошибка.
    Fail(std::time::Duration),
    /// Не уложилась — отдать собранное.
    Collect(std::time::Duration),
}

impl ExecLimit {
    pub fn duration(&self) -> std::time::Duration {
        match self {
            ExecLimit::Fail(d) | ExecLimit::Collect(d) => *d,
        }
    }
}
