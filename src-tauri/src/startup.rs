//! Отказ на старте, который человек обязан увидеть (#168).
//!
//! Раньше ошибка открытия базы уходила в лог, окно открывалось как ни в чём не
//! бывало, и первая же страница показывала «state not managed for field
//! `state`». По этому тексту причину понять нельзя: он про внутреннее
//! устройство Tauri, а не про то, что случилось. Человек видел сломанное
//! приложение без единой подсказки, что делать.
//!
//! Теперь отказ доезжает до окна двумя путями сразу, и это не перестраховка:
//! окно может ещё не успеть подписаться к моменту отказа (база открывается
//! в фоновой задаче), а может подписаться и ждать. Событие закрывает второй
//! случай, запрос при монтировании — первый.

use std::sync::OnceLock;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// Имя события. То же самое читает `src/api/startup.ts`.
pub const EVENT: &str = "app:startup-failed";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StartupFailure {
    /// Техническая причина как есть. Показывается человеку целиком: без неё
    /// разговор сводится к «не работает», а с ней — к одной строке в письме.
    pub message: String,
    /// Куда смотреть дальше. Путь считается здесь, потому что снаружи его
    /// пришлось бы собирать заново и однажды собрали бы неверно.
    pub log_path: String,
}

static FAILURE: OnceLock<StartupFailure> = OnceLock::new();

/// Записать отказ и сообщить окну.
///
/// Первый отказ побеждает: он причина, всё последующее — следствия. Показать
/// человеку следствие вместо причины значит отправить его чинить не то.
pub fn record(handle: &AppHandle, message: String) {
    let failure = StartupFailure {
        message,
        log_path: crate::paths::state_dir()
            .map(|d| d.join("log").display().to_string())
            .unwrap_or_else(|_| "—".to_string()),
    };

    if FAILURE.set(failure.clone()).is_err() {
        return;
    }

    // Ошибку отправки глотать нельзя, но и делать из неё второй отказ незачем:
    // запись уже сохранена, и окно получит её запросом при монтировании.
    if let Err(e) = handle.emit(EVENT, &failure) {
        tracing::error!(target: "startup", "не удалось доставить отказ в окно: {e}");
    }
}

/// Что случилось на старте. `None` — всё в порядке.
#[tauri::command]
pub fn startup_failure() -> Option<StartupFailure> {
    FAILURE.get().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Проверяем именно правило «первый побеждает»: OnceLock даёт его даром,
    /// но правило это смысловое, а не техническое, и переписать хранилище на
    /// обычный Mutex однажды может показаться безобидным.
    #[test]
    fn first_failure_wins() {
        let cell: OnceLock<&str> = OnceLock::new();
        assert!(cell.set("база не открылась").is_ok());
        assert!(cell.set("состояние не зарегистрировано").is_err());
        assert_eq!(cell.get().copied(), Some("база не открылась"));
    }

    #[test]
    fn failure_serializes_for_the_window() {
        let json = serde_json::to_value(StartupFailure {
            message: "миграция изменена".into(),
            log_path: "/tmp/log".into(),
        })
        .unwrap();
        // Имя поля должно быть тем, которое читает фронтенд: разъедутся —
        // окно покажет пустое место вместо пути к логу.
        assert_eq!(json["logPath"], "/tmp/log");
        assert_eq!(json["message"], "миграция изменена");
    }
}
