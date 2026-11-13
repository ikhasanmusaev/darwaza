use russh_sftp::client::SftpSession;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{oneshot, Mutex, RwLock};
use uuid::Uuid;

use super::session::{SessionHandle, SessionStatus};

/// Контекст ожидающего host-key prompt'а. Хранится в `SshManager` до тех пор,
/// пока UI не пришлёт accept/reject через `ssh_accept_host_key`/`ssh_reject_host_key`.
pub struct PendingHostKeyPrompt {
    pub hostname: String,
    pub key_type: String,
    pub fingerprint: String,
    pub responder: oneshot::Sender<bool>,
}

/// Ожидающий ответа запрос аутентификации (#153).
///
/// Сервер с 2FA присылает набор вопросов («Verification code:»), на которые
/// отвечает человек. Ждём его ответа так же, как ждём решения по host-ключу.
pub struct PendingAuthPrompt {
    pub responder: oneshot::Sender<Option<Vec<String>>>,
}

#[derive(Clone, Default)]
pub struct SshManager {
    sessions: Arc<RwLock<HashMap<Uuid, SessionHandle>>>,
    pending_prompts: Arc<Mutex<HashMap<Uuid, PendingHostKeyPrompt>>>,
    pending_auth: Arc<Mutex<HashMap<Uuid, PendingAuthPrompt>>>,
    /// Открытые SFTP-подсистемы, по одной на сессию (#120).
    ///
    /// Кешируем: открытие канала + рукопожатие стоят round-trip, а файловый
    /// браузер дёргает операции пачками. `Arc`, потому что все методы
    /// `SftpSession` берут `&self` и её можно держать за пределами локов.
    sftp: Arc<Mutex<HashMap<Uuid, Arc<SftpSession>>>>,
}

impl SshManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn insert(&self, id: Uuid, handle: SessionHandle) {
        self.sessions.write().await.insert(id, handle);
    }

    pub async fn remove(&self, id: Uuid) {
        self.sessions.write().await.remove(&id);
        // На всякий случай — если сессия умерла с висящим prompt'ом, вернуть false.
        if let Some(prompt) = self.pending_prompts.lock().await.remove(&id) {
            let _ = prompt.responder.send(false);
        }
        // Висящий запрос кода отменяем: отвечать уже некому.
        if let Some(auth) = self.pending_auth.lock().await.remove(&id) {
            let _ = auth.responder.send(None);
        }
        // SFTP-подсистема живёт поверх той же SSH-сессии: раз сессии нет,
        // держать её бессмысленно — иначе следующее подключение к тому же
        // хосту получит из кеша мёртвый канал.
        self.sftp.lock().await.remove(&id);
    }

    /// Взять уже открытую SFTP-подсистему сессии, если она есть.
    pub async fn get_sftp(&self, id: Uuid) -> Option<Arc<SftpSession>> {
        self.sftp.lock().await.get(&id).cloned()
    }

    /// Положить открытую SFTP-подсистему в кеш.
    ///
    /// Если параллельный вызов успел положить свою — возвращаем ту, что уже
    /// в кеше, а свою отдаём вызывающему на выброс. Так оба вызова работают
    /// с одной подсистемой, и лишний канал не остаётся висеть незамеченным.
    pub async fn put_sftp(&self, id: Uuid, sftp: Arc<SftpSession>) -> Arc<SftpSession> {
        let mut guard = self.sftp.lock().await;
        guard.entry(id).or_insert(sftp).clone()
    }

    /// Убрать SFTP-подсистему из кеша (например, после ошибки транспорта).
    pub async fn drop_sftp(&self, id: Uuid) -> Option<Arc<SftpSession>> {
        self.sftp.lock().await.remove(&id)
    }

    pub async fn set_status(&self, id: Uuid, status: SessionStatus) {
        if let Some(h) = self.sessions.write().await.get_mut(&id) {
            h.status = status;
        }
    }

    pub async fn send_cmd(&self, id: Uuid, cmd: super::session::SessionCmd) -> Result<(), String> {
        // Отправитель берём копией и замок отпускаем до отправки: если очередь
        // этой сессии полна, ждать должна только она, а не все вкладки разом.
        let cmd_tx = self
            .sessions
            .read()
            .await
            .get(&id)
            .map(|handle| handle.cmd_tx.clone())
            .ok_or_else(|| format!("session {id} not found"))?;
        cmd_tx
            .send(cmd)
            .await
            .map_err(|e| format!("session channel closed: {e}"))
    }

    /// Зарегистрировать ожидающий host-key prompt. Возвращает Err если для
    /// этой сессии уже есть незакрытый prompt.
    pub async fn register_prompt(
        &self,
        id: Uuid,
        prompt: PendingHostKeyPrompt,
    ) -> Result<(), String> {
        let mut guard = self.pending_prompts.lock().await;
        if guard.contains_key(&id) {
            return Err(format!("session {id} already has pending host-key prompt"));
        }
        guard.insert(id, prompt);
        Ok(())
    }

    /// Зарегистрировать ожидание ответа на запрос аутентификации.
    pub async fn register_auth_prompt(
        &self,
        id: Uuid,
        prompt: PendingAuthPrompt,
    ) -> Result<(), String> {
        let mut guard = self.pending_auth.lock().await;
        if guard.contains_key(&id) {
            return Err(format!("session {id} already waits for an auth response"));
        }
        guard.insert(id, prompt);
        Ok(())
    }

    pub async fn take_auth_prompt(&self, id: Uuid) -> Option<PendingAuthPrompt> {
        self.pending_auth.lock().await.remove(&id)
    }

    /// Извлечь pending prompt для отправки решения (accept/reject).
    pub async fn take_prompt(&self, id: Uuid) -> Option<PendingHostKeyPrompt> {
        self.pending_prompts.lock().await.remove(&id)
    }

    pub async fn snapshot(&self) -> Vec<SessionSnapshot> {
        self.sessions
            .read()
            .await
            .iter()
            // Соединения экрана SFTP — не вкладки: после перезагрузки окна
            // восстанавливать их терминалом нельзя.
            .filter(|(_, h)| !h.files_only)
            .map(|(id, h)| SessionSnapshot {
                session_id: *id,
                host_id: h.host_id,
                status: h.status,
            })
            .collect()
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    pub session_id: Uuid,
    pub host_id: i64,
    pub status: SessionStatus,
}
