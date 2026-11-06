//! Фоновые передачи файлов с прогрессом и отменой (#124).
//!
//! Раньше загрузка и скачивание были синхронными: команда возвращалась только
//! по завершении, и на большом файле UI просто ждал молча. Для перетаскивания
//! это не годится — жест заканчивается сразу, а передача идёт дальше и должна
//! быть видимой.
//!
//! Поэтому передача запускается фоновой задачей, отдаёт `id` немедленно и
//! шлёт события прогресса. Отмена — через флаг, который задача проверяет
//! на каждом куске.

use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TransferKind {
    Upload,
    Download,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum TransferStatus {
    Running,
    Done,
    Failed,
    Cancelled,
}

/// Событие прогресса, уходящее в UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferProgress {
    pub id: Uuid,
    pub kind: TransferKind,
    /// Имя файла — его и показывает панель передач.
    pub name: String,
    pub transferred: u64,
    /// Полный размер, если известен. У некоторых источников его нет.
    pub total: Option<u64>,
    pub status: TransferStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Куда легло — чтобы UI мог показать путь после скачивания.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
}

struct TransferHandle {
    cancel: Arc<AtomicBool>,
}

/// Реестр активных передач.
#[derive(Clone, Default)]
pub struct TransferManager {
    active: Arc<Mutex<HashMap<Uuid, TransferHandle>>>,
}

impl TransferManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Зарегистрировать передачу, получив флаг отмены для её задачи.
    pub async fn register(&self, id: Uuid) -> Arc<AtomicBool> {
        let cancel = Arc::new(AtomicBool::new(false));
        self.active.lock().await.insert(
            id,
            TransferHandle {
                cancel: cancel.clone(),
            },
        );
        cancel
    }

    pub async fn finish(&self, id: Uuid) {
        self.active.lock().await.remove(&id);
    }

    /// Попросить передачу остановиться. `false` — такой передачи уже нет.
    pub async fn cancel(&self, id: Uuid) -> bool {
        match self.active.lock().await.get(&id) {
            Some(handle) => {
                handle.cancel.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    pub async fn cancel_all(&self) {
        for handle in self.active.lock().await.values() {
            handle.cancel.store(true, Ordering::Relaxed);
        }
    }
}

/// Как часто слать прогресс.
///
/// На чанк по 32 КиБ файл в 500 МБ дал бы шестнадцать тысяч событий — WebView
/// захлебнётся, а пользователь всё равно не различит. Шлём по времени.
pub const PROGRESS_INTERVAL_MS: u128 = 120;

/// Нужно ли слать событие: прошёл интервал или передача завершилась.
pub fn should_emit(elapsed_ms: u128, finished: bool) -> bool {
    finished || elapsed_ms >= PROGRESS_INTERVAL_MS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancel_sets_the_flag_for_a_running_transfer() {
        let manager = TransferManager::new();
        let id = Uuid::new_v4();
        let flag = manager.register(id).await;

        assert!(!flag.load(Ordering::Relaxed));
        assert!(manager.cancel(id).await, "передача должна найтись");
        assert!(flag.load(Ordering::Relaxed), "флаг отмены должен встать");
    }

    #[tokio::test]
    async fn cancelling_unknown_transfer_reports_false() {
        let manager = TransferManager::new();
        assert!(!manager.cancel(Uuid::new_v4()).await);
    }

    #[tokio::test]
    async fn finished_transfer_can_no_longer_be_cancelled() {
        // Иначе отмена уже завершённой передачи выглядела бы успешной,
        // а UI показал бы «отменено» поверх готового файла.
        let manager = TransferManager::new();
        let id = Uuid::new_v4();
        manager.register(id).await;
        manager.finish(id).await;
        assert!(!manager.cancel(id).await);
    }

    #[test]
    fn progress_is_throttled_but_never_swallows_the_final_event() {
        assert!(!should_emit(10, false), "частые события давят WebView");
        assert!(should_emit(PROGRESS_INTERVAL_MS, false));
        assert!(
            should_emit(0, true),
            "завершение должно доехать независимо от таймера"
        );
    }
}
