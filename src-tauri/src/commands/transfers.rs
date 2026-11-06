//! Фоновые передачи файлов: запуск, прогресс, отмена (#124).
//!
//! Отличие от `sftp_upload`/`sftp_download` в том, что здесь команда
//! возвращается **сразу**, отдав `id` передачи, а сама работа идёт в фоновой
//! задаче. Это нужно перетаскиванию: жест заканчивается мгновенно, а файл
//! качается дальше и виден в панели передач.

use crate::transfers::{should_emit, TransferKind, TransferProgress, TransferStatus};
use crate::AppState;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{info, warn};
use uuid::Uuid;

const TRANSFER_CHUNK: usize = 32 * 1024;
const TRANSFER_EVENT: &str = "transfer:progress";

/// Ошибка наружу уходит кодом, причина — в журнал (#177).
fn map_err<E: std::fmt::Display>(e: E) -> String {
    crate::errors::opaque("transfers", e)
}

fn file_name_of(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

/// Каталог, куда складывать перетащенное наружу.
///
/// Какую папку пользователь выбрал при дропе, WebView не сообщает — эта
/// информация до нас не доходит. Поэтому кладём в заданный каталог, а
/// пользователь при желании перенесёт файл сам.
#[tauri::command]
pub async fn transfers_download_dir(state: State<'_, AppState>) -> Result<String, String> {
    download_dir_for(&state.db).await
}

pub(crate) async fn download_dir_for(db: &sqlx::SqlitePool) -> Result<String, String> {
    let configured: Option<(String,)> =
        sqlx::query_as("SELECT value FROM settings WHERE key = 'sftp.download_dir'")
            .fetch_optional(db)
            .await
            .map_err(map_err)?;

    if let Some((value,)) = configured {
        if !value.trim().is_empty() {
            return Ok(value);
        }
    }

    let dirs = directories::UserDirs::new().ok_or("не удалось определить домашний каталог")?;
    let dir = dirs
        .download_dir()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| dirs.home_dir().to_path_buf());
    Ok(dir.to_string_lossy().to_string())
}

/// Не затирать существующий файл: `report.txt` → `report (2).txt`.
///
/// Перетаскивание легко повторить дважды, и молча перезаписать уже скачанное
/// было бы неприятным сюрпризом.
pub(crate) fn unique_path_in(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| name.to_string());
    let ext = path.extension().map(|e| e.to_string_lossy().to_string());

    for n in 2..1000 {
        let candidate_name = match &ext {
            Some(e) => format!("{stem} ({n}).{e}"),
            None => format!("{stem} ({n})"),
        };
        let candidate = dir.join(candidate_name);
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(name)
}

#[allow(clippy::too_many_arguments)]
fn emit(
    app: &AppHandle,
    id: Uuid,
    kind: TransferKind,
    name: &str,
    transferred: u64,
    total: Option<u64>,
    status: TransferStatus,
    error: Option<String>,
    local_path: Option<String>,
) {
    let payload = TransferProgress {
        id,
        kind,
        name: name.to_string(),
        transferred,
        total,
        status,
        error,
        local_path,
    };
    if let Err(e) = app.emit(TRANSFER_EVENT, &payload) {
        warn!(target: "transfers", "не удалось отправить прогресс: {e}");
    }
}

/// Скачать файл в фоне. Возвращает id передачи сразу.
#[tauri::command]
pub async fn transfer_download(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: Uuid,
    remote_path: String,
    // Каталог назначения. Пусто — берём настроенный по умолчанию.
    target_dir: Option<String>,
) -> Result<Uuid, String> {
    let sftp = super::sftp::sftp_for(&state, session_id).await?;
    let dir = match target_dir.filter(|d| !d.trim().is_empty()) {
        Some(d) => d,
        None => download_dir_for(&state.db).await?,
    };
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("не удалось создать '{dir}': {e}"))?;

    let name = file_name_of(&remote_path);
    let local_path = unique_path_in(Path::new(&dir), &name);

    // Размер нужен для прогресса; если сервер его не отдал — покажем без процентов.
    let total = sftp
        .metadata(remote_path.clone())
        .await
        .ok()
        .and_then(|m| m.size);

    let id = Uuid::new_v4();
    let cancel = state.transfers.register(id).await;
    let manager = state.transfers.clone();

    tokio::spawn(async move {
        let result = run_download(
            &app,
            id,
            &sftp,
            &remote_path,
            &local_path,
            total,
            &name,
            cancel,
        )
        .await;
        manager.finish(id).await;

        match result {
            Ok(true) => info!(target: "transfers", "скачано: {}", local_path.display()),
            Ok(false) => {
                // Отменённая передача оставляет обрезанный файл — он выглядит
                // целым и этим опасен, поэтому удаляем.
                let _ = tokio::fs::remove_file(&local_path).await;
                info!(target: "transfers", "скачивание отменено: {remote_path}");
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&local_path).await;
                warn!(target: "transfers", "скачивание не удалось: {e}");
            }
        }
    });

    Ok(id)
}

#[allow(clippy::too_many_arguments)]
async fn run_download(
    app: &AppHandle,
    id: Uuid,
    sftp: &Arc<russh_sftp::client::SftpSession>,
    remote_path: &str,
    local_path: &Path,
    total: Option<u64>,
    name: &str,
    cancel: Arc<AtomicBool>,
) -> Result<bool, String> {
    let local_display = local_path.to_string_lossy().to_string();
    emit(
        app,
        id,
        TransferKind::Download,
        name,
        0,
        total,
        TransferStatus::Running,
        None,
        Some(local_display.clone()),
    );

    let mut remote = match sftp.open(remote_path.to_string()).await {
        Ok(f) => f,
        Err(e) => {
            let msg = map_err(e);
            emit(
                app,
                id,
                TransferKind::Download,
                name,
                0,
                total,
                TransferStatus::Failed,
                Some(msg.clone()),
                None,
            );
            return Err(msg);
        }
    };
    let mut local = match tokio::fs::File::create(local_path).await {
        Ok(f) => f,
        Err(e) => {
            let msg = format!("не удалось создать '{local_display}': {e}");
            emit(
                app,
                id,
                TransferKind::Download,
                name,
                0,
                total,
                TransferStatus::Failed,
                Some(msg.clone()),
                None,
            );
            return Err(msg);
        }
    };

    let mut buf = vec![0u8; TRANSFER_CHUNK];
    let mut transferred: u64 = 0;
    let mut last = Instant::now();

    loop {
        if cancel.load(Ordering::Relaxed) {
            emit(
                app,
                id,
                TransferKind::Download,
                name,
                transferred,
                total,
                TransferStatus::Cancelled,
                None,
                None,
            );
            return Ok(false);
        }

        let n = match remote.read(&mut buf).await {
            Ok(n) => n,
            Err(e) => {
                let msg = map_err(e);
                emit(
                    app,
                    id,
                    TransferKind::Download,
                    name,
                    transferred,
                    total,
                    TransferStatus::Failed,
                    Some(msg.clone()),
                    None,
                );
                return Err(msg);
            }
        };
        if n == 0 {
            break;
        }
        if let Err(e) = local.write_all(&buf[..n]).await {
            let msg = format!("запись '{local_display}': {e}");
            emit(
                app,
                id,
                TransferKind::Download,
                name,
                transferred,
                total,
                TransferStatus::Failed,
                Some(msg.clone()),
                None,
            );
            return Err(msg);
        }
        transferred += n as u64;

        if should_emit(last.elapsed().as_millis(), false) {
            last = Instant::now();
            emit(
                app,
                id,
                TransferKind::Download,
                name,
                transferred,
                total,
                TransferStatus::Running,
                None,
                Some(local_display.clone()),
            );
        }
    }

    if let Err(e) = local.flush().await {
        let msg = format!("сброс на диск '{local_display}': {e}");
        emit(
            app,
            id,
            TransferKind::Download,
            name,
            transferred,
            total,
            TransferStatus::Failed,
            Some(msg.clone()),
            None,
        );
        return Err(msg);
    }

    emit(
        app,
        id,
        TransferKind::Download,
        name,
        transferred,
        Some(transferred),
        TransferStatus::Done,
        None,
        Some(local_display),
    );
    Ok(true)
}

/// Загрузить файл на сервер в фоне.
#[tauri::command]
pub async fn transfer_upload(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: Uuid,
    local_path: String,
    remote_dir: String,
) -> Result<Uuid, String> {
    let sftp = super::sftp::sftp_for(&state, session_id).await?;
    let name = file_name_of(&local_path);
    let remote_path = crate::sftp::join_path(&remote_dir, &name);

    let total = tokio::fs::metadata(&local_path).await.ok().map(|m| m.len());

    let id = Uuid::new_v4();
    let cancel = state.transfers.register(id).await;
    let manager = state.transfers.clone();

    tokio::spawn(async move {
        let result = run_upload(
            &app,
            id,
            &sftp,
            &local_path,
            &remote_path,
            total,
            &name,
            cancel,
        )
        .await;
        manager.finish(id).await;

        match result {
            Ok(true) => info!(target: "transfers", "загружено: {remote_path}"),
            Ok(false) => {
                // Недогруженный файл на сервере тоже выглядит целым — убираем.
                let _ = sftp.remove_file(remote_path.clone()).await;
                info!(target: "transfers", "загрузка отменена: {local_path}");
            }
            Err(e) => {
                let _ = sftp.remove_file(remote_path.clone()).await;
                warn!(target: "transfers", "загрузка не удалась: {e}");
            }
        }
    });

    Ok(id)
}

#[allow(clippy::too_many_arguments)]
async fn run_upload(
    app: &AppHandle,
    id: Uuid,
    sftp: &Arc<russh_sftp::client::SftpSession>,
    local_path: &str,
    remote_path: &str,
    total: Option<u64>,
    name: &str,
    cancel: Arc<AtomicBool>,
) -> Result<bool, String> {
    emit(
        app,
        id,
        TransferKind::Upload,
        name,
        0,
        total,
        TransferStatus::Running,
        None,
        None,
    );

    let mut local = match tokio::fs::File::open(local_path).await {
        Ok(f) => f,
        Err(e) => {
            let msg = format!("не удалось открыть '{local_path}': {e}");
            emit(
                app,
                id,
                TransferKind::Upload,
                name,
                0,
                total,
                TransferStatus::Failed,
                Some(msg.clone()),
                None,
            );
            return Err(msg);
        }
    };
    let mut remote = match sftp.create(remote_path.to_string()).await {
        Ok(f) => f,
        Err(e) => {
            let msg = map_err(e);
            emit(
                app,
                id,
                TransferKind::Upload,
                name,
                0,
                total,
                TransferStatus::Failed,
                Some(msg.clone()),
                None,
            );
            return Err(msg);
        }
    };

    let mut buf = vec![0u8; TRANSFER_CHUNK];
    let mut transferred: u64 = 0;
    let mut last = Instant::now();

    loop {
        if cancel.load(Ordering::Relaxed) {
            emit(
                app,
                id,
                TransferKind::Upload,
                name,
                transferred,
                total,
                TransferStatus::Cancelled,
                None,
                None,
            );
            return Ok(false);
        }

        let n = match local.read(&mut buf).await {
            Ok(n) => n,
            Err(e) => {
                let msg = format!("чтение '{local_path}': {e}");
                emit(
                    app,
                    id,
                    TransferKind::Upload,
                    name,
                    transferred,
                    total,
                    TransferStatus::Failed,
                    Some(msg.clone()),
                    None,
                );
                return Err(msg);
            }
        };
        if n == 0 {
            break;
        }
        if let Err(e) = remote.write_all(&buf[..n]).await {
            let msg = map_err(e);
            emit(
                app,
                id,
                TransferKind::Upload,
                name,
                transferred,
                total,
                TransferStatus::Failed,
                Some(msg.clone()),
                None,
            );
            return Err(msg);
        }
        transferred += n as u64;

        if should_emit(last.elapsed().as_millis(), false) {
            last = Instant::now();
            emit(
                app,
                id,
                TransferKind::Upload,
                name,
                transferred,
                total,
                TransferStatus::Running,
                None,
                None,
            );
        }
    }

    if let Err(e) = remote.flush().await {
        let msg = map_err(e);
        emit(
            app,
            id,
            TransferKind::Upload,
            name,
            transferred,
            total,
            TransferStatus::Failed,
            Some(msg.clone()),
            None,
        );
        return Err(msg);
    }

    emit(
        app,
        id,
        TransferKind::Upload,
        name,
        transferred,
        Some(transferred),
        TransferStatus::Done,
        None,
        None,
    );
    Ok(true)
}

/// Остановить передачу. `false` — она уже завершилась.
#[tauri::command]
pub async fn transfer_cancel(state: State<'_, AppState>, id: Uuid) -> Result<bool, String> {
    Ok(state.transfers.cancel(id).await)
}

/// Заглушка, чтобы `Manager` не считался неиспользованным на некоторых сборках.
#[allow(dead_code)]
fn _assert_manager_in_scope(app: &AppHandle) -> bool {
    app.webview_windows().is_empty()
}
