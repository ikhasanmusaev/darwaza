use crate::AppState;
use std::collections::HashMap;
use tauri::State;

/// Ошибка наружу уходит кодом, причина — в журнал (#177).
fn map_err<E: std::fmt::Display>(e: E) -> String {
    crate::errors::opaque("settings", e)
}

/// Ключи, которые окно вправе записывать и удалять. Остальные (ключи шифрования
/// `security.*`, адреса синхронизации и отчётов о сбоях, `app.onboarded`) ведут
/// свои команды и не должны переписываться любым вызовом из WebView. Нужен новый
/// ключ в интерфейсе — добавьте его сюда и в `tests` ниже.
const WRITABLE_KEYS: &[&str] = &[
    "terminal.font.family",
    "terminal.font.size",
    "terminal.theme",
    "ssh.default_keep_alive",
    "app.theme",
    "session.restore_tabs",
    "session.open_hosts",
    "sync.auto",
    crate::commands::probe::PROBE_KEY,
    crate::attention::SETTING_KEY,
];

fn check_writable(key: &str) -> Result<(), String> {
    if WRITABLE_KEYS.contains(&key) {
        Ok(())
    } else {
        tracing::warn!(target: "settings", "запись ключа вне списка отклонена: {key}");
        Err(crate::errors::codes::SETTING_NOT_ALLOWED.to_string())
    }
}

#[tauri::command]
pub async fn settings_get(
    state: State<'_, AppState>,
    key: String,
) -> Result<Option<String>, String> {
    sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?1")
        .bind(&key)
        .fetch_optional(&state.db)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn settings_set(
    state: State<'_, AppState>,
    key: String,
    value: String,
) -> Result<(), String> {
    check_writable(&key)?;
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(&key)
    .bind(&value)
    .execute(&state.db)
    .await
    .map_err(map_err)?;
    Ok(())
}

#[tauri::command]
pub async fn settings_remove(state: State<'_, AppState>, key: String) -> Result<(), String> {
    check_writable(&key)?;
    sqlx::query("DELETE FROM settings WHERE key = ?1")
        .bind(&key)
        .execute(&state.db)
        .await
        .map_err(map_err)?;
    Ok(())
}

#[tauri::command]
pub async fn settings_all(state: State<'_, AppState>) -> Result<HashMap<String, String>, String> {
    let rows = sqlx::query_as::<_, (String, String)>("SELECT key, value FROM settings")
        .fetch_all(&state.db)
        .await
        .map_err(map_err)?;
    Ok(rows.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interface_keys_are_writable() {
        for key in ["app.theme", "hosts.probe", "notify.decision", "sync.auto", "session.open_hosts"] {
            assert!(check_writable(key).is_ok(), "{key}");
        }
    }

    #[test]
    fn keys_of_other_commands_are_refused() {
        for key in [
            "security.master_data_key_pw",
            "sync.endpoint",
            "diagnostics.crash_endpoint",
            "app.onboarded",
            "",
        ] {
            assert_eq!(
                check_writable(key).unwrap_err(),
                crate::errors::codes::SETTING_NOT_ALLOWED,
                "{key}"
            );
        }
    }
}
