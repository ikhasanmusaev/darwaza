use directories::UserDirs;
use serde::Serialize;
use std::path::PathBuf;
use tokio::fs;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConfigEntry {
    /// Алиас из `Host` директивы (например "web1.prod").
    pub alias: String,
    /// HostName (DNS или IP). Может отсутствовать — тогда используем alias.
    pub hostname: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    /// Путь к приватному ключу (как написан в конфиге, с `~` если был).
    pub identity_file: Option<String>,
    pub proxy_jump: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConfigPreview {
    pub path: String,
    pub entries: Vec<SshConfigEntry>,
}

fn default_ssh_config_path() -> Option<PathBuf> {
    UserDirs::new().map(|d| d.home_dir().join(".ssh").join("config"))
}

/// Ошибка наружу уходит кодом, причина — в журнал (#177).
fn map_err<E: std::fmt::Display>(e: E) -> String {
    crate::errors::opaque("ssh_config", e)
}

/// Простой парсер ssh_config:
///   - читает блоки `Host <pattern>`
///   - пропускает блоки с wildcard (`*`, `?`) — это не конкретные хосты
///   - извлекает HostName, User, Port, IdentityFile, ProxyJump
///   - игнорирует Include (плоский парсинг для MVP)
fn parse_ssh_config(content: &str) -> Vec<SshConfigEntry> {
    let mut entries: Vec<SshConfigEntry> = Vec::new();
    let mut current: Option<SshConfigEntry> = None;

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let mut parts = line.splitn(2, |c: char| c.is_whitespace() || c == '=');
        let key = parts.next().unwrap_or("").trim();
        let value = parts.next().unwrap_or("").trim();
        if key.is_empty() {
            continue;
        }

        let key_lc = key.to_ascii_lowercase();

        if key_lc == "host" {
            if let Some(entry) = current.take() {
                entries.push(entry);
            }
            let alias = value
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            if alias.is_empty() || alias.contains('*') || alias.contains('?') {
                current = None;
            } else {
                current = Some(SshConfigEntry {
                    alias,
                    hostname: None,
                    user: None,
                    port: None,
                    identity_file: None,
                    proxy_jump: None,
                });
            }
            continue;
        }

        if let Some(entry) = current.as_mut() {
            match key_lc.as_str() {
                "hostname" => entry.hostname = Some(value.to_string()),
                "user" => entry.user = Some(value.to_string()),
                "port" => entry.port = value.parse().ok(),
                "identityfile" => entry.identity_file = Some(value.to_string()),
                "proxyjump" => entry.proxy_jump = Some(value.to_string()),
                _ => {} // другие директивы пока игнорим
            }
        }
    }

    if let Some(entry) = current.take() {
        entries.push(entry);
    }

    entries
}

#[tauri::command]
pub async fn ssh_config_preview() -> Result<SshConfigPreview, String> {
    let path =
        default_ssh_config_path().ok_or_else(|| "cannot resolve user home dir".to_string())?;

    if !path.exists() {
        return Ok(SshConfigPreview {
            path: path.display().to_string(),
            entries: Vec::new(),
        });
    }

    let content = fs::read_to_string(&path).await.map_err(map_err)?;
    let entries = parse_ssh_config(&content);

    Ok(SshConfigPreview {
        path: path.display().to_string(),
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_blocks() {
        let cfg = "\
# A comment
Host web1
    HostName 10.0.0.1
    User ubuntu
    Port 2222
    IdentityFile ~/.ssh/id_ed25519

Host db1
    HostName db.internal
    User root
";
        let entries = parse_ssh_config(cfg);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].alias, "web1");
        assert_eq!(entries[0].hostname.as_deref(), Some("10.0.0.1"));
        assert_eq!(entries[0].user.as_deref(), Some("ubuntu"));
        assert_eq!(entries[0].port, Some(2222));
        assert_eq!(
            entries[0].identity_file.as_deref(),
            Some("~/.ssh/id_ed25519")
        );

        assert_eq!(entries[1].alias, "db1");
        assert_eq!(entries[1].port, None);
        assert_eq!(entries[1].identity_file, None);
    }

    #[test]
    fn skips_wildcard_hosts() {
        let cfg = "\
Host *
    User defaultuser
    Port 22

Host bastion
    HostName bastion.example.com
    User admin
";
        let entries = parse_ssh_config(cfg);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].alias, "bastion");
    }

    #[test]
    fn supports_equals_separator() {
        let cfg = "Host alpha\n    HostName=1.1.1.1\n    Port=22\n";
        let entries = parse_ssh_config(cfg);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].hostname.as_deref(), Some("1.1.1.1"));
        assert_eq!(entries[0].port, Some(22));
    }

    #[test]
    fn proxy_jump_captured() {
        let cfg = "Host inner\n    HostName 10.1.0.1\n    ProxyJump bastion\n";
        let entries = parse_ssh_config(cfg);
        assert_eq!(entries[0].proxy_jump.as_deref(), Some("bastion"));
    }
}
