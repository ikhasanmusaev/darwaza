pub mod models;

use anyhow::{Context, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::path::PathBuf;
use std::str::FromStr;
use tracing::info;

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

pub fn data_dir() -> Result<PathBuf> {
    let dir = crate::paths::data_dir()?;
    crate::paths::ensure_private_dir(&dir)?;
    Ok(dir)
}

pub fn db_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("app.db"))
}

pub async fn init_pool() -> Result<SqlitePool> {
    let path = db_path()?;
    crate::paths::tighten_existing_data();
    info!(target: "db", "opening sqlite at {}", path.display());

    let url = format!("sqlite://{}", path.display());
    let opts = SqliteConnectOptions::from_str(&url)?
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await
        .context("failed to open sqlite pool")?;

    MIGRATOR
        .run(&pool)
        .await
        .context("failed to apply migrations")?;

    // Файлы журнала SQLite появляются после первого обращения.
    crate::paths::tighten_existing_data();

    info!(target: "db", "migrations applied");
    Ok(pool)
}

/// Стабильный идентификатор записи для синхронизации (#180).
///
/// Первичные ключи остаются числовыми — они связывают таблицы и адресуют
/// секреты в keyring. Этот идентификатор нужен для другого: он должен
/// совпадать на всех устройствах, а автоинкремент на каждой машине свой.
///
/// Проставляется **явно при вставке**. Триггер, дописывающий его сам, был бы
/// фокусом: журнал изменений заполняется тем же триггером, и разобраться,
/// откуда взялось значение, стало бы заметно труднее.
pub fn new_sync_id() -> String {
    uuid::Uuid::new_v4().to_string()
}
