//! Модели SFTP-слоя (#120).
//!
//! Сам транспорт — `russh_sftp::client::SftpSession` поверх дополнительного
//! канала той же SSH-сессии (см. `commands/sftp.rs`). Здесь только то, что
//! уходит в UI.

use russh_sftp::protocol::FileType;
use serde::Serialize;

/// Запись в списке файлов удалённой директории.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpEntry {
    /// Имя файла без пути.
    pub name: String,
    /// Полный абсолютный путь — по нему UI и ходит дальше.
    pub path: String,
    pub kind: SftpEntryKind,
    /// Размер в байтах. У директорий смысла не имеет, но сервер его отдаёт.
    pub size: u64,
    /// Время модификации, unix-секунды. `None` если сервер не прислал атрибут.
    pub modified: Option<i64>,
    /// Права в виде unix-режима (например 0o644). `None` если атрибута нет.
    pub permissions: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SftpEntryKind {
    Dir,
    File,
    Symlink,
    Other,
}

impl From<FileType> for SftpEntryKind {
    fn from(t: FileType) -> Self {
        match t {
            FileType::Dir => SftpEntryKind::Dir,
            FileType::File => SftpEntryKind::File,
            FileType::Symlink => SftpEntryKind::Symlink,
            FileType::Other => SftpEntryKind::Other,
        }
    }
}

/// Склеить путь родителя с именем ребёнка.
///
/// SFTP-пути всегда POSIX-овые (`/`), независимо от ОС клиента, поэтому
/// `std::path::Path` здесь не годится: на Windows он подставит `\`.
pub fn join_path(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{}/{}", parent.trim_end_matches('/'), name)
    }
}

/// Путь на уровень выше. Для корня возвращает сам корень.
pub fn parent_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(idx) => trimmed[..idx].to_string(),
    }
}

/// Директории идут первыми, дальше — по имени без учёта регистра.
/// Сортируем в backend, чтобы любой потребитель получал одинаковый порядок.
pub fn sort_entries(entries: &mut [SftpEntry]) {
    entries.sort_by(|a, b| {
        let a_dir = a.kind == SftpEntryKind::Dir;
        let b_dir = b.kind == SftpEntryKind::Dir;
        b_dir
            .cmp(&a_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_paths_posix_style() {
        assert_eq!(join_path("/", "etc"), "/etc");
        assert_eq!(join_path("/home/user", "file.txt"), "/home/user/file.txt");
        // лишний слэш у родителя не должен давать "//"
        assert_eq!(join_path("/home/user/", "file.txt"), "/home/user/file.txt");
    }

    #[test]
    fn walks_up_to_parent() {
        assert_eq!(parent_path("/home/user/file.txt"), "/home/user");
        assert_eq!(parent_path("/home"), "/");
        // из корня выше некуда
        assert_eq!(parent_path("/"), "/");
        assert_eq!(parent_path("/home/user/"), "/home");
    }

    #[test]
    fn sorts_dirs_first_then_case_insensitive() {
        let mk = |name: &str, kind: SftpEntryKind| SftpEntry {
            name: name.to_string(),
            path: join_path("/x", name),
            kind,
            size: 0,
            modified: None,
            permissions: None,
        };
        let mut entries = vec![
            mk("beta.txt", SftpEntryKind::File),
            mk("Alpha", SftpEntryKind::Dir),
            mk("alpha.txt", SftpEntryKind::File),
            mk("zeta", SftpEntryKind::Dir),
        ];
        sort_entries(&mut entries);
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "zeta", "alpha.txt", "beta.txt"]);
    }
}
