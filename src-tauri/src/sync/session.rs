//! Состояние входа в аккаунт.
//!
//! Живёт в памяти приложения, а не в базе: **ключ шифрования не должен
//! попадать в файл, который уезжает вместе с резервной копией.** Токены и ключ
//! складываются в системный keyring — там же, где пароли к серверам, и с той
//! же защитой: доступ к разблокированному сеансу означает доступ к ним.
//!
//! Из этого следует ограничение, которое лучше назвать вслух: после
//! перезапуска приложение восстанавливает вход **без** ввода пароля. Это
//! сознательный выбор в пользу того, как уже устроены остальные секреты, а не
//! недосмотр. Кто хочет строже — включает мастер-пароль, и тогда keyring
//! открывается только после него.

use std::sync::Arc;
use tokio::sync::RwLock;

/// Где в keyring лежат данные аккаунта. Номер один и тот же: аккаунт
/// единственный.
const KIND_REFRESH: &str = "sync_refresh";
const KIND_KEY: &str = "sync_key";
/// Ключ, которым записи шифровались до перехода на `symmetricKey` (#180c).
/// Нужен, чтобы отправленное раньше продолжало читаться.
const KIND_LEGACY_KEY: &str = "sync_key_legacy";
const ONLY: i64 = 0;

#[derive(Clone)]
pub struct Active {
    pub email: String,
    pub access_token: String,
    pub refresh_token: String,
    /// Ключ шифрования записей — `symmetricKey` с сервера. Из него нельзя
    /// получить пароль.
    pub key: [u8; 32],
    /// Ключ, которым записи шифровались до 2026-09-22 (#180c): выведенный из
    /// пароля. Только для чтения старого, никогда для записи.
    pub legacy_key: Option<[u8; 32]>,
}

impl Active {
    /// Ключи в том виде, в каком их ждёт проход синхронизации.
    pub fn record_keys(&self) -> crate::sync::RecordKeys {
        crate::sync::RecordKeys {
            current: self.key,
            legacy: self.legacy_key,
        }
    }
}

#[derive(Clone, Default)]
pub struct SyncSession {
    inner: Arc<RwLock<Option<Active>>>,
}

impl SyncSession {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn current(&self) -> Option<Active> {
        self.inner.read().await.clone()
    }

    pub async fn is_signed_in(&self) -> bool {
        self.inner.read().await.is_some()
    }

    /// Запомнить вход и сохранить в keyring.
    ///
    /// Неудача записи в keyring **не отменяет вход**: работать в этом сеансе
    /// человек сможет, просто после перезапуска придётся войти снова. Отказать
    /// во входе из-за недоступного хранилища было бы хуже.
    pub async fn set(&self, active: Active) {
        if let Err(e) = crate::keyring_store::set(KIND_REFRESH, ONLY, &active.refresh_token) {
            tracing::warn!(target: "sync", "не удалось сохранить токен: {e}");
        }
        match active.legacy_key {
            Some(legacy) => {
                if let Err(e) = crate::keyring_store::set(KIND_LEGACY_KEY, ONLY, &hex(&legacy)) {
                    tracing::warn!(target: "sync", "не удалось сохранить прежний ключ: {e}");
                }
            }
            // Прежнего ключа нет — убираем и след от предыдущего входа:
            // иначе чужой старый ключ пережил бы смену аккаунта.
            None => {
                let _ = crate::keyring_store::delete(KIND_LEGACY_KEY, ONLY);
            }
        }
        if let Err(e) = crate::keyring_store::set(KIND_KEY, ONLY, &hex(&active.key)) {
            tracing::warn!(target: "sync", "не удалось сохранить ключ: {e}");
        }
        *self.inner.write().await = Some(active);
    }

    /// Обновить токены, не трогая ключ и адрес.
    pub async fn refreshed(&self, access: String, refresh: String) {
        let mut guard = self.inner.write().await;
        if let Some(active) = guard.as_mut() {
            active.access_token = access.clone();
            active.refresh_token = refresh.clone();
        }
        drop(guard);
        if let Err(e) = crate::keyring_store::set(KIND_REFRESH, ONLY, &refresh) {
            tracing::warn!(target: "sync", "не удалось обновить токен: {e}");
        }
    }

    /// Выйти: забыть всё и здесь, и в keyring.
    pub async fn clear(&self) {
        *self.inner.write().await = None;
        let _ = crate::keyring_store::delete(KIND_REFRESH, ONLY);
        let _ = crate::keyring_store::delete(KIND_KEY, ONLY);
        let _ = crate::keyring_store::delete(KIND_LEGACY_KEY, ONLY);
    }

    /// Восстановить вход после перезапуска.
    ///
    /// Возвращает `false`, если восстанавливать нечего. Отсутствие данных —
    /// обычное дело: человек мог не заводить аккаунт вовсе.
    pub async fn restore(&self, email: String) -> bool {
        let Ok(Some(refresh)) = crate::keyring_store::get(KIND_REFRESH, ONLY) else {
            return false;
        };
        let Ok(Some(key_hex)) = crate::keyring_store::get(KIND_KEY, ONLY) else {
            return false;
        };
        let Some(key) = unhex(&key_hex) else {
            tracing::warn!(target: "sync", "сохранённый ключ испорчен, нужен повторный вход");
            return false;
        };

        // Прежний ключ может и отсутствовать — его нет у аккаунтов,
        // заведённых после перехода. Испорченную запись просто не берём:
        // отказывать во входе из-за нечитаемого ключа от старых записей
        // было бы хуже, чем не прочитать эти записи.
        let legacy_key = match crate::keyring_store::get(KIND_LEGACY_KEY, ONLY) {
            Ok(Some(hex)) => unhex(&hex),
            _ => None,
        };

        *self.inner.write().await = Some(Active {
            email,
            // Токен доступа живёт минуты, хранить его незачем: получим
            // обновлением по refresh при первом же обращении.
            access_token: String::new(),
            refresh_token: refresh,
            key,
            legacy_key,
        });
        true
    }
}

fn hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, chunk) in text.as_bytes().chunks(2).enumerate() {
        let pair = std::str::from_utf8(chunk).ok()?;
        out[i] = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_survives_the_round_trip_through_text() {
        let key = [
            0u8, 1, 15, 16, 127, 128, 254, 255, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 17, 18,
            19, 20, 21, 22, 23, 24, 25, 26, 27, 28,
        ];
        assert_eq!(unhex(&hex(&key)), Some(key));
    }

    #[test]
    fn a_damaged_key_is_refused_instead_of_being_padded() {
        // Молча дополнить нулями значило бы получить ключ, которым ничего не
        // расшифруется, и искать причину в другом месте.
        assert_eq!(unhex(""), None);
        assert_eq!(unhex("abcd"), None);
        assert_eq!(unhex(&"z".repeat(64)), None);
    }

    #[tokio::test]
    async fn a_fresh_session_is_signed_out() {
        let session = SyncSession::new();
        assert!(!session.is_signed_in().await);
        assert!(session.current().await.is_none());
    }
}
