//! Команды синхронизации: вход, выход, проход, состояние.
//!
//! Все они устроены так, чтобы **отсутствие аккаунта не мешало работать**.
//! Приложение было локальным и остаётся им: не вошёл — ничего не сломалось,
//! сервер недоступен — ничего не сломалось. Это принцип, а не любезность.

use crate::sync::api::ApiClient;
use crate::sync::session::Active;
use crate::sync::{engine, keys};
use crate::AppState;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tracing::{info, warn};

/// Ключи настроек. Адрес сервера и адрес почты — не секреты, им место здесь.
const ENDPOINT_KEY: &str = "sync.endpoint";
const EMAIL_KEY: &str = "sync.email";

/// Адрес по умолчанию.
///
/// Пусто намеренно: пока публичного сервера нет, подставленный адрес отправлял
/// бы запросы в никуда и порождал невнятные ошибки. Кому нужно — вписывает
/// свой, в том числе поднятый самостоятельно.
/// Адрес нашего сервера синхронизации.
///
/// Раньше здесь была пустая строка, и человек видел поле «Адрес сервера»,
/// которое обязан заполнить значением, известным только разработчику. Для
/// него это выглядело как «введите то, чего вы не знаете», и синхронизация не
/// работала ни у кого.
///
/// Свой сервер поднять по-прежнему можно — поле осталось, но убрано под
/// «изменить»: это редкий случай, а не первый шаг.
const DEFAULT_ENDPOINT: &str = "https://api.darwaza.uz/api/v1";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountStatus {
    /// Куда ходим. Пусто — синхронизация не настроена.
    pub endpoint: String,
    pub email: Option<String>,
    pub signed_in: bool,
    /// Сколько записей ждёт отправки.
    pub pending: i64,
    /// Последняя ошибка. Показывается ненавязчиво, не мешая работать.
    pub last_error: Option<String>,
    /// Когда последний проход удался, Unix-секунды. `None` — ещё не было.
    pub last_sync_at: Option<i64>,
    pub cursor: i64,
}

/// Событие для окна: состояние синхронизации изменилось — проход удался или
/// нет. Окно обновляет подвал и настройки само, а не опрашивает по таймеру.
pub const SYNC_STATE_EVENT: &str = "sync://state";

async fn account_status(state: &AppState) -> Result<AccountStatus, String> {
    let health = crate::sync::health(&state.db)
        .await
        .map_err(|e| format!("{e}"))?;

    Ok(AccountStatus {
        endpoint: endpoint_of(state).await,
        email: setting(state, EMAIL_KEY).await,
        signed_in: state.sync.is_signed_in().await,
        pending: health.pending,
        last_error: health.last_error,
        last_sync_at: health.last_sync_at,
        cursor: crate::sync::cursor(&state.db).await.unwrap_or(0),
    })
}

/// Сообщить окну о новом состоянии. Ошибку отправки глушим: некому показать —
/// не повод считать проход неудавшимся.
async fn announce(app: &AppHandle, state: &AppState) {
    if let Ok(status) = account_status(state).await {
        let _ = app.emit(SYNC_STATE_EVENT, &status);
    }
}

async fn setting(state: &AppState, key: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?1")
        .bind(key)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
        .filter(|v| !v.is_empty())
}

async fn save_setting(state: &AppState, key: &str, value: &str) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(&state.db)
    .await
    .map(|_| ())
    .map_err(|e| format!("{e}"))
}

fn client(endpoint: &str) -> Result<ApiClient, String> {
    if endpoint.trim().is_empty() {
        return Err("адрес сервера не задан".to_string());
    }
    ApiClient::new(endpoint).map_err(|e| format!("{e}"))
}

async fn endpoint_of(state: &AppState) -> String {
    setting(state, ENDPOINT_KEY)
        .await
        .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string())
}

/// Код для интерфейса из любой ошибки синхронизации (#178).
///
/// Отказ сервера переводится по своему коду, всё прочее — «синхронизация не
/// удалась». Полная причина уходит в журнал: раньше она попадала прямо на
/// экран, и человек читал «Запрос не соответствует контракту
/// (VALIDATION_FAILED)» — фразу, из которой не следует ни одного действия.
fn sync_error_code(e: &anyhow::Error) -> String {
    let code = e
        .downcast_ref::<crate::sync::api::ApiError>()
        .map(|api| api.ui_code())
        .unwrap_or("errors.syncFailed");
    tracing::warn!(target: "sync", code, error = %e, "проход синхронизации не удался");
    code.to_string()
}

/// Действующий токен доступа для одного запроса.
///
/// После перезапуска приложения токена доступа нет вовсе — в keyring лежит
/// только refresh, потому что access живёт минуты. Поэтому «обновить и
/// повторить» здесь обычный ход событий, а не восстановление после сбоя.
async fn with_access<F, Fut>(
    state: &AppState,
    api: &ApiClient,
    active: &Active,
    call: F,
) -> Result<(), String>
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<()>>,
{
    let access = if active.access_token.is_empty() {
        let refreshed = api
            .refresh(&active.refresh_token)
            .await
            .map_err(|e| sync_error_code(&e))?;
        state
            .sync
            .refreshed(refreshed.access_token.clone(), refreshed.refresh_token)
            .await;
        refreshed.access_token
    } else {
        active.access_token.clone()
    };

    match call(access).await {
        Ok(()) => Ok(()),
        Err(e) => {
            let stale = e
                .downcast_ref::<crate::sync::api::ApiError>()
                .is_some_and(|api_err| api_err.needs_refresh());
            if !stale {
                return Err(sync_error_code(&e));
            }
            let refreshed = api
                .refresh(&active.refresh_token)
                .await
                .map_err(|e| sync_error_code(&e))?;
            state
                .sync
                .refreshed(refreshed.access_token.clone(), refreshed.refresh_token)
                .await;
            call(refreshed.access_token)
                .await
                .map_err(|e| sync_error_code(&e))
        }
    }
}

#[tauri::command]
pub async fn sync_status(state: State<'_, AppState>) -> Result<AccountStatus, String> {
    account_status(&state).await
}

#[tauri::command]
pub async fn sync_set_endpoint(state: State<'_, AppState>, url: String) -> Result<(), String> {
    let trimmed = url.trim();
    let acceptable =
        trimmed.is_empty() || trimmed.starts_with("http://") || trimmed.starts_with("https://");
    if !acceptable {
        // Узнать об опечатке при первой синхронизации — худший момент.
        return Err(crate::errors::codes::INVALID_URL.to_string());
    }
    save_setting(&state, ENDPOINT_KEY, trimmed).await
}

#[tauri::command]
pub async fn sync_register(
    state: State<'_, AppState>,
    email: String,
    password: String,
) -> Result<String, String> {
    let api = client(&endpoint_of(&state).await)?;

    let master = keys::master_key(&email, &password).map_err(|e| format!("{e}"))?;
    let auth = keys::auth_hash(&master, &password).map_err(|e| format!("{e}"))?;
    let enc = keys::encryption_key(&master).map_err(|e| format!("{e}"))?;

    // Ключ записей — случайный, а не выведенный из пароля напрямую: смена
    // пароля тогда не требует перешифровать всё заново.
    let mut symmetric = [0u8; 32];
    getrandom::fill(&mut symmetric).map_err(|e| format!("нет случайности: {e}"))?;
    let protected = keys::seal_for_wire(&enc, &symmetric).map_err(|e| format!("{e}"))?;

    // Ключ восстановления выпускается **сразу**, а не когда-нибудь потом
    // (#180c). Та же логика, что у мастер-пароля: отложенный ключ не
    // выпускают никогда, а без него у забытого пароля нет запасного входа
    // вовсе — сброса по почте здесь не существует и не появится.
    let raw = crate::recovery_key::generate()?;
    let recovery = keys::recovery_keys(&raw).map_err(|e| format!("{e}"))?;
    let protected_rk =
        keys::seal_for_wire(&recovery.enc, &symmetric).map_err(|e| format!("{e}"))?;

    api.register(
        &email,
        &auth,
        &protected,
        &recovery.auth_hash,
        &protected_rk,
    )
    .await
    .map_err(|e| sync_error_code(&e))?;

    save_setting(&state, EMAIL_KEY, &email).await?;
    info!(target: "sync", "аккаунт заведён, ждём подтверждения адреса");
    Ok(crate::recovery_key::format(&raw))
}

#[tauri::command]
pub async fn sync_login(
    state: State<'_, AppState>,
    email: String,
    password: String,
    totp_code: Option<String>,
) -> Result<(), String> {
    let api = client(&endpoint_of(&state).await)?;

    let master = keys::master_key(&email, &password).map_err(|e| format!("{e}"))?;
    let auth = keys::auth_hash(&master, &password).map_err(|e| format!("{e}"))?;
    let enc = keys::encryption_key(&master).map_err(|e| format!("{e}"))?;

    let session = api
        .login(&email, &auth, totp_code.as_deref())
        .await
        .map_err(|e| format!("{e}"))?;

    // Код, а не текст ошибки: человеку в окно уходит переводимый код, а
    // подробность — в журнал (#177).
    let (key, legacy_key) = keys::record_keys(&enc, session.protected_symmetric_key.as_deref())
        .map_err(|e| {
            warn!(target: "sync", "ключ записей не получен: {e}");
            crate::errors::codes::SYNC_PROTECTED_KEY_UNREADABLE.to_string()
        })?;

    state
        .sync
        .set(Active {
            email: email.clone(),
            access_token: session.access_token,
            refresh_token: session.refresh_token,
            key,
            legacy_key,
        })
        .await;
    save_setting(&state, EMAIL_KEY, &email).await?;
    info!(target: "sync", "вход выполнен");
    Ok(())
}

/// Выпустить или перевыпустить ключ восстановления аккаунта (#180c).
///
/// Требует текущий пароль, а не только открытую сессию: перевыпуск **отзывает
/// прежнюю бумажку**, и делать это по одной лишь украденной сессии нельзя.
///
/// Ключ возвращается **после** ответа сервера. Показанный заранее окажется
/// недействительным, если запрос не дойдёт, — а человек его уже переписал на
/// бумагу.
#[tauri::command]
pub async fn sync_recovery_issue(
    state: State<'_, AppState>,
    password: String,
) -> Result<String, String> {
    let Some(active) = state.sync.current().await else {
        return Err(crate::errors::codes::SYNC_NOT_SIGNED_IN.to_string());
    };
    let api = client(&endpoint_of(&state).await)?;

    let master = keys::master_key(&active.email, &password).map_err(|e| format!("{e}"))?;
    let auth = keys::auth_hash(&master, &password).map_err(|e| format!("{e}"))?;

    let raw = crate::recovery_key::generate()?;
    let recovery = keys::recovery_keys(&raw).map_err(|e| format!("{e}"))?;
    // Под обёртку идёт тот же ключ записей, что и под пароль: обёрток две,
    // ключ один. Положи сюда что-то другое — и восстановление вернуло бы
    // ключ, которым ничего не зашифровано.
    let wrapped = keys::seal_for_wire(&recovery.enc, &active.key).map_err(|e| format!("{e}"))?;

    with_access(&state, &api, &active, |access| {
        let api = &api;
        let auth = &auth;
        let recovery_auth = &recovery.auth_hash;
        let wrapped = &wrapped;
        async move {
            api.recovery_reissue(&access, auth, recovery_auth, wrapped)
                .await
        }
    })
    .await?;

    info!(target: "sync", "ключ восстановления выпущен");
    Ok(crate::recovery_key::format(&raw))
}

/// Восстановить доступ ключом с бумаги (#180c).
///
/// Возвращает **новый** ключ восстановления: прежний перестаёт работать в тот
/// же момент, когда начинает работать новый пароль, и оставить человека без
/// запасного входа нельзя.
///
/// Второй фактор здесь не спрашивается. Цена названа прямо в документации:
/// ключ на бумаге равносилен паролю и коду вместе, и защищать его надо так же.
#[tauri::command]
pub async fn sync_recover(
    state: State<'_, AppState>,
    email: String,
    recovery_key: String,
    new_password: String,
) -> Result<String, String> {
    if new_password.trim().is_empty() {
        return Err(crate::errors::codes::MASTER_EMPTY_PASSWORD.to_string());
    }
    let api = client(&endpoint_of(&state).await)?;

    let raw = crate::recovery_key::parse(&recovery_key)
        .ok_or_else(|| crate::errors::codes::MASTER_RECOVERY_MALFORMED.to_string())?;
    let recovery = keys::recovery_keys(&raw).map_err(|e| format!("{e}"))?;

    let begun = api
        .recover_begin(&email, &recovery.auth_hash)
        .await
        .map_err(|e| sync_error_code(&e))?;

    // Достаём ключ записей из обёртки, снимаемой бумагой. Не открылась —
    // ключ не тот; сервер этого не знает и знать не должен.
    let symmetric = keys::open_from_wire(&recovery.enc, &begun.protected_symmetric_key_rk)
        .map_err(|_| crate::errors::codes::MASTER_RECOVERY_WRONG.to_string())?;
    let symmetric: [u8; 32] = symmetric
        .try_into()
        .map_err(|_| crate::errors::codes::SYNC_PROTECTED_KEY_UNREADABLE.to_string())?;

    // Новый пароль и новая бумажка — считаем всё до отправки, чтобы запрос
    // ушёл один.
    let master = keys::master_key(&email, &new_password).map_err(|e| format!("{e}"))?;
    let new_auth = keys::auth_hash(&master, &new_password).map_err(|e| format!("{e}"))?;
    let new_enc = keys::encryption_key(&master).map_err(|e| format!("{e}"))?;
    let new_protected = keys::seal_for_wire(&new_enc, &symmetric).map_err(|e| format!("{e}"))?;

    let next_raw = crate::recovery_key::generate()?;
    let next = keys::recovery_keys(&next_raw).map_err(|e| format!("{e}"))?;
    let next_protected = keys::seal_for_wire(&next.enc, &symmetric).map_err(|e| format!("{e}"))?;

    api.recover_complete(
        &begun.recovery_token,
        &new_auth,
        &new_protected,
        &next.auth_hash,
        &next_protected,
    )
    .await
    .map_err(|e| sync_error_code(&e))?;

    // Все сессии отозваны, включая здешнюю: пароль сменился, входить заново
    // обязаны все. Забываем вход, чтобы приложение не делало вид, что он есть.
    state.sync.clear().await;
    save_setting(&state, EMAIL_KEY, &email).await?;

    info!(target: "sync", "доступ восстановлен, выпущен новый ключ");
    Ok(crate::recovery_key::format(&next_raw))
}

#[tauri::command]
pub async fn sync_logout(state: State<'_, AppState>) -> Result<(), String> {
    if let Some(active) = state.sync.current().await {
        if let Ok(api) = client(&endpoint_of(&state).await) {
            // Отказ сервера не должен мешать выйти локально: человек нажал
            // «выйти», и после этого на его машине секретов быть не должно
            // независимо от того, что ответила сеть.
            if let Err(e) = api.logout(&active.refresh_token).await {
                warn!(target: "sync", "сервер не подтвердил выход: {e}");
            }
        }
    }
    state.sync.clear().await;
    Ok(())
}

/// Выполнить проход синхронизации.
#[tauri::command]
pub async fn sync_now(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<engine::SyncReport, String> {
    let outcome = sync_now_inner(&state).await;
    announce(&app, &state).await;
    outcome
}

async fn sync_now_inner(state: &AppState) -> Result<engine::SyncReport, String> {
    let Some(active) = state.sync.current().await else {
        return Err("вход в аккаунт не выполнен".to_string());
    };
    let api = client(&endpoint_of(state).await)?;

    // Токен доступа живёт минуты, поэтому одна попытка обновления — обычный
    // ход событий, а не восстановление после сбоя.
    let access =
        match engine::run(&state.db, &active.record_keys(), &api, &active.access_token).await {
            Ok(report) => {
                crate::sync::record_sync_success(&state.db)
                    .await
                    .map_err(|e| format!("{e}"))?;
                return Ok(report);
            }
            Err(e) => match e.downcast_ref::<crate::sync::api::ApiError>() {
                Some(api_err) if api_err.needs_refresh() => {
                    let refreshed = api
                        .refresh(&active.refresh_token)
                        .await
                        .map_err(|e| format!("{e}"))?;
                    state
                        .sync
                        .refreshed(refreshed.access_token.clone(), refreshed.refresh_token)
                        .await;
                    refreshed.access_token
                }
                _ => {
                    let code = sync_error_code(&e);
                    // Ошибку запоминаем, но она не мешает работать дальше.
                    let _ = crate::sync::record_sync_failure(&state.db, &code).await;
                    return Err(code);
                }
            },
        };

    match engine::run(&state.db, &active.record_keys(), &api, &access).await {
        Ok(report) => {
            crate::sync::record_sync_success(&state.db)
                .await
                .map_err(|e| format!("{e}"))?;
            Ok(report)
        }
        Err(e) => {
            let code = sync_error_code(&e);
            let _ = crate::sync::record_sync_failure(&state.db, &code).await;
            Err(code)
        }
    }
}

/// Надо ли спросить, что делать с уже накопленными записями.
#[tauri::command]
pub async fn sync_first_run_question(state: State<'_, AppState>) -> Result<bool, String> {
    let local = crate::sync::local_record_count(&state.db)
        .await
        .map_err(|e| format!("{e}"))?;
    let cursor = crate::sync::cursor(&state.db)
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(crate::sync::should_ask_before_first_sync(local, cursor))
}

#[tauri::command]
pub async fn sync_apply_first_choice(
    state: State<'_, AppState>,
    choice: crate::sync::FirstSyncChoice,
) -> Result<i64, String> {
    crate::sync::apply_first_sync_choice(&state.db, choice)
        .await
        .map_err(|e| format!("{e}"))
}
