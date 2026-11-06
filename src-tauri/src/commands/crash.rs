//! Команды отчётов о сбоях (#171).

use crate::crash::{self, CrashReport};
use crate::AppState;
use tauri::State;
use tracing::info;

/// Ошибка наружу уходит кодом, причина — в журнал (#177).
fn map_err<E: std::fmt::Display>(e: E) -> String {
    crate::errors::passthrough("crash", e)
}

#[tauri::command]
pub async fn crash_reports_enabled(state: State<'_, AppState>) -> Result<bool, String> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = ?1")
        .bind(crash::SETTING_KEY)
        .fetch_optional(&state.db)
        .await
        .map_err(map_err)?;
    Ok(row.map(|r| r.0).as_deref() == Some("true"))
}

/// Включить или выключить сбор. Флаг дублируется в память: panic-хук читает
/// именно её — лезть в БД из паники нельзя, там уже может быть всё сломано.
#[tauri::command]
pub async fn crash_reports_set_enabled(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(crash::SETTING_KEY)
    .bind(if enabled { "true" } else { "false" })
    .execute(&state.db)
    .await
    .map_err(map_err)?;
    crash::set_enabled(enabled);
    Ok(())
}

#[tauri::command]
pub async fn crash_reports_list() -> Result<Vec<CrashReport>, String> {
    crash::list().await.map_err(map_err)
}

#[tauri::command]
pub async fn crash_report_read(path: String) -> Result<String, String> {
    crash::read(&path).await.map_err(map_err)
}

#[tauri::command]
pub async fn crash_report_remove(path: String) -> Result<(), String> {
    crash::remove(&path).await.map_err(map_err)
}

#[tauri::command]
pub async fn crash_reports_dir() -> Result<String, String> {
    crash::crashes_dir()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(map_err)
}

// ────────────────────────────────────────────────────────────────────
// Отправка и сохранение отчёта (#171a)
// ────────────────────────────────────────────────────────────────────

async fn endpoint_of(db: &sqlx::SqlitePool) -> Result<Option<String>, String> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = ?1")
        .bind(crash::ENDPOINT_KEY)
        .fetch_optional(db)
        .await
        .map_err(map_err)?;
    Ok(row.map(|r| r.0).filter(|v| !v.trim().is_empty()))
}

#[tauri::command]
pub async fn crash_endpoint_get(state: State<'_, AppState>) -> Result<Option<String>, String> {
    endpoint_of(&state.db).await
}

#[tauri::command]
pub async fn crash_endpoint_set(state: State<'_, AppState>, url: String) -> Result<(), String> {
    let trimmed = url.trim();
    crash::validate_endpoint(trimmed).map_err(map_err)?;
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(crash::ENDPOINT_KEY)
    .bind(trimmed)
    .execute(&state.db)
    .await
    .map_err(map_err)?;
    Ok(())
}

/// Отправить отчёт на настроенный адрес. Только по явному нажатию.
#[tauri::command]
pub async fn crash_report_send(state: State<'_, AppState>, path: String) -> Result<u16, String> {
    let endpoint = endpoint_of(&state.db)
        .await?
        .ok_or("адрес для отправки не задан")?;
    let body = crash::read(&path).await.map_err(map_err)?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(crash::SEND_TIMEOUT_SECS))
        .build()
        .map_err(map_err)?;

    let response = client
        .post(&endpoint)
        .header("content-type", "text/plain; charset=utf-8")
        .body(body)
        .send()
        .await
        .map_err(|e| format!("не удалось отправить: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("сервер ответил {status}"));
    }
    info!(target: "crash", "отчёт отправлен, ответ {status}");
    Ok(status.as_u16())
}

/// Сохранить копию отчёта в каталог загрузок — чтобы приложить к письму.
#[tauri::command]
pub async fn crash_report_export(
    state: State<'_, AppState>,
    path: String,
) -> Result<String, String> {
    let text = crash::read(&path).await.map_err(map_err)?;
    let dir = super::transfers::download_dir_for(&state.db).await?;
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("не удалось создать '{dir}': {e}"))?;

    let name = std::path::Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "crash.txt".to_string());
    let target = super::transfers::unique_path_in(std::path::Path::new(&dir), &name);

    tokio::fs::write(&target, text)
        .await
        .map_err(|e| format!("не удалось записать '{}': {e}", target.display()))?;
    Ok(target.to_string_lossy().to_string())
}
