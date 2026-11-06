//! Команды записи сессий (#144).

use crate::recording::{self, RecordingChunk, RecordingFile};
use crate::ssh::session::SessionCmd;
use crate::AppState;
use tauri::State;
use tokio::sync::oneshot;
use uuid::Uuid;

/// Ошибка наружу уходит кодом, причина — в журнал (#177).
fn map_err<E: std::fmt::Display>(e: E) -> String {
    crate::errors::opaque("recordings", e)
}

/// Начать запись активной сессии. Возвращает путь к файлу.
#[tauri::command]
pub async fn session_recording_start(
    state: State<'_, AppState>,
    session_id: Uuid,
) -> Result<String, String> {
    let (tx, rx) = oneshot::channel();
    state
        .ssh
        .send_cmd(session_id, SessionCmd::StartRecording { responder: tx })
        .await?;
    rx.await
        .map_err(|_| format!("сессия {session_id} закрылась при старте записи"))?
}

/// Остановить запись. Возвращает путь к закрытому файлу.
#[tauri::command]
pub async fn session_recording_stop(
    state: State<'_, AppState>,
    session_id: Uuid,
) -> Result<String, String> {
    let (tx, rx) = oneshot::channel();
    state
        .ssh
        .send_cmd(session_id, SessionCmd::StopRecording { responder: tx })
        .await?;
    rx.await
        .map_err(|_| format!("сессия {session_id} закрылась при остановке записи"))?
}

#[tauri::command]
pub async fn recordings_list() -> Result<Vec<RecordingFile>, String> {
    recording::list().await.map_err(map_err)
}

#[tauri::command]
pub async fn recordings_remove(path: String) -> Result<(), String> {
    recording::remove(&path).await.map_err(map_err)
}

/// Прочитать кусок записи для проигрывателя. `offset` — байт, с которого
/// читать; первый вызов с нуля, дальше — `nextOffset` из ответа.
#[tauri::command]
pub async fn recording_read(
    path: String,
    offset: u64,
    max_bytes: Option<u64>,
) -> Result<RecordingChunk, String> {
    recording::read_chunk(&path, offset, max_bytes.unwrap_or(recording::CHUNK_MAX))
        .await
        .map_err(map_err)
}

/// Каталог с записями — чтобы UI мог показать его пользователю.
#[tauri::command]
pub async fn recordings_dir() -> Result<String, String> {
    recording::recordings_dir()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(map_err)
}
