//! Команды мастер-пароля (#111–#113).
//!
//! Порядок операций при включении/выключении важен: секреты переносятся между
//! keyring и зашифрованным хранилищем **после** смены режима, потому что
//! направление переноса определяется именно им.

use crate::master::MasterStatus;
use crate::secrets;
use crate::AppState;
use tauri::State;
use tracing::info;

/// Ошибка наружу уходит кодом, причина — в журнал (#177).
fn map_err<E: std::fmt::Display>(e: E) -> String {
    crate::errors::passthrough("master", e)
}

#[tauri::command]
pub async fn master_status(state: State<'_, AppState>) -> Result<MasterStatus, String> {
    state.master.status(&state.db).await.map_err(map_err)
}

/// Включить мастер-пароль и перенести секреты из keyring в хранилище.
#[tauri::command]
pub async fn master_enable(state: State<'_, AppState>, password: String) -> Result<usize, String> {
    state
        .master
        .enable(&state.db, &password)
        .await
        .map_err(map_err)?;
    let moved = secrets::migrate(&state.db, &state.master, true).await?;
    info!(target: "master", "мастер-пароль включён, перенесено секретов: {moved}");
    Ok(moved)
}

/// Выключить: вернуть секреты в keyring, затем стереть хранилище.
///
/// Перенос идёт до `disable`, пока ключ ещё в памяти — после выключения
/// расшифровать содержимое будет уже нечем.
#[tauri::command]
pub async fn master_disable(
    state: State<'_, AppState>,
    password: String,
    code: Option<String>,
) -> Result<usize, String> {
    state
        .master
        .unlock(&state.db, &password, code.as_deref())
        .await
        .map_err(map_err)?;
    let moved = secrets::migrate(&state.db, &state.master, false).await?;
    state
        .master
        .disable(&state.db, &password)
        .await
        .map_err(map_err)?;
    info!(target: "master", "мастер-пароль выключен, возвращено в keyring: {moved}");
    Ok(moved)
}

#[tauri::command]
pub async fn master_unlock(
    state: State<'_, AppState>,
    password: String,
    code: Option<String>,
) -> Result<(), String> {
    state
        .master
        .unlock(&state.db, &password, code.as_deref())
        .await
        .map_err(map_err)
}

// ────────────────────────────────────────────────────────────────────
// Второй фактор на вход в аккаунт (#153b)
// ────────────────────────────────────────────────────────────────────

/// Черновик второго фактора: секрет и ссылка для аутентификатора.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TotpSetup {
    pub secret: String,
    /// `otpauth://` — то же, что кодируют в QR.
    pub uri: String,
}

/// Сгенерировать секрет. **Пока не сохраняем**: сперва человек должен
/// доказать, что его аутентификатор выдаёт те же коды, иначе он запрёт
/// себя снаружи собственного хранилища.
#[tauri::command]
pub async fn master_totp_begin(state: State<'_, AppState>) -> Result<TotpSetup, String> {
    if !state.master.is_unlocked().await {
        return Err("сначала разблокируйте хранилище".to_string());
    }
    let secret = crate::totp::generate_secret().map_err(map_err)?;
    let uri = crate::totp::build_uri(&secret, "Darwaza");
    Ok(TotpSetup { secret, uri })
}

/// Подтвердить код и включить второй фактор.
#[tauri::command]
pub async fn master_totp_confirm(
    state: State<'_, AppState>,
    secret: String,
    code: String,
) -> Result<(), String> {
    if !state.master.is_unlocked().await {
        return Err("сначала разблокируйте хранилище".to_string());
    }
    let config = crate::totp::parse_input(&secret).map_err(map_err)?;
    if !crate::totp::verify(&config, &code).map_err(map_err)? {
        return Err("код не совпал — проверьте время на телефоне и попробуйте ещё раз".to_string());
    }
    state
        .master
        .set_totp(&state.db, &config)
        .await
        .map_err(map_err)?;
    info!(target: "master", "второй фактор включён");
    Ok(())
}

/// Выключить второй фактор. Требует кода: иначе снять защиту мог бы любой,
/// кто подошёл к открытому окну.
#[tauri::command]
pub async fn master_totp_disable(state: State<'_, AppState>, code: String) -> Result<(), String> {
    if !state.master.is_unlocked().await {
        return Err("сначала разблокируйте хранилище".to_string());
    }
    let raw = state
        .master
        .load_secret(&state.db, crate::master::TOTP_KIND, crate::master::TOTP_ID)
        .await
        .map_err(map_err)?
        .ok_or("второй фактор не включён")?;
    let config: crate::totp::TotpConfig = serde_json::from_str(&raw).map_err(map_err)?;
    if !crate::totp::verify(&config, &code).map_err(map_err)? {
        return Err("неверный код".to_string());
    }
    state.master.clear_totp(&state.db).await.map_err(map_err)?;
    info!(target: "master", "второй фактор выключен");
    Ok(())
}

#[tauri::command]
pub async fn master_lock(state: State<'_, AppState>) -> Result<(), String> {
    state.master.lock().await;
    Ok(())
}

#[tauri::command]
pub async fn master_change_password(
    state: State<'_, AppState>,
    old_password: String,
    new_password: String,
) -> Result<(), String> {
    state
        .master
        .change_password(&state.db, &old_password, &new_password)
        .await
        .map_err(map_err)
}

/// Выпустить ключ восстановления (#180b).
///
/// Возвращает ключ ровно один раз: в базе лежит только его обёртка, и узнать
/// его заново неоткуда. Прежний ключ этим же вызовом перестаёт работать.
#[tauri::command]
pub async fn master_recovery_issue(state: State<'_, AppState>) -> Result<String, String> {
    let key = state
        .master
        .issue_recovery_key(&state.db)
        .await
        .map_err(map_err)?;
    // В журнал — факт выпуска, но никогда не сам ключ.
    info!(target: "master", "выпущен ключ восстановления, прежний отозван");
    Ok(key)
}

/// Восстановить доступ ключом восстановления и задать новый пароль (#180b).
///
/// Возвращает **новый** ключ взамен использованного — его тоже показывают один
/// раз.
#[tauri::command]
pub async fn master_recover(
    state: State<'_, AppState>,
    recovery_key: String,
    new_password: String,
) -> Result<String, String> {
    let replacement = state
        .master
        .recover(&state.db, &recovery_key, &new_password)
        .await
        .map_err(map_err)?;
    info!(target: "master", "доступ восстановлен ключом, пароль и ключ заменены");
    Ok(replacement)
}

/// Где секреты лежат прямо сейчас (#169, #171).
///
/// Один ответ на два вопроса: форма спрашивает его до того, как человек начал
/// набирать пароль, а настройки показывают то же самое постоянно. Раньше
/// узнать это можно было только одним способом — попробовать сохранить и
/// прочитать техническую ошибку.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretsPlace {
    /// Есть ли на машине рабочее системное хранилище.
    pub keyring_available: bool,
    /// Включён ли мастер-пароль.
    pub master_enabled: bool,
    /// Разблокирован ли он сейчас.
    pub master_unlocked: bool,
    /// Куда пойдёт следующий сохранённый секрет:
    /// `master` | `keyring` | `nowhere`.
    ///
    /// `nowhere` — единственный случай, требующий действия человека: сохранять
    /// секрет некуда, и молчать об этом до нажатия «Сохранить» нельзя.
    pub target: &'static str,
}

/// Куда пойдёт следующий секрет.
///
/// Вынесено отдельной функцией не ради красоты: это три ветки, от которых
/// зависит, увидит ли человек предупреждение вовремя. Внутри команды Tauri их
/// не проверить — нужен `State`, — и они оставались непокрытыми.
///
/// Мастер-пароль главнее найденного хранилища намеренно: включив его, человек
/// сделал выбор, и наличие KWallet этот выбор не отменяет.
fn secrets_target(master_enabled: bool, keyring_available: bool) -> &'static str {
    match (master_enabled, keyring_available) {
        (true, _) => "master",
        (false, true) => "keyring",
        (false, false) => "nowhere",
    }
}

#[tauri::command]
pub async fn secrets_place(state: State<'_, AppState>) -> Result<SecretsPlace, String> {
    let status = state.master.status(&state.db).await.map_err(map_err)?;
    let keyring_available = crate::keyring_store::is_available();

    let target = secrets_target(status.enabled, keyring_available);

    Ok(SecretsPlace {
        keyring_available,
        master_enabled: status.enabled,
        master_unlocked: status.unlocked,
        target,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Все четыре сочетания, а не одно.
    ///
    /// От этой развилки зависит, скажут ли человеку «сохранять некуда» до
    /// того, как он наберёт пароль, — а не после «Сохранить», как было.
    #[test]
    fn every_combination_of_storage_has_an_answer() {
        // Мастер-пароль включён — он и есть хранилище, независимо от сейфа.
        assert_eq!(secrets_target(true, true), "master");
        assert_eq!(secrets_target(true, false), "master");

        // Мастер-пароль выключен: решает наличие системного сейфа.
        assert_eq!(secrets_target(false, true), "keyring");

        // Единственный случай, требующий действия человека.
        assert_eq!(secrets_target(false, false), "nowhere");
    }

    /// Включённый мастер-пароль не отменяется найденным сейфом.
    ///
    /// Отдельной проверкой, потому что соблазн «раз сейф есть, писать в него»
    /// выглядит разумным и молча отменил бы выбор человека: секреты уехали бы
    /// в хранилище, доступное любой программе его сессии.
    #[test]
    fn a_found_keyring_does_not_override_the_master_password() {
        assert_eq!(secrets_target(true, true), secrets_target(true, false));
    }
}
