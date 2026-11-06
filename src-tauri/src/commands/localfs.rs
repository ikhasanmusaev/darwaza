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

// ─────────────────────────────────────────────────────────────────────────
// Правки локальной стороны экрана SFTP
// ─────────────────────────────────────────────────────────────────────────

/// Проверить имя новой записи: одно имя, не путь. `a/b` и `..` вывели бы запись
/// за пределы открытой папки, и человек не увидел бы, где она оказалась.
fn plain_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    let bad = name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0');
    if bad {
        return Err("имя не может быть пустым и содержать разделители пути".to_string());
    }
    Ok(name)
}

/// Создать папку `name` внутри `dir`.
#[tauri::command]
pub async fn local_mkdir(dir: String, name: String) -> Result<String, String> {
    let path = Path::new(&dir).join(plain_name(&name)?);
    tokio::fs::create_dir(&path).await.map_err(map_err)?;
    Ok(path.to_string_lossy().to_string())
}

/// Переименовать запись внутри её же папки. Существующую запись не затирает.
#[tauri::command]
pub async fn local_rename(path: String, new_name: String) -> Result<String, String> {
    let from = PathBuf::from(&path);
    let parent = from
        .parent()
        .ok_or_else(|| "у корня нет имени".to_string())?;
    let to = parent.join(plain_name(&new_name)?);
    // `rename` на unix молча затирает существующий файл: проверяем сами.
    if tokio::fs::symlink_metadata(&to).await.is_ok() {
        return Err("запись с таким именем уже есть".to_string());
    }
    tokio::fs::rename(&from, &to).await.map_err(map_err)?;
    Ok(to.to_string_lossy().to_string())
}

/// Удалить файл или **пустую** папку. Рекурсивного удаления нет намеренно, как
/// и на сервере: ошибка здесь стоит чужих данных.
#[tauri::command]
pub async fn local_remove(path: String, is_dir: bool) -> Result<(), String> {
    if is_dir {
        tokio::fs::remove_dir(&path).await.map_err(map_err)
    } else {
        tokio::fs::remove_file(&path).await.map_err(map_err)
    }
}

#[cfg(test)]
mod edit_tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("darwaza-localfs-{}-{name}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_new_name_cannot_be_a_path() {
        for bad in ["", "  ", ".", "..", "a/b", "../x", "a\\b"] {
            assert!(plain_name(bad).is_err(), "{bad:?} должно быть отвергнуто");
        }
        assert_eq!(plain_name(" новая папка ").unwrap(), "новая папка");
    }

    #[tokio::test]
    async fn rename_never_overwrites_and_remove_never_recurses() {
        let dir = scratch("ops");
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();

        // Занятое имя — отказ, оба файла целы.
        assert!(local_rename(a.display().to_string(), "b.txt".into()).await.is_err());
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "b");

        let renamed = local_rename(a.display().to_string(), "c.txt".into()).await.unwrap();
        assert!(Path::new(&renamed).exists() && !a.exists());

        // Папка с содержимым не удаляется.
        let sub = local_mkdir(dir.display().to_string(), "sub".into()).await.unwrap();
        std::fs::write(Path::new(&sub).join("x"), "x").unwrap();
        assert!(local_remove(sub.clone(), true).await.is_err());
        assert!(Path::new(&sub).join("x").exists());

        std::fs::remove_file(Path::new(&sub).join("x")).unwrap();
        local_remove(sub, true).await.unwrap();
        local_remove(b.display().to_string(), false).await.unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }
}
