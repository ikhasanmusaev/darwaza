//! Команды управления пробросом портов (#130, #133).

use crate::db::models::{PortForward, PortForwardInput};
use crate::forwarding::ForwardSnapshot;
use crate::AppState;
use tauri::State;
use uuid::Uuid;

const SELECT_FIELDS: &str = "id, host_id, kind, label, local_host, local_port, \
    remote_host, remote_port, auto_start, created_at, updated_at";

/// Ошибка базы при чтении: занятая пара и мёртвая ссылка разбираются
/// отдельно, остальное — «не удалось прочитать» (#177).
fn load_err(e: sqlx::Error) -> String {
    crate::errors::db_load("port_forwards", e)
}

/// То же при записи.
fn save_err(e: sqlx::Error) -> String {
    crate::errors::db_save("port_forwards", e)
}

/// Проверка на входе: правило без нужных полей не запустится, но обнаружится
/// это только при подключении — лучше отсечь сразу.
fn validate(data: &PortForwardInput) -> Result<(), String> {
    if !(1..=65535).contains(&data.local_port) && data.local_port != 0 {
        return Err(crate::errors::invalid(
            "port_forwards",
            format!("локальный порт {} вне 1-65535", data.local_port),
            crate::errors::codes::INVALID_PORT,
        ));
    }
    match data.kind.as_str() {
        "local" | "remote" => {
            let host = data.remote_host.as_deref().unwrap_or("").trim().to_string();
            if host.is_empty() {
                return Err(crate::errors::invalid(
                    "port_forwards",
                    format!("проброс '{}' без remote_host", data.kind),
                    crate::errors::codes::REMOTE_HOST_REQUIRED,
                ));
            }
            match data.remote_port {
                Some(p) if (1..=65535).contains(&p) => Ok(()),
                _ => Err(crate::errors::invalid(
                    "port_forwards",
                    format!("проброс '{}' без remote_port", data.kind),
                    crate::errors::codes::REMOTE_PORT_REQUIRED,
                )),
            }
        }
        // Адрес назначения приходит по SOCKS5, заранее его знать не нужно.
        "dynamic" => Ok(()),
        other => Err(crate::errors::opaque(
            "port_forwards",
            format!("неизвестный вид проброса: {other}"),
        )),
    }
}

#[tauri::command]
pub async fn port_forwards_list(
    state: State<'_, AppState>,
    host_id: Option<i64>,
) -> Result<Vec<PortForward>, String> {
    let rows = match host_id {
        Some(id) => {
            let sql = format!(
                "SELECT {SELECT_FIELDS} FROM port_forwards WHERE host_id = ?1 ORDER BY local_port"
            );
            sqlx::query_as::<_, PortForward>(&sql)
                .bind(id)
                .fetch_all(&state.db)
                .await
        }
        None => {
            let sql =
                format!("SELECT {SELECT_FIELDS} FROM port_forwards ORDER BY host_id, local_port");
            sqlx::query_as::<_, PortForward>(&sql)
                .fetch_all(&state.db)
                .await
        }
    };
    rows.map_err(load_err)
}

#[tauri::command]
pub async fn port_forwards_create(
    state: State<'_, AppState>,
    data: PortForwardInput,
) -> Result<PortForward, String> {
    validate(&data)?;
    let sql = format!(
        "INSERT INTO port_forwards \
            (host_id, kind, label, local_host, local_port, remote_host, remote_port, auto_start, \
             sync_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
         RETURNING {SELECT_FIELDS}"
    );
    sqlx::query_as::<_, PortForward>(&sql)
        .bind(data.host_id)
        .bind(&data.kind)
        .bind(&data.label)
        .bind(data.local_host.as_deref().unwrap_or("127.0.0.1"))
        .bind(data.local_port)
        .bind(&data.remote_host)
        .bind(data.remote_port)
        .bind(data.auto_start.unwrap_or(false))
        .bind(crate::db::new_sync_id())
        .fetch_one(&state.db)
        .await
        .map_err(save_err)
}

#[tauri::command]
pub async fn port_forwards_update(
    state: State<'_, AppState>,
    id: i64,
    data: PortForwardInput,
) -> Result<PortForward, String> {
    validate(&data)?;
    let sql = format!(
        "UPDATE port_forwards SET \
            host_id = ?2, kind = ?3, label = ?4, local_host = ?5, local_port = ?6, \
            remote_host = ?7, remote_port = ?8, auto_start = COALESCE(?9, auto_start), \
            updated_at = CURRENT_TIMESTAMP \
         WHERE id = ?1 \
         RETURNING {SELECT_FIELDS}"
    );
    sqlx::query_as::<_, PortForward>(&sql)
        .bind(id)
        .bind(data.host_id)
        .bind(&data.kind)
        .bind(&data.label)
        .bind(data.local_host.as_deref().unwrap_or("127.0.0.1"))
        .bind(data.local_port)
        .bind(&data.remote_host)
        .bind(data.remote_port)
        .bind(data.auto_start)
        .fetch_optional(&state.db)
        .await
        .map_err(save_err)?
        .ok_or_else(|| crate::errors::not_found("port_forwards", "правило проброса", id))
}

#[tauri::command]
pub async fn port_forwards_remove(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    // Сначала останавливаем: иначе удалённое правило останется висеть
    // слушающим сокетом до конца сессии.
    state.forwards.stop(id).await;
    sqlx::query("DELETE FROM port_forwards WHERE id = ?1")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(save_err)?;
    Ok(())
}

/// Запустить правило на активной сессии. Возвращает фактический адрес прослушки
/// (при `local_port = 0` порт выдаёт ОС).
#[tauri::command]
pub async fn port_forward_start(
    state: State<'_, AppState>,
    id: i64,
    session_id: Uuid,
) -> Result<String, String> {
    let sql = format!("SELECT {SELECT_FIELDS} FROM port_forwards WHERE id = ?1");
    let rule = sqlx::query_as::<_, PortForward>(&sql)
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(save_err)?
        .ok_or_else(|| crate::errors::not_found("port_forwards", "правило проброса", id))?;

    // remote (`ssh -R`) устроен иначе: слушает сервер, а входящие каналы
    // приходят к нам сами — локальный сокет здесь не нужен.
    if rule.kind == "remote" {
        return start_remote(&state, &rule, session_id).await;
    }

    state
        .forwards
        .start(
            state.ssh.clone(),
            session_id,
            crate::forwarding::ForwardSpec {
                forward_id: rule.id,
                kind: rule.kind.clone(),
                local_host: rule.local_host.clone(),
                local_port: rule.local_port as u16,
                remote_host: rule.remote_host.clone(),
                remote_port: rule.remote_port.map(|p| p as u16),
            },
        )
        .await
}

/// Запуск remote-проброса: просим сервер слушать порт и запоминаем, куда
/// отдавать пришедшие оттуда соединения.
async fn start_remote(
    state: &AppState,
    rule: &PortForward,
    session_id: Uuid,
) -> Result<String, String> {
    // Пустой адрес означает «сервер выбирает сам» — так его понимает OpenSSH.
    let bind_address = rule.remote_host.clone().unwrap_or_default();
    let requested_port = rule.remote_port.unwrap_or(0) as u32;

    let (tx, rx) = tokio::sync::oneshot::channel();
    state
        .ssh
        .send_cmd(
            session_id,
            crate::ssh::session::SessionCmd::RequestRemoteForward {
                address: bind_address.clone(),
                port: requested_port,
                responder: tx,
            },
        )
        .await?;

    let bound_port = rx
        .await
        .map_err(|_| format!("сессия {session_id} закрылась при запросе прослушки"))??;

    // Цель регистрируем после успеха: иначе при отказе сервера в реестре
    // осталась бы запись, ведущая в никуда.
    state
        .forwards
        .register_remote_target(
            session_id,
            bound_port,
            rule.local_host.clone(),
            rule.local_port as u16,
        )
        .await;
    state
        .forwards
        .track_remote(rule.id, session_id, bind_address.clone(), bound_port)
        .await;

    Ok(format!("{bind_address}:{bound_port} (на сервере)"))
}

#[tauri::command]
pub async fn port_forward_stop(state: State<'_, AppState>, id: i64) -> Result<bool, String> {
    // Для remote нужно ещё и попросить сервер прекратить слушать.
    if let Some((session_id, address, port)) = state.forwards.remote_binding(id).await {
        let _ = state
            .ssh
            .send_cmd(
                session_id,
                crate::ssh::session::SessionCmd::CancelRemoteForward {
                    address: address.clone(),
                    port,
                },
            )
            .await;
        state
            .forwards
            .unregister_remote_target(session_id, port)
            .await;
    }
    Ok(state.forwards.stop(id).await)
}

#[tauri::command]
pub async fn port_forwards_active(
    state: State<'_, AppState>,
) -> Result<Vec<ForwardSnapshot>, String> {
    Ok(state.forwards.snapshot().await)
}
