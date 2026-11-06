//! Просмотр журналов из настроек.
//!
//! Ошибки обещают «подробности записаны в журнал», а действия помощника пишутся
//! ради доверия (#204). И то и другое бесполезно, если человеку негде это увидеть.

use crate::AppState;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{FromRow, SqlitePool};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

fn map_err<E: std::fmt::Display>(e: E) -> String {
    crate::errors::opaque("logs", e)
}

#[derive(Debug, Clone, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentLogEntry {
    pub id: i64,
    pub at: DateTime<Utc>,
    pub host: Option<String>,
    pub action: String,
    pub detail: Option<String>,
    pub outcome: String,
    pub exit_code: Option<i64>,
    pub note: Option<String>,
}

/// Последние действия по всем хостам, новые сверху. Порядок по `id`: время
/// пишется с точностью до секунды.
async fn agent_log_recent(
    pool: &SqlitePool,
    limit: i64,
) -> Result<Vec<AgentLogEntry>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, at, host, action, detail, outcome, exit_code, note FROM agent_log \
         ORDER BY id DESC LIMIT ?1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}

#[tauri::command]
pub async fn agent_log_list(
    state: State<'_, AppState>,
    limit: Option<i64>,
) -> Result<Vec<AgentLogEntry>, String> {
    agent_log_recent(&state.db, limit.unwrap_or(200).clamp(1, 1000))
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn logs_dir() -> Result<String, String> {
    crate::log_dir()
        .map(|p| p.to_string_lossy().to_string())
        .ok_or_else(|| map_err("каталог журналов недоступен"))
}

/// Путь берётся здесь, а не с фронтенда: команда открывает только этот каталог.
#[tauri::command]
pub async fn logs_open_dir(app: AppHandle) -> Result<(), String> {
    let dir = crate::log_dir().ok_or_else(|| map_err("каталог журналов недоступен"))?;
    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(map_err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    /// Отказы видны наравне с выполненным, свежее сверху, лимит режет старое.
    #[tokio::test]
    async fn newest_first_refusals_included_and_limit_drops_oldest() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("память");
        crate::db::MIGRATOR.run(&pool).await.expect("миграции");

        for (action, outcome) in [("run", "ok"), ("read", "failed"), ("run", "refused")] {
            sqlx::query(
                "INSERT INTO agent_log (host, action, detail, outcome) VALUES ('db', ?1, 'x', ?2)",
            )
            .bind(action)
            .bind(outcome)
            .execute(&pool)
            .await
            .expect("запись");
        }

        let rows = agent_log_recent(&pool, 2).await.expect("чтение");
        let outcomes: Vec<_> = rows.iter().map(|r| r.outcome.as_str()).collect();
        assert_eq!(outcomes, ["refused", "failed"]);
    }
}
