//! Первый запуск (#235): показывать ли мастер и сохранение ключа восстановления.

use crate::AppState;
use tauri::State;

const ONBOARDED_KEY: &str = "app.onboarded";

fn map_err<E: std::fmt::Display>(e: E) -> String {
    crate::errors::opaque("onboarding", e)
}

async fn count(state: &AppState, table: &str) -> Result<i64, String> {
    // Имя таблицы — только из этого модуля, не от интерфейса.
    sqlx::query_scalar::<_, i64>(&format!("SELECT COUNT(*) FROM {table}"))
        .fetch_one(&state.db)
        .await
        .map_err(map_err)
}

/// Нужен ли мастер первого запуска.
///
/// Только тем, у кого приложение действительно новое. Человек, обновившийся
/// с прежней версии, уже имеет хосты, ключи или аккаунт, и встречать его
/// выбором «локальный режим или аккаунт» значило бы спрашивать о том, что он
/// давно решил, — а то и подтолкнуть к повторной настройке поверх рабочей.
#[tauri::command]
pub async fn onboarding_needed(state: State<'_, AppState>) -> Result<bool, String> {
    let done: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?1")
        .bind(ONBOARDED_KEY)
        .fetch_optional(&state.db)
        .await
        .map_err(map_err)?;
    if done.is_some() {
        return Ok(false);
    }

    if count(&state, "hosts").await? > 0 || count(&state, "identities").await? > 0 {
        return Ok(false);
    }
    if state.sync.is_signed_in().await {
        return Ok(false);
    }
    let master = state.master.status(&state.db).await.map_err(map_err)?;
    Ok(!master.enabled)
}

/// Отметить, что мастер пройден или пропущен. Спрашивать второй раз не надо.
#[tauri::command]
pub async fn onboarding_done(state: State<'_, AppState>) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?1, '1') \
         ON CONFLICT(key) DO UPDATE SET value = '1'",
    )
    .bind(ONBOARDED_KEY)
    .execute(&state.db)
    .await
    .map_err(map_err)?;
    Ok(())
}

/// Сохранить ключ восстановления в файл, который выбрал человек.
///
/// - **Права `0600`** с момента создания: файл с секретом, равносильным
///   паролю, не должен успеть побыть читаемым для других на машине.
/// - **Существующий файл не затирается**: путь выбирал человек, но выбрать он
///   мог и чужой файл. Создание идёт флагом «только новый».
/// - Ключ проверяется на форму до записи: в файл не должна попасть строка,
///   которая ключом не является.
/// - В журнал не пишется ничего, кроме факта ошибки.
#[tauri::command]
pub async fn recovery_key_save(path: String, key: String) -> Result<(), String> {
    use crate::errors::codes;
    use std::io::Write;

    let parsed = crate::recovery_key::parse(&key)
        .ok_or_else(|| codes::MASTER_RECOVERY_MALFORMED.to_string())?;
    let text = crate::recovery_key::format(&parsed);

    let target = std::path::Path::new(&path);
    if !target.is_absolute() {
        return Err(map_err("путь не абсолютный"));
    }

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options.open(target).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            codes::FILE_EXISTS.to_string()
        } else {
            crate::errors::opaque("recovery_key_save", e)
        }
    })?;

    // Запись не удалась — файл, который мы же и создали, пустым не оставляем:
    // повтор с тем же путём упёрся бы в «файл уже есть», хотя ключа в нём нет.
    let written = writeln!(file, "Darwaza recovery key\n\n{text}\n").and_then(|()| file.sync_all());
    if let Err(e) = written {
        drop(file);
        std::fs::remove_file(target).ok();
        return Err(map_err(e));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn scratch(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("darwaza-test-{}-{name}", uuid::Uuid::new_v4()))
    }

    const KEY: &str = "7K2P-9QXM-4HVT-8WNB-3RJD-6FZC-5YGM-2AKT";

    #[cfg(unix)] // права файла в стиле unix; на Windows их задаёт профиль пользователя
    #[test]
    fn the_key_file_is_private_from_the_first_byte() {
        let path = scratch("key.txt");
        // С `umask 0`: при привычной маске `022` права получились бы узкими и
        // без `mode(0o600)`, и тест ничего бы не проверял.
        let saved = crate::paths::tests::with_open_umask(|| {
            tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap()
                .block_on(recovery_key_save(
                    path.display().to_string(),
                    KEY.to_string(),
                ))
        });
        saved.expect("сохранилось");

        let mode = std::fs::metadata(&path).expect("файл").permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "права должны быть 0600, а не {mode:o}");
        let text = std::fs::read_to_string(&path).expect("читается");
        assert!(text.contains(KEY));
        std::fs::remove_file(&path).ok();
    }

    #[tokio::test]
    async fn an_existing_file_is_never_overwritten() {
        let path = scratch("existing.txt");
        std::fs::write(&path, "чужое содержимое").expect("создан");

        let result = recovery_key_save(path.display().to_string(), KEY.to_string()).await;

        assert_eq!(result, Err(crate::errors::codes::FILE_EXISTS.to_string()));
        assert_eq!(
            std::fs::read_to_string(&path).expect("читается"),
            "чужое содержимое"
        );
        std::fs::remove_file(&path).ok();
    }

    #[tokio::test]
    async fn only_a_real_key_and_an_absolute_path_are_accepted() {
        let path = scratch("bad.txt");
        let result = recovery_key_save(path.display().to_string(), "не ключ".to_string()).await;
        assert_eq!(
            result,
            Err(crate::errors::codes::MASTER_RECOVERY_MALFORMED.to_string())
        );
        assert!(!path.exists(), "файл не должен появиться");

        assert!(
            recovery_key_save("relative.txt".to_string(), KEY.to_string())
                .await
                .is_err()
        );
        assert!(!std::path::Path::new("relative.txt").exists());
    }
}
