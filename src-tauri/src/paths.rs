//! Каталоги приложения и перенос из-под прежнего имени.
//!
//! Каталоги данных завязаны на имя продукта, а имя сменилось с «Termius
//! Analog» на «Darwaza». Просто поменять строку нельзя: приложение стало бы
//! смотреть в новый пустой каталог, и для человека это выглядит как потеря
//! всех хостов, записей и настроек разом.
//!
//! Поэтому при первом запуске под новым именем прежние каталоги
//! переименовываются. Ровно один раз и только если новых ещё нет.

use anyhow::{Context, Result};
use directories::ProjectDirs;
use std::path::PathBuf;
use tracing::{info, warn};

const QUALIFIER: &str = "dev";
const ORGANIZATION: &str = "darwaza";
const APPLICATION: &str = "darwaza";

/// Прежнее имя. Остаётся в коде ради переноса и уберётся, когда переносить
/// станет нечего — то есть после первого публичного выпуска.
const LEGACY_ORGANIZATION: &str = "termius-analog";
const LEGACY_APPLICATION: &str = "termius-analog";

pub fn project_dirs() -> Result<ProjectDirs> {
    ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
        .context("не удалось определить каталоги приложения")
}

fn legacy_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from(QUALIFIER, LEGACY_ORGANIZATION, LEGACY_APPLICATION)
}

/// Создать каталог и закрыть его от других пользователей (`0700`).
///
/// Права ставятся и на уже существующий каталог: он мог появиться при
/// прежней версии с `0755`, и тогда записи сессий читал бы любой местный
/// пользователь.
pub fn ensure_private_dir(dir: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("создание {}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .with_context(|| format!("права {}", dir.display()))?;
    }
    Ok(())
}

/// Ужесточить права существующего файла до `0600`. Нет файла — не ошибка.
pub fn tighten_file(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if path.is_file() {
            if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
                tracing::warn!(target: "paths", "права {}: {e}", path.display());
            }
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Закрыть то, что уже лежит на диске: каталог данных и записей — `0700`,
/// база и записи сессий — `0600`.
///
/// Вызывается при старте. Новые файлы создаются сразу закрытыми, но у человека,
/// который пользовался прежней версией, они лежат с правами по умолчанию.
pub fn tighten_existing_data() {
    let Ok(data) = data_dir() else { return };
    if data.is_dir() {
        if let Err(e) = ensure_private_dir(&data) {
            tracing::warn!(target: "paths", "{e:#}");
        }
    }
    for name in ["app.db", "app.db-wal", "app.db-shm"] {
        tighten_file(&data.join(name));
    }
    let recordings = data.join("recordings");
    if recordings.is_dir() {
        if let Err(e) = ensure_private_dir(&recordings) {
            tracing::warn!(target: "paths", "{e:#}");
        }
        if let Ok(entries) = std::fs::read_dir(&recordings) {
            for entry in entries.flatten() {
                tighten_file(&entry.path());
            }
        }
    }
}

/// Каталог данных: база, записи сессий.
pub fn data_dir() -> Result<PathBuf> {
    Ok(project_dirs()?.data_dir().to_path_buf())
}

/// Каталог состояния: логи, отчёты о сбоях. На части систем его нет —
/// тогда используем каталог данных.
pub fn state_dir() -> Result<PathBuf> {
    let dirs = project_dirs()?;
    Ok(dirs
        .state_dir()
        .unwrap_or_else(|| dirs.data_dir())
        .to_path_buf())
}

/// Нужно ли переносить: старое есть, нового ещё нет.
///
/// Вынесено отдельной функцией, чтобы правило можно было проверить тестом,
/// не создавая настоящих каталогов в домашней папке.
pub fn should_migrate(legacy_exists: bool, current_exists: bool) -> bool {
    legacy_exists && !current_exists
}

/// Перенести каталоги из-под прежнего имени. Вызывается один раз при старте.
///
/// Ошибку переноса не считаем фатальной: приложение должно запуститься даже
/// если перенести не вышло. Но и молчать нельзя — иначе человек решит, что
/// данные пропали, и мы об этом не узнаем.
pub fn migrate_from_legacy_name() {
    let (Ok(current), Some(legacy)) = (project_dirs(), legacy_dirs()) else {
        return;
    };

    for (from, to, what) in [
        (
            legacy.data_dir().to_path_buf(),
            current.data_dir().to_path_buf(),
            "данные",
        ),
        (
            legacy
                .state_dir()
                .unwrap_or_else(|| legacy.data_dir())
                .to_path_buf(),
            current
                .state_dir()
                .unwrap_or_else(|| current.data_dir())
                .to_path_buf(),
            "состояние",
        ),
    ] {
        if from == to || !should_migrate(from.exists(), to.exists()) {
            continue;
        }
        if let Some(parent) = to.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::rename(&from, &to) {
            Ok(()) => info!(
                target: "paths",
                "{what} перенесены из {} в {}", from.display(), to.display()
            ),
            Err(e) => warn!(
                target: "paths",
                "не удалось перенести {what} из {}: {e}", from.display()
            ),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// Выполнить с `umask 0`. Права файла по умолчанию — `0666 & ~umask`, и при
    /// привычной `022` или `077` тест на `0600` проходил бы и без исправления:
    /// чтобы он что-то проверял, маску надо снять. `umask` общая для процесса,
    /// поэтому тесты, которые её трогают, идут по очереди.
    pub(crate) fn with_open_umask<T>(f: impl FnOnce() -> T) -> T {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let old = unsafe { libc::umask(0) };
        let result = f();
        unsafe { libc::umask(old) };
        result
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("darwaza-perm-{}-{name}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        dir
    }

    #[test]
    fn a_private_dir_is_0700_even_when_it_already_existed_open() {
        let dir = scratch("dir");
        with_open_umask(|| {
            std::fs::create_dir_all(&dir).unwrap();
            assert_eq!(
                std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
                0o777,
                "условие теста: каталог создан открытым"
            );
            ensure_private_dir(&dir).unwrap();
        });
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tighten_file_closes_an_open_file_and_ignores_a_missing_one() {
        let dir = scratch("file");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("old.cast");
        std::fs::write(&file, "x").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();

        tighten_file(&file);
        tighten_file(&dir.join("нет-такого"));

        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn migrates_only_when_old_exists_and_new_does_not() {
        assert!(should_migrate(true, false), "старое есть, нового нет");
        assert!(
            !should_migrate(true, true),
            "новое уже есть — перенос затёр бы свежие данные"
        );
        assert!(!should_migrate(false, false), "переносить нечего");
        assert!(!should_migrate(false, true), "уже переехали");
    }

    #[test]
    fn current_name_is_not_the_legacy_one() {
        // Если имена совпадут, перенос превратится в бесконечную проверку
        // самого себя, а сама смена имени окажется невыполненной.
        assert_ne!(APPLICATION, LEGACY_APPLICATION);
        assert_ne!(ORGANIZATION, LEGACY_ORGANIZATION);
    }
}
