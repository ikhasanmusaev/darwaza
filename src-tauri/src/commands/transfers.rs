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
    for n in 2..1000 {
        let candidate = dir.join(numbered_name(name, n));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(name)
}

/// «отчёт.txt» → «отчёт (2).txt»: номер перед расширением.
fn numbered_name(name: &str, n: u32) -> String {
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| name.to_string());
    match path.extension() {
        Some(e) => format!("{stem} ({n}).{}", e.to_string_lossy()),
        None => format!("{stem} ({n})"),
    }
}

/// Свободное имя в папке на сервере — то же правило, что и при скачивании.
///
/// Без этого загрузка файла с уже существующим именем затирала его, а отмена
/// или обрыв на середине удаляли недогруженное — то есть прежний файл пропадал
/// совсем.
async fn unique_remote_path(
    sftp: &russh_sftp::client::SftpSession,
    dir: &str,
    name: &str,
) -> Result<String, String> {
    let first = crate::sftp::join_path(dir, name);
    if !sftp.try_exists(first.clone()).await.map_err(map_err)? {
        return Ok(first);
    }
    for n in 2..1000 {
        let candidate = crate::sftp::join_path(dir, &numbered_name(name, n));
        if !sftp.try_exists(candidate.clone()).await.map_err(map_err)? {
            return Ok(candidate);
        }
    }
    Err(map_err(format!("в {dir} не нашлось свободного имени для {name}")))
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
        current_path: None,
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
    let remote_path = unique_remote_path(&sftp, &remote_dir, &name).await?;

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
    // Только новый файл: даже если между проверкой имени и открытием кто-то
    // создал такой же, чужое не затрём — получим ошибку.
    let new_file = russh_sftp::protocol::OpenFlags::CREATE
        | russh_sftp::protocol::OpenFlags::EXCLUDE
        | russh_sftp::protocol::OpenFlags::WRITE;
    let mut remote = match sftp
        .open_with_flags(remote_path.to_string(), new_file)
        .await
    {
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

// ─────────────────────────────────────────────────────────────────────────
// Папки целиком и между серверами (экран SFTP)
// ─────────────────────────────────────────────────────────────────────────

/// Одна сторона передачи, как её называет окно.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Endpoint {
    Local,
    #[serde(rename_all = "camelCase")]
    Remote { session_id: Uuid },
}

async fn tree_side(
    state: &AppState,
    endpoint: &Endpoint,
) -> Result<Arc<dyn crate::transfers::tree::TreeFs>, String> {
    use crate::transfers::tree::{LocalTree, RemoteTree};
    Ok(match endpoint {
        Endpoint::Local => Arc::new(LocalTree),
        Endpoint::Remote { session_id } => {
            Arc::new(RemoteTree(super::sftp::sftp_for(state, *session_id).await?))
        }
    })
}

/// Передать файл или папку целиком: компьютер ↔ сервер и сервер → сервер.
///
/// Возвращает id сразу; ход — событиями `transfer:progress` (по общему размеру
/// дерева), отмена — `transfer_cancel`. Подробности правил — `transfers::tree`.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn transfer_tree(
    app: AppHandle,
    state: State<'_, AppState>,
    source: Endpoint,
    source_path: String,
    is_dir: bool,
    target: Endpoint,
    target_dir: String,
    // Что делать с одноимённым на месте назначения; по умолчанию — класть рядом.
    on_conflict: Option<crate::transfers::tree::OnConflict>,
) -> Result<Uuid, String> {
    use crate::transfers::tree::{copy_tree, Outcome};
    let on_conflict = on_conflict.unwrap_or_default();

    let kind = match (&source, &target) {
        (Endpoint::Local, Endpoint::Remote { .. }) => TransferKind::Upload,
        (Endpoint::Remote { .. }, Endpoint::Local) => TransferKind::Download,
        (Endpoint::Remote { .. }, Endpoint::Remote { .. }) => TransferKind::Copy,
        (Endpoint::Local, Endpoint::Local) => {
            return Err(crate::errors::invalid(
                "transfers",
                "копирование внутри компьютера не здесь",
                crate::errors::codes::UNEXPECTED,
            ))
        }
    };
    let src = tree_side(&state, &source).await?;
    let dst = tree_side(&state, &target).await?;

    // Файл или папка на этом компьютере — смотрим сами: при перетаскивании из
    // файлового менеджера система отдаёт только пути, и окно этого не знает.
    let is_dir = match &source {
        Endpoint::Local => tokio::fs::metadata(&source_path)
            .await
            .map(|m| m.is_dir())
            .unwrap_or(is_dir),
        Endpoint::Remote { .. } => is_dir,
    };

    // Имя берём по правилам источника: путь сервера всегда через `/`.
    let name = match &source {
        Endpoint::Local => file_name_of(&source_path),
        Endpoint::Remote { .. } => source_path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(&source_path)
            .to_string(),
    };
    if name.is_empty() {
        return Err("не удалось определить имя".to_string());
    }

    let id = Uuid::new_v4();
    let cancel = state.transfers.register(id).await;
    let manager = state.transfers.clone();
    let shown = if is_dir { format!("{name}/") } else { name.clone() };

    tokio::spawn(async move {
        emit(&app, id, kind, &shown, 0, None, TransferStatus::Running, None, None);
        let mut last = Instant::now();
        let progress_app = app.clone();
        let progress_name = shown.clone();
        let mut on_progress = move |p: crate::transfers::tree::Progress| {
            if should_emit(last.elapsed().as_millis(), false) {
                last = Instant::now();
                let payload = TransferProgress {
                    id,
                    kind,
                    name: progress_name.clone(),
                    transferred: p.transferred,
                    total: p.total,
                    status: TransferStatus::Running,
                    error: None,
                    local_path: None,
                    current_path: Some(p.current),
                };
                let _ = progress_app.emit(TRANSFER_EVENT, &payload);
            }
        };

        let result = copy_tree(
            src.as_ref(),
            &source_path,
            &name,
            is_dir,
            dst.as_ref(),
            &target_dir,
            on_conflict,
            &cancel,
            &mut on_progress,
        )
        .await;
        manager.finish(id).await;

        match result {
            Ok(Outcome::Done { files, bytes, skipped, dest }) => {
                info!(target: "transfers", "передано {files} файл(ов), {bytes} байт, пропущено ссылок и особых файлов: {skipped}");
                let local = matches!(target, Endpoint::Local).then_some(dest);
                emit(&app, id, kind, &shown, bytes, Some(bytes), TransferStatus::Done, None, local);
            }
            Ok(Outcome::Cancelled { files }) => {
                info!(target: "transfers", "передача отменена, готовых файлов: {files}");
                emit(&app, id, kind, &shown, 0, None, TransferStatus::Cancelled, None, None);
            }
            Err(e) => {
                // Причина — в журнал; наружу идёт код и путь файла, на котором
                // остановились (#177).
                let code = crate::errors::opaque("transfers", format!("{}: {}", e.path, e.message));
                warn!(target: "transfers", "передача остановилась на {}: {}", e.path, e.message);
                let payload = TransferProgress {
                    id,
                    kind,
                    name: shown.clone(),
                    transferred: 0,
                    total: None,
                    status: TransferStatus::Failed,
                    error: Some(code),
                    local_path: None,
                    current_path: Some(e.path),
                };
                let _ = app.emit(TRANSFER_EVENT, &payload);
            }
        }
    });

    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::numbered_name;

    #[test]
    fn the_number_goes_before_the_extension() {
        assert_eq!(numbered_name("отчёт.txt", 2), "отчёт (2).txt");
        assert_eq!(numbered_name("archive.tar.gz", 3), "archive.tar (3).gz");
        assert_eq!(numbered_name("Makefile", 2), "Makefile (2)");
        assert_eq!(numbered_name(".bashrc", 2), ".bashrc (2)");
    }
}
