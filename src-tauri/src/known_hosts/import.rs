//! Импорт `~/.ssh/known_hosts` (#175).
//!
//! Формат файла описан в `sshd(8)`. Разбираем не всё: часть записей
//! принципиально несовместима с тем, как устроен наш TOFU.
//!
//! - **Хешированные записи** (`|1|соль|хеш`) пропускаем: имя хоста из них не
//!   восстанавливается — в том и смысл хеширования. Наш TOFU ищет по точному
//!   `host:port`, сопоставить такую запись не с чем.
//! - **Шаблоны** (`*.example.com`) пропускаем по той же причине: сравнение у нас
//!   точное, а не по маске.
//! - **`@revoked`** пропускаем — это отозванные ключи, доверять им нельзя.
//! - **`@cert-authority`** пропускаем: это ключ центра сертификации, а не ключ
//!   конкретного хоста.
//!
//! Всё пропущенное считается и показывается пользователю, чтобы «импортировалось
//! меньше, чем в файле» не выглядело потерей данных.

use russh::keys::ssh_key::{HashAlg, PublicKey};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Одна запись, готовая к импорту.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportEntry {
    /// Нормализованный `host:port` — в том же виде, в каком хранит наша таблица.
    pub hostname: String,
    pub key_type: String,
    /// `SHA256:...` — тот же формат, что отдаёт russh при подключении.
    pub fingerprint: String,
}

/// Что нашлось в файле.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub path: String,
    pub entries: Vec<ImportEntry>,
    /// Сколько записей пропущено и почему.
    pub skipped_hashed: usize,
    pub skipped_wildcard: usize,
    pub skipped_marker: usize,
    pub skipped_unparsable: usize,
}

pub fn default_path() -> Option<PathBuf> {
    directories::UserDirs::new().map(|d| d.home_dir().join(".ssh").join("known_hosts"))
}

/// Привести имя хоста к виду `host:port`, как хранит наша таблица.
///
/// В файле порт по умолчанию не пишется, а нестандартный оформляется как
/// `[host]:port`.
fn normalize_host(raw: &str) -> String {
    if let Some(rest) = raw.strip_prefix('[') {
        if let Some((host, port)) = rest.split_once("]:") {
            return format!("{host}:{port}");
        }
        // `[host]` без порта — редкость, но встречается.
        if let Some(host) = rest.strip_suffix(']') {
            return format!("{host}:22");
        }
    }
    format!("{raw}:22")
}

fn is_wildcard(host: &str) -> bool {
    host.contains('*') || host.contains('?') || host.starts_with('!')
}

/// Разобрать содержимое файла.
pub fn parse(content: &str, path: String) -> ImportPreview {
    let mut entries: Vec<ImportEntry> = Vec::new();
    let mut skipped_hashed = 0;
    let mut skipped_wildcard = 0;
    let mut skipped_marker = 0;
    let mut skipped_unparsable = 0;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Маркеры @revoked / @cert-authority идут первым полем.
        if line.starts_with('@') {
            skipped_marker += 1;
            continue;
        }

        let mut parts = line.split_whitespace();
        let (hosts, key_type, key_data) = match (parts.next(), parts.next(), parts.next()) {
            (Some(h), Some(t), Some(k)) => (h, t, k),
            _ => {
                skipped_unparsable += 1;
                continue;
            }
        };

        if hosts.starts_with("|1|") {
            skipped_hashed += 1;
            continue;
        }

        // Ключ разбираем один раз на строку: у записи может быть несколько имён.
        let openssh = format!("{key_type} {key_data}");
        let fingerprint = match PublicKey::from_openssh(&openssh) {
            Ok(key) => key.fingerprint(HashAlg::Sha256).to_string(),
            Err(_) => {
                skipped_unparsable += 1;
                continue;
            }
        };

        for host in hosts.split(',') {
            let host = host.trim();
            if host.is_empty() {
                continue;
            }
            if is_wildcard(host) {
                skipped_wildcard += 1;
                continue;
            }
            entries.push(ImportEntry {
                hostname: normalize_host(host),
                key_type: key_type.to_string(),
                fingerprint: fingerprint.clone(),
            });
        }
    }

    // Одинаковые (host, тип) могут встретиться дважды — оставляем первое.
    entries.dedup_by(|a, b| a.hostname == b.hostname && a.key_type == b.key_type);

    ImportPreview {
        path,
        entries,
        skipped_hashed,
        skipped_wildcard,
        skipped_marker,
        skipped_unparsable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Настоящий ed25519-ключ: парсер обязан принять именно валидный.
    const ED25519: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJ9k6Zt2VBaBUJ1kZ5B2xM3rMbHVsGVXhGBOXlwl0aVn";

    #[test]
    fn parses_plain_host_with_default_port() {
        let preview = parse(&format!("example.com {ED25519}"), "test".into());
        assert_eq!(preview.entries.len(), 1);
        assert_eq!(preview.entries[0].hostname, "example.com:22");
        assert_eq!(preview.entries[0].key_type, "ssh-ed25519");
        assert!(preview.entries[0].fingerprint.starts_with("SHA256:"));
    }

    #[test]
    fn parses_bracketed_non_standard_port() {
        let preview = parse(&format!("[example.com]:2222 {ED25519}"), "test".into());
        assert_eq!(preview.entries[0].hostname, "example.com:2222");
    }

    #[test]
    fn expands_comma_separated_hosts() {
        let preview = parse(&format!("a.example.com,10.0.0.1 {ED25519}"), "test".into());
        let hosts: Vec<_> = preview
            .entries
            .iter()
            .map(|e| e.hostname.as_str())
            .collect();
        assert_eq!(hosts, ["a.example.com:22", "10.0.0.1:22"]);
    }

    #[test]
    fn skips_hashed_entries() {
        // Имя хоста из хеша не восстановить — сопоставлять не с чем.
        let line = format!("|1|F1E1+abc=|9kZ2xM3rMbHVsGVXhGBOXlwl0aVn= {ED25519}");
        let preview = parse(&line, "test".into());
        assert!(preview.entries.is_empty());
        assert_eq!(preview.skipped_hashed, 1);
    }

    #[test]
    fn skips_wildcards_and_markers() {
        let content = format!(
            "*.example.com {ED25519}\n\
             @revoked bad.example.com {ED25519}\n\
             @cert-authority *.ca.example.com {ED25519}\n"
        );
        let preview = parse(&content, "test".into());
        assert!(preview.entries.is_empty());
        assert_eq!(preview.skipped_wildcard, 1);
        assert_eq!(preview.skipped_marker, 2);
    }

    #[test]
    fn skips_comments_and_garbage() {
        let content = format!(
            "# комментарий\n\
             \n\
             сломанная строка\n\
             good.example.com {ED25519}\n"
        );
        let preview = parse(&content, "test".into());
        assert_eq!(preview.entries.len(), 1);
        assert_eq!(preview.entries[0].hostname, "good.example.com:22");
        assert_eq!(preview.skipped_unparsable, 1);
    }
}
