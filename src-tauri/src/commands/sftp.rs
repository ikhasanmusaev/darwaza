//! SFTP-команды поверх активной SSH-сессии (#120, #123).
//!
//! Подсистема поднимается лениво — при первой файловой операции — и дальше
//! живёт в `SshManager` до конца сессии.

use crate::sftp::{join_path, parent_path, sort_entries, SftpEntry};
use crate::ssh::session::SessionCmd;
use crate::AppState;
use russh_sftp::client::SftpSession;
use std::sync::Arc;
use tauri::State;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;
use tracing::{info, warn};
use uuid::Uuid;

/// Сколько байт за раз гоняем при загрузке/скачивании.
/// 32 КиБ — компромисс: заметно быстрее мелких чтений и не раздувает память
/// при параллельных передачах.
const TRANSFER_CHUNK: usize = 32 * 1024;

/// Ошибка наружу уходит кодом, причина — в журнал (#177).
fn map_err<E: std::fmt::Display>(e: E) -> String {
    crate::errors::opaque("sftp", e)
}

/// Получить SFTP-подсистему сессии, открыв её при первом обращении.
pub(crate) async fn sftp_for(
    state: &AppState,
    session_id: Uuid,
) -> Result<Arc<SftpSession>, String> {
    if let Some(existing) = state.ssh.get_sftp(session_id).await {
        return Ok(existing);
    }

    let (tx, rx) = oneshot::channel();
    state
        .ssh
        .send_cmd(session_id, SessionCmd::OpenSftp { responder: tx })
        .await?;

    let channel = rx
        .await
        .map_err(|_| format!("session {session_id} closed while opening sftp"))??;

    let sftp = SftpSession::new(channel.into_stream())
        .await
        .map_err(|e| format!("sftp handshake failed: {e}"))?;
    info!(target: "sftp", "session {session_id}: sftp subsystem ready");

    Ok(state.ssh.put_sftp(session_id, Arc::new(sftp)).await)
}

/// Домашняя директория пользователя на сервере — точка входа файлового браузера.
#[tauri::command]
pub async fn sftp_home(state: State<'_, AppState>, session_id: Uuid) -> Result<String, String> {
    let sftp = sftp_for(&state, session_id).await?;
    sftp.canonicalize(".").await.map_err(map_err)
}

/// Список файлов директории. Директории идут первыми, дальше — по имени.
#[tauri::command]
pub async fn sftp_list(
    state: State<'_, AppState>,
    session_id: Uuid,
    path: String,
) -> Result<Vec<SftpEntry>, String> {
    let sftp = sftp_for(&state, session_id).await?;
    let dir = sftp.read_dir(path.clone()).await.map_err(map_err)?;

    let mut entries: Vec<SftpEntry> = dir
        .into_iter()
        .filter_map(|e| {
            let name = e.file_name();
            // "." и ".." навигацией занимается UI (кнопка "вверх"),
            // в списке они только мешают.
            if name == "." || name == ".." {
                return None;
            }
            let meta = e.metadata();
            Some(SftpEntry {
                path: join_path(&path, &name),
                name,
                kind: meta.file_type().into(),
                size: meta.size.unwrap_or(0),
                modified: meta.mtime.map(|t| t as i64),
                permissions: meta.permissions,
            })
        })
        .collect();

    sort_entries(&mut entries);
    Ok(entries)
}

/// Путь на уровень выше — чтобы UI не занимался разбором путей.
#[tauri::command]
pub async fn sftp_parent(path: String) -> Result<String, String> {
    Ok(parent_path(&path))
}

#[tauri::command]
pub async fn sftp_mkdir(
    state: State<'_, AppState>,
    session_id: Uuid,
    path: String,
) -> Result<(), String> {
    let sftp = sftp_for(&state, session_id).await?;
    sftp.create_dir(path).await.map_err(map_err)
}

/// Удалить файл или директорию.
///
/// Директория должна быть пустой — рекурсивного удаления намеренно нет:
/// снести чужое дерево одним кликом слишком легко, а подтверждения на каждый
/// уровень у нас нет.
#[tauri::command]
pub async fn sftp_remove(
    state: State<'_, AppState>,
    session_id: Uuid,
    path: String,
    is_dir: bool,
) -> Result<(), String> {
    let sftp = sftp_for(&state, session_id).await?;
    if is_dir {
        sftp.remove_dir(path).await.map_err(map_err)
    } else {
        sftp.remove_file(path).await.map_err(map_err)
    }
}

#[tauri::command]
pub async fn sftp_rename(
    state: State<'_, AppState>,
    session_id: Uuid,
    from: String,
    to: String,
) -> Result<(), String> {
    let sftp = sftp_for(&state, session_id).await?;
    sftp.rename(from, to).await.map_err(map_err)
}

/// Скачать удалённый файл в локальный путь (#123).
///
/// Гоняем чанками, а не `sftp.read()` целиком: иначе 2-гигабайтный лог
/// приедет в память процесса.
#[tauri::command]
pub async fn sftp_download(
    state: State<'_, AppState>,
    session_id: Uuid,
    remote_path: String,
    local_path: String,
) -> Result<u64, String> {
    let sftp = sftp_for(&state, session_id).await?;

    let mut remote = sftp.open(remote_path.clone()).await.map_err(map_err)?;
    let mut local = tokio::fs::File::create(&local_path)
        .await
        .map_err(|e| format!("create '{local_path}' failed: {e}"))?;

    let mut buf = vec![0u8; TRANSFER_CHUNK];
    let mut total: u64 = 0;
    loop {
        let n = remote.read(&mut buf).await.map_err(map_err)?;
        if n == 0 {
            break;
        }
        local
            .write_all(&buf[..n])
            .await
            .map_err(|e| format!("write '{local_path}' failed: {e}"))?;
        total += n as u64;
    }
    local
        .flush()
        .await
        .map_err(|e| format!("flush '{local_path}' failed: {e}"))?;

    info!(target: "sftp", "session {session_id}: downloaded {remote_path} ({total} bytes)");
    Ok(total)
}

/// Загрузить локальный файл на сервер (#123).
#[tauri::command]
pub async fn sftp_upload(
    state: State<'_, AppState>,
    session_id: Uuid,
    local_path: String,
    remote_path: String,
) -> Result<u64, String> {
    let sftp = sftp_for(&state, session_id).await?;

    let mut local = tokio::fs::File::open(&local_path)
        .await
        .map_err(|e| format!("open '{local_path}' failed: {e}"))?;
    let mut remote = sftp.create(remote_path.clone()).await.map_err(map_err)?;

    let mut buf = vec![0u8; TRANSFER_CHUNK];
    let mut total: u64 = 0;
    loop {
        let n = local
            .read(&mut buf)
            .await
            .map_err(|e| format!("read '{local_path}' failed: {e}"))?;
        if n == 0 {
            break;
        }
        remote.write_all(&buf[..n]).await.map_err(map_err)?;
        total += n as u64;
    }
    remote.flush().await.map_err(map_err)?;

    info!(target: "sftp", "session {session_id}: uploaded {remote_path} ({total} bytes)");
    Ok(total)
}

/// Закрыть SFTP-подсистему, не трогая саму SSH-сессию и терминал.
#[tauri::command]
pub async fn sftp_close(state: State<'_, AppState>, session_id: Uuid) -> Result<(), String> {
    if let Some(sftp) = state.ssh.drop_sftp(session_id).await {
        if let Err(e) = sftp.close().await {
            warn!(target: "sftp", "session {session_id}: sftp close failed: {e}");
        }
    }
    Ok(())
}
