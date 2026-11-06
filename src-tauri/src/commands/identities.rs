use crate::db::models::{Identity, IdentityInput};
use crate::keyring_store::kinds;
use crate::AppState;
use tauri::State;

const SELECT_FIELDS: &str = "id, label, kind, key_path, keyring_ref, has_passphrase, \
    has_totp, public_key, created_at, updated_at";

/// Ошибка базы при чтении: занятая пара и мёртвая ссылка разбираются
/// отдельно, остальное — «не удалось прочитать» (#177).
fn load_err(e: sqlx::Error) -> String {
    crate::errors::db_load("identities", e)
}

/// То же при записи.
fn save_err(e: sqlx::Error) -> String {
    crate::errors::db_save("identities", e)
}

fn validate_kind(value: &str) -> Result<(), String> {
    match value {
        "password" | "private_key" | "agent" => Ok(()),
        other => Err(crate::errors::opaque(
            "validate_kind",
            format!("недопустимый kind: {other}"),
        )),
    }
}

/// Запись списка: ключ и сколько хостов им пользуется (#236).
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityListItem {
    #[sqlx(flatten)]
    #[serde(flatten)]
    pub identity: Identity,
    /// Сколько хостов ссылаются на этот ключ. Удаление молча обнуляет у них
    /// `identity_id` (`ON DELETE SET NULL`), и без этого числа человек
    /// не узнал бы, скольких оно коснётся.
    pub used_by: i64,
}

pub async fn list_with_usage(db: &sqlx::SqlitePool) -> Result<Vec<IdentityListItem>, sqlx::Error> {
    let sql = format!(
        "SELECT {SELECT_FIELDS}, \
         (SELECT COUNT(*) FROM hosts WHERE hosts.identity_id = identities.id) AS used_by \
         FROM identities ORDER BY label"
    );
    sqlx::query_as::<_, IdentityListItem>(&sql)
        .fetch_all(db)
        .await
}

/// Имена хостов, которые останутся без входа, если удалить ключ.
pub async fn host_names_using(db: &sqlx::SqlitePool, id: i64) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT name FROM hosts WHERE identity_id = ?1 ORDER BY name")
        .bind(id)
        .fetch_all(db)
        .await
}

#[tauri::command]
pub async fn identities_list(state: State<'_, AppState>) -> Result<Vec<IdentityListItem>, String> {
    list_with_usage(&state.db).await.map_err(load_err)
}

/// Какие хосты пользуются ключом — показать перед удалением.
#[tauri::command]
pub async fn identities_used_by(
    state: State<'_, AppState>,
    id: i64,
) -> Result<Vec<String>, String> {
    host_names_using(&state.db, id).await.map_err(load_err)
}

#[tauri::command]
pub async fn identities_get(state: State<'_, AppState>, id: i64) -> Result<Identity, String> {
    let sql = format!("SELECT {SELECT_FIELDS} FROM identities WHERE id = ?1");
    sqlx::query_as::<_, Identity>(&sql)
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(load_err)?
        .ok_or_else(|| crate::errors::not_found("identities", "идентификация", id))
}

#[tauri::command]
pub async fn identities_create(
    state: State<'_, AppState>,
    data: IdentityInput,
) -> Result<Identity, String> {
    validate_kind(&data.kind)?;
    let has_passphrase = data.has_passphrase.unwrap_or(false);

    let sql = format!(
        "INSERT INTO identities (label, kind, key_path, keyring_ref, has_passphrase, public_key, sync_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
         RETURNING {SELECT_FIELDS}"
    );

    sqlx::query_as::<_, Identity>(&sql)
        .bind(&data.label)
        .bind(&data.kind)
        .bind(&data.key_path)
        .bind(&data.keyring_ref)
        .bind(has_passphrase)
        .bind(&data.public_key)
        .bind(crate::db::new_sync_id())
        .fetch_one(&state.db)
        .await
        .map_err(save_err)
}

#[tauri::command]
pub async fn identities_update(
    state: State<'_, AppState>,
    id: i64,
    data: IdentityInput,
) -> Result<Identity, String> {
    validate_kind(&data.kind)?;

    let sql = format!(
        "UPDATE identities SET \
            label = ?2, kind = ?3, key_path = ?4, keyring_ref = ?5, \
            has_passphrase = COALESCE(?6, has_passphrase), public_key = ?7, \
            updated_at = CURRENT_TIMESTAMP \
         WHERE id = ?1 \
         RETURNING {SELECT_FIELDS}"
    );

    sqlx::query_as::<_, Identity>(&sql)
        .bind(id)
        .bind(&data.label)
        .bind(&data.kind)
        .bind(&data.key_path)
        .bind(&data.keyring_ref)
        .bind(data.has_passphrase)
        .bind(&data.public_key)
        .fetch_optional(&state.db)
        .await
        .map_err(save_err)?
        .ok_or_else(|| crate::errors::not_found("identities", "идентификация", id))
}

#[tauri::command]
pub async fn identities_remove(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    // Удалить связанные секреты до удаления самой записи.
    // Чистим оба хранилища — см. `secrets::delete`.
    crate::secrets::delete(&state.db, &state.master, kinds::PASSWORD, id).await;
    crate::secrets::delete(&state.db, &state.master, kinds::PASSPHRASE, id).await;

    sqlx::query("DELETE FROM identities WHERE id = ?1")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(save_err)?;
    Ok(())
}

#[tauri::command]
pub async fn identity_set_password(
    state: State<'_, AppState>,
    id: i64,
    password: String,
) -> Result<(), String> {
    // Проверяем, что identity существует, чтобы не плодить orphan-записи в keyring.
    let exists = sqlx::query("SELECT 1 FROM identities WHERE id = ?1")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(save_err)?
        .is_some();
    if !exists {
        return Err(crate::errors::not_found("identities", "идентификация", id));
    }

    crate::secrets::set(&state.db, &state.master, kinds::PASSWORD, id, &password).await
}

#[tauri::command]
pub async fn identity_set_passphrase(
    state: State<'_, AppState>,
    id: i64,
    passphrase: String,
) -> Result<(), String> {
    let exists = sqlx::query("SELECT 1 FROM identities WHERE id = ?1")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(save_err)?
        .is_some();
    if !exists {
        return Err(crate::errors::not_found("identities", "идентификация", id));
    }
    crate::secrets::set(&state.db, &state.master, kinds::PASSPHRASE, id, &passphrase).await
}

// ВАЖНО: НЕТ команд `identity_get_password` / `identity_get_passphrase`.
// Чтение секретов выполняется ТОЛЬКО внутри Rust-backend (`ssh::resolve_auth`),
// чтобы не выставлять plaintext-доступ в WebView — это снижает attack surface
// на случай XSS / уязвимости в зависимости. См. code-review f7ea788..caee272.

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> sqlx::SqlitePool {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("память");
        crate::db::MIGRATOR.run(&db).await.expect("миграции");
        db
    }

    async fn host(db: &sqlx::SqlitePool, n: i64, name: &str, identity: Option<i64>) {
        sqlx::query(
            "INSERT INTO hosts (id, name, hostname, username, auth_method, identity_id, sync_id) \
             VALUES (?1, ?2, 'h', 'u', 'password', ?3, ?4)",
        )
        .bind(n)
        .bind(name)
        .bind(identity)
        .bind(format!("00000000-0000-4000-8000-00000000000{n}"))
        .execute(db)
        .await
        .expect("хост");
    }

    #[tokio::test]
    async fn the_list_counts_hosts_using_each_key_and_removal_would_detach_them() {
        let db = db().await;
        for (id, label) in [(1, "a"), (2, "b")] {
            sqlx::query(
                "INSERT INTO identities (id, label, kind, sync_id) VALUES (?1, ?2, 'password', ?3)",
            )
            .bind(id)
            .bind(label)
            .bind(format!("10000000-0000-4000-8000-00000000000{id}"))
            .execute(&db)
            .await
            .expect("ключ");
        }
        host(&db, 1, "web", Some(1)).await;
        host(&db, 2, "db", Some(1)).await;
        host(&db, 3, "nas", None).await;

        let list = list_with_usage(&db).await.expect("список");
        assert_eq!(list[0].identity.label, "a");
        assert_eq!(list[0].used_by, 2);
        assert_eq!(list[1].used_by, 0);

        assert_eq!(
            host_names_using(&db, 1).await.expect("имена"),
            ["db", "web"]
        );

        // То, о чём предупреждает число: удаление отвязывает хосты молча.
        sqlx::query("DELETE FROM identities WHERE id = 1")
            .execute(&db)
            .await
            .expect("удалён");
        let left: Option<i64> = sqlx::query_scalar("SELECT identity_id FROM hosts WHERE id = 1")
            .fetch_one(&db)
            .await
            .expect("хост");
        assert!(left.is_none());
    }
}
