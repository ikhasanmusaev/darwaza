//! HTTP-клиент к API Darwaza.
//!
//! Формы запросов взяты из `docs/06-web/03-api-contract.md` и
//! `04-sync-protocol.md` и **сверены с работающим сервером** сессией
//! `darwaza-web` перед тем, как это было написано. Расхождения документа с
//! кодом, найденные при сверке, отмечены ниже прямо в местах, где они важны:
//! иначе следующий читатель поверит документу и напишет то же самое заново.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Сколько ждать ответа. Без предела зависший сервер держал бы приложение.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Предел на размер блоба. Сервер отвергает больший с `VALIDATION_FAILED`.
///
/// Проверяем и у себя: узнать о переполнении из отказа сервера можно, но
/// тогда впустую уедет весь пакет, а виноватую запись придётся вылавливать
/// из `details`.
pub const MAX_BLOB_BYTES: usize = 64 * 1024;

/// Как клиент называет себя серверу.
///
/// Список **закрытый и в нижнем регистре**: `"Linux"` с большой буквы даёт
/// 400. Поэтому значение не собирается из `std::env::consts::OS` вслепую.
pub fn platform() -> &'static str {
    match std::env::consts::OS {
        "linux" => "linux",
        "windows" => "windows",
        "macos" => "macos",
        // Ничего лучше нет: соврать знакомым значением честнее, чем получить
        // отказ входа на системе, которую мы не предусмотрели.
        _ => "linux",
    }
}

/// Перевод между именем локальной таблицы и видом записи на проводе.
///
/// Локально запись зовётся по таблице (`hosts`), на сервере — в единственном
/// числе (`host`). Расхождение не выдумано: сервер хранит вид как метку и не
/// знает про наши таблицы, а мы не хотим завязывать схему базы на чужой
/// словарь.
///
/// Перевод обязан быть **полным и обратимым**: пропущенный вид означает, что
/// записи этого рода перестанут уезжать, и заметит это только пользователь со
/// вторым устройством. На это есть тест.
pub fn wire_kind(table: &str) -> Option<&'static str> {
    Some(match table {
        "hosts" => "host",
        "groups" => "group",
        "identities" => "identity",
        "snippets" => "snippet",
        "port_forwards" => "port_forward",
        "known_hosts" => "known_host",
        _ => return None,
    })
}

/// Обратный перевод: вид с сервера в имя локальной таблицы.
pub fn table_kind(wire: &str) -> Option<&'static str> {
    Some(match wire {
        "host" => "hosts",
        "group" => "groups",
        "identity" => "identities",
        "snippet" => "snippets",
        "port_forward" => "port_forwards",
        "known_host" => "known_hosts",
        // `setting` сервер принимает, но настройки мы не синхронизируем:
        // они про устройство, а не про аккаунт.
        _ => return None,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub name: String,
    pub platform: String,
}

impl DeviceInfo {
    /// Имя машины — то, что человек увидит в списке устройств кабинета.
    pub fn current() -> Self {
        let name = hostname().unwrap_or_else(|| "Устройство".to_string());
        Self {
            name,
            platform: platform().to_string(),
        }
    }
}

fn hostname() -> Option<String> {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Запись в том виде, в каком она ходит по проводу.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireRecord {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    /// У удалённой записи **null**: блоб очищается при мягком удалении.
    ///
    /// `skip_serializing_if` здесь стоять не должно, и это оплачено. Контракт
    /// объявляет поле как «строка или null» — то есть присылать его
    /// обязательно. Пропущенное поле сервер отвергает целиком
    /// (`missingProperty: "blob"`), и вместе с надгробием падает весь проход,
    /// включая ни в чём не виноватые записи того же запроса. Любое удаление
    /// ломало синхронизацию.
    pub blob: Option<String>,
    /// Всегда `null`: nonce едет первыми байтами внутри блоба, отдельным полем
    /// он не используется.
    ///
    /// Раньше сюда уезжала пустая строка, чтобы «не путать с удалённой». Толку
    /// не было: ajv на стороне сервера приводит `""` к `null`, и в базе всё
    /// равно оказывался `null` — только беззвучно. Удалённую запись отличает
    /// поле `deleted`, а не пустой nonce.
    pub nonce: Option<String>,
    /// Минимум **1**. Ноль сервер отвергает, а для новой записи присылается
    /// именно `1` — и сервер присвоит ей тоже `1`, а не `2`. Правило
    /// «прислал N — стало N+1» верно только для уже существующих.
    pub revision: i64,
    #[serde(default)]
    pub deleted: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Accepted {
    pub id: String,
    pub revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub id: String,
    pub server_revision: i64,
    pub server: WireRecord,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushResult {
    pub cursor: i64,
    #[serde(default)]
    pub accepted: Vec<Accepted>,
    /// **Конфликты приходят внутри 200, а не кодом 409.**
    ///
    /// Отдельного кода нет намеренно: часть записей в одном запросе может
    /// быть принята, а часть отвергнута, и один статус на весь ответ этого
    /// не выразил бы. Ветвиться надо по непустому списку.
    ///
    /// Пока это не было записано в контракте, там стояла строка про
    /// несуществующий `SYNC_CONFLICT` — и она заставляла писать обработчик
    /// 409, который не сработал бы ни разу.
    #[serde(default)]
    pub conflicts: Vec<Conflict>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Changes {
    #[serde(default)]
    pub changes: Vec<WireRecord>,
    /// Позиция **последней отданной записи**, а не счётчик пользователя.
    /// Сохранять надо именно её, иначе при постраничной выдаче клиент
    /// перескочит через неполученное.
    pub cursor: i64,
    #[serde(default)]
    pub has_more: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub cursor: i64,
    pub records: i64,
    pub bytes: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
    #[serde(default)]
    pub protected_symmetric_key: Option<String>,
}

/// Ответ на начало восстановления (#180c).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recovery {
    /// Обёртка `symmetricKey`, снимаемая ключом с бумаги.
    pub protected_symmetric_key_rk: String,
    /// Одноразовый, живёт десять минут, годится только для завершения.
    /// Ни доступом, ни сессией не является.
    pub recovery_token: String,
}

/// Отказ сервера в разобранном виде.
#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: u16,
    /// Код из конверта ошибки. По нему и ветвимся.
    pub code: String,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

impl std::error::Error for ApiError {}

impl ApiError {
    /// Нужен ли повтор после обновления токена.
    ///
    /// Один код на все беды с токеном — отсутствующий, просроченный,
    /// подделанный, мусорный. Отдельного кода нет намеренно, поэтому
    /// различать их и не пытаемся.
    pub fn needs_refresh(&self) -> bool {
        self.code == "TOKEN_EXPIRED"
    }

    /// Требуется код второго фактора.
    pub fn needs_totp(&self) -> bool {
        self.code == "TOTP_REQUIRED"
    }

    /// Код для интерфейса (#178).
    ///
    /// Перевод кода сервера в наш словарь. Список короткий намеренно: сюда
    /// попадает лишь то, с чем человек может что-то сделать. Всё прочее —
    /// «синхронизация не удалась», а подробность уходит в журнал: показывать
    /// «Запрос не соответствует контракту (VALIDATION_FAILED)» человеку
    /// бессмысленно, чинить это ему нечем.
    pub fn ui_code(&self) -> &'static str {
        match self.code.as_str() {
            "EMAIL_NOT_VERIFIED" => "errors.syncEmailNotVerified",
            "INVALID_CREDENTIALS" => "errors.syncWrongCredentials",
            "TOTP_REQUIRED" => "errors.masterTotpRequired",
            "TOTP_INVALID" => "errors.masterTotpWrong",
            "TOKEN_REUSED" => "errors.syncSignedOut",
            "RATE_LIMITED" => "errors.syncRateLimited",
            _ => "errors.syncFailed",
        }
    }

    /// Цепочка refresh-токенов отозвана: повторный вход неизбежен.
    pub fn session_is_gone(&self) -> bool {
        self.code == "TOKEN_REUSED"
    }
}

#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    code: String,
    message: String,
    #[serde(default)]
    details: Option<serde_json::Value>,
}

pub struct ApiClient {
    base: String,
    http: reqwest::Client,
}

impl ApiClient {
    pub fn new(base_url: &str) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .context("создание HTTP-клиента")?;
        Ok(Self {
            base: base_url.trim_end_matches('/').to_string(),
            http,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    /// Разобрать ответ: успех в `T`, отказ — в `ApiError` с кодом.
    async fn parse<T: for<'de> Deserialize<'de>>(response: reqwest::Response) -> Result<T> {
        let status = response.status();
        let text = response.text().await.context("чтение ответа")?;

        if status.is_success() {
            // 204 без тела — законный ответ там, где `T` это `()`.
            if text.trim().is_empty() {
                return serde_json::from_str("null").context("пустой ответ не подходит");
            }
            return serde_json::from_str(&text)
                .with_context(|| format!("ответ не разбирается: {text}"));
        }

        // Конверт ошибки известен; если он не разобрался, отдаём хотя бы код
        // и тело — молчаливый отказ хуже некрасивого.
        let parsed: Result<ErrorEnvelope, _> = serde_json::from_str(&text);
        let err = match parsed {
            Ok(envelope) => ApiError {
                status: status.as_u16(),
                code: envelope.error.code,
                message: envelope.error.message,
                details: envelope.error.details,
            },
            Err(_) => ApiError {
                status: status.as_u16(),
                code: "UNKNOWN".to_string(),
                message: format!("сервер ответил {status}: {text}"),
                details: None,
            },
        };
        Err(err.into())
    }

    pub async fn register(
        &self,
        email: &str,
        auth_hash: &str,
        protected_symmetric_key: &str,
        recovery_auth_hash: &str,
        protected_symmetric_key_rk: &str,
    ) -> Result<()> {
        let body = serde_json::json!({
            "email": email,
            "authHash": auth_hash,
            "protectedSymmetricKey": protected_symmetric_key,
            // Запасной вход заводится вместе с аккаунтом (#180c): аккаунта
            // без ключа восстановления просто не существует.
            "recoveryAuthHash": recovery_auth_hash,
            "protectedSymmetricKeyRk": protected_symmetric_key_rk,
            // Параметры берутся из модуля ключей, а не пишутся здесь:
            // разойдись они с тем, чем выводился ключ, вход перестал бы
            // работать без единой подсказки.
            "kdf": {
                "memoryKib": super::keys::KDF_MEMORY_KIB,
                "iterations": super::keys::KDF_ITERATIONS,
                "parallelism": super::keys::KDF_PARALLELISM,
            },
        });
        let res = self
            .http
            .post(self.url("/auth/register"))
            .json(&body)
            .send()
            .await
            .context("запрос регистрации")?;
        Self::parse::<serde_json::Value>(res).await.map(|_| ())
    }

    /// Начало восстановления по ключу с бумаги (#180c).
    ///
    /// Сервер отвечает **неразличимо** для несуществующего адреса и неверного
    /// ключа — один и тот же отказ. Поэтому и здесь различать нечего: любая
    /// неудача означает «адрес или ключ не подошли».
    pub async fn recover_begin(&self, email: &str, recovery_auth_hash: &str) -> Result<Recovery> {
        let res = self
            .http
            .post(self.url("/auth/recover/begin"))
            .json(&serde_json::json!({
                "email": email,
                "recoveryAuthHash": recovery_auth_hash,
            }))
            .send()
            .await
            .context("запрос восстановления")?;
        Self::parse(res).await
    }

    /// Завершение восстановления: новый пароль и новый ключ с бумаги — **одним
    /// запросом**.
    ///
    /// Два запроса означали бы человека, у которого второй не дошёл: пароль
    /// уже сменился, а бумажка осталась прежней — та самая, что побывала в
    /// чужих руках. Сервер делает это одной транзакцией.
    #[allow(clippy::too_many_arguments)]
    pub async fn recover_complete(
        &self,
        recovery_token: &str,
        new_auth_hash: &str,
        new_protected_symmetric_key: &str,
        new_recovery_auth_hash: &str,
        new_protected_symmetric_key_rk: &str,
    ) -> Result<()> {
        let res = self
            .http
            .post(self.url("/auth/recover/complete"))
            .json(&serde_json::json!({
                "recoveryToken": recovery_token,
                "newAuthHash": new_auth_hash,
                "newProtectedSymmetricKey": new_protected_symmetric_key,
                "newRecoveryAuthHash": new_recovery_auth_hash,
                "newProtectedSymmetricKeyRk": new_protected_symmetric_key_rk,
            }))
            .send()
            .await
            .context("завершение восстановления")?;
        Self::parse::<serde_json::Value>(res).await.map(|_| ())
    }

    /// Перевыпуск ключа восстановления.
    ///
    /// Требует и токен доступа, и подтверждение текущего пароля: перевыпуск
    /// отзывает прежнюю бумажку, и делать это по одной лишь украденной сессии
    /// нельзя.
    pub async fn recovery_reissue(
        &self,
        access: &str,
        auth_hash: &str,
        new_recovery_auth_hash: &str,
        new_protected_symmetric_key_rk: &str,
    ) -> Result<()> {
        let res = self
            .http
            .post(self.url("/auth/recovery/reissue"))
            .bearer_auth(access)
            .json(&serde_json::json!({
                "authHash": auth_hash,
                "newRecoveryAuthHash": new_recovery_auth_hash,
                "newProtectedSymmetricKeyRk": new_protected_symmetric_key_rk,
            }))
            .send()
            .await
            .context("перевыпуск ключа восстановления")?;
        Self::parse::<serde_json::Value>(res).await.map(|_| ())
    }

    pub async fn verify_email(&self, token: &str) -> Result<()> {
        let res = self
            .http
            .post(self.url("/auth/verify-email"))
            .json(&serde_json::json!({ "token": token }))
            .send()
            .await
            .context("запрос подтверждения адреса")?;
        Self::parse::<serde_json::Value>(res).await.map(|_| ())
    }

    pub async fn login(
        &self,
        email: &str,
        auth_hash: &str,
        totp_code: Option<&str>,
    ) -> Result<Session> {
        let mut body = serde_json::json!({
            "email": email,
            "authHash": auth_hash,
            "device": DeviceInfo::current(),
        });
        // Поле добавляем только когда код есть: пустая строка — не «нет
        // кода», а неверный код.
        if let Some(code) = totp_code {
            body["totpCode"] = serde_json::Value::String(code.to_string());
        }
        let res = self
            .http
            .post(self.url("/auth/login"))
            .json(&body)
            .send()
            .await
            .context("запрос входа")?;
        Self::parse(res).await
    }

    pub async fn refresh(&self, refresh_token: &str) -> Result<Session> {
        let res = self
            .http
            .post(self.url("/auth/refresh"))
            .json(&serde_json::json!({ "refreshToken": refresh_token }))
            .send()
            .await
            .context("обновление токена")?;
        Self::parse(res).await
    }

    /// Выход. Тело **обязательно**, хотя в контракте показан только код 204.
    pub async fn logout(&self, refresh_token: &str) -> Result<()> {
        let res = self
            .http
            .post(self.url("/auth/logout"))
            .json(&serde_json::json!({ "refreshToken": refresh_token }))
            .send()
            .await
            .context("выход")?;
        Self::parse::<serde_json::Value>(res).await.map(|_| ())
    }

    pub async fn changes(&self, access: &str, since: i64, limit: i64) -> Result<Changes> {
        // `since=0` и отсутствие параметра — одно и то же: полная выгрузка.
        //
        // Строку запроса собираем руками: `reqwest` подключён без лишних
        // возможностей, а оба значения — числа, и экранировать в них нечего.
        let url = format!(
            "{}?since={since}&limit={}",
            self.url("/sync/changes"),
            limit.clamp(1, 500)
        );
        let res = self
            .http
            .get(url)
            .bearer_auth(access)
            .send()
            .await
            .context("запрос изменений")?;
        Self::parse(res).await
    }

    pub async fn push(
        &self,
        access: &str,
        base_cursor: i64,
        changes: &[WireRecord],
    ) -> Result<PushResult> {
        // `baseCursor` сервером сейчас не читается, но обязателен. Шлём
        // последний известный курсор, а не ноль: если поле однажды начнут
        // использовать, оно не соврёт.
        let body = serde_json::json!({ "baseCursor": base_cursor, "changes": changes });
        let res = self
            .http
            .post(self.url("/sync/push"))
            .bearer_auth(access)
            .json(&body)
            .send()
            .await
            .context("отправка изменений")?;
        Self::parse(res).await
    }

    pub async fn status(&self, access: &str) -> Result<SyncStatus> {
        let res = self
            .http
            .get(self.url("/sync/status"))
            .bearer_auth(access)
            .send()
            .await
            .context("состояние синхронизации")?;
        Self::parse(res).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_is_lowercase_and_from_the_closed_list() {
        // "Linux" с большой буквы сервер отвергает с 400.
        let p = platform();
        assert!(
            ["linux", "windows", "macos", "web"].contains(&p),
            "недопустимое значение: {p}"
        );
    }

    #[test]
    fn a_deleted_record_may_carry_no_blob() {
        // У мягко удалённой записи блоб очищается, и приходит null.
        let json = r#"{"id":"a","type":"host","blob":null,"nonce":null,
                       "revision":3,"deleted":true}"#;
        let record: WireRecord = serde_json::from_str(json).unwrap();
        assert!(record.blob.is_none());
        assert!(record.deleted);
    }

    #[test]
    fn a_conflict_arrives_inside_a_successful_answer() {
        // Главное расхождение контракта с сервером: SYNC_CONFLICT (409) не
        // приходит никогда, конфликт лежит в теле ответа 200.
        let json = r#"{"cursor":2,"accepted":[],
            "conflicts":[{"id":"x","serverRevision":2,
              "server":{"id":"x","type":"host","blob":"Yg==","nonce":"bg==",
                        "revision":2,"deleted":false}}]}"#;
        let result: PushResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(result.conflicts[0].server_revision, 2);
        assert!(result.accepted.is_empty());
    }

    #[test]
    fn an_empty_push_answer_parses_without_the_optional_lists() {
        let result: PushResult = serde_json::from_str(r#"{"cursor":7}"#).unwrap();
        assert_eq!(result.cursor, 7);
        assert!(result.accepted.is_empty() && result.conflicts.is_empty());
    }

    #[test]
    fn a_tombstone_sends_blob_and_nonce_as_null_not_missing() {
        // Ошибка, за которую заплачено: `skip_serializing_if` выбрасывал поля
        // из JSON, а контракт объявляет их как «строка или null» — то есть
        // присылать обязательно. Сервер отвечал `missingProperty: "blob"` и
        // отвергал весь запрос, поэтому одно удаление роняло весь проход.
        let tombstone = WireRecord {
            id: "d79cd9f1-dd8b-db50-79f9-a1d67d6e05ae".into(),
            kind: "host".into(),
            blob: None,
            nonce: None,
            revision: 3,
            deleted: true,
        };
        let text = serde_json::to_string(&tombstone).unwrap();
        assert!(
            text.contains("\"blob\":null"),
            "поле blob обязано быть: {text}"
        );
        assert!(
            text.contains("\"nonce\":null"),
            "поле nonce обязано быть: {text}"
        );
        assert!(text.contains("\"deleted\":true"));
    }

    #[test]
    fn a_new_record_keeps_every_field_the_contract_requires() {
        let record = WireRecord {
            id: "a".into(),
            kind: "host".into(),
            blob: Some("Yg==".into()),
            nonce: None,
            revision: 1,
            deleted: false,
        };
        let text = serde_json::to_string(&record).unwrap();
        assert!(text.contains("\"type\":\"host\""), "получено: {text}");
        assert!(text.contains("\"revision\":1"));
        // Даже у обычной записи поле присутствует — пустой строки здесь
        // больше нет: сервер всё равно приводил её к null, только молча.
        assert!(text.contains("\"nonce\":null"), "получено: {text}");
    }

    #[test]
    fn error_codes_are_recognised_by_meaning_not_by_status() {
        let expired = ApiError {
            status: 401,
            code: "TOKEN_EXPIRED".into(),
            message: "…".into(),
            details: None,
        };
        assert!(expired.needs_refresh());
        assert!(!expired.session_is_gone());

        let reused = ApiError {
            status: 401,
            code: "TOKEN_REUSED".into(),
            message: "…".into(),
            details: None,
        };
        // Обновлять нечего: цепочка отозвана, нужен полный вход.
        assert!(!reused.needs_refresh());
        assert!(reused.session_is_gone());
    }

    #[test]
    fn the_blob_limit_matches_what_the_server_enforces() {
        assert_eq!(MAX_BLOB_BYTES, 65536);
    }

    #[test]
    fn every_syncable_table_has_a_wire_name_and_back() {
        // Пропущенный вид означает, что записи этого рода перестанут уезжать,
        // и заметит это только пользователь со вторым устройством.
        for table in super::super::SYNCABLE.iter() {
            let wire = wire_kind(table).unwrap_or_else(|| panic!("нет вида для '{table}'"));
            assert_eq!(
                table_kind(wire),
                Some(table as &str),
                "перевод '{table}' → '{wire}' необратим"
            );
        }
    }

    #[test]
    fn the_table_name_itself_is_not_a_valid_wire_kind() {
        // Ровно эта ошибка и была: отправляли 'hosts', сервер ждал 'host'.
        assert_eq!(table_kind("hosts"), None);
        assert_eq!(wire_kind("host"), None);
    }

    #[test]
    fn settings_are_not_synchronised() {
        // Сервер такой вид принимает, но настройки про устройство, а не про
        // аккаунт: шрифт терминала на ноутбуке не должен ехать на рабочую
        // машину.
        assert_eq!(table_kind("setting"), None);
    }
}
