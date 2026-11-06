//! Передача файлов между компьютером и сервером для агента (#212).
//!
//! Отдельно от `tools.rs`, где живут сами инструменты: здесь нет ни окна
//! подтверждения, ни журнала, ни `AppHandle` — только файлы и SFTP. Поэтому это
//! можно проверить живым тестом против настоящего sshd, а инструменты поверх
//! этого — нет.
//!
//! Решение владельца 2026-09-17: агенту разрешены **любые** локальные пути, но
//! всё, что трогает файлы, — и скачивание, и загрузка — идёт только после
//! подтверждения. Подтверждение само по себе защищает, пока его читают,
//! поэтому здесь строже то, что влияет на *показанное* человеку: путь в плане
//! обязан быть тем путём, по которому пойдёт запись.

use russh_sftp::client::error::Error as SftpError;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::{FileType, StatusCode};
use std::path::{Component, Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Сколько байт за раз гоняем. Как в файловом браузере приложения.
const CHUNK: usize = 32 * 1024;

/// Суффикс копии, в которую уходит заменяемый файл на сервере.
///
/// Тот же, что у правки файла (`darwaza_write_file`): человек, увидевший
/// `.darwaza-bak`, должен понимать, откуда он, не разбираясь, каким
/// инструментом его сделали.
pub const BACKUP_SUFFIX: &str = ".darwaza-bak";

/// Суффикс недокачанного файла на компьютере.
const PART_SUFFIX: &str = ".darwaza-part";

/// `~\путь` — только на Windows: там обратная косая черта разделяет каталоги.
/// На unix это обычный символ имени файла, и раскрывать её значило бы менять
/// смысл пути.
fn tilde_backslash(raw: &str) -> Option<&str> {
    if cfg!(windows) {
        raw.strip_prefix("~\\")
    } else {
        None
    }
}

/// Разобрать локальный путь из запроса агента.
///
/// Требуется **абсолютный** путь (или начинающийся с `~`). Относительный
/// читался бы от каталога, в котором запущено приложение, — а его не знают ни
/// агент, ни человек, подтверждающий план. Подтверждение пути, который
/// означает не то, что написано, ничего не стоит.
///
/// По той же причине отвергаются `..`: `/home/u/x/../.ssh/id_ed25519` в плане
/// выглядит как файл в `x`, а пишется в `.ssh`.
pub fn local_path(raw: &str, home: Option<&Path>) -> Result<PathBuf, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("не указан локальный путь".to_string());
    }

    let path = if raw == "~" {
        home.ok_or("не удалось определить домашний каталог")?
            .to_path_buf()
    } else if let Some(rest) = raw.strip_prefix("~/").or_else(|| tilde_backslash(raw)) {
        home.ok_or("не удалось определить домашний каталог")?
            .join(rest)
    } else {
        PathBuf::from(raw)
    };

    if !path.is_absolute() {
        return Err(format!(
            "локальный путь должен быть абсолютным или начинаться с ~: '{raw}'"
        ));
    }
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(format!(
            "в локальном пути не должно быть '..': человек подтверждает то, что видит, \
             а запись ушла бы в другое место — '{raw}'"
        ));
    }
    Ok(path)
}

/// Размер для человека: «12,3 МБ». План читают, а не разбирают.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["байт", "КБ", "МБ", "ГБ"];
    if bytes < 1024 {
        return format!("{bytes} байт");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{:.1} {}", value, UNITS[unit]).replace('.', ",")
}

/// Что лежит по пути на сервере. `Ok(None)` — ничего.
///
/// «Нет файла» и «нет доступа» различаются намеренно: второе — не повод
/// считать путь свободным и писать поверх.
pub async fn remote_kind(
    sftp: &SftpSession,
    path: &str,
) -> Result<Option<(FileType, u64)>, String> {
    match sftp.metadata(path.to_string()).await {
        Ok(meta) => Ok(Some((meta.file_type(), meta.size.unwrap_or(0)))),
        Err(SftpError::Status(status)) if status.status_code == StatusCode::NoSuchFile => Ok(None),
        Err(e) => Err(format!("{path}: {e}")),
    }
}

/// Скачать файл с сервера на компьютер.
///
/// Пишется во временный файл рядом с целевым и переименовывается в конце.
/// Оборванная посреди передача не оставляет на месте нужного файла половину —
/// ровно так 31 августа обнулился CHANGELOG: открытие на запись обрезало
/// файл, а запись упала.
pub async fn download(sftp: &SftpSession, remote: &str, local: &Path) -> Result<u64, String> {
    let parent = local
        .parent()
        .ok_or_else(|| format!("у пути нет каталога: {}", local.display()))?;
    if !parent.is_dir() {
        // Создавать каталоги самим не будем: человек подтверждал файл, а не
        // новое дерево каталогов на своём диске.
        return Err(format!("каталога нет: {}", parent.display()));
    }
    if local.is_dir() {
        return Err(format!(
            "по этому пути каталог, а не файл: {}",
            local.display()
        ));
    }

    let part = {
        let mut name = local
            .file_name()
            .ok_or_else(|| format!("у пути нет имени файла: {}", local.display()))?
            .to_os_string();
        name.push(PART_SUFFIX);
        local.with_file_name(name)
    };

    let result = copy_to_part(sftp, remote, &part).await;
    match result {
        Ok(total) => {
            tokio::fs::rename(&part, local).await.map_err(|e| {
                format!(
                    "не удалось поставить файл на место {}: {e}",
                    local.display()
                )
            })?;
            Ok(total)
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&part).await;
            Err(e)
        }
    }
}

async fn copy_to_part(sftp: &SftpSession, remote: &str, part: &Path) -> Result<u64, String> {
    let mut source = sftp
        .open(remote.to_string())
        .await
        .map_err(|e| format!("{remote}: {e}"))?;
    let mut target = tokio::fs::File::create(part)
        .await
        .map_err(|e| format!("{}: {e}", part.display()))?;

    let mut buf = vec![0u8; CHUNK];
    let mut total = 0u64;
    loop {
        let n = source
            .read(&mut buf)
            .await
            .map_err(|e| format!("{remote}: {e}"))?;
        if n == 0 {
            break;
        }
        target
            .write_all(&buf[..n])
            .await
            .map_err(|e| format!("{}: {e}", part.display()))?;
        total += n as u64;
    }
    // sync_all, а не flush: до переименования данные обязаны быть на диске,
    // иначе сбой питания оставит на месте нужного файла пустоту.
    target
        .sync_all()
        .await
        .map_err(|e| format!("{}: {e}", part.display()))?;
    Ok(total)
}

/// Итог загрузки на сервер.
#[derive(Debug, PartialEq, Eq)]
pub struct UploadOutcome {
    pub bytes: u64,
    /// Куда сохранён заменённый файл, если он был.
    pub backup: Option<String>,
}

/// Загрузить файл с компьютера на сервер.
///
/// Заменяемый файл не перезаписывается, а **переименовывается** в копию: это
/// мгновенно при любом размере и не читает старое содержимое целиком. Если
/// загрузка оборвалась, копия возвращается на место.
pub async fn upload(
    sftp: &SftpSession,
    local: &Path,
    remote: &str,
) -> Result<UploadOutcome, String> {
    let mut source = tokio::fs::File::open(local)
        .await
        .map_err(|e| format!("{}: {e}", local.display()))?;
    if !source
        .metadata()
        .await
        .map_err(|e| format!("{}: {e}", local.display()))?
        .is_file()
    {
        return Err(format!("это не файл: {}", local.display()));
    }

    place(sftp, remote, &mut source, &local.display().to_string()).await
}

/// Скопировать файл с одного сервера на другой, не касаясь диска человека
/// (#222).
///
/// Через приложение, а не `scp` между серверами: второе требует, чтобы они
/// видели друг друга по сети и чтобы у одного был ключ от другого. При
/// переезде верно обычно ни то, ни другое — а приложение подключено к обоим
/// уже сейчас.
///
/// Содержимое идёт **потоком**: кусок прочитали с одного, тут же записали на
/// другой. Промежуточного файла нет, поэтому размер копируемого не упирается
/// ни в место на диске человека, ни в память.
pub async fn copy_between(
    from: &SftpSession,
    from_path: &str,
    to: &SftpSession,
    to_path: &str,
) -> Result<UploadOutcome, String> {
    match remote_kind(from, from_path).await? {
        None => return Err(format!("на исходном сервере нет файла: {from_path}")),
        Some((FileType::Dir, _)) => {
            return Err(format!(
                "на исходном сервере по этому пути каталог: {from_path}"
            ))
        }
        Some(_) => {}
    }

    let mut source = from
        .open(from_path.to_string())
        .await
        .map_err(|e| format!("{from_path}: {e}"))?;

    place(to, to_path, &mut source, from_path).await
}

/// Положить содержимое из любого источника в файл на сервере.
///
/// Общая часть загрузки с компьютера и копирования между серверами. Разделять
/// её нельзя: откладывание прежнего файла в копию и возврат копии на место при
/// обрыве — единственное, что стоит между ошибкой и потерей чужого конфига.
/// Два похожих куска кода расходятся, и расходится обычно как раз обработка
/// неудачи, которую некому проверять.
///
/// `source_name` нужен только для сообщений: в них должно быть видно, откуда
/// шли данные.
async fn place<R>(
    sftp: &SftpSession,
    remote: &str,
    source: &mut R,
    source_name: &str,
) -> Result<UploadOutcome, String>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let backup = match remote_kind(sftp, remote).await? {
        None => None,
        Some((FileType::Dir, _)) => {
            return Err(format!("на сервере по этому пути каталог: {remote}"));
        }
        Some(_) => {
            let backup = format!("{remote}{BACKUP_SUFFIX}");
            // Прежняя копия — наша же, от прошлой замены. SFTP не
            // переименовывает поверх существующего файла, поэтому убираем её.
            if remote_kind(sftp, &backup).await?.is_some() {
                sftp.remove_file(backup.clone())
                    .await
                    .map_err(|e| format!("{backup}: {e}"))?;
            }
            sftp.rename(remote.to_string(), backup.clone())
                .await
                .map_err(|e| format!("не удалось отложить прежний файл {remote}: {e}"))?;
            Some(backup)
        }
    };

    match write_remote(sftp, source, remote, source_name).await {
        Ok(bytes) => Ok(UploadOutcome { bytes, backup }),
        Err(e) => {
            let _ = sftp.remove_file(remote.to_string()).await;
            let restored = match &backup {
                Some(b) => sftp.rename(b.clone(), remote.to_string()).await.is_ok(),
                None => true,
            };
            Err(if restored {
                format!("передача оборвалась, на сервере всё как было: {e}")
            } else {
                format!(
                    "передача оборвалась, и прежний файл НЕ вернулся на место — он лежит в {}: {e}",
                    backup.as_deref().unwrap_or("?")
                )
            })
        }
    }
}

async fn write_remote<R>(
    sftp: &SftpSession,
    source: &mut R,
    remote: &str,
    source_name: &str,
) -> Result<u64, String>
where
    R: tokio::io::AsyncRead + Unpin,
{
    // `create`, а не `write`: у второго нет CREATE и TRUNCATE (#208).
    let mut target = sftp
        .create(remote.to_string())
        .await
        .map_err(|e| format!("{remote}: {e}"))?;
    let mut buf = vec![0u8; CHUNK];
    let mut total = 0u64;
    loop {
        let n = source
            .read(&mut buf)
            .await
            .map_err(|e| format!("чтение {source_name}: {e}"))?;
        if n == 0 {
            break;
        }
        target
            .write_all(&buf[..n])
            .await
            .map_err(|e| format!("{remote}: {e}"))?;
        total += n as u64;
    }
    // shutdown, а не flush: закрытие по Drop глотает ошибку (#208).
    target
        .shutdown()
        .await
        .map_err(|e| format!("{remote}: {e}"))?;
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Домашний каталог подставной, но абсолютный на той системе, где идёт тест:
    /// `/home/someone` на Windows не абсолютен, и проверка пути отвергала бы его.
    fn home() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\Users\someone")
        } else {
            PathBuf::from("/home/someone")
        }
    }

    #[test]
    fn a_tilde_path_is_expanded_to_home() {
        assert_eq!(
            local_path("~/logs/app.log", Some(&home())).unwrap(),
            home().join("logs").join("app.log")
        );
        // На Windows `~\` — такая же запись домашнего каталога, как `~/`.
        #[cfg(windows)]
        assert_eq!(
            local_path(r"~\logs\app.log", Some(&home())).unwrap(),
            home().join(r"logs\app.log")
        );
        assert_eq!(local_path("~", Some(&home())).unwrap(), home());
    }

    /// Относительный путь читался бы от каталога запуска приложения — его не
    /// знают ни агент, ни человек, подтверждающий план.
    #[test]
    fn a_relative_path_is_refused() {
        assert!(local_path("logs/app.log", Some(&home())).is_err());
        assert!(local_path("./app.log", Some(&home())).is_err());
    }

    /// `..` делает показанный в плане путь не тем, куда пойдёт запись.
    #[test]
    fn a_path_with_parent_components_is_refused() {
        let err = local_path("/home/someone/x/../.ssh/id_ed25519", Some(&home())).unwrap_err();
        assert!(err.contains(".."), "{err}");
        assert!(local_path("~/x/../../etc/passwd", Some(&home())).is_err());
    }

    #[test]
    fn an_empty_path_is_refused() {
        assert!(local_path("   ", Some(&home())).is_err());
    }

    #[test]
    fn sizes_are_written_for_people() {
        assert_eq!(human_size(512), "512 байт");
        assert_eq!(human_size(2048), "2,0 КБ");
        assert_eq!(human_size(5 * 1024 * 1024 + 300 * 1024), "5,3 МБ");
    }
}
