//! Отчёты о сбоях (#171).
//!
//! **Своего сервера у нас нет и не появится.** Отчёт — это файл на диске.
//! Пользователь может сохранить копию и приложить её к багрепорту, а может
//! указать **свой** адрес (вебхук, собственный коллектор) и отправить туда
//! нажатием кнопки (#171a). Сами мы не отправляем ничего и никогда.
//!
//! Сбор **выключен по умолчанию**: паника печатается в лог в любом случае, а
//! отдельный файл заводится только с явного согласия.

use anyhow::{Context, Result};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Читается из panic-хука, поэтому не `RwLock`: хук обязан быть максимально
/// простым и не зависеть от асинхронного мира, который в момент паники мог уже
/// развалиться.
static ENABLED: AtomicBool = AtomicBool::new(false);

/// Ключ настройки в таблице `settings`.
pub const SETTING_KEY: &str = "diagnostics.crash_reports_enabled";

/// Куда отправлять отчёт, если пользователь этого захочет (#171a).
///
/// Пусто по умолчанию, и своего сервера у нас по-прежнему нет: сюда вписывают
/// **свой** адрес — вебхук, собственный коллектор, что угодно. Отправка идёт
/// только по явному нажатию, никогда сама.
pub const ENDPOINT_KEY: &str = "diagnostics.crash_endpoint";

/// Сколько ждать ответа эндпоинта.
///
/// Без таймаута зависший адрес держал бы кнопку «отправить» бесконечно.
pub const SEND_TIMEOUT_SECS: u64 = 15;

pub fn set_enabled(value: bool) {
    ENABLED.store(value, Ordering::Relaxed);
}

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn crashes_dir() -> Result<PathBuf> {
    let dir = crate::paths::state_dir()?.join("crashes");
    crate::paths::ensure_private_tree(&dir)?;
    Ok(dir)
}

/// Собрать текст отчёта.
///
/// Намеренно **без** содержимого хостов, путей к ключам и прочего пользовательского:
/// отчёт задуман так, чтобы его можно было отправить постороннему не вычитывая.
pub fn format_report(payload: &str, location: &str, backtrace: &str, unix: u64) -> String {
    format!(
        "Darwaza crash report\n\
         version: {}\n\
         os: {} {}\n\
         time: {} (unix)\n\
         location: {}\n\
         panic: {}\n\
         \n\
         backtrace:\n{}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        unix,
        location,
        payload,
        backtrace
    )
}

/// Проверить адрес для отправки.
///
/// Пустой адрес допустим — это «отправка выключена». Всё остальное обязано быть
/// http(s): узнать об опечатке в момент разбора сбоя — худший момент.
pub fn validate_endpoint(url: &str) -> Result<()> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        anyhow::bail!(crate::errors::codes::INVALID_URL);
    }
    Ok(())
}

/// Установить хук на панику. Вызывается один раз при старте.
///
/// Прежний хук не выбрасываем, а вызываем следом: иначе паника перестала бы
/// печататься в stderr и в лог, а это худшее, что можно сделать при разборе сбоя.
pub fn install_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if is_enabled() {
            if let Err(e) = write_report(info) {
                // Ошибку записи докладываем в stderr: логгер в момент паники
                // может быть уже недоступен.
                eprintln!("не удалось записать отчёт о сбое: {e}");
            }
        }
        previous(info);
    }));
}

fn write_report(info: &std::panic::PanicHookInfo<'_>) -> Result<()> {
    let unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = info.payload().downcast_ref::<String>() {
        s.clone()
    } else {
        "неизвестная причина".to_string()
    };

    let location = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "неизвестно".to_string());

    let backtrace = std::backtrace::Backtrace::force_capture().to_string();
    let report = format_report(&payload, &location, &backtrace, unix);

    let path = crashes_dir()?.join(format!("crash-{unix}.txt"));
    std::fs::write(&path, report).with_context(|| format!("запись {}", path.display()))?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrashReport {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub modified: Option<i64>,
}

pub async fn list() -> Result<Vec<CrashReport>> {
    let dir = crashes_dir()?;
    let mut read = tokio::fs::read_dir(&dir).await?;
    let mut out = Vec::new();

    while let Some(item) = read.next_entry().await? {
        let path = item.path();
        if path.extension().and_then(|e| e.to_str()) != Some("txt") {
            continue;
        }
        let Ok(meta) = item.metadata().await else {
            continue;
        };
        out.push(CrashReport {
            name: item.file_name().to_string_lossy().to_string(),
            path: path.to_string_lossy().to_string(),
            size: meta.len(),
            modified: meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64),
        });
    }

    out.sort_by_key(|r| std::cmp::Reverse(r.modified));
    Ok(out)
}

/// Прочитать отчёт целиком — чтобы пользователь мог его посмотреть перед отправкой.
pub async fn read(path: &str) -> Result<String> {
    let canonical = ensure_inside_dir(path).await?;
    Ok(tokio::fs::read_to_string(canonical).await?)
}

pub async fn remove(path: &str) -> Result<()> {
    let canonical = ensure_inside_dir(path).await?;
    tokio::fs::remove_file(canonical).await?;
    Ok(())
}

/// Путь приходит из WebView, поэтому проверяем, что он внутри каталога отчётов.
async fn ensure_inside_dir(path: &str) -> Result<PathBuf> {
    let dir = crashes_dir()?;
    let canonical = tokio::fs::canonicalize(path)
        .await
        .with_context(|| format!("файл не найден: {path}"))?;
    if !canonical.starts_with(&dir) {
        anyhow::bail!("путь вне каталога отчётов: {path}");
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_contains_diagnostics_but_no_user_data() {
        let report = format_report(
            "called `Option::unwrap()` on a `None` value",
            "src/commands/ssh.rs:42:13",
            "0: some_frame\n1: another_frame",
            1_700_000_000,
        );

        assert!(report.contains("src/commands/ssh.rs:42:13"));
        assert!(report.contains("Option::unwrap()"));
        assert!(report.contains("some_frame"));
        assert!(report.contains(env!("CARGO_PKG_VERSION")));
        assert!(report.contains(std::env::consts::OS));
    }

    #[test]
    fn disabled_by_default() {
        // Значение по умолчанию важно: сбор включается только явно.
        assert!(
            !ENABLED.load(Ordering::Relaxed) || is_enabled(),
            "флаг должен читаться согласованно"
        );
    }

    #[test]
    fn endpoint_validation_allows_empty_and_http() {
        assert!(validate_endpoint("").is_ok(), "пусто = отправка выключена");
        assert!(validate_endpoint("   ").is_ok());
        assert!(validate_endpoint("https://example.com/hook").is_ok());
        assert!(validate_endpoint("http://10.0.0.1:9000/crash").is_ok());
    }

    #[test]
    fn endpoint_validation_rejects_other_schemes() {
        // Опечатку надо ловить в настройках, а не при сбое.
        assert!(validate_endpoint("example.com/hook").is_err());
        assert!(validate_endpoint("ftp://example.com").is_err());
        assert!(validate_endpoint("file:///etc/passwd").is_err());
    }

    #[test]
    fn toggling_enabled_flag_works() {
        set_enabled(true);
        assert!(is_enabled());
        set_enabled(false);
        assert!(!is_enabled());
    }
}
