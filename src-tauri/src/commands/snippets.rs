use crate::db::models::{Snippet, SnippetInput};
use crate::AppState;
use tauri::State;

const SELECT_FIELDS: &str =
    "id, name, command, description, sort_order, agent_access, created_at, updated_at";

/// Ошибка базы при чтении: занятая пара и мёртвая ссылка разбираются
/// отдельно, остальное — «не удалось прочитать» (#177).
fn load_err(e: sqlx::Error) -> String {
    crate::errors::db_load("snippets", e)
}

/// То же при записи.
fn save_err(e: sqlx::Error) -> String {
    crate::errors::db_save("snippets", e)
}

/// Пустое имя/команда делают сниппет бесполезным и ломают UI-список —
/// отсекаем на входе, а не молча пишем в БД.
fn validate(data: &SnippetInput) -> Result<(), String> {
    if data.name.trim().is_empty() {
        return Err("snippet name must not be empty".into());
    }
    if data.command.trim().is_empty() {
        return Err("snippet command must not be empty".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn snippets_list(state: State<'_, AppState>) -> Result<Vec<Snippet>, String> {
    let sql = format!("SELECT {SELECT_FIELDS} FROM snippets ORDER BY sort_order, name");
    sqlx::query_as::<_, Snippet>(&sql)
        .fetch_all(&state.db)
        .await
        .map_err(load_err)
}

#[tauri::command]
pub async fn snippets_get(state: State<'_, AppState>, id: i64) -> Result<Snippet, String> {
    let sql = format!("SELECT {SELECT_FIELDS} FROM snippets WHERE id = ?1");
    sqlx::query_as::<_, Snippet>(&sql)
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(load_err)?
        .ok_or_else(|| crate::errors::not_found("snippets", "сниппет", id))
}

#[tauri::command]
pub async fn snippets_create(
    state: State<'_, AppState>,
    data: SnippetInput,
) -> Result<Snippet, String> {
    validate(&data)?;

    let sql = format!(
        "INSERT INTO snippets (name, command, description, sort_order, agent_access, sync_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         RETURNING {SELECT_FIELDS}"
    );

    sqlx::query_as::<_, Snippet>(&sql)
        .bind(data.name.trim())
        .bind(&data.command)
        .bind(&data.description)
        .bind(data.sort_order.unwrap_or(0))
        .bind(data.agent_access)
        .bind(crate::db::new_sync_id())
        .fetch_one(&state.db)
        .await
        .map_err(save_err)
}

#[tauri::command]
pub async fn snippets_update(
    state: State<'_, AppState>,
    id: i64,
    data: SnippetInput,
) -> Result<Snippet, String> {
    validate(&data)?;

    let sql = format!(
        "UPDATE snippets SET \
            name = ?2, command = ?3, description = ?4, \
            sort_order = COALESCE(?5, sort_order), \
            agent_access = ?6, \
            updated_at = CURRENT_TIMESTAMP \
         WHERE id = ?1 \
         RETURNING {SELECT_FIELDS}"
    );

    sqlx::query_as::<_, Snippet>(&sql)
        .bind(id)
        .bind(data.name.trim())
        .bind(&data.command)
        .bind(&data.description)
        .bind(data.sort_order)
        .bind(data.agent_access)
        .fetch_optional(&state.db)
        .await
        .map_err(save_err)?
        .ok_or_else(|| crate::errors::not_found("snippets", "сниппет", id))
}

#[tauri::command]
pub async fn snippets_remove(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    sqlx::query("DELETE FROM snippets WHERE id = ?1")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(save_err)?;
    Ok(())
}
