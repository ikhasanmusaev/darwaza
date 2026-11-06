use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionLogEntry {
    pub id: i64,
    pub host_id: i64,
    pub session_id: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub status: String,
    pub error: Option<String>,
}

const FIELDS: &str = "id, host_id, session_id, started_at, ended_at, status, error";

pub async fn start(pool: &SqlitePool, host_id: i64, session_id: Uuid) -> Result<i64, sqlx::Error> {
    let sid = session_id.to_string();
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO connection_log (host_id, session_id) VALUES (?1, ?2) RETURNING id",
    )
    .bind(host_id)
    .bind(&sid)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

pub async fn finish(
    pool: &SqlitePool,
    session_id: Uuid,
    status: &str,
    error: Option<&str>,
) -> Result<(), sqlx::Error> {
    let sid = session_id.to_string();
    sqlx::query(
        "UPDATE connection_log SET ended_at = datetime('now'), status = ?2, error = ?3 \
         WHERE session_id = ?1 AND ended_at IS NULL",
    )
    .bind(&sid)
    .bind(status)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list(pool: &SqlitePool, limit: i64) -> Result<Vec<ConnectionLogEntry>, sqlx::Error> {
    let sql = format!("SELECT {FIELDS} FROM connection_log ORDER BY started_at DESC LIMIT ?1");
    sqlx::query_as::<_, ConnectionLogEntry>(&sql)
        .bind(limit)
        .fetch_all(pool)
        .await
}

pub async fn list_by_host(
    pool: &SqlitePool,
    host_id: i64,
    limit: i64,
) -> Result<Vec<ConnectionLogEntry>, sqlx::Error> {
    let sql = format!(
        "SELECT {FIELDS} FROM connection_log WHERE host_id = ?1 ORDER BY started_at DESC LIMIT ?2"
    );
    sqlx::query_as::<_, ConnectionLogEntry>(&sql)
        .bind(host_id)
        .bind(limit)
        .fetch_all(pool)
        .await
}
