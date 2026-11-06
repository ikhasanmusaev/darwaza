//! Листинг локальной файловой системы для левой колонки файлового браузера (#122).
//!
//! Отдельно от `sftp.rs`: там удалённая сторона, здесь локальная. Структуры
//! записей намеренно продублированы, а не переиспользованы — у локальной ФС
//! свои особенности (symlink'и резолвятся через `std::fs`, путь платформенный),
//! и связывать их одним типом значит потом расплетать.
//!
//! Наружу отдаём **только листинг**: содержимое файлов в WebView не уходит,
//! чтением при выгрузке занимается `sftp_upload` внутри Rust.

use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalEntry {
    pub name: String,
    pub path: String,
    pub kind: LocalEntryKind,
    pub size: u64,
    /// Unix-секунды. `None` если ФС не отдала время модификации.
    pub modified: Option<i64>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LocalEntryKind {
    Dir,
    File,
    Symlink,
    Other,
}

/// Ошибка наружу уходит кодом, причина — в журнал (#177).
fn map_err<E: std::fmt::Display>(e: E) -> String {
    crate::errors::opaque("localfs", e)
}

/// Домашняя директория пользователя — точка входа левой колонки.
#[tauri::command]
pub async fn local_home() -> Result<String, String> {
    let dir = directories::UserDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .ok_or_else(|| "cannot resolve home directory".to_string())?;
    Ok(dir.to_string_lossy().to_string())
}

/// Путь на уровень выше. Для корня возвращает сам корень.
#[tauri::command]
pub async fn local_parent(path: String) -> Result<String, String> {
    let p = PathBuf::from(&path);
    Ok(match p.parent() {
        Some(parent) => parent.to_string_lossy().to_string(),
        None => path,
    })
}

#[tauri::command]
pub async fn local_list(path: String) -> Result<Vec<LocalEntry>, String> {
    let dir = Path::new(&path);
    let mut read = tokio::fs::read_dir(dir)
        .await
        .map_err(|e| format!("read dir '{path}' failed: {e}"))?;

    let mut entries = Vec::new();
    while let Some(item) = read.next_entry().await.map_err(map_err)? {
        let name = item.file_name().to_string_lossy().to_string();

        // symlink_metadata, а не metadata: иначе битая ссылка уронит весь
        // листинг, а рабочая покажется типом цели, а не ссылкой.
        let meta = match tokio::fs::symlink_metadata(item.path()).await {
            Ok(m) => m,
            // Файл могли удалить между read_dir и stat — пропускаем, а не падаем.
            Err(_) => continue,
        };

        let kind = if meta.is_symlink() {
            LocalEntryKind::Symlink
        } else if meta.is_dir() {
            LocalEntryKind::Dir
        } else if meta.is_file() {
            LocalEntryKind::File
        } else {
            LocalEntryKind::Other
        };

        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64);

        entries.push(LocalEntry {
            path: item.path().to_string_lossy().to_string(),
            name,
            kind,
            size: meta.len(),
            modified,
        });
    }

    // Тот же порядок, что и у удалённой панели: папки первыми, дальше по имени.
    entries.sort_by(|a, b| {
        let a_dir = a.kind == LocalEntryKind::Dir;
        let b_dir = b.kind == LocalEntryKind::Dir;
        b_dir
            .cmp(&a_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(entries)
}
