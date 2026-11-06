//! Синхронизация по расписанию.
//!
//! Нажимать кнопку — не работа человека. Но и синхронизировать на каждое
//! нажатие клавиши нельзя: правка хоста идёт полями, и отправка после каждого
//! означала бы десяток запросов на одно осмысленное изменение.
//!
//! Поэтому две причины для прохода, а не одна: **накопились изменения** и
//! **прошло время**. Первая закрывает случай «поправил и ушёл на другую
//! машину», вторая — «на другой машине кто-то поправил, а я сижу и не знаю».
//!
//! Отказ синхронизации **никогда не мешает работать**. Он запоминается,
//! показывается ненавязчиво в настройках и удлиняет паузу до следующей
//! попытки. Приложение остаётся полностью рабочим без сети — это принцип.

use std::time::Duration;

/// Как часто заглядывать, есть ли что делать.
///
/// Полминуты — не частота синхронизации, а частота **проверки**. Сам проход
/// случается заметно реже, по правилам ниже.
pub const TICK: Duration = Duration::from_secs(30);

/// Сколько ждать после последнего изменения, прежде чем отправлять.
///
/// Правка идёт полями, и отправка после каждого дала бы десяток запросов на
/// одно осмысленное изменение. Пять секунд — дольше, чем пауза между двумя
/// нажатиями, и короче, чем время до переключения на другую машину.
pub const QUIET_PERIOD: Duration = Duration::from_secs(5);

/// Через сколько синхронизироваться, даже если сами ничего не меняли.
///
/// Нужно, чтобы узнать о чужих правках. Пять минут — незаметно для человека и
/// не назойливо для сервера.
pub const IDLE_INTERVAL: Duration = Duration::from_secs(300);

/// Предел паузы после неудач.
///
/// Без предела отключённая на ночь сеть отодвинула бы следующую попытку на
/// сутки, и утром человек обнаружил бы, что ничего не синхронизируется.
pub const MAX_BACKOFF: Duration = Duration::from_secs(1800);

/// Пауза после `failures` неудач подряд.
///
/// Удвоение, но с потолком: сервер, лежащий десять минут, не должен получить
/// шквал запросов, а человек не должен ждать сутки после того, как сеть
/// вернулась.
pub fn backoff(failures: u32) -> Duration {
    if failures == 0 {
        return Duration::ZERO;
    }
    let seconds = IDLE_INTERVAL
        .as_secs()
        .saturating_mul(1u64 << failures.min(6));
    Duration::from_secs(seconds.min(MAX_BACKOFF.as_secs()))
}

/// Почему стоит синхронизироваться прямо сейчас.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Reason {
    /// Накопились неотправленные изменения, и правки затихли.
    Pending,
    /// Давно не заглядывали — у соседа могло что-то поменяться.
    Idle,
}

/// Решить, пора ли.
///
/// Отдельная чистая функция намеренно: правила расписания — это то, что
/// проверяется тестом, а не наблюдением за приложением в течение часа.
pub fn due(
    pending: i64,
    since_change: Duration,
    since_sync: Duration,
    failures: u32,
) -> Option<Reason> {
    // Пауза после неудач важнее любых причин: иначе при недоступном сервере
    // мы бы долбились каждые полминуты.
    if since_sync < backoff(failures) {
        return None;
    }

    if pending > 0 && since_change >= QUIET_PERIOD {
        return Some(Reason::Pending);
    }
    if since_sync >= IDLE_INTERVAL {
        return Some(Reason::Idle);
    }
    None
}

// ─────────────────────────────────────────────────────────────────────────
// Фоновая задача
// ─────────────────────────────────────────────────────────────────────────

use crate::AppState;
use tauri::{AppHandle, Emitter, Manager};
use tokio::time::Instant;
use tracing::{info, warn};

/// Событие для окна: проход завершился. Интерфейс обновляет состояние сам,
/// а не опрашивает его по таймеру.
pub const SYNC_EVENT: &str = "sync://done";

/// Ключ настройки: включена ли синхронизация по расписанию.
pub const AUTO_KEY: &str = "sync.auto";

/// Запустить фоновую синхронизацию.
///
/// **Отказ здесь не мешает приложению работать.** Синхронизация — дополнение,
/// а не условие: человек без аккаунта или без сети должен спокойно
/// подключаться к своим серверам.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut failures: u32 = 0;
        let mut last_sync = Instant::now();
        let mut last_pending: i64 = 0;
        let mut last_change = Instant::now();

        loop {
            tokio::time::sleep(TICK).await;

            let state = app.state::<AppState>();
            if !state.sync.is_signed_in().await {
                continue;
            }
            if !auto_enabled(&state).await {
                continue;
            }

            let pending = match crate::sync::health(&state.db).await {
                Ok(health) => health.pending,
                Err(e) => {
                    warn!(target: "sync", "не удалось прочитать состояние: {e}");
                    continue;
                }
            };

            // Момент последней правки отслеживаем по изменению счётчика: он
            // растёт ровно тогда, когда что-то изменили. Отдельного сигнала от
            // команд не заводим — его пришлось бы не забыть в каждой из них, а
            // забытый означал бы, что правки этого рода не уезжают.
            if pending != last_pending {
                last_change = Instant::now();
                last_pending = pending;
            }

            let reason = due(
                pending,
                last_change.elapsed(),
                last_sync.elapsed(),
                failures,
            );
            let Some(reason) = reason else { continue };

            match crate::commands::sync_now(app.clone(), state.clone()).await {
                Ok(report) => {
                    failures = 0;
                    last_sync = Instant::now();
                    // Удачный проход отправил всё, что ждало. Если за это
                    // время появилось новое, следующая проверка увидит рост с
                    // нуля, отсчитает паузу на затихание и отправит.
                    last_pending = 0;
                    info!(
                        target: "sync",
                        "проход по расписанию ({reason:?}): принято {}, отправлено {}",
                        report.received, report.sent
                    );
                    // Ошибку отправки события глушим: некому показать — не
                    // повод считать синхронизацию неудавшейся.
                    let _ = app.emit(SYNC_EVENT, &report);
                }
                Err(e) => {
                    failures = failures.saturating_add(1);
                    last_sync = Instant::now();
                    // Тихо: отказ синхронизации не должен прерывать работу.
                    // Он виден в настройках и удлиняет паузу до следующей
                    // попытки.
                    warn!(target: "sync", "проход по расписанию не удался ({failures}): {e}");
                }
            }
        }
    });
}

/// Включена ли автоматическая синхронизация.
///
/// По умолчанию **да**: человек, заведший аккаунт, ожидает, что оно работает
/// само. Кому нужно иначе — выключает.
async fn auto_enabled(state: &AppState) -> bool {
    let value: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?1")
        .bind(AUTO_KEY)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    value.as_deref() != Some("0")
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEVER: Duration = Duration::from_secs(86_400);

    #[test]
    fn nothing_to_do_means_no_sync() {
        assert_eq!(due(0, NEVER, Duration::from_secs(10), 0), None);
    }

    #[test]
    fn changes_wait_for_the_typing_to_stop() {
        // Правка идёт полями: отправлять после каждого — десяток запросов на
        // одно осмысленное изменение.
        assert_eq!(
            due(3, Duration::from_secs(1), Duration::from_secs(10), 0),
            None
        );
        assert_eq!(
            due(3, QUIET_PERIOD, Duration::from_secs(10), 0),
            Some(Reason::Pending)
        );
    }

    #[test]
    fn we_look_for_other_devices_changes_even_with_nothing_of_our_own() {
        // Иначе о правке с другой машины человек узнал бы только нажав кнопку.
        assert_eq!(due(0, NEVER, IDLE_INTERVAL, 0), Some(Reason::Idle));
    }

    #[test]
    fn after_a_failure_we_wait_longer_instead_of_hammering() {
        // Недоступный сервер не должен получать запрос каждые полминуты.
        assert_eq!(due(5, NEVER, Duration::from_secs(60), 1), None);
        assert!(backoff(1) > IDLE_INTERVAL);
    }

    #[test]
    fn the_wait_after_failures_has_a_ceiling() {
        // Без предела выключенная на ночь сеть отодвинула бы попытку на сутки,
        // и утром человек обнаружил бы, что ничего не синхронизируется.
        assert_eq!(backoff(50), MAX_BACKOFF);
        assert!(backoff(50) <= MAX_BACKOFF);
    }

    #[test]
    fn a_success_resets_the_wait() {
        assert_eq!(backoff(0), Duration::ZERO);
        assert_eq!(
            due(1, QUIET_PERIOD, Duration::from_secs(1), 0),
            Some(Reason::Pending)
        );
    }

    #[test]
    fn pending_changes_win_over_the_idle_timer() {
        // Порядок важен для отчёта: человеку полезнее знать, что уехали его
        // правки, чем что подошёл срок.
        assert_eq!(due(2, NEVER, NEVER, 0), Some(Reason::Pending));
    }
}
