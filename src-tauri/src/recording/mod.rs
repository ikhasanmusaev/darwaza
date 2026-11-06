//! Запись сессии в файл (#144).
//!
//! Формат — **asciinema v2** (`.cast`), а не сырой дамп: такие записи
//! проигрываются стандартным `asciinema play`, конвертируются в gif сторонними
//! инструментами и остаются валидными, даже если запись оборвалась на середине
//! (файл построчный: заголовок + по событию на строку).
//!
//! Пишем только вывод сервера (`"o"`). Ввод пользователя намеренно не
//! записывается: в него попадают пароли, которые шелл не отображает, и утечь
//! они не должны — тем более в файл, который потом кому-то отправят.

use anyhow::{Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::time::Instant;

/// Каталог с записями: `$XDG_DATA_HOME/darwaza/recordings`.
pub fn recordings_dir() -> Result<PathBuf> {
    let dir = crate::paths::data_dir()?.join("recordings");
    crate::paths::ensure_private_dir(&dir)?;
    Ok(dir)
}

/// Убрать из имени хоста всё, что нельзя класть в имя файла.
fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('_');
    if trimmed.is_empty() {
        "session".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Имя файла записи: `<хост>-<дата>.cast`.
pub fn build_path(host_label: &str, started_unix: u64) -> Result<PathBuf> {
    let stamp = chrono::DateTime::from_timestamp(started_unix as i64, 0)
        .map(|t| t.format("%Y%m%d-%H%M%S").to_string())
        .unwrap_or_else(|| started_unix.to_string());
    Ok(recordings_dir()?.join(format!("{}-{}.cast", sanitize(host_label), stamp)))
}

#[derive(Serialize)]
struct CastHeader {
    version: u8,
    width: u32,
    height: u32,
    timestamp: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
}

/// Какие файлы пишутся прямо сейчас. Список записей помечает их, а окно
/// показывает кнопку «стоп» вместо проигрывания. Живёт здесь, а не в сессии:
/// список строится без обращения к задачам сессий.
fn active() -> &'static std::sync::Mutex<std::collections::HashSet<PathBuf>> {
    static ACTIVE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<PathBuf>>> =
        std::sync::OnceLock::new();
    ACTIVE.get_or_init(Default::default)
}

/// Ключ набора: каталог приводится к настоящему пути, имя файла остаётся.
///
/// Каталог данных может лежать за символической ссылкой (`~/.local/share` на
/// другом разделе — обычное дело), и тогда путь, с которым файл открыли, и
/// путь, с которым его потом проверяют, различаются. Файл целиком не
/// канонизируем: удалённого уже нет, а метку снять всё равно надо.
fn active_key(path: &Path) -> PathBuf {
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => std::fs::canonicalize(parent)
            .map(|dir| dir.join(name))
            .unwrap_or_else(|_| path.to_path_buf()),
        _ => path.to_path_buf(),
    }
}

fn mark_active(path: &Path, on: bool) {
    let path = active_key(path);
    let path = path.as_path();
    // Отравленный замок не повод ронять запись: метка — подсказка для окна.
    if let Ok(mut set) = active().lock() {
        if on {
            set.insert(path.to_path_buf());
        } else {
            set.remove(path);
        }
    }
}

fn is_active(path: &Path) -> bool {
    let path = active_key(path);
    let path = path.as_path();
    active()
        .lock()
        .map(|set| set.contains(path))
        .unwrap_or(false)
}

impl Drop for Recorder {
    /// Метка снимается и тогда, когда сессия оборвалась, не вызвав `finish`:
    /// иначе оборванная запись числилась бы идущей до перезапуска приложения.
    fn drop(&mut self) {
        mark_active(&self.path, false);
    }
}

/// Открытая запись одной сессии.
pub struct Recorder {
    writer: BufWriter<tokio::fs::File>,
    started: Instant,
    path: PathBuf,
    /// Хвост незавершённой UTF-8 последовательности с прошлого чанка.
    ///
    /// Вывод приходит произвольными кусками, и многобайтовый символ легко
    /// рвётся между ними. Если каждый чанк переводить в строку по отдельности,
    /// на границах появится мусор — поэтому недобранные байты переносим дальше.
    pending: Vec<u8>,
}

/// Создать файл записи закрытым с первого байта: `0600`, и только новый.
///
/// В записи — весь вывод терминала, включая то, что человек не хотел бы
/// показывать: содержимое конфигов, токены, эхо паролей. `create_new` не
/// затирает чужой файл и не идёт по ссылке; если имя занято (две вкладки одного
/// хоста в одну секунду), берём следующее, а не пишем в общий файл.
async fn create_private(path: PathBuf) -> Result<(tokio::fs::File, PathBuf)> {
    for attempt in 0..100u32 {
        let candidate = if attempt == 0 {
            path.clone()
        } else {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("session");
            path.with_file_name(format!("{stem}-{}.cast", attempt + 1))
        };
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        match options.open(&candidate).await {
            Ok(file) => return Ok((file, candidate)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e).with_context(|| format!("создание {}", candidate.display())),
        }
    }
    anyhow::bail!("не нашлось свободного имени для записи {}", path.display())
}

impl Recorder {
    /// Начать запись: создать файл и записать заголовок.
    pub async fn start(path: PathBuf, cols: u32, rows: u32, title: Option<String>) -> Result<Self> {
        let started_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let (file, path) = create_private(path).await?;
        let mut writer = BufWriter::new(file);

        let header = CastHeader {
            version: 2,
            width: cols,
            height: rows,
            timestamp: started_unix,
            title,
        };
        let line = serde_json::to_string(&header)?;
        writer.write_all(line.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        // Заголовок на диск сразу: список читает его у идущей записи, а в
        // буфере он до первого сброса не виден.
        writer.flush().await?;

        mark_active(&path, true);
        Ok(Self {
            writer,
            started: Instant::now(),
            path,
            pending: Vec::new(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Записать очередной кусок вывода.
    pub async fn write_output(&mut self, chunk: &[u8]) -> Result<()> {
        self.pending.extend_from_slice(chunk);

        // Отделяем корректный UTF-8 префикс; хвост оставляем до следующего чанка.
        let text = match std::str::from_utf8(&self.pending) {
            Ok(s) => {
                let owned = s.to_string();
                self.pending.clear();
                owned
            }
            Err(e) => {
                let valid_up_to = e.valid_up_to();
                if valid_up_to == 0 {
                    // Пока нечего писать: символ ещё не дособрался.
                    // Но если "хвост" разросся, значит это не UTF-8 вовсе —
                    // сбрасываем с заменой, иначе будем копить вечно.
                    if self.pending.len() > 8 {
                        let lossy = String::from_utf8_lossy(&self.pending).into_owned();
                        self.pending.clear();
                        lossy
                    } else {
                        return Ok(());
                    }
                } else {
                    let owned = String::from_utf8_lossy(&self.pending[..valid_up_to]).into_owned();
                    self.pending.drain(..valid_up_to);
                    owned
                }
            }
        };

        if text.is_empty() {
            return Ok(());
        }

        let elapsed = self.started.elapsed().as_secs_f64();
        // Событие asciinema: [время, "o", данные]. serde_json сам экранирует
        // управляющие последовательности — их в терминальном выводе много.
        let event = serde_json::to_string(&(elapsed, "o", text))?;
        self.writer.write_all(event.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;
        Ok(())
    }

    /// Закрыть запись, дописав буфер на диск.
    pub async fn finish(mut self) -> Result<PathBuf> {
        // Недобранный хвост всё же пишем: лучше один заменённый символ,
        // чем потерянный конец сессии.
        if !self.pending.is_empty() {
            let lossy = String::from_utf8_lossy(&self.pending).into_owned();
            let elapsed = self.started.elapsed().as_secs_f64();
            let event = serde_json::to_string(&(elapsed, "o", lossy))?;
            self.writer.write_all(event.as_bytes()).await?;
            self.writer.write_all(b"\n").await?;
        }
        self.writer.flush().await?;
        Ok(self.path.clone())
    }
}

/// Файл записи для списка в UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingFile {
    pub name: String,
    pub path: String,
    pub size: u64,
    /// Unix-секунды.
    pub modified: Option<i64>,
    /// Подпись из заголовка: «имя хоста (адрес)».
    pub title: Option<String>,
    /// Когда запись началась, Unix-секунды (из заголовка).
    pub started_at: Option<i64>,
    /// Длительность в секундах: по последнему событию; у идущей записи — с
    /// начала до сейчас, потому что события в ней ещё лежат в буфере.
    pub duration_seconds: Option<f64>,
    /// Пишется ли сейчас. Такую запись нельзя проигрывать как законченную.
    pub active: bool,
}

/// Заголовок и последняя временная метка файла записи.
///
/// Читаем только начало и хвост: записи бывают в десятки мегабайт, а список
/// строится целиком при каждом открытии экрана.
async fn peek(path: &Path) -> (Option<String>, Option<i64>, Option<f64>) {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let Ok(mut file) = tokio::fs::File::open(path).await else {
        return (None, None, None);
    };
    let Ok(meta) = file.metadata().await else {
        return (None, None, None);
    };

    let mut head = vec![0u8; 4096.min(meta.len() as usize)];
    if file.read_exact(&mut head).await.is_err() {
        return (None, None, None);
    }
    let header: Option<serde_json::Value> = head
        .split(|b| *b == b'\n')
        .next()
        .and_then(|line| serde_json::from_slice(line).ok());
    let title = header
        .as_ref()
        .and_then(|h| h.get("title"))
        .and_then(|t| t.as_str())
        .map(str::to_string);
    let started_at = header
        .as_ref()
        .and_then(|h| h.get("timestamp"))
        .and_then(|t| t.as_i64());

    let tail_len = 8192.min(meta.len());
    let mut tail = vec![0u8; tail_len as usize];
    let seeked = file
        .seek(std::io::SeekFrom::Start(meta.len() - tail_len))
        .await
        .is_ok();
    let duration = if seeked && file.read_exact(&mut tail).await.is_ok() {
        tail.split(|b| *b == b'\n')
            .rev()
            .filter(|line| !line.is_empty())
            .find_map(|line| serde_json::from_slice::<serde_json::Value>(line).ok())
            .and_then(|event| event.get(0).and_then(|t| t.as_f64()))
    } else {
        None
    };

    (title, started_at, duration)
}

pub async fn list() -> Result<Vec<RecordingFile>> {
    list_in(&recordings_dir()?).await
}

pub async fn list_in(dir: &Path) -> Result<Vec<RecordingFile>> {
    let mut read = tokio::fs::read_dir(dir).await?;
    let mut out = Vec::new();

    while let Some(item) = read.next_entry().await? {
        let path = item.path();
        if path.extension().and_then(|e| e.to_str()) != Some("cast") {
            continue;
        }
        let meta = match item.metadata().await {
            Ok(m) => m,
            // Файл могли удалить между чтением каталога и stat — пропускаем.
            Err(_) => continue,
        };
        let (title, started_at, last_event) = peek(&path).await;
        let running = is_active(&path);
        let duration_seconds = if running {
            started_at.map(|s| (chrono::Utc::now().timestamp() - s).max(0) as f64)
        } else {
            last_event
        };
        out.push(RecordingFile {
            title,
            started_at,
            duration_seconds,
            active: running,
            name: item.file_name().to_string_lossy().to_string(),
            path: path.to_string_lossy().to_string(),
            size: meta.len(),
            modified: meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64),
        });
    }

    // Свежие сверху — их ищут чаще.
    out.sort_by_key(|f| std::cmp::Reverse(f.modified));
    Ok(out)
}

/// Путь внутри каталога записей или ошибка. Команды приходят из WebView, и
/// уводить чтение и удаление за пределы каталога нельзя.
async fn resolve_inside(dir: &Path, path: &str) -> Result<PathBuf> {
    let dir = tokio::fs::canonicalize(dir).await?;
    let canonical = tokio::fs::canonicalize(PathBuf::from(path))
        .await
        .with_context(|| format!("файл не найден: {path}"))?;
    if !canonical.starts_with(&dir) {
        anyhow::bail!("путь вне каталога записей: {path}");
    }
    Ok(canonical)
}

/// Кусок записи для проигрывателя в приложении (#230).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingChunk {
    /// Целые строки файла как есть: первая — заголовок, остальные — события
    /// `[время, "o", текст]`. Разбирает проигрыватель.
    pub lines: Vec<String>,
    /// С какого байта читать следующий кусок.
    pub next_offset: u64,
    /// Дочитали до конца того, что уже есть в файле. У идущей записи позже
    /// может появиться продолжение.
    pub eof: bool,
}

/// Предел куска: проигрыватель берёт файл частями, а не целиком в память.
pub const CHUNK_MAX: u64 = 512 * 1024;
/// Меньше этого кусок не бывает: слишком малый заставил бы читателя крутиться
/// на длинных строках.
pub const CHUNK_MIN: u64 = 4 * 1024;
/// Самая длинная строка записи, которую согласны отдать целиком.
pub const LINE_MAX: u64 = 8 * 1024 * 1024;
const LINE_STEP: u64 = 64 * 1024;

pub async fn read_chunk(path: &str, offset: u64, max_bytes: u64) -> Result<RecordingChunk> {
    read_chunk_in(&recordings_dir()?, path, offset, max_bytes).await
}

pub async fn read_chunk_in(
    dir: &Path,
    path: &str,
    offset: u64,
    max_bytes: u64,
) -> Result<RecordingChunk> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let target = resolve_inside(dir, path).await?;
    let mut file = tokio::fs::File::open(&target).await?;
    let len = file.metadata().await?.len();
    if offset >= len {
        return Ok(RecordingChunk {
            lines: Vec::new(),
            next_offset: offset.min(len),
            eof: true,
        });
    }

    file.seek(std::io::SeekFrom::Start(offset)).await?;
    let want = max_bytes.clamp(CHUNK_MIN, CHUNK_MAX).min(len - offset);
    let mut buf = vec![0u8; want as usize];
    file.read_exact(&mut buf).await?;

    // Строка длиннее куска (большое событие вывода) не режется посередине:
    // дочитываем до конца строки, но не дальше разумного предела. Обрезок
    // сломал бы и эту строку, и следующий кусок, который начался бы с её хвоста.
    while !buf.contains(&b'\n') && offset + (buf.len() as u64) < len {
        if buf.len() as u64 >= LINE_MAX {
            anyhow::bail!("строка записи длиннее {LINE_MAX} байт");
        }
        let more = (len - offset - buf.len() as u64).min(LINE_STEP) as usize;
        let at = buf.len();
        buf.resize(at + more, 0);
        file.read_exact(&mut buf[at..]).await?;
    }

    // Берём только целые строки: обрезанная посередине не разберётся ни у
    // кого, а продолжение придёт со следующим куском.
    let complete = match buf.iter().rposition(|b| *b == b'\n') {
        Some(last) => last + 1,
        None => 0,
    };
    let text = String::from_utf8_lossy(&buf[..complete]).into_owned();
    let lines = text.lines().map(str::to_string).collect();

    Ok(RecordingChunk {
        lines,
        next_offset: offset + complete as u64,
        eof: offset + complete as u64 >= len,
    })
}

/// Удалить запись. Путь проверяем: команда приходит из WebView, и уводить
/// удаление за пределы каталога записей нельзя.
pub async fn remove(path: &str) -> Result<()> {
    remove_in(&recordings_dir()?, path).await
}

pub async fn remove_in(dir: &Path, path: &str) -> Result<()> {
    let canonical = resolve_inside(dir, path).await?;
    if is_active(&canonical) {
        anyhow::bail!("запись ещё идёт: сначала остановите её");
    }
    tokio::fs::remove_file(&canonical).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_host_names_for_filenames() {
        assert_eq!(sanitize("web-1.prod"), "web-1.prod");
        assert_eq!(sanitize("root@10.0.0.1:22"), "root_10.0.0.1_22");
        // Пустое имя не должно давать файл с именем из одних подчёркиваний.
        assert_eq!(sanitize("///"), "session");
        assert_eq!(sanitize(""), "session");
    }

    /// Запись закрыта с первого байта: `0600`, а не `0666 & ~umask`.
    ///
    /// Тест идёт с `umask 0`, иначе при обычной маске права получились бы
    /// узкими и без исправления.
    #[test]
    fn a_recording_is_private_from_the_first_byte() {
        let dir = std::env::temp_dir().join(format!("darwaza-rec-perm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("private.cast");

        let created = crate::paths::tests::with_open_umask(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let rec = Recorder::start(path.clone(), 80, 24, None).await.unwrap();
                    rec.path().to_path_buf()
                })
        });

        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&created).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "права {mode:o}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Занятое имя не затирается: вторая запись получает другое.
    #[tokio::test]
    async fn an_existing_recording_is_never_overwritten() {
        let dir = std::env::temp_dir().join(format!("darwaza-rec-excl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("same.cast");
        std::fs::write(&path, "чужое").unwrap();

        let rec = Recorder::start(path.clone(), 80, 24, None).await.unwrap();

        assert_ne!(rec.path(), path.as_path());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "чужое");
        drop(rec);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Пометка «идёт» не зависит от того, каким путём называют файл. Каталог за
    /// символической ссылкой: открыли по ссылке, проверяют по настоящему пути —
    /// и идущую запись нельзя было бы отличить от остановленной.
    #[tokio::test]
    async fn a_running_recording_is_recognised_through_a_symlinked_directory() {
        let base = std::env::temp_dir().join(format!("darwaza-rec-link-{}", std::process::id()));
        let real = base.join("real");
        let link = base.join("link");
        std::fs::remove_dir_all(&base).ok();
        std::fs::create_dir_all(&real).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let rec = Recorder::start(link.join("live.cast"), 80, 24, None)
            .await
            .unwrap();

        // Удаляют по настоящему пути, а открыли через ссылку.
        let refused = remove_in(&real, real.join("live.cast").to_str().unwrap()).await;
        assert!(refused.is_err(), "идущую запись удалять нельзя");
        assert!(real.join("live.cast").exists());
        drop(rec);
        std::fs::remove_dir_all(&base).ok();
    }

    #[tokio::test]
    async fn splits_multibyte_across_chunks_without_corruption() {
        let dir = std::env::temp_dir().join(format!("darwaza-rec-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("split.cast");

        let mut rec = Recorder::start(path.clone(), 80, 24, None)
            .await
            .expect("start");

        // "привет" в UTF-8 — двухбайтовые символы. Рвём поток посреди символа.
        let text = "привет".as_bytes().to_vec();
        let (head, tail) = text.split_at(3);
        rec.write_output(head).await.expect("write head");
        rec.write_output(tail).await.expect("write tail");
        rec.finish().await.expect("finish");

        // События могут лечь несколькими строками (первый чанк оборвался
        // посреди символа), поэтому склеиваем полезную нагрузку и сравниваем её,
        // а не ищем подстроку в сыром файле.
        let content = std::fs::read_to_string(&path).expect("read");
        let mut lines = content.lines();
        lines.next().expect("заголовок");
        let joined: String = lines
            .map(|l| {
                let event: (f64, String, String) =
                    serde_json::from_str(l).expect("событие — валидный JSON");
                event.2
            })
            .collect();

        assert_eq!(
            joined, "привет",
            "символ на границе чанков должен склеиться без потерь"
        );
        assert!(
            !joined.contains('\u{FFFD}'),
            "в записи не должно быть замен на U+FFFD: {joined}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn writes_valid_asciinema_header() {
        let dir = std::env::temp_dir().join(format!("darwaza-rec-hdr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("header.cast");

        let rec = Recorder::start(path.clone(), 120, 40, Some("web-1".into()))
            .await
            .expect("start");
        rec.finish().await.expect("finish");

        let content = std::fs::read_to_string(&path).expect("read");
        let first = content.lines().next().expect("есть заголовок");
        let parsed: serde_json::Value =
            serde_json::from_str(first).expect("заголовок — валидный JSON");
        assert_eq!(parsed["version"], 2);
        assert_eq!(parsed["width"], 120);
        assert_eq!(parsed["height"], 40);
        assert_eq!(parsed["title"], "web-1");

        std::fs::remove_dir_all(&dir).ok();
    }

    fn scratch_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("darwaza-rec-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("каталог");
        dir
    }

    #[tokio::test]
    async fn the_list_knows_title_duration_and_what_is_still_running() {
        let dir = scratch_dir();

        let done = dir.join("done.cast");
        let mut rec = Recorder::start(done.clone(), 80, 24, Some("web-1.prod (10.0.0.5)".into()))
            .await
            .expect("старт");
        rec.write_output(b"hello").await.expect("вывод");
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        rec.write_output(b" world").await.expect("вывод");
        rec.finish().await.expect("конец");

        let running = dir.join("running.cast");
        let live = Recorder::start(running.clone(), 80, 24, Some("db (10.0.0.6)".into()))
            .await
            .expect("старт");

        let list = list_in(&dir).await.expect("список");
        let done_item = list.iter().find(|f| f.name == "done.cast").expect("done");
        assert_eq!(done_item.title.as_deref(), Some("web-1.prod (10.0.0.5)"));
        assert!(!done_item.active);
        assert!(done_item.started_at.is_some());
        assert!(done_item.duration_seconds.expect("длительность") > 0.0);

        let live_item = list
            .iter()
            .find(|f| f.name == "running.cast")
            .expect("running");
        assert!(live_item.active, "идущая запись помечена");
        assert_eq!(live_item.title.as_deref(), Some("db (10.0.0.6)"));

        // Идущую запись удалять нельзя: файл открыт, и остановка потом упрётся
        // в пропавший файл.
        assert!(remove_in(&dir, &running.display().to_string())
            .await
            .is_err());
        assert!(running.exists());

        // Оборванная без `finish` запись не числится идущей вечно.
        drop(live);
        let list = list_in(&dir).await.expect("список");
        assert!(
            !list
                .iter()
                .find(|f| f.name == "running.cast")
                .expect("running")
                .active
        );

        remove_in(&dir, &running.display().to_string())
            .await
            .expect("удалена");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Строка длиннее куска приходит целой, а не обрезанной посередине.
    #[tokio::test]
    async fn a_line_longer_than_the_chunk_is_returned_whole() {
        let dir = scratch_dir();
        let path = dir.join("long.cast");
        let mut rec = Recorder::start(path.clone(), 80, 24, None).await.unwrap();
        let big = "я".repeat(20_000); // 40 000 байт, больше CHUNK_MIN
        rec.write_output(big.as_bytes()).await.unwrap();
        rec.write_output(b"tail\r\n").await.unwrap();
        rec.finish().await.unwrap();

        let shown = path.display().to_string();
        let mut offset = 0;
        let mut lines = Vec::new();
        loop {
            let chunk = read_chunk_in(&dir, &shown, offset, CHUNK_MIN)
                .await
                .unwrap();
            lines.extend(chunk.lines);
            offset = chunk.next_offset;
            if chunk.eof {
                break;
            }
        }
        for line in &lines {
            serde_json::from_str::<serde_json::Value>(line)
                .unwrap_or_else(|e| panic!("строка обрезана: {} байт: {e}", line.len()));
        }
        assert!(lines.iter().any(|l| l.contains(&big)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn playback_reads_whole_lines_in_chunks_and_stays_inside_the_directory() {
        let dir = scratch_dir();
        let path = dir.join("play.cast");
        let mut rec = Recorder::start(path.clone(), 80, 24, None)
            .await
            .expect("старт");
        for i in 0..300 {
            rec.write_output(format!("строка {i}\r\n").as_bytes())
                .await
                .expect("вывод");
        }
        rec.finish().await.expect("конец");

        let shown = path.display().to_string();
        let mut offset = 0;
        let mut lines = Vec::new();
        loop {
            // Малый предел, чтобы кусков вышло несколько и границы строк проверились.
            let chunk = read_chunk_in(&dir, &shown, offset, CHUNK_MIN)
                .await
                .expect("кусок");
            lines.extend(chunk.lines);
            offset = chunk.next_offset;
            if chunk.eof {
                break;
            }
        }
        assert_eq!(
            lines.len(),
            301,
            "заголовок и триста событий, ничего не потеряно"
        );
        for line in &lines {
            serde_json::from_str::<serde_json::Value>(line)
                .unwrap_or_else(|e| panic!("строка разорвана на границе куска: {line}: {e}"));
        }

        // За пределы каталога записей читать нельзя.
        let outside = std::env::temp_dir().join(format!("outside-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&outside, "секрет").expect("файл");
        assert!(read_chunk_in(&dir, &outside.display().to_string(), 0, 100)
            .await
            .is_err());
        assert!(remove_in(&dir, &outside.display().to_string())
            .await
            .is_err());
        assert!(outside.exists());

        std::fs::remove_file(&outside).ok();
        std::fs::remove_dir_all(&dir).ok();
    }
}
