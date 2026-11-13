//! Передача файла или папки целиком между любыми двумя сторонами (#sftp-tree).
//!
//! Сторона — компьютер или сервер по SFTP. Движок не знает, какая именно: ему
//! нужен только `TreeFs`. Поэтому один и тот же код ходит по дереву, создаёт
//! папки и гонит содержимое **потоком** в обе стороны и между двумя серверами —
//! без промежуточных файлов на диске.
//!
//! Правила, ради которых это написано так, а не проще:
//! - **ничего не затирается без спроса**: по умолчанию верхняя папка (или файл)
//!   получает свободное имя `имя (2)`, а всё внутри кладётся в только что
//!   созданное место. Заменить можно только явно (`OnConflict::Replace`), и тогда
//!   новый файл сначала целиком пишется рядом во временный, а прежний
//!   подменяется им в самом конце: обрыв или отмена прежний файл не трогают;
//! - **символические ссылки не обходятся и не копируются**: по ссылке можно
//!   выйти из выбранной папки (на `/` или в чужой каталог), а копия ссылки
//!   на другом конце означала бы совсем другое;
//! - **оборвавшийся файл не остаётся**: недокопированный файл выглядит целым,
//!   поэтому на ошибке и при отмене он удаляется; уже скопированные до него
//!   остаются, и об этом говорит итог;
//! - **ошибка называет файл**, на котором всё остановилось.

use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const CHUNK: usize = 32 * 1024;
/// Предохранители: дерево, которого человек не ожидал, не должно подвесить
/// приложение и забить диск.
const MAX_ENTRIES: usize = 200_000;
const MAX_DEPTH: usize = 64;

/// Что делать, если в папке назначения уже есть запись с таким именем.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OnConflict {
    /// Положить рядом под свободным именем `имя (2)`.
    #[default]
    Rename,
    /// Файл заменить, папку слить: что совпало по имени — заменить, остальное
    /// в ней не трогать.
    Replace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Dir,
    File,
    /// Ссылка, устройство, сокет — не копируется.
    Other,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    pub kind: NodeKind,
    pub size: u64,
}

/// Одна сторона передачи.
#[async_trait]
pub trait TreeFs: Send + Sync {
    /// Склеить путь папки и имени по правилам этой стороны.
    fn join(&self, dir: &str, name: &str) -> String;
    async fn kind_of(&self, path: &str) -> Result<Option<(NodeKind, u64)>, String>;
    async fn list(&self, dir: &str) -> Result<Vec<Node>, String>;
    async fn mkdir(&self, path: &str) -> Result<(), String>;
    async fn open_read(&self, path: &str) -> Result<Box<dyn AsyncRead + Unpin + Send>, String>;
    async fn create_write(&self, path: &str) -> Result<Box<dyn AsyncWrite + Unpin + Send>, String>;
    async fn remove_file(&self, path: &str) -> Result<(), String>;
    /// Поставить готовый `tmp` на место `target`, который уже есть. Прежний
    /// файл должен остаться целым, если подменить не вышло.
    async fn replace_with(&self, tmp: &str, target: &str) -> Result<(), String>;
}

/// Ошибка с путём, на котором остановились.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeError {
    /// Путь относительно выбранной папки; для одиночного файла — его имя.
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Done { files: usize, bytes: u64, skipped: usize, dest: String },
    Cancelled { files: usize },
}

#[derive(Debug, Clone)]
pub struct Progress {
    pub transferred: u64,
    /// Полный размер дерева, известен после обхода.
    pub total: Option<u64>,
    /// Файл, который копируется сейчас.
    pub current: String,
}

struct Plan {
    /// Папки, родители раньше детей. Пути относительные, через `/`.
    dirs: Vec<String>,
    files: Vec<(String, u64)>,
    skipped: usize,
}

fn under(fs: &dyn TreeFs, root: &str, rel: &str) -> String {
    rel.split('/')
        .filter(|p| !p.is_empty())
        .fold(root.to_string(), |acc, part| fs.join(&acc, part))
}

fn join_rel(rel: &str, name: &str) -> String {
    if rel.is_empty() {
        name.to_string()
    } else {
        format!("{rel}/{name}")
    }
}

/// Папка и имя записи относительно `top`: `a/b/two.txt` → (`top/a/b`, `two.txt`).
fn split_target(fs: &dyn TreeFs, top: &str, rel: &str) -> (String, String) {
    match rel.rsplit_once('/') {
        Some((dir, name)) => (under(fs, top, dir), name.to_string()),
        None => (top.to_string(), rel.to_string()),
    }
}

/// Имя временного файла рядом с заменяемым: скрытое и с меткой, чтобы
/// оставшееся после сбоя питания было видно, чьё оно.
fn temp_name(name: &str) -> String {
    let tag = uuid::Uuid::new_v4().simple().to_string();
    format!(".{name}.{}.darwaza-part", &tag[..8])
}

/// Обход источника: сколько папок, файлов и байт. Ссылки пропускаются.
async fn scan(src: &dyn TreeFs, root: &str, cancel: &AtomicBool) -> Result<Plan, TreeError> {
    let mut plan = Plan { dirs: Vec::new(), files: Vec::new(), skipped: 0 };
    // Очередь, а не рекурсия: так родители гарантированно идут раньше детей.
    let mut queue = std::collections::VecDeque::from([String::new()]);
    while let Some(rel) = queue.pop_front() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if rel.split('/').filter(|p| !p.is_empty()).count() >= MAX_DEPTH {
            return Err(TreeError { path: rel, message: "слишком глубокая вложенность".into() });
        }
        let dir = under(src, root, &rel);
        let nodes = src
            .list(&dir)
            .await
            .map_err(|message| TreeError { path: rel.clone(), message })?;
        for node in nodes {
            let child = join_rel(&rel, &node.name);
            match node.kind {
                NodeKind::Dir => {
                    plan.dirs.push(child.clone());
                    queue.push_back(child);
                }
                NodeKind::File => plan.files.push((child, node.size)),
                NodeKind::Other => plan.skipped += 1,
            }
            if plan.dirs.len() + plan.files.len() > MAX_ENTRIES {
                return Err(TreeError {
                    path: rel,
                    message: "слишком много файлов в одной передаче".into(),
                });
            }
        }
    }
    Ok(plan)
}

/// Свободное имя в папке назначения: `имя`, `имя (2)`, `имя (3)`…
async fn free_name(dst: &dyn TreeFs, dir: &str, name: &str) -> Result<String, String> {
    if dst.kind_of(&dst.join(dir, name)).await?.is_none() {
        return Ok(name.to_string());
    }
    // Расширение не трогаем у папок, у файлов вставляем номер перед ним.
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 2..1000 {
        let candidate = format!("{stem} ({n}){ext}");
        if dst.kind_of(&dst.join(dir, &candidate)).await?.is_none() {
            return Ok(candidate);
        }
    }
    Err(format!("не нашлось свободного имени для {name}"))
}

/// Скопировать один файл потоком. `Ok(false)` — отменено на середине.
async fn copy_file(
    src: &dyn TreeFs,
    from: &str,
    dst: &dyn TreeFs,
    to: &str,
    cancel: &AtomicBool,
    transferred: &mut u64,
    on_chunk: &mut (dyn FnMut(u64) + Send),
) -> Result<bool, String> {
    let mut reader = src.open_read(from).await?;
    let mut writer = dst.create_write(to).await?;
    let mut buf = vec![0u8; CHUNK];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let n = reader.read(&mut buf).await.map_err(|e| format!("чтение: {e}"))?;
        if n == 0 {
            break;
        }
        writer.write_all(&buf[..n]).await.map_err(|e| format!("запись: {e}"))?;
        *transferred += n as u64;
        on_chunk(*transferred);
    }
    writer.flush().await.map_err(|e| format!("запись: {e}"))?;
    writer.shutdown().await.map_err(|e| format!("запись: {e}"))?;
    Ok(true)
}

/// Скопировать `src_path` (папку или файл) в `dst_dir` на другой стороне.
///
/// `on_progress` зовётся часто; частоту ограничивает вызывающий.
#[allow(clippy::too_many_arguments)]
pub async fn copy_tree(
    src: &dyn TreeFs,
    src_path: &str,
    name: &str,
    is_dir: bool,
    dst: &dyn TreeFs,
    dst_dir: &str,
    on_conflict: OnConflict,
    cancel: &AtomicBool,
    on_progress: &mut (dyn FnMut(Progress) + Send),
) -> Result<Outcome, TreeError> {
    let plan = if is_dir {
        let plan = scan(src, src_path, cancel).await?;
        if cancel.load(Ordering::Relaxed) {
            return Ok(Outcome::Cancelled { files: 0 });
        }
        plan
    } else {
        let size = match src.kind_of(src_path).await {
            Ok(Some((NodeKind::File, size))) => size,
            Ok(_) => {
                return Err(TreeError { path: name.into(), message: "это не обычный файл".into() })
            }
            Err(message) => return Err(TreeError { path: name.into(), message }),
        };
        Plan { dirs: Vec::new(), files: vec![(String::new(), size)], skipped: 0 }
    };
    let total: u64 = plan.files.iter().map(|(_, s)| *s).sum();

    let top_err = |message: String| TreeError { path: name.into(), message };
    // `merging` — кладём поверх того, что уже лежит под этим именем.
    let (top, merging) = match on_conflict {
        OnConflict::Rename => {
            let free = free_name(dst, dst_dir, name).await.map_err(top_err)?;
            (dst.join(dst_dir, &free), false)
        }
        OnConflict::Replace => {
            let top = dst.join(dst_dir, name);
            match dst.kind_of(&top).await.map_err(top_err)? {
                None => (top, false),
                Some((NodeKind::Dir, _)) if is_dir => (top, true),
                Some((NodeKind::File, _)) if !is_dir => (top, true),
                Some(_) => {
                    return Err(top_err(if is_dir {
                        "на месте папки лежит файл с тем же именем".into()
                    } else {
                        "на месте файла лежит папка с тем же именем".into()
                    }))
                }
            }
        }
    };

    if is_dir {
        if !merging {
            dst.mkdir(&top).await.map_err(top_err)?;
        }
        for rel in &plan.dirs {
            let path = under(dst, &top, rel);
            let rel_err = |message: String| TreeError { path: rel.clone(), message };
            if merging {
                match dst.kind_of(&path).await.map_err(rel_err)? {
                    None => dst.mkdir(&path).await.map_err(rel_err)?,
                    Some((NodeKind::Dir, _)) => {}
                    Some(_) => return Err(rel_err("на месте папки лежит файл с тем же именем".into())),
                }
            } else {
                dst.mkdir(&path).await.map_err(rel_err)?;
            }
        }
    }

    let mut transferred = 0u64;
    let mut copied = 0usize;
    for (rel, _) in &plan.files {
        let from = if is_dir { under(src, src_path, rel) } else { src_path.to_string() };
        let to = if is_dir { under(dst, &top, rel) } else { top.clone() };
        let shown = if is_dir { rel.clone() } else { name.to_string() };
        let current = shown.clone();
        let mut tick = |done: u64| {
            on_progress(Progress {
                transferred: done,
                total: Some(total),
                current: current.clone(),
            });
        };
        // Заменяем, только если файл там правда есть: остальное пишется как обычно.
        let replacing = merging
            && match dst.kind_of(&to).await {
                Ok(None) => false,
                Ok(Some((NodeKind::File, _))) => true,
                Ok(Some(_)) => {
                    return Err(TreeError {
                        path: shown,
                        message: "на месте файла лежит папка с тем же именем".into(),
                    })
                }
                Err(message) => return Err(TreeError { path: shown, message }),
            };
        let write_to = if replacing {
            let (dir, file) = if is_dir {
                split_target(dst, &top, rel)
            } else {
                (dst_dir.to_string(), name.to_string())
            };
            dst.join(&dir, &temp_name(&file))
        } else {
            to.clone()
        };
        match copy_file(src, &from, dst, &write_to, cancel, &mut transferred, &mut tick).await {
            Ok(true) => {
                if replacing {
                    if let Err(message) = dst.replace_with(&write_to, &to).await {
                        dst.remove_file(&write_to).await.ok();
                        return Err(TreeError { path: shown, message });
                    }
                }
                copied += 1;
            }
            Ok(false) => {
                dst.remove_file(&write_to).await.ok();
                return Ok(Outcome::Cancelled { files: copied });
            }
            Err(message) => {
                dst.remove_file(&write_to).await.ok();
                return Err(TreeError { path: shown, message });
            }
        }
    }
    Ok(Outcome::Done { files: copied, bytes: transferred, skipped: plan.skipped, dest: top })
}

// ─────────────────────────────────────────────────────────────────────────
// Стороны
// ─────────────────────────────────────────────────────────────────────────

/// Компьютер.
pub struct LocalTree;

#[async_trait]
impl TreeFs for LocalTree {
    fn join(&self, dir: &str, name: &str) -> String {
        std::path::Path::new(dir).join(name).to_string_lossy().to_string()
    }

    async fn kind_of(&self, path: &str) -> Result<Option<(NodeKind, u64)>, String> {
        match tokio::fs::symlink_metadata(path).await {
            Ok(m) if m.is_dir() => Ok(Some((NodeKind::Dir, 0))),
            Ok(m) if m.is_file() => Ok(Some((NodeKind::File, m.len()))),
            Ok(_) => Ok(Some((NodeKind::Other, 0))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    async fn list(&self, dir: &str) -> Result<Vec<Node>, String> {
        let mut read = tokio::fs::read_dir(dir).await.map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        while let Some(item) = read.next_entry().await.map_err(|e| e.to_string())? {
            // symlink_metadata: ссылка остаётся ссылкой и в дерево не входит.
            let Ok(meta) = tokio::fs::symlink_metadata(item.path()).await else { continue };
            let kind = if meta.is_dir() {
                NodeKind::Dir
            } else if meta.is_file() {
                NodeKind::File
            } else {
                NodeKind::Other
            };
            out.push(Node {
                name: item.file_name().to_string_lossy().to_string(),
                kind,
                size: if kind == NodeKind::File { meta.len() } else { 0 },
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    async fn mkdir(&self, path: &str) -> Result<(), String> {
        tokio::fs::create_dir(path).await.map_err(|e| e.to_string())
    }

    async fn open_read(&self, path: &str) -> Result<Box<dyn AsyncRead + Unpin + Send>, String> {
        let f = tokio::fs::File::open(path).await.map_err(|e| e.to_string())?;
        Ok(Box::new(f))
    }

    async fn create_write(&self, path: &str) -> Result<Box<dyn AsyncWrite + Unpin + Send>, String> {
        // `create_new`: если файл вдруг появился между проверкой имени и
        // записью, лучше отказ, чем затирание.
        let f = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .await
            .map_err(|e| e.to_string())?;
        Ok(Box::new(f))
    }

    async fn remove_file(&self, path: &str) -> Result<(), String> {
        tokio::fs::remove_file(path).await.map_err(|e| e.to_string())
    }

    async fn replace_with(&self, tmp: &str, target: &str) -> Result<(), String> {
        // `rename` поверх существующего файла заменяет его одним шагом и на
        // unix, и на Windows (MoveFileEx с заменой).
        tokio::fs::rename(tmp, target).await.map_err(|e| e.to_string())
    }
}

/// Сервер по SFTP.
pub struct RemoteTree(pub std::sync::Arc<russh_sftp::client::SftpSession>);

#[async_trait]
impl TreeFs for RemoteTree {
    fn join(&self, dir: &str, name: &str) -> String {
        crate::sftp::join_path(dir, name)
    }

    async fn kind_of(&self, path: &str) -> Result<Option<(NodeKind, u64)>, String> {
        use russh_sftp::client::error::Error as SftpError;
        use russh_sftp::protocol::{FileType, StatusCode};
        match self.0.symlink_metadata(path.to_string()).await {
            Ok(meta) => {
                let kind = match meta.file_type() {
                    FileType::Dir => NodeKind::Dir,
                    FileType::File => NodeKind::File,
                    _ => NodeKind::Other,
                };
                Ok(Some((kind, meta.size.unwrap_or(0))))
            }
            Err(SftpError::Status(s)) if s.status_code == StatusCode::NoSuchFile => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    async fn list(&self, dir: &str) -> Result<Vec<Node>, String> {
        use russh_sftp::protocol::FileType;
        let entries = self.0.read_dir(dir.to_string()).await.map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for e in entries {
            let name = e.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let meta = e.metadata();
            let kind = match meta.file_type() {
                FileType::Dir => NodeKind::Dir,
                FileType::File => NodeKind::File,
                _ => NodeKind::Other,
            };
            out.push(Node { name, kind, size: meta.size.unwrap_or(0) });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    async fn mkdir(&self, path: &str) -> Result<(), String> {
        self.0.create_dir(path.to_string()).await.map_err(|e| e.to_string())
    }

    async fn open_read(&self, path: &str) -> Result<Box<dyn AsyncRead + Unpin + Send>, String> {
        let f = self.0.open(path.to_string()).await.map_err(|e| e.to_string())?;
        Ok(Box::new(f))
    }

    async fn create_write(&self, path: &str) -> Result<Box<dyn AsyncWrite + Unpin + Send>, String> {
        // Только новый файл, как и на компьютере: если он появился между
        // проверкой имени и записью, лучше отказ, чем затирание.
        use russh_sftp::protocol::OpenFlags;
        let flags = OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE;
        let f = self
            .0
            .open_with_flags(path.to_string(), flags)
            .await
            .map_err(|e| e.to_string())?;
        Ok(Box::new(f))
    }

    async fn remove_file(&self, path: &str) -> Result<(), String> {
        self.0.remove_file(path.to_string()).await.map_err(|e| e.to_string())
    }

    async fn replace_with(&self, tmp: &str, target: &str) -> Result<(), String> {
        // SFTP v3 не переименовывает поверх существующего файла. Поэтому
        // прежний сначала отходит в сторону, новый встаёт на его место, и
        // только потом прежний удаляется. Не встал — прежний возвращается.
        let aside = format!("{target}.darwaza-old");
        if self.0.try_exists(aside.clone()).await.unwrap_or(false) {
            self.0
                .remove_file(aside.clone())
                .await
                .map_err(|e| format!("{aside}: {e}"))?;
        }
        self.0
            .rename(target.to_string(), aside.clone())
            .await
            .map_err(|e| format!("не удалось отложить прежний файл: {e}"))?;
        if let Err(e) = self.0.rename(tmp.to_string(), target.to_string()).await {
            return match self.0.rename(aside.clone(), target.to_string()).await {
                Ok(()) => Err(format!("не удалось поставить новый файл, прежний на месте: {e}")),
                Err(_) => Err(format!(
                    "не удалось поставить новый файл, и прежний НЕ вернулся — он лежит в {aside}: {e}"
                )),
            };
        }
        if let Err(e) = self.0.remove_file(aside.clone()).await {
            tracing::warn!(target: "transfers", "прежний файл остался рядом, {aside}: {e}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("darwaza-tree-{}-{name}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_tree(root: &Path) {
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::create_dir_all(root.join("empty")).unwrap();
        std::fs::write(root.join("top.txt"), vec![1u8; 100]).unwrap();
        std::fs::write(root.join("a/one.bin"), vec![2u8; 70_000]).unwrap();
        std::fs::write(root.join("a/b/two.txt"), "два").unwrap();
    }

    async fn run(
        src: &Path,
        name: &str,
        dst_dir: &Path,
        cancel: &AtomicBool,
    ) -> (Result<Outcome, TreeError>, Vec<Progress>) {
        let mut seen = Vec::new();
        let mut cb = |p: Progress| seen.push(p);
        let result = copy_tree(
            &LocalTree,
            &src.join(name).to_string_lossy(),
            name,
            src.join(name).is_dir(),
            &LocalTree,
            &dst_dir.to_string_lossy(),
            OnConflict::Rename,
            cancel,
            &mut cb,
        )
        .await;
        (result, seen)
    }

    #[tokio::test]
    async fn a_folder_is_copied_whole_with_structure_and_empty_dirs() {
        let src = scratch("whole-src");
        let dst = scratch("whole-dst");
        sample_tree(&src.join("proj"));

        let (result, progress) = run(&src, "proj", &dst, &AtomicBool::new(false)).await;
        let Outcome::Done { files, bytes, .. } = result.unwrap() else { panic!("не Done") };
        assert_eq!(files, 3);
        assert_eq!(bytes, 100 + 70_000 + "два".len() as u64);
        assert_eq!(std::fs::read(dst.join("proj/a/one.bin")).unwrap(), vec![2u8; 70_000]);
        assert_eq!(std::fs::read_to_string(dst.join("proj/a/b/two.txt")).unwrap(), "два");
        assert!(dst.join("proj/empty").is_dir(), "пустая папка тоже переезжает");

        // Прогресс идёт по общему размеру дерева и доходит до него.
        let last = progress.last().unwrap();
        assert_eq!(last.total, Some(bytes));
        assert_eq!(last.transferred, bytes);
        assert!(progress.windows(2).all(|w| w[0].transferred <= w[1].transferred));
    }

    #[tokio::test]
    async fn nothing_is_overwritten_the_copy_gets_a_free_name() {
        let src = scratch("name-src");
        let dst = scratch("name-dst");
        sample_tree(&src.join("proj"));
        std::fs::create_dir_all(dst.join("proj")).unwrap();
        std::fs::write(dst.join("proj/top.txt"), "чужое").unwrap();

        let (result, _) = run(&src, "proj", &dst, &AtomicBool::new(false)).await;
        assert!(matches!(result.unwrap(), Outcome::Done { .. }));
        assert_eq!(std::fs::read_to_string(dst.join("proj/top.txt")).unwrap(), "чужое");
        assert!(dst.join("proj (2)/top.txt").exists());

        // Одиночный файл: номер перед расширением.
        std::fs::write(src.join("r.txt"), "новое").unwrap();
        std::fs::write(dst.join("r.txt"), "старое").unwrap();
        let (result, _) = run(&src, "r.txt", &dst, &AtomicBool::new(false)).await;
        assert!(matches!(result.unwrap(), Outcome::Done { .. }));
        assert_eq!(std::fs::read_to_string(dst.join("r.txt")).unwrap(), "старое");
        assert_eq!(std::fs::read_to_string(dst.join("r (2).txt")).unwrap(), "новое");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinks_are_neither_followed_nor_copied() {
        let src = scratch("link-src");
        let dst = scratch("link-dst");
        sample_tree(&src.join("proj"));
        // Ссылка на каталог ВНЕ выбранной папки: пойти по ней значило бы
        // выйти за пределы того, что человек выбрал.
        let outside = scratch("link-outside");
        std::fs::write(outside.join("secret.txt"), "не должно уехать").unwrap();
        std::os::unix::fs::symlink(&outside, src.join("proj/escape")).unwrap();

        let (result, _) = run(&src, "proj", &dst, &AtomicBool::new(false)).await;
        let Outcome::Done { skipped, files, .. } = result.unwrap() else { panic!() };
        assert_eq!(skipped, 1);
        assert_eq!(files, 3);
        assert!(!dst.join("proj/escape").exists());
        assert!(!dst.join("proj/escape/secret.txt").exists());
    }

    #[tokio::test]
    async fn cancelling_removes_the_half_written_file_and_keeps_finished_ones() {
        let src = scratch("cancel-src");
        let dst = scratch("cancel-dst");
        sample_tree(&src.join("proj"));
        let cancel = AtomicBool::new(false);

        // Отмена приходит, как только набрался первый же файл.
        let mut cb = |p: Progress| {
            if p.transferred >= 100 {
                cancel.store(true, Ordering::Relaxed);
            }
        };
        let result = copy_tree(
            &LocalTree,
            &src.join("proj").to_string_lossy(),
            "proj",
            true,
            &LocalTree,
            &dst.to_string_lossy(),
            OnConflict::Rename,
            &cancel,
            &mut cb,
        )
        .await;
        assert!(matches!(result.unwrap(), Outcome::Cancelled { .. }));

        // На месте нет файлов, которые выглядят целыми, но обрезаны: каждый
        // лежащий файл совпадает с источником полностью.
        for rel in ["top.txt", "a/one.bin", "a/b/two.txt"] {
            let copy = dst.join("proj").join(rel);
            if copy.exists() {
                assert_eq!(
                    std::fs::read(&copy).unwrap(),
                    std::fs::read(src.join("proj").join(rel)).unwrap(),
                    "{rel} лежит недокопированным"
                );
            }
        }
    }

    #[tokio::test]
    async fn the_error_names_the_file_where_it_stopped() {
        let src = scratch("err-src");
        sample_tree(&src.join("proj"));
        struct Broken;
        #[async_trait]
        impl TreeFs for Broken {
            fn join(&self, d: &str, n: &str) -> String { LocalTree.join(d, n) }
            async fn kind_of(&self, p: &str) -> Result<Option<(NodeKind, u64)>, String> { LocalTree.kind_of(p).await }
            async fn list(&self, d: &str) -> Result<Vec<Node>, String> { LocalTree.list(d).await }
            async fn mkdir(&self, p: &str) -> Result<(), String> { LocalTree.mkdir(p).await }
            async fn open_read(&self, p: &str) -> Result<Box<dyn AsyncRead + Unpin + Send>, String> {
                if p.ends_with("two.txt") { Err("доступ запрещён".into()) } else { LocalTree.open_read(p).await }
            }
            async fn create_write(&self, p: &str) -> Result<Box<dyn AsyncWrite + Unpin + Send>, String> { LocalTree.create_write(p).await }
            async fn remove_file(&self, p: &str) -> Result<(), String> { LocalTree.remove_file(p).await }
            async fn replace_with(&self, t: &str, p: &str) -> Result<(), String> { LocalTree.replace_with(t, p).await }
        }
        let dst2 = scratch("err-dst2");
        let mut cb = |_p: Progress| {};
        let err = copy_tree(
            &Broken,
            &src.join("proj").to_string_lossy(),
            "proj",
            true,
            &LocalTree,
            &dst2.to_string_lossy(),
            OnConflict::Rename,
            &AtomicBool::new(false),
            &mut cb,
        )
        .await
        .unwrap_err();
        assert_eq!(err.path, "a/b/two.txt");
        assert!(err.message.contains("доступ запрещён"));
        // Файл, на котором остановились, не остался пустым огрызком.
        assert!(!dst2.join("proj/a/b/two.txt").exists());
    }

    #[tokio::test]
    async fn a_dir_that_is_too_deep_is_refused() {
        let src = scratch("deep-src");
        let dst = scratch("deep-dst");
        let mut deep = src.join("d");
        for _ in 0..70 {
            deep = deep.join("x");
        }
        std::fs::create_dir_all(&deep).unwrap();
        let (result, _) = run(&src, "d", &dst, &AtomicBool::new(false)).await;
        assert!(result.unwrap_err().message.contains("вложенность"));
    }

    async fn replace(src: &Path, name: &str, dst_dir: &Path) -> Result<Outcome, TreeError> {
        let mut cb = |_p: Progress| {};
        copy_tree(
            &LocalTree,
            &src.join(name).to_string_lossy(),
            name,
            src.join(name).is_dir(),
            &LocalTree,
            &dst_dir.to_string_lossy(),
            OnConflict::Replace,
            &AtomicBool::new(false),
            &mut cb,
        )
        .await
    }

    /// Временных `.darwaza-part` после передачи остаться не должно.
    fn no_temp_files(dir: &Path) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            assert!(!name.ends_with(".darwaza-part"), "остался временный {name}");
            if entry.path().is_dir() {
                no_temp_files(&entry.path());
            }
        }
    }

    #[tokio::test]
    async fn replace_puts_the_new_file_in_place_of_the_old_one() {
        let src = scratch("repl-src");
        let dst = scratch("repl-dst");
        std::fs::write(src.join("app.conf"), "новый").unwrap();
        std::fs::write(dst.join("app.conf"), "старый, длиннее нового").unwrap();

        let Outcome::Done { dest, .. } = replace(&src, "app.conf", &dst).await.unwrap() else {
            panic!("не Done")
        };
        assert_eq!(dest, dst.join("app.conf").to_string_lossy());
        assert_eq!(std::fs::read_to_string(dst.join("app.conf")).unwrap(), "новый");
        assert!(!dst.join("app (2).conf").exists(), "заменить — не класть рядом");
        no_temp_files(&dst);
    }

    #[tokio::test]
    async fn replace_merges_a_folder_and_keeps_what_only_the_target_has() {
        let src = scratch("merge-src");
        let dst = scratch("merge-dst");
        sample_tree(&src.join("proj"));
        std::fs::create_dir_all(dst.join("proj/a")).unwrap();
        std::fs::write(dst.join("proj/a/one.bin"), "старое").unwrap();
        std::fs::write(dst.join("proj/a/mine.txt"), "только здесь").unwrap();

        replace(&src, "proj", &dst).await.unwrap();

        assert_eq!(std::fs::read(dst.join("proj/a/one.bin")).unwrap(), vec![2u8; 70_000]);
        assert_eq!(std::fs::read_to_string(dst.join("proj/a/mine.txt")).unwrap(), "только здесь");
        assert_eq!(std::fs::read_to_string(dst.join("proj/a/b/two.txt")).unwrap(), "два");
        assert!(!dst.join("proj (2)").exists());
        no_temp_files(&dst);
    }

    #[tokio::test]
    async fn replace_refuses_to_put_a_folder_over_a_file() {
        let src = scratch("kind-src");
        let dst = scratch("kind-dst");
        sample_tree(&src.join("proj"));
        std::fs::write(dst.join("proj"), "это файл").unwrap();

        let err = replace(&src, "proj", &dst).await.unwrap_err();
        assert!(err.message.contains("файл с тем же именем"));
        assert_eq!(std::fs::read_to_string(dst.join("proj")).unwrap(), "это файл");
    }

    #[tokio::test]
    async fn a_failed_replace_leaves_the_old_file_untouched() {
        let src = scratch("fail-src");
        let dst = scratch("fail-dst");
        std::fs::write(src.join("app.conf"), "новый").unwrap();
        std::fs::write(dst.join("app.conf"), "старый").unwrap();
        struct BrokenRead;
        #[async_trait]
        impl TreeFs for BrokenRead {
            fn join(&self, d: &str, n: &str) -> String { LocalTree.join(d, n) }
            async fn kind_of(&self, p: &str) -> Result<Option<(NodeKind, u64)>, String> { LocalTree.kind_of(p).await }
            async fn list(&self, d: &str) -> Result<Vec<Node>, String> { LocalTree.list(d).await }
            async fn mkdir(&self, p: &str) -> Result<(), String> { LocalTree.mkdir(p).await }
            async fn open_read(&self, _p: &str) -> Result<Box<dyn AsyncRead + Unpin + Send>, String> {
                Err("связь оборвалась".into())
            }
            async fn create_write(&self, p: &str) -> Result<Box<dyn AsyncWrite + Unpin + Send>, String> { LocalTree.create_write(p).await }
            async fn remove_file(&self, p: &str) -> Result<(), String> { LocalTree.remove_file(p).await }
            async fn replace_with(&self, t: &str, p: &str) -> Result<(), String> { LocalTree.replace_with(t, p).await }
        }
        let mut cb = |_p: Progress| {};
        let err = copy_tree(
            &BrokenRead,
            &src.join("app.conf").to_string_lossy(),
            "app.conf",
            false,
            &LocalTree,
            &dst.to_string_lossy(),
            OnConflict::Replace,
            &AtomicBool::new(false),
            &mut cb,
        )
        .await;
        // kind_of источника проходит, ломается чтение — прежний файл цел.
        assert!(err.is_err());
        assert_eq!(std::fs::read_to_string(dst.join("app.conf")).unwrap(), "старый");
        no_temp_files(&dst);
    }
}
