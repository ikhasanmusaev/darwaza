pub mod agent;
mod attention;
mod commands;
mod connection_log;
mod crash;
mod crypto;
pub mod db;
pub mod errors;
mod forwarding;
pub mod keyring_store;
mod known_hosts;
mod master;
mod paths;
#[cfg(windows)]
pub mod winsec;
mod pty;
pub mod recording;
pub mod recovery_key;
mod secrets;
mod sftp;
mod ssh;
mod startup;
pub mod sync;
mod totp;
pub mod transfers;

use sqlx::SqlitePool;
use std::path::PathBuf;
use std::time::Duration;
use tauri::{Manager, RunEvent};
use tracing::{error, info, warn};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

pub struct AppState {
    pub db: SqlitePool,
    pub ssh: ssh::SshManager,
    pub forwards: forwarding::ForwardManager,
    pub master: master::MasterKeyring,
    pub transfers: transfers::TransferManager,
    /// Планы агента, ожидающие решения человека (#205).
    pub plans: agent::PlanQueue,
    /// Вход в аккаунт синхронизации (#190-195).
    pub sync: sync::session::SyncSession,
}

/// Поднять канал для агента (#202).
///
/// Строго ПОСЛЕ регистрации состояния: обработчик достаёт из него базу и
/// менеджер сессий, и запущенный раньше он падал бы на первом же запросе.
///
/// Неудача канала **не мешает приложению работать**. Агент — дополнение, а
/// не условие: человек, у которого занят сокет или нет прав на каталог,
/// должен спокойно подключаться к своим серверам, а не смотреть на отказ
/// запуска.
async fn start_agent_channel(handle: &tauri::AppHandle) {
    let token = match agent::ensure_token() {
        Ok(token) => token,
        Err(e) => {
            warn!(target: "agent", "канал не поднят, нет токена: {e:#}");
            return;
        }
    };

    let tools = std::sync::Arc::new(agent::tools::Tools::new(handle.clone()));
    match agent::serve(tools, token).await {
        Ok(path) => info!(target: "agent", "канал агента открыт: {}", path.display()),
        Err(e) => warn!(target: "agent", "канал агента не открыт: {e:#}"),
    }
}

/// Каталог для логов: предпочтение XDG_STATE_HOME, иначе data_dir.
fn log_dir() -> Option<PathBuf> {
    let dir = paths::state_dir().ok()?.join("log");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Инициализирует tracing с двумя выходами: stderr (ANSI) и daily-ротированный
/// файл `app.log.YYYY-MM-DD` в state_dir.
///
/// Возвращает `WorkerGuard` — его нужно держать живым на всё время работы app,
/// иначе non-blocking writer может потерять последние записи при выходе.
fn init_logging() -> Option<WorkerGuard> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let stderr_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);

    match log_dir() {
        Some(dir) => {
            let file_appender = tracing_appender::rolling::daily(&dir, "app.log");
            let (file_writer, guard) = tracing_appender::non_blocking(file_appender);
            let file_layer = tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(file_writer);

            tracing_subscriber::registry()
                .with(filter)
                .with(stderr_layer)
                .with(file_layer)
                .init();
            info!(target: "logging", "logs writing to {}", dir.display());
            Some(guard)
        }
        None => {
            tracing_subscriber::registry()
                .with(filter)
                .with(stderr_layer)
                .init();
            None
        }
    }
}

/// Software-рендеринг там, где нет доступного GPU.
///
/// Без DRM-устройства (VM без GPU-проброса, headless-сервер, контейнер) Mesa
/// зависает при создании GL-контекста для WebKit: окно открывается **чёрным**,
/// `setup()` до инициализации БД не доходит, приложение выглядит намертво
/// повисшим. Проверено на чистой Ubuntu 24.04 (#078a): с
/// `LIBGL_ALWAYS_SOFTWARE=1` старт занимает 2 секунды, без него не стартует
/// вообще.
///
/// На системах с GPU (`/dev/dri` на месте) ничего не меняем — там аппаратный
/// рендеринг работает и он быстрее. Переменную, выставленную пользователем
/// вручную, тоже не трогаем.
#[cfg(target_os = "linux")]
fn ensure_software_gl_fallback() {
    if std::env::var_os("LIBGL_ALWAYS_SOFTWARE").is_some() {
        return;
    }
    if !std::path::Path::new("/dev/dri").exists() {
        std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1");
        info!(
            target: "startup",
            "/dev/dri не найден — включаю software-рендеринг (LIBGL_ALWAYS_SOFTWARE=1)"
        );
    }
}

/// Корректно закрыть все активные SSH-сессии перед выходом.
/// Шлём Disconnect в каждый mpsc → backend задачи отправят SSH disconnect
/// и закроют сокеты. Ждём короткий тайм-аут, дальше OS закроет всё что осталось.
async fn shutdown_sessions(manager: &ssh::SshManager) {
    let sessions = manager.snapshot().await;
    if sessions.is_empty() {
        return;
    }
    info!(target: "shutdown", "closing {} active session(s)", sessions.len());
    for s in sessions {
        if let Err(e) = manager
            .send_cmd(s.session_id, ssh::session::SessionCmd::Disconnect)
            .await
        {
            // norm — сессия могла уже сама завершиться
            tracing::debug!(target: "shutdown", "disconnect cmd for {}: {e}", s.session_id);
        }
    }
    // Дать backend-таскам ~300мс на отправку SSH disconnect и закрытие TCP.
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // _log_guard держим живым до конца run() чтобы non-blocking writer
    // успел сбросить буфер при выходе.
    let _log_guard = init_logging();

    // Хук ставим до всего остального: паника на раннем старте тоже должна
    // попасть в отчёт, если сбор включён.
    crash::install_hook();

    // Перенос данных из-под прежнего имени продукта — до открытия базы,
    // иначе она создастся пустой в новом каталоге, а старая останется
    // лежать рядом никем не замеченной.
    paths::migrate_from_legacy_name();

    // До создания окна: GTK/WebKit подхватывают переменную при инициализации.
    #[cfg(target_os = "linux")]
    ensure_software_gl_fallback();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // Проверка обновлений (#164). Плагин ничего не делает сам: он даёт
        // команду, которую зовёт интерфейс по кнопке. Автоматическая проверка
        // при запуске сознательно не включена — приложение обязано работать
        // без сети, и лезть в неё без спроса ему незачем.
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Уведомление «ждут решения» (#252): текст без деталей, см. attention.rs.
        .plugin(tauri_plugin_notification::init())
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Focused(true) = event {
                attention::clear(window.app_handle());
            }
        })
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                match db::init_pool().await {
                    Ok(pool) => {
                        // Поднимаем сохранённое согласие на сбор отчётов:
                        // panic-хук читает только память, в БД он не полезет.
                        if let Ok(Some((value,))) = sqlx::query_as::<_, (String,)>(
                            "SELECT value FROM settings WHERE key = ?1",
                        )
                        .bind(crash::SETTING_KEY)
                        .fetch_optional(&pool)
                        .await
                        {
                            crash::set_enabled(value == "true");
                        }

                        handle.manage(AppState {
                            db: pool,
                            ssh: ssh::SshManager::new(),
                            forwards: forwarding::ForwardManager::new(),
                            master: master::MasterKeyring::new(),
                            transfers: transfers::TransferManager::new(),
                            plans: agent::PlanQueue::new(),
                            sync: sync::session::SyncSession::new(),
                        });

                        // Однократный перенос паролей keyring со старого имени
                        // службы: чтение переносит только прочитанное.
                        {
                            let state = handle.state::<AppState>();
                            secrets::sweep_legacy_keyring(&state.db).await;
                        }

                        start_agent_channel(&handle).await;
                        // Синхронизация по расписанию. Строго после
                        // регистрации состояния: задача берёт из него базу и
                        // сессию.
                        sync::schedule::spawn(handle.clone());
                    }
                    Err(e) => {
                        error!(target: "db", "failed to init db: {e:#}");
                        // Окно уже открыто и будет работать как ни в чём не
                        // бывало — без состояния, то есть никак. Человеку об
                        // этом говорим прямо, а не оставляем ему разбираться
                        // по сообщению про `state` (#168).
                        startup::record(&handle, format!("{e:#}"));
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            startup::startup_failure,
            commands::ping,
            commands::groups_list,
            commands::groups_create,
            commands::groups_update,
            commands::groups_remove,
            commands::hosts_list,
            commands::hosts_get,
            commands::hosts_create,
            commands::hosts_update,
            commands::hosts_remove,
            commands::ssh_config_preview,
            commands::identities_list,
            commands::identities_used_by,
            commands::identities_get,
            commands::identities_create,
            commands::identities_update,
            commands::identities_remove,
            commands::identity_set_password,
            commands::identity_set_passphrase,
            commands::ssh_connect,
            commands::sftp_connect,
            commands::ssh_send,
            commands::ssh_resize,
            commands::ssh_latency,
            commands::ssh_disconnect,
            commands::ssh_active_sessions,
            commands::ssh_accept_host_key,
            commands::ssh_reject_host_key,
            commands::ssh_submit_auth_prompt,
            commands::ssh_cancel_auth_prompt,
            commands::known_hosts_list,
            commands::known_hosts_remove,
            commands::known_hosts_clear,
            commands::known_hosts_import_preview,
            commands::known_hosts_import,
            commands::settings_get,
            commands::settings_set,
            commands::settings_remove,
            commands::settings_all,
            commands::system_fonts,
            commands::hosts_probe,
            commands::onboarding_needed,
            commands::onboarding_done,
            commands::recovery_key_save,
            commands::connection_logs_list,
            commands::agent_log_list,
            commands::logs_dir,
            commands::logs_open_dir,
            commands::sync_status,
            commands::sync_set_endpoint,
            commands::sync_register,
            commands::sync_login,
            commands::sync_logout,
            commands::sync_recovery_issue,
            commands::sync_recover,
            commands::sync_now,
            commands::sync_first_run_question,
            commands::sync_apply_first_choice,
            commands::agent_plans_pending,
            commands::agent_plan_approve,
            commands::agent_plan_reject,
            commands::agent_plans_reject_all,
            commands::snippets_list,
            commands::snippets_get,
            commands::snippets_create,
            commands::snippets_update,
            commands::snippets_remove,
            commands::sftp_home,
            commands::sftp_list,
            commands::sftp_parent,
            commands::sftp_mkdir,
            commands::sftp_remove,
            commands::sftp_rename,
            commands::sftp_download,
            commands::sftp_upload,
            commands::sftp_close,
            commands::local_home,
            commands::local_parent,
            commands::local_list,
            commands::clipboard::clipboard_read_text,
            commands::local_mkdir,
            commands::local_rename,
            commands::local_remove,
            commands::port_forwards_list,
            commands::port_forwards_create,
            commands::port_forwards_update,
            commands::port_forwards_remove,
            commands::port_forward_start,
            commands::port_forward_stop,
            commands::port_forwards_active,
            commands::master_status,
            commands::master_recovery_issue,
            commands::master_recover,
            commands::secrets_place,
            commands::master_enable,
            commands::master_disable,
            commands::master_unlock,
            commands::master_lock,
            commands::master_change_password,
            commands::master_totp_begin,
            commands::master_totp_confirm,
            commands::master_totp_disable,
            commands::session_recording_start,
            commands::session_recording_stop,
            commands::recordings_list,
            commands::recordings_remove,
            commands::recording_read,
            commands::recordings_dir,
            commands::transfer_download,
            commands::transfer_upload,
            commands::transfer_tree,
            commands::transfer_cancel,
            commands::transfers_download_dir,
            commands::crash_reports_enabled,
            commands::crash_reports_set_enabled,
            commands::crash_reports_list,
            commands::crash_report_read,
            commands::crash_report_remove,
            commands::crash_reports_dir,
            commands::crash_endpoint_get,
            commands::crash_endpoint_set,
            commands::crash_report_send,
            commands::crash_report_export,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|handle, event| {
        // Graceful shutdown: при выходе из приложения корректно закрываем все
        // активные SSH-сессии прежде чем процесс умрёт.
        if let RunEvent::ExitRequested { .. } = event {
            if let Some(state) = handle.try_state::<AppState>() {
                let manager = state.ssh.clone();
                tauri::async_runtime::block_on(async move {
                    shutdown_sessions(&manager).await;
                });
            }
        }
    });
}
