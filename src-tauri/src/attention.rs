//! Привлечение внимания, когда нужно решение человека (#252).
//!
//! Окно не в фокусе — человек не знает, что его ждут (план агента к тому же
//! сам отклоняется по таймауту). Два сигнала: подсветка окна в панели задач и
//! системное уведомление.
//!
//! Уведомление **без деталей**, всегда один и тот же текст: оно видно на
//! заблокированном экране и остаётся в журнале ОС, поэтому ни команд, ни имён
//! хостов, ни путей в нём быть не может. Подробности человек увидит в окне.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, UserAttentionType};
#[cfg(not(target_os = "linux"))]
use tauri_plugin_notification::NotificationExt;

/// Настройка «Уведомлять, когда нужно моё решение». Нет записи — включено;
/// выключает только значение «0».
pub const SETTING_KEY: &str = "notify.decision";

/// Не чаще одного уведомления в минуту: десять планов подряд — не десять звонков.
const MIN_GAP: Duration = Duration::from_secs(60);

/// Текст уведомления. Константа намеренно: подставлять в неё нечего.
pub const NOTIFICATION_TITLE: &str = "Darwaza";
pub const NOTIFICATION_BODY: &str = "Darwaza ждёт вашего решения";

/// Ограничитель частоты. Отдельный тип, чтобы проверять без часов и окна.
#[derive(Default)]
pub struct Throttle {
    last: Mutex<Option<Instant>>,
}

impl Throttle {
    /// `true` — можно уведомлять, и момент запоминается.
    pub fn allow(&self, now: Instant) -> bool {
        let mut last = self.last.lock().unwrap_or_else(|p| p.into_inner());
        match *last {
            Some(prev) if now.saturating_duration_since(prev) < MIN_GAP => false,
            _ => {
                *last = Some(now);
                true
            }
        }
    }
}

static THROTTLE: std::sync::OnceLock<Throttle> = std::sync::OnceLock::new();

/// Включена ли настройка. Ошибка чтения — «включено»: молчать, когда человека
/// ждут, хуже, чем лишний раз напомнить; но в журнал пишем.
pub async fn enabled(db: &SqlitePool) -> bool {
    match sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?1")
        .bind(SETTING_KEY)
        .fetch_optional(db)
        .await
    {
        Ok(value) => value.as_deref() != Some("0"),
        Err(e) => {
            tracing::warn!(target: "attention", "не прочиталась настройка уведомлений: {e}");
            true
        }
    }
}

/// Нужно решение человека. Если окно в фокусе — ничего не делаем: он и так
/// смотрит на него.
pub async fn needs_decision(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if window.is_focused().unwrap_or(false) {
        return;
    }
    // Без состояния настройку не прочитать, а значит и не проверить: молчим.
    let Some(state) = app.try_state::<crate::AppState>() else {
        return;
    };
    if !enabled(&state.db).await {
        return;
    }
    // Подсветка не ограничена по частоте сознательно: она не звучит, снимается
    // при фокусе и не копится. Троттлинг — только у уведомления.
    if let Err(e) = window.request_user_attention(Some(UserAttentionType::Critical)) {
        tracing::warn!(target: "attention", "подсветка окна не удалась: {e}");
    }
    if !THROTTLE.get_or_init(Throttle::default).allow(Instant::now()) {
        return;
    }
    show(app, NOTIFICATION_TITLE, NOTIFICATION_BODY);
}

/// Единственное место, где уведомление уходит в систему. Тексты — `&'static
/// str`: собрать их из имени хоста или команды на лету здесь нельзя по типу.
fn show(app: &AppHandle, title: &'static str, body: &'static str) {
    #[cfg(target_os = "linux")]
    {
        let _ = app;
        linux::show(title, body);
    }
    #[cfg(not(target_os = "linux"))]
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::warn!(target: "attention", "уведомление не показалось: {e}");
    }
}

/// Окно получило фокус — подсветку снимаем (на части окружений сама не гаснет).
pub fn clear(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.request_user_attention(None);
    }
    #[cfg(target_os = "linux")]
    linux::clear();
}

/// Linux (KDE Plasma, Wayland): то, чего не умеют окно GTK и плагин уведомлений.
///
/// На Wayland `request_user_attention` сводится к подсказке urgency GTK, которую
/// композитор без токена xdg-activation игнорирует: кнопка в диспетчере задач не
/// мигает. Поэтому добавлены два канала, которые Plasma понимает без токена:
/// сигнал `com.canonical.Unity.LauncherEntry.Update` с `urgent` (кнопка
/// подсвечивается, пока человек не откроет окно) и уведомление с urgency=critical,
/// `desktop-entry` и без срока жизни (Plasma не закрывает такое само).
#[cfg(target_os = "linux")]
mod linux {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::OnceLock;

    use zbus::blocking::Connection;
    use zbus::zvariant::Value;

    /// Имя `.desktop`-файла без суффикса: по нему Plasma находит кнопку приложения.
    pub const DESKTOP_ENTRY: &str = "darwaza";
    const LAUNCHER_PATH: &str = "/com/canonical/unity/launcherentry/darwaza";
    const LAUNCHER_IFACE: &str = "com.canonical.Unity.LauncherEntry";

    static BUS: OnceLock<Option<Connection>> = OnceLock::new();
    /// Номер последнего уведомления: новое заменяет его, а не копится стопкой.
    static LAST_ID: AtomicU32 = AtomicU32::new(0);

    fn bus() -> Option<&'static Connection> {
        BUS.get_or_init(|| match Connection::session() {
            Ok(c) => Some(c),
            Err(e) => {
                tracing::warn!(target: "attention", "нет сессионной шины D-Bus: {e}");
                None
            }
        })
        .as_ref()
    }

    /// Аргументы сигнала LauncherEntry. Отдельно, чтобы проверять без шины.
    pub fn launcher_update(urgent: bool) -> (String, HashMap<&'static str, Value<'static>>) {
        let mut props = HashMap::new();
        props.insert("urgent", Value::from(urgent));
        (format!("application://{DESKTOP_ENTRY}.desktop"), props)
    }

    /// Подсказки уведомления: критичное, привязано к приложению.
    pub fn notification_hints() -> HashMap<&'static str, Value<'static>> {
        let mut hints = HashMap::new();
        hints.insert("urgency", Value::from(2u8));
        hints.insert("desktop-entry", Value::from(DESKTOP_ENTRY));
        hints
    }

    fn set_urgent(urgent: bool) {
        let Some(bus) = bus() else { return };
        let body = launcher_update(urgent);
        if let Err(e) = bus.emit_signal(None::<&str>, LAUNCHER_PATH, LAUNCHER_IFACE, "Update", &body) {
            tracing::warn!(target: "attention", "LauncherEntry не отправился: {e}");
        }
    }

    pub fn show(title: &'static str, body: &'static str) {
        set_urgent(true);
        let Some(bus) = bus() else { return };
        let reply = bus.call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            // expire_timeout 0 — не закрывать самому: человек мог отойти.
            &(
                "Darwaza",
                LAST_ID.load(Ordering::Relaxed),
                DESKTOP_ENTRY,
                title,
                body,
                Vec::<&str>::new(),
                notification_hints(),
                0i32,
            ),
        );
        match reply.and_then(|m| m.body().deserialize::<u32>()) {
            Ok(id) => LAST_ID.store(id, Ordering::Relaxed),
            Err(e) => tracing::warn!(target: "attention", "уведомление не показалось: {e}"),
        }
    }

    /// Окно в фокусе: гасим подсветку и закрываем висящее уведомление.
    pub fn clear() {
        let id = LAST_ID.swap(0, Ordering::Relaxed);
        if id == 0 {
            return;
        }
        set_urgent(false);
        if let Some(bus) = bus() {
            let _ = bus.call_method(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                Some("org.freedesktop.Notifications"),
                "CloseNotification",
                &(id,),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_notification_within_a_minute_is_dropped() {
        let t = Throttle::default();
        let start = Instant::now();
        assert!(t.allow(start));
        assert!(!t.allow(start + Duration::from_secs(59)));
        assert!(t.allow(start + Duration::from_secs(60)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_urgency_is_critical_and_tied_to_the_desktop_file() {
        use zbus::zvariant::Value;
        let (uri, props) = linux::launcher_update(true);
        assert_eq!(uri, "application://darwaza.desktop");
        assert!(matches!(props.get("urgent"), Some(Value::Bool(true))));
        assert!(matches!(linux::launcher_update(false).1.get("urgent"), Some(Value::Bool(false))));
        let hints = linux::notification_hints();
        assert!(matches!(hints.get("urgency"), Some(Value::U8(2))));
        assert!(matches!(hints.get("desktop-entry"), Some(Value::Str(s)) if s.as_str() == "darwaza"));
        // Только эти подсказки: ни хоста, ни команды в уведомление не попадает.
        assert_eq!(hints.len(), 2);
    }

    #[test]
    fn notification_text_is_the_fixed_one() {
        // Типы `show` (&'static str) не дают подставить имя хоста или команду;
        // тест фиксирует сам текст, чтобы его правка была осознанной.
        assert_eq!(NOTIFICATION_TITLE, "Darwaza");
        assert_eq!(NOTIFICATION_BODY, "Darwaza ждёт вашего решения");
    }

    #[tokio::test]
    async fn setting_is_on_by_default_and_off_only_by_zero() {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("память");
        crate::db::MIGRATOR.run(&db).await.expect("миграции");
        assert!(enabled(&db).await);
        sqlx::query("INSERT INTO settings (key, value) VALUES (?1, '0')")
            .bind(SETTING_KEY)
            .execute(&db)
            .await
            .unwrap();
        assert!(!enabled(&db).await);
        sqlx::query("UPDATE settings SET value = '1' WHERE key = ?1")
            .bind(SETTING_KEY)
            .execute(&db)
            .await
            .unwrap();
        assert!(enabled(&db).await);
    }
}
