//! Доступность хостов для списка и боковой панели (#228).
//!
//! Проба — это обращение к чужому адресу, о котором человек не просил: поэтому
//! **по умолчанию выключена**, и выключена здесь, в Rust, а не только в
//! интерфейсе. Пока настройка `hosts.probe` не равна `1`, команда не делает
//! ни одного сетевого вызова и отвечает «неизвестно».
//!
//! Проверяется только TCP-рукопожатие до порта SSH: без входа, без обмена
//! ключами, без единого байта данных. Сервер видит в журнале обрыв соединения —
//! этим всё и ограничено.

use crate::AppState;
use serde::Serialize;
use sqlx::SqlitePool;
use std::time::Duration;
use tauri::State;
use tokio::net::TcpStream;
use tokio::sync::Semaphore;

pub const PROBE_KEY: &str = "hosts.probe";

const TIMEOUT: Duration = Duration::from_secs(2);
const MAX_HOSTS: usize = 64;
const PARALLEL: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Reachability {
    /// Порт ответил.
    Reachable,
    /// Порт не ответил за отведённое время или отказал.
    Unreachable,
    /// Не проверяли: проба выключена, хост за промежуточным или адрес не разобрать.
    /// Это **не** «не в сети»: не проверяли ≠ недоступен.
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostReachability {
    pub host_id: i64,
    pub state: Reachability,
}

async fn enabled(db: &SqlitePool) -> bool {
    let value: Option<String> =
        match sqlx::query_scalar("SELECT value FROM settings WHERE key = ?1")
            .bind(PROBE_KEY)
            .fetch_optional(db)
            .await
        {
            Ok(value) => value,
            // Безопасно — считаем выключенной, — но молча нельзя: иначе «проба не
            // работает» не отличить от «человек её не включал».
            Err(e) => {
                tracing::warn!(target: "probe", "не прочиталась настройка пробы: {e}");
                None
            }
        };
    value.as_deref() == Some("1")
}

/// Что опрашивать, а что вернуть как «неизвестно». Повторы убираются, порядок
/// сохраняется; сверх предела хосты не опрашиваются, но в ответе остаются —
/// окно иначе не отличило бы «не проверяли» от «не ответил».
fn split_ids(host_ids: Vec<i64>) -> (Vec<i64>, Vec<i64>) {
    let mut seen = std::collections::HashSet::new();
    let unique: Vec<i64> = host_ids.into_iter().filter(|id| seen.insert(*id)).collect();
    let skipped = unique.iter().skip(MAX_HOSTS).copied().collect();
    let probe = unique.into_iter().take(MAX_HOSTS).collect();
    (probe, skipped)
}

async fn reach(address: String, port: u16) -> Reachability {
    match tokio::time::timeout(TIMEOUT, TcpStream::connect((address.as_str(), port))).await {
        Ok(Ok(_)) => Reachability::Reachable,
        _ => Reachability::Unreachable,
    }
}

/// Проверить доступность перечисленных хостов.
///
/// Вызывать только для тех, что сейчас на виду: опрашивать весь список в
/// фоне — именно та сетевая активность, о которой человек не просил.
#[tauri::command]
pub async fn hosts_probe(
    state: State<'_, AppState>,
    host_ids: Vec<i64>,
) -> Result<Vec<HostReachability>, String> {
    probe_hosts(&state.db, host_ids).await
}

/// Вся логика отдельно от команды, чтобы проверять её без приложения.
pub async fn probe_hosts(
    db: &SqlitePool,
    host_ids: Vec<i64>,
) -> Result<Vec<HostReachability>, String> {
    let (ids, skipped) = split_ids(host_ids);
    let unknown = |host_id| HostReachability {
        host_id,
        state: Reachability::Unknown,
    };

    if !enabled(db).await {
        return Ok(ids.into_iter().chain(skipped).map(unknown).collect());
    }

    let gate = std::sync::Arc::new(Semaphore::new(PARALLEL));
    let mut tasks = Vec::new();

    for host_id in ids {
        let row: Option<(String, i64, Option<i64>)> =
            sqlx::query_as("SELECT hostname, port, jump_host_id FROM hosts WHERE id = ?1")
                .bind(host_id)
                .fetch_optional(db)
                .await
                .map_err(|e| crate::errors::opaque("hosts_probe", e))?;

        let gate = gate.clone();
        tasks.push(tokio::spawn(async move {
            let state = match row {
                // За промежуточным хостом напрямую не достучаться: пробовать
                // значило бы показать «не в сети» тому, кто в порядке.
                Some((_, _, Some(_))) | None => Reachability::Unknown,
                Some((address, port, None)) => match u16::try_from(port) {
                    Ok(port) => {
                        let _permit = gate.acquire().await;
                        reach(address, port).await
                    }
                    Err(_) => Reachability::Unknown,
                },
            };
            HostReachability { host_id, state }
        }));
    }

    let mut out = Vec::new();
    for task in tasks {
        out.push(
            task.await
                .map_err(|e| crate::errors::opaque("hosts_probe", e))?,
        );
    }
    out.extend(skipped.into_iter().map(unknown));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db_with_host(port: u16, jump: Option<i64>) -> SqlitePool {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("память");
        crate::db::MIGRATOR.run(&db).await.expect("миграции");
        sqlx::query(
            "INSERT INTO hosts (id, name, hostname, port, username, auth_method, jump_host_id, sync_id) \
             VALUES (1, 'h', '127.0.0.1', ?1, 'u', 'password', ?2, '11111111-1111-4111-8111-111111111111')",
        )
        .bind(i64::from(port))
        .bind(jump)
        .execute(&db)
        .await
        .expect("хост");
        db
    }

    /// Сверх предела хосты не пропадают из ответа, а повторы не опрашиваются дважды.
    #[test]
    fn extra_hosts_come_back_unknown_and_duplicates_collapse() {
        let ids: Vec<i64> = (1..=70).chain([1, 2, 3]).collect();

        let (probe, skipped) = split_ids(ids);

        assert_eq!(probe.len(), MAX_HOSTS);
        assert_eq!(skipped, (65..=70).collect::<Vec<i64>>());
        assert_eq!(probe[..3], [1, 2, 3], "порядок сохраняется");
    }

    #[tokio::test]
    async fn the_probe_is_off_by_default_and_makes_no_network_call() {
        // Порт открыт и ответил бы — но человек пробу не включал, поэтому
        // ответ обязан быть «неизвестно», а обращения к порту быть не должно.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("порт");
        let port = listener.local_addr().expect("адрес").port();
        let db = db_with_host(port, None).await;

        let answer = probe_hosts(&db, vec![1]).await.expect("ответ");
        assert_eq!(answer[0].state, Reachability::Unknown);

        let accepted = tokio::time::timeout(Duration::from_millis(200), listener.accept()).await;
        assert!(
            accepted.is_err(),
            "к выключенной пробе никто не подключался"
        );
    }

    #[tokio::test]
    async fn once_enabled_it_tells_reachable_from_unknown_behind_a_jump_host() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("порт");
        let port = listener.local_addr().expect("адрес").port();
        let db = db_with_host(port, None).await;
        sqlx::query("INSERT INTO settings (key, value) VALUES (?1, '1')")
            .bind(PROBE_KEY)
            .execute(&db)
            .await
            .expect("включено");

        let answer = probe_hosts(&db, vec![1, 999]).await.expect("ответ");
        assert_eq!(answer[0].state, Reachability::Reachable);
        assert_eq!(
            answer[1].state,
            Reachability::Unknown,
            "хоста нет — не «не в сети»"
        );

        sqlx::query("UPDATE hosts SET jump_host_id = 1 WHERE id = 1")
            .execute(&db)
            .await
            .expect("через промежуточный");
        let answer = probe_hosts(&db, vec![1]).await.expect("ответ");
        assert_eq!(answer[0].state, Reachability::Unknown);
    }

    #[tokio::test]
    async fn an_open_port_is_reachable_and_a_closed_one_is_not() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("порт");
        let open = listener.local_addr().expect("адрес").port();

        assert_eq!(
            reach("127.0.0.1".into(), open).await,
            Reachability::Reachable
        );

        drop(listener);
        assert_eq!(
            reach("127.0.0.1".into(), open).await,
            Reachability::Unreachable
        );
    }
}
