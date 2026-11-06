use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: i64,
    pub name: String,
    pub parent_id: Option<i64>,
    pub sort_order: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupInput {
    pub name: String,
    #[serde(default)]
    pub parent_id: Option<i64>,
    #[serde(default)]
    pub sort_order: Option<i64>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Host {
    pub id: i64,
    pub group_id: Option<i64>,
    pub name: String,
    pub hostname: String,
    pub port: i64,
    pub username: String,
    pub identity_id: Option<i64>,
    pub auth_method: String,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub jump_host_id: Option<i64>,
    /// Пробрасывать ли ssh-agent на удалённый хост (#150a).
    pub forward_agent: bool,
    /// Разрешён ли доступ агента к этому хосту (#201). Выключено по умолчанию.
    pub agent_access: bool,
    /// Читающие команды с `sudo`, разрешённые агенту на этом хосте (#214).
    ///
    /// По одной в строке. Пусто — ничего не разрешено, и любая команда с
    /// `sudo` идёт планом с подтверждением.
    pub agent_sudo_read: Option<String>,
    /// Заметки помощника о хосте (#217). Пишет агент, читает человек.
    pub agent_notes: Option<String>,
    pub keep_alive_interval: i64,
    pub description: Option<String>,
    pub last_connected_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub id: i64,
    pub label: String,
    pub kind: String,
    pub key_path: Option<String>,
    pub keyring_ref: Option<String>,
    pub has_passphrase: bool,
    /// Заведён ли TOTP-секрет (#153a). Сам секрет лежит в keyring/vault.
    pub has_totp: bool,
    pub public_key: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityInput {
    pub label: String,
    pub kind: String,
    #[serde(default)]
    pub key_path: Option<String>,
    #[serde(default)]
    pub keyring_ref: Option<String>,
    #[serde(default)]
    pub has_passphrase: Option<bool>,
    #[serde(default)]
    pub public_key: Option<String>,
}

/// Сохранённая команда, отправляемая в активный терминал.
/// В `command` допустимы плейсхолдеры `{{host}}`, `{{user}}`, `{{port}}`, `{{name}}` —
/// подстановка выполняется на стороне UI по данным активной сессии (см. #105).
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snippet {
    pub id: i64,
    pub name: String,
    pub command: String,
    pub description: Option<String>,
    pub sort_order: i64,
    /// Виден ли сниппет агенту (#220). По умолчанию нет.
    pub agent_access: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnippetInput {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub sort_order: Option<i64>,
    /// Открыть ли сниппет агенту (#220). Отсутствие поля — «нет».
    #[serde(default)]
    pub agent_access: bool,
}

/// Правило проброса портов (#130).
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortForward {
    pub id: i64,
    pub host_id: i64,
    /// `local` | `remote` | `dynamic`
    pub kind: String,
    pub label: Option<String>,
    pub local_host: String,
    pub local_port: i64,
    pub remote_host: Option<String>,
    pub remote_port: Option<i64>,
    pub auto_start: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortForwardInput {
    pub host_id: i64,
    pub kind: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub local_host: Option<String>,
    pub local_port: i64,
    #[serde(default)]
    pub remote_host: Option<String>,
    #[serde(default)]
    pub remote_port: Option<i64>,
    #[serde(default)]
    pub auto_start: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostInput {
    #[serde(default)]
    pub group_id: Option<i64>,
    pub name: String,
    pub hostname: String,
    pub port: i64,
    pub username: String,
    #[serde(default)]
    pub identity_id: Option<i64>,
    pub auth_method: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub jump_host_id: Option<i64>,
    #[serde(default)]
    pub forward_agent: Option<bool>,
    #[serde(default)]
    pub agent_access: Option<bool>,
    #[serde(default)]
    pub agent_sudo_read: Option<String>,
    #[serde(default)]
    pub agent_notes: Option<String>,
    #[serde(default)]
    pub keep_alive_interval: Option<i64>,
    #[serde(default)]
    pub description: Option<String>,
    /// Inline password — если задан и identity_id пустой,
    /// backend авто-создаст identity + сохранит пароль в keyring.
    #[serde(default)]
    pub password: Option<String>,
}
