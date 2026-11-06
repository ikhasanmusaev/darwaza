//! Единая точка доступа к секретам (#110).
//!
//! Куда именно ляжет пароль или passphrase, зависит от того, включён ли
//! мастер-пароль:
//!
//! - **выключен** — системный keyring, как и раньше;
//! - **включён** — зашифрованное хранилище в БД (`vault`), ключ от которого
//!   существует только в памяти после разблокировки.
//!
//! Смысл фасада в том, чтобы вызывающий код (identities, ssh) не знал об этом
//! выборе и не дублировал ветвление в пяти местах.

use crate::keyring_store;
use crate::master::MasterKeyring;
use sqlx::SqlitePool;

/// Ошибка наружу уходит кодом, причина — в журнал (#177).
fn map_err<E: std::fmt::Display>(e: E) -> String {
    crate::errors::opaque("mod", e)
}

/// Сохранить секрет.
pub async fn set(
    db: &SqlitePool,
    master: &MasterKeyring,
    kind: &str,
    id: i64,
    value: &str,
) -> Result<(), String> {
    if MasterKeyring::is_enabled(db).await.map_err(map_err)? {
        master
            .store_secret(db, kind, id, value)
            .await
            .map_err(map_err)
    } else {
        keyring_store::set(kind, id, value)
    }
}

/// Достать секрет. `Ok(None)` — записи нет.
pub async fn get(
    db: &SqlitePool,
    master: &MasterKeyring,
    kind: &str,
    id: i64,
) -> Result<Option<String>, String> {
    if MasterKeyring::is_enabled(db).await.map_err(map_err)? {
        master.load_secret(db, kind, id).await.map_err(map_err)
    } else {
        keyring_store::get(kind, id)
    }
}

/// Удалить секрет из обоих хранилищ.
///
/// Чистим и keyring, и vault независимо от текущего режима: после
/// переключения режима в старом хранилище могли остаться записи, и
/// удаление identity не должно оставлять их висеть.
pub async fn delete(db: &SqlitePool, master: &MasterKeyring, kind: &str, id: i64) {
    let _ = keyring_store::delete(kind, id);
    let _ = master.delete_secret(db, kind, id).await;
}

/// Перенести секреты между хранилищами при включении/выключении мастер-пароля.
///
/// Вызывается **после** того, как режим уже переключён: направление переноса
/// определяется новым режимом.
///
/// Возвращает число перенесённых записей. Ошибки отдельных identity не
/// прерывают перенос: у одной может не быть пароля вовсе, и это норма.
pub async fn migrate(
    db: &SqlitePool,
    master: &MasterKeyring,
    to_vault: bool,
) -> Result<usize, String> {
    let ids: Vec<(i64,)> = sqlx::query_as("SELECT id FROM identities")
        .fetch_all(db)
        .await
        .map_err(map_err)?;

    let mut moved = 0usize;
    for (id,) in ids {
        for kind in [
            keyring_store::kinds::PASSWORD,
            keyring_store::kinds::PASSPHRASE,
        ] {
            if to_vault {
                // keyring → vault
                match keyring_store::get(kind, id) {
                    Ok(Some(secret)) => {
                        master
                            .store_secret(db, kind, id, &secret)
                            .await
                            .map_err(map_err)?;
                        let _ = keyring_store::delete(kind, id);
                        moved += 1;
                    }
                    _ => continue,
                }
            } else {
                // vault → keyring
                match master.load_secret(db, kind, id).await {
                    Ok(Some(secret)) => {
                        keyring_store::set(kind, id, &secret)?;
                        let _ = master.delete_secret(db, kind, id).await;
                        moved += 1;
                    }
                    _ => continue,
                }
            }
        }
    }
    Ok(moved)
}
