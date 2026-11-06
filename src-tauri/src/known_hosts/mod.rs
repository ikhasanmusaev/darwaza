// TOFU + работа с таблицей known_hosts.
// См. docs/01-architecture/ssh-flow.md (раздел TOFU)

pub mod import;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownHost {
    pub id: i64,
    pub hostname: String,
    pub key_type: String,
    pub fingerprint: String,
    pub accepted_at: DateTime<Utc>,
}

const FIELDS: &str = "id, hostname, key_type, fingerprint, accepted_at";

/// Найти запись по hostname (нормализованному до "host:port") и алгоритму ключа.
pub async fn find(
    pool: &SqlitePool,
    hostname: &str,
    key_type: &str,
) -> Result<Option<KnownHost>, sqlx::Error> {
    let sql = format!("SELECT {FIELDS} FROM known_hosts WHERE hostname = ?1 AND key_type = ?2");
    sqlx::query_as::<_, KnownHost>(&sql)
        .bind(hostname)
        .bind(key_type)
        .fetch_optional(pool)
        .await
}

/// Принять/обновить fingerprint. Upsert по уникальному индексу (hostname, key_type).
pub async fn accept(
    pool: &SqlitePool,
    hostname: &str,
    key_type: &str,
    fingerprint: &str,
) -> Result<KnownHost, sqlx::Error> {
    let sql = format!(
        "INSERT INTO known_hosts (hostname, key_type, fingerprint, sync_id) \
         VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(hostname, key_type) DO UPDATE \
            SET fingerprint = excluded.fingerprint, accepted_at = CURRENT_TIMESTAMP \
         RETURNING {FIELDS}"
    );
    sqlx::query_as::<_, KnownHost>(&sql)
        .bind(hostname)
        .bind(key_type)
        .bind(fingerprint)
        .bind(crate::db::new_sync_id())
        .fetch_one(pool)
        .await
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<KnownHost>, sqlx::Error> {
    let sql = format!("SELECT {FIELDS} FROM known_hosts ORDER BY hostname, key_type");
    sqlx::query_as::<_, KnownHost>(&sql).fetch_all(pool).await
}

pub async fn remove(pool: &SqlitePool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM known_hosts WHERE id = ?1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn clear(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM known_hosts").execute(pool).await?;
    Ok(())
}
