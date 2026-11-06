use crate::connection_log::{self, ConnectionLogEntry};
use crate::AppState;
use tauri::State;

/// Ошибка наружу уходит кодом, причина — в журнал (#177).
fn map_err<E: std::fmt::Display>(e: E) -> String {
    crate::errors::opaque("connection_logs", e)
}

#[tauri::command]
pub async fn connection_logs_list(
    state: State<'_, AppState>,
    host_id: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<ConnectionLogEntry>, String> {
    let lim = limit.unwrap_or(100);
    match host_id {
        Some(hid) => connection_log::list_by_host(&state.db, hid, lim)
            .await
            .map_err(map_err),
        None => connection_log::list(&state.db, lim).await.map_err(map_err),
    }
}
