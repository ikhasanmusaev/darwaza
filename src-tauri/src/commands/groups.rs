use crate::db::models::{Group, GroupInput};
use crate::AppState;
use tauri::State;

/// Ошибка базы при чтении: занятая пара и мёртвая ссылка разбираются
/// отдельно, остальное — «не удалось прочитать» (#177).
fn load_err(e: sqlx::Error) -> String {
    crate::errors::db_load("groups", e)
}

/// То же при записи.
fn save_err(e: sqlx::Error) -> String {
    crate::errors::db_save("groups", e)
}

#[tauri::command]
pub async fn groups_list(state: State<'_, AppState>) -> Result<Vec<Group>, String> {
    sqlx::query_as::<_, Group>(
        "SELECT id, name, parent_id, sort_order, created_at, updated_at \
         FROM groups ORDER BY sort_order, name",
    )
    .fetch_all(&state.db)
    .await
    .map_err(load_err)
}

#[tauri::command]
pub async fn groups_create(state: State<'_, AppState>, data: GroupInput) -> Result<Group, String> {
    let sort_order = data.sort_order.unwrap_or(0);
    let row = sqlx::query_as::<_, Group>(
        "INSERT INTO groups (name, parent_id, sort_order, sync_id) \
         VALUES (?1, ?2, ?3, ?4) \
         RETURNING id, name, parent_id, sort_order, created_at, updated_at",
    )
    .bind(&data.name)
    .bind(data.parent_id)
    .bind(sort_order)
    .bind(crate::db::new_sync_id())
    .fetch_one(&state.db)
    .await
    .map_err(save_err)?;
    Ok(row)
}

#[tauri::command]
pub async fn groups_update(
    state: State<'_, AppState>,
    id: i64,
    data: GroupInput,
) -> Result<Group, String> {
    let row = sqlx::query_as::<_, Group>(
        "UPDATE groups \
         SET name = ?2, parent_id = ?3, sort_order = COALESCE(?4, sort_order), \
             updated_at = CURRENT_TIMESTAMP \
         WHERE id = ?1 \
         RETURNING id, name, parent_id, sort_order, created_at, updated_at",
    )
    .bind(id)
    .bind(&data.name)
    .bind(data.parent_id)
    .bind(data.sort_order)
    .fetch_optional(&state.db)
    .await
    .map_err(save_err)?
    .ok_or_else(|| crate::errors::not_found("groups", "группа", id))?;
    Ok(row)
}

#[tauri::command]
pub async fn groups_remove(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    sqlx::query("DELETE FROM groups WHERE id = ?1")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(save_err)?;
    Ok(())
}
