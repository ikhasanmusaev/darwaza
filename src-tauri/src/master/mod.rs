//! Мастер-пароль: Argon2id + AES-256-GCM (#110–#113), ключ восстановления (#180b).
//!
//! Зачем он нужен, когда есть keyring, — см. `docs/01-architecture/security.md`:
//! на headless-машине keyring может быть недоступен, а системный keyring
//! разблокируется автоматически при входе в систему, то есть доступен любому
//! процессу пользователя. Мастер-пароль даёт слой, который знает только человек.
//!
//! Что здесь важно понимать:
//!
//! - **Ключи нигде не хранятся открыто.** Ключ из пароля выводится при
//!   разблокировке и живёт только в памяти процесса; при блокировке
//!   затирается (`zeroize`).
//! - **Соль и параметры KDF лежат открыто** в таблице `settings` — это не
//!   секрет, их назначение в том, чтобы одинаковые пароли давали разные ключи.
//! - **Проверка пароля идёт через расшифровку verifier'а**, а не через сравнение
//!   хэшей: если AES-GCM не смог проверить тег аутентификации, пароль неверный.
//!   Так проверка не даёт отдельного оракула помимо самой расшифровки.
//!
//! ## Две версии устройства хранилища
//!
//! **Первая** (до 2026-09-17): секреты шифровались прямо ключом из пароля.
//! Смена пароля перешифровывала всё хранилище.
//!
//! **Вторая**: секреты шифруются случайным **ключом данных**, а он сам лежит в
//! двух обёртках — ключом из пароля и ключом восстановления. Без этого ключ
//! восстановления невозможен в принципе: положи под него копию ключа из
//! пароля — и после первой же смены пароля копия устареет, а бумажка молча
//! перестанет работать ровно тогда, когда понадобится.
//!
//! Смена пароля теперь меняет только одну обёртку, и ключ восстановления
//! остаётся действительным.
//!
//! Хранилище первой версии переводится во вторую **при разблокировке**, одной
//! транзакцией: сбой на середине оставляет его первой версии целиком, а не
//! наполовину.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::{anyhow, bail, Context, Result};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use sqlx::SqlitePool;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use zeroize::{Zeroize, Zeroizing};

use crate::errors::codes;

/// Ключи в таблице `settings`.
pub mod keys {
    pub const ENABLED: &str = "security.master_password_enabled";
    pub const SALT: &str = "security.master_salt";
    pub const VERIFIER_NONCE: &str = "security.master_verifier_nonce";
    pub const VERIFIER: &str = "security.master_verifier";
    pub const KDF_M_COST: &str = "security.master_kdf_m_cost";
    pub const KDF_T_COST: &str = "security.master_kdf_t_cost";
    pub const KDF_P_COST: &str = "security.master_kdf_p_cost";

    /// Версия устройства хранилища. Нет значения — первая.
    pub const VAULT_VERSION: &str = "security.master_vault_version";
    /// Ключ данных, обёрнутый ключом из пароля.
    pub const DATA_KEY_BY_PASSWORD_NONCE: &str = "security.master_data_key_pw_nonce";
    pub const DATA_KEY_BY_PASSWORD: &str = "security.master_data_key_pw";
    /// Ключ данных, обёрнутый ключом восстановления. Нет значения — ключ не
    /// выпущен.
    pub const DATA_KEY_BY_RECOVERY_NONCE: &str = "security.master_data_key_rk_nonce";
    pub const DATA_KEY_BY_RECOVERY: &str = "security.master_data_key_rk";

    /// Всё, что относится к мастер-паролю. Выключение убирает ровно этот
    /// список — забытый ключ здесь означал бы, что после выключения и
    /// повторного включения старая обёртка ключа восстановления лежит рядом с
    /// новой и ничего не открывает.
    pub const ALL: &[&str] = &[
        ENABLED,
        SALT,
        VERIFIER_NONCE,
        VERIFIER,
        KDF_M_COST,
        KDF_T_COST,
        KDF_P_COST,
        VAULT_VERSION,
        DATA_KEY_BY_PASSWORD_NONCE,
        DATA_KEY_BY_PASSWORD,
        DATA_KEY_BY_RECOVERY_NONCE,
        DATA_KEY_BY_RECOVERY,
    ];
}

/// Текущая версия устройства хранилища.
const VAULT_VERSION_CURRENT: &str = "2";

/// Открытый текст verifier'а. Значение неважно — важно, что оно известно
/// заранее и потому расшифровка либо сходится, либо нет.
const VERIFIER_PLAINTEXT: &[u8] = b"darwaza:master-verifier:v1";

/// Открытый текст verifier'а хранилищ, заведённых до 0.1.1 (прежнее имя
/// продукта). Принимается при проверке пароля и после успешного ввода
/// заменяется на `VERIFIER_PLAINTEXT`; хранилище при этом не меняется.
const LEGACY_VERIFIER_PLAINTEXT: &[u8] = b"termius-analog:master-verifier:v1";

/// Параметры Argon2id: 64 МиБ, 3 прохода, 1 поток.
///
/// Компромисс для десктопа: на типичной машине это доли секунды, но делает
/// перебор дорогим. Параметры сохраняются рядом с солью — если в будущем их
/// поднять, старые записи продолжат расшифровываться своими значениями.
///
/// В модульных тестах параметры намеренно слабые: тесты проверяют логику
/// хранилища, а не стойкость Argon2, и с настоящими они шли бы минутами.
/// Параметры пишутся в `settings` при включении, поэтому тесты проходят тем же
/// путём, что и настоящее приложение, — только с другими числами.
#[cfg(not(test))]
const DEFAULT_M_COST: u32 = 64 * 1024;
#[cfg(not(test))]
const DEFAULT_T_COST: u32 = 3;
#[cfg(test)]
const DEFAULT_M_COST: u32 = 8 * 1024;
#[cfg(test)]
const DEFAULT_T_COST: u32 = 1;
const DEFAULT_P_COST: u32 = 1;

/// После скольких неудач включается пауза и насколько (#113).
///
/// Счётчик общий для пароля и ключа восстановления: иначе перебирать можно
/// было бы попеременно, получая удвоенное число попыток до паузы.
const MAX_ATTEMPTS: u32 = 5;
const LOCKOUT: Duration = Duration::from_secs(30);

/// Контекст при выводе ключа из ключа восстановления.
///
/// Нужен, чтобы тот же набор байт, использованный где-то ещё, не давал того же
/// ключа. Версия в строке — на случай, если схему однажды придётся менять.
const RECOVERY_CONTEXT: &[u8] = b"darwaza:master-recovery:v1";

/// 32 байта ключа. Затирается при уничтожении.
///
/// Один тип и для ключа из пароля, и для ключа данных, и для ключа из ключа
/// восстановления: всё это 256 бит для AES-GCM, и различаются они только
/// ролью.
#[derive(Clone)]
struct DerivedKey([u8; 32]);

impl Drop for DerivedKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Default)]
struct Inner {
    /// Ключ данных. `Some` — разблокировано. Живёт только в памяти.
    key: Option<DerivedKey>,
    failed_attempts: u32,
    /// Момент, до которого попытки отклоняются.
    locked_until: Option<Instant>,
}

/// Состояние мастер-пароля. Хранится в `AppState`.
#[derive(Clone, Default)]
pub struct MasterKeyring {
    inner: Arc<RwLock<Inner>>,
}

/// Наименьшая длина нового мастер-пароля, в знаках (не в байтах). Единственное
/// место с числом: UI берёт его из `MasterStatus::min_length`.
pub const MIN_PASSWORD_CHARS: usize = 12;

/// Правило для **нового** пароля — при включении, смене и восстановлении.
/// Вход (`unlock`) его не применяет: пароли, заданные до правила, должны
/// продолжать открывать хранилище.
fn check_new_password(password: &str) -> Result<()> {
    if password.is_empty() {
        bail!(codes::MASTER_EMPTY_PASSWORD);
    }
    if password.chars().count() < MIN_PASSWORD_CHARS {
        bail!(codes::MASTER_PASSWORD_TOO_SHORT);
    }
    Ok(())
}

/// Что показать в UI.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MasterStatus {
    /// Включён ли мастер-пароль вообще.
    pub enabled: bool,
    /// Разблокирован ли он сейчас (ключ в памяти).
    pub unlocked: bool,
    /// Сколько секунд осталось до конца паузы после неудачных попыток.
    pub lockout_seconds_left: u64,
    pub failed_attempts: u32,
    /// Включён ли второй фактор на вход в аккаунт (#153b).
    pub totp_enabled: bool,
    /// Выпущен ли ключ восстановления (#180b).
    pub recovery_key_set: bool,
    /// Наименьшая длина нового пароля в знаках: подсказку в UI брать отсюда.
    pub min_length: usize,
    /// Параметры Argon2id: записанные, если мастер-пароль включён, иначе те,
    /// с которыми он будет включён. Подпись под полем пароля берётся отсюда:
    /// число, вписанное в разметку, однажды разойдётся с настоящим.
    pub kdf: KdfShown,
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KdfShown {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

/// Где лежит секрет второго фактора.
///
/// Он хранится **в самом хранилище**, зашифрованным мастер-ключом. Это не
/// случайно: положить его рядом в открытом виде значило бы отдать второй
/// фактор любому, кто получил файл базы.
pub const TOTP_KIND: &str = "master_totp";

pub const TOTP_ID: i64 = 0;

type Tx<'a> = sqlx::Transaction<'a, sqlx::Sqlite>;

fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).map_err(|e| anyhow!("источник случайности недоступен: {e}"))?;
    Ok(buf)
}

async fn setting_get(db: &SqlitePool, key: &str) -> Result<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = ?1")
        .bind(key)
        .fetch_optional(db)
        .await?;
    Ok(row.map(|r| r.0))
}

/// Значение, записанное в base64. `Ok(None)` — записи нет.
async fn setting_bytes(db: &SqlitePool, key: &str) -> Result<Option<Vec<u8>>> {
    match setting_get(db, key).await? {
        None => Ok(None),
        Some(value) => Ok(Some(
            B64.decode(value)
                .with_context(|| format!("настройка {key} повреждена"))?,
        )),
    }
}

async fn put(tx: &mut Tx<'_>, key: &str, value: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Вывести ключ из пароля. Сам по себе дорогой — вызывается только при
/// включении, разблокировке, смене пароля и восстановлении.
fn derive_key(
    password: &str,
    salt: &[u8],
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
) -> Result<DerivedKey> {
    let params = Params::new(m_cost, t_cost, p_cost, Some(32))
        .map_err(|e| anyhow!("некорректные параметры Argon2: {e}"))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

    let mut out = [0u8; 32];
    argon
        .hash_password_into(password.as_bytes(), salt, &mut out)
        .map_err(|e| anyhow!("не удалось вывести ключ: {e}"))?;
    Ok(DerivedKey(out))
}

/// Ключ, которым оборачивается ключ данных под ключ восстановления.
///
/// HMAC, а не Argon2: ключ восстановления — 160 случайных бит, перебирать его
/// бессмысленно, и медленная функция здесь ничего не добавляет, кроме
/// ожидания. Argon2 нужен там, где на входе пароль, придуманный человеком.
fn recovery_kek(recovery_key: &[u8; crate::recovery_key::KEY_BYTES]) -> Result<DerivedKey> {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(recovery_key)
        .map_err(|e| anyhow!("ключ HMAC: {e}"))?;
    mac.update(RECOVERY_CONTEXT);
    let out = mac.finalize().into_bytes();
    let mut key = [0u8; 32];
    key.copy_from_slice(&out);
    Ok(DerivedKey(key))
}

fn cipher(key: &DerivedKey) -> Aes256Gcm {
    // Длина ключа проверена типом: DerivedKey — ровно 32 байта.
    Aes256Gcm::new(&Key::<Aes256Gcm>::from(key.0))
}

/// Преобразовать срез в nonce нужной длины (96 бит для AES-GCM).
fn to_nonce(bytes: &[u8]) -> Result<Nonce<<Aes256Gcm as aes_gcm::AeadCore>::NonceSize>> {
    Nonce::try_from(bytes).map_err(|_| anyhow!("некорректная длина nonce: {}", bytes.len()))
}

/// Зашифровать. Возвращает (nonce, ciphertext).
fn encrypt(key: &DerivedKey, plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    // Nonce обязан быть уникальным на каждое шифрование этим ключом —
    // повтор в GCM ломает конфиденциальность, поэтому берём случайный.
    let nonce_bytes = random_bytes::<12>()?;
    let nonce = to_nonce(&nonce_bytes)?;
    let ct = cipher(key)
        .encrypt(&nonce, plaintext)
        .map_err(|_| anyhow!("не удалось зашифровать секрет"))?;
    Ok((nonce_bytes.to_vec(), ct))
}

fn decrypt(key: &DerivedKey, nonce: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>> {
    cipher(key)
        .decrypt(&to_nonce(nonce)?, ciphertext)
        // Единственная причина отказа, которую мы можем показать наружу —
        // «не сходится». Отличать «не тот ключ» от «данные испорчены» нельзя:
        // в GCM это одна и та же проверка тега.
        .map_err(|_| anyhow!("не удалось расшифровать: неверный пароль или повреждённые данные"))
}

/// Обернуть ключ данных ключом-обёрткой.
fn wrap(kek: &DerivedKey, data_key: &DerivedKey) -> Result<(Vec<u8>, Vec<u8>)> {
    encrypt(kek, &data_key.0)
}

/// Развернуть ключ данных. Ошибка — не тот ключ-обёртка.
fn unwrap(kek: &DerivedKey, nonce: &[u8], ciphertext: &[u8]) -> Result<DerivedKey> {
    let plain = Zeroizing::new(decrypt(kek, nonce, ciphertext)?);
    let bytes: [u8; 32] = plain
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("ключ данных повреждён: не 32 байта"))?;
    Ok(DerivedKey(bytes))
}

/// Всё, что делает пароль паролем: соль, параметры, verifier и обёртка ключа
/// данных. Собирается целиком заранее, чтобы запись была одной транзакцией.
struct PasswordRecord {
    salt: [u8; 16],
    verifier_nonce: Vec<u8>,
    verifier: Vec<u8>,
    data_key_nonce: Vec<u8>,
    data_key: Vec<u8>,
}

impl PasswordRecord {
    /// Завести пароль для данного ключа данных.
    fn new(password: &str, data_key: &DerivedKey) -> Result<Self> {
        let salt = random_bytes::<16>()?;
        let kek = derive_key(
            password,
            &salt,
            DEFAULT_M_COST,
            DEFAULT_T_COST,
            DEFAULT_P_COST,
        )?;
        let (verifier_nonce, verifier) = encrypt(&kek, VERIFIER_PLAINTEXT)?;
        let (data_key_nonce, wrapped) = wrap(&kek, data_key)?;
        Ok(Self {
            salt,
            verifier_nonce,
            verifier,
            data_key_nonce,
            data_key: wrapped,
        })
    }

    async fn write(&self, tx: &mut Tx<'_>) -> Result<()> {
        put(tx, keys::SALT, &B64.encode(self.salt)).await?;
        put(tx, keys::VERIFIER_NONCE, &B64.encode(&self.verifier_nonce)).await?;
        put(tx, keys::VERIFIER, &B64.encode(&self.verifier)).await?;
        // Параметры пишутся всегда, а не только при включении. Прежняя смена
        // пароля выводила ключ с параметрами по умолчанию, но не записывала их:
        // стоило им однажды измениться, и после смены пароля разблокировка
        // вывела бы ключ со старыми числами и новой солью — не тот.
        put(tx, keys::KDF_M_COST, &DEFAULT_M_COST.to_string()).await?;
        put(tx, keys::KDF_T_COST, &DEFAULT_T_COST.to_string()).await?;
        put(tx, keys::KDF_P_COST, &DEFAULT_P_COST.to_string()).await?;
        put(
            tx,
            keys::DATA_KEY_BY_PASSWORD_NONCE,
            &B64.encode(&self.data_key_nonce),
        )
        .await?;
        put(tx, keys::DATA_KEY_BY_PASSWORD, &B64.encode(&self.data_key)).await?;
        put(tx, keys::VAULT_VERSION, VAULT_VERSION_CURRENT).await?;
        Ok(())
    }
}

/// Новый ключ восстановления для данного ключа данных.
///
/// Возвращает запись для человека и обёртку для базы.
fn new_recovery_key(data_key: &DerivedKey) -> Result<(String, Vec<u8>, Vec<u8>)> {
    let raw = Zeroizing::new(random_bytes::<{ crate::recovery_key::KEY_BYTES }>()?);
    let kek = recovery_kek(&raw)?;
    let (nonce, wrapped) = wrap(&kek, data_key)?;
    Ok((crate::recovery_key::format(&raw), nonce, wrapped))
}

async fn write_recovery(tx: &mut Tx<'_>, nonce: &[u8], wrapped: &[u8]) -> Result<()> {
    put(tx, keys::DATA_KEY_BY_RECOVERY_NONCE, &B64.encode(nonce)).await?;
    put(tx, keys::DATA_KEY_BY_RECOVERY, &B64.encode(wrapped)).await?;
    Ok(())
}

impl MasterKeyring {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn is_enabled(db: &SqlitePool) -> Result<bool> {
        Ok(setting_get(db, keys::ENABLED).await?.as_deref() == Some("true"))
    }

    pub async fn is_unlocked(&self) -> bool {
        self.inner.read().await.key.is_some()
    }

    /// Выпущен ли ключ восстановления.
    pub async fn has_recovery_key(db: &SqlitePool) -> Result<bool> {
        Ok(setting_get(db, keys::DATA_KEY_BY_RECOVERY).await?.is_some())
    }

    pub async fn status(&self, db: &SqlitePool) -> Result<MasterStatus> {
        let inner = self.inner.read().await;
        let left = inner
            .locked_until
            .and_then(|t| t.checked_duration_since(Instant::now()))
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Ok(MasterStatus {
            enabled: Self::is_enabled(db).await?,
            unlocked: inner.key.is_some(),
            lockout_seconds_left: left,
            failed_attempts: inner.failed_attempts,
            totp_enabled: Self::is_totp_enabled(db).await?,
            recovery_key_set: Self::has_recovery_key(db).await?,
            min_length: MIN_PASSWORD_CHARS,
            kdf: Self::kdf_shown(db).await?,
        })
    }

    async fn kdf_shown(db: &SqlitePool) -> Result<KdfShown> {
        async fn stored(db: &SqlitePool, key: &str, default: u32) -> Result<u32> {
            Ok(setting_get(db, key)
                .await?
                .and_then(|v| v.parse().ok())
                .unwrap_or(default))
        }
        Ok(KdfShown {
            memory_kib: stored(db, keys::KDF_M_COST, DEFAULT_M_COST).await?,
            iterations: stored(db, keys::KDF_T_COST, DEFAULT_T_COST).await?,
            parallelism: stored(db, keys::KDF_P_COST, DEFAULT_P_COST).await?,
        })
    }

    /// Заведён ли второй фактор. Проверяется по наличию записи в хранилище,
    /// а не по отдельному флагу: так «включено» и «есть чем проверять» не
    /// могут разъехаться.
    pub async fn is_totp_enabled(db: &SqlitePool) -> Result<bool> {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT 1 FROM vault WHERE kind = ?1 AND ref_id = ?2")
                .bind(TOTP_KIND)
                .bind(TOTP_ID)
                .fetch_optional(db)
                .await?;
        Ok(row.is_some())
    }

    /// Прочитать секрет второго фактора заданным ключом.
    ///
    /// Отдельный метод нужен именно для разблокировки: там ключ уже получен,
    /// но в память ещё не положен — обычный `load_secret` его не увидит.
    async fn load_totp_with_key(
        &self,
        db: &SqlitePool,
        key: &DerivedKey,
    ) -> Result<crate::totp::TotpConfig> {
        // Байты, а не base64-строки: `store_secret` кладёт nonce и шифртекст
        // сырыми в колонки BLOB. Прежде здесь читались строки, и вход с
        // включённым вторым фактором падал на разборе колонки — то есть был
        // невозможен вовсе, с 20 августа. e2e этого не видели: backend там
        // подменён. Ловит `unlock_with_second_factor_works_against_a_real_database`.
        let row: Option<(Vec<u8>, Vec<u8>)> =
            sqlx::query_as("SELECT nonce, ciphertext FROM vault WHERE kind = ?1 AND ref_id = ?2")
                .bind(TOTP_KIND)
                .bind(TOTP_ID)
                .fetch_optional(db)
                .await?;
        let (nonce, cipher) = row.context("секрет второго фактора не найден")?;
        let plain = decrypt(key, &nonce, &cipher).context("не удалось расшифровать секрет")?;
        let text = String::from_utf8(plain).context("секрет не UTF-8")?;
        serde_json::from_str(&text).context("секрет второго фактора повреждён")
    }

    /// Сохранить секрет второго фактора. Требует разблокированного хранилища.
    pub async fn set_totp(&self, db: &SqlitePool, config: &crate::totp::TotpConfig) -> Result<()> {
        let text = serde_json::to_string(config)?;
        self.store_secret(db, TOTP_KIND, TOTP_ID, &text).await
    }

    pub async fn clear_totp(&self, db: &SqlitePool) -> Result<()> {
        self.delete_secret(db, TOTP_KIND, TOTP_ID).await
    }

    /// Включить мастер-пароль.
    ///
    /// Хранилище сразу заводится второй версии и остаётся разблокированным,
    /// чтобы человеку не пришлось вводить пароль дважды. Ключ восстановления
    /// здесь не выпускается: его показывают один раз, и делать это надо
    /// отдельным шагом, когда человек готов его записать.
    pub async fn enable(&self, db: &SqlitePool, password: &str) -> Result<()> {
        check_new_password(password)?;
        if Self::is_enabled(db).await? {
            bail!(codes::MASTER_ALREADY_ENABLED);
        }

        let data_key = DerivedKey(random_bytes::<32>()?);
        let record = PasswordRecord::new(password, &data_key)?;

        // Одной транзакцией: половина настроек без другой половины — это
        // включённый мастер-пароль, который нельзя разблокировать.
        let mut tx = db.begin().await?;
        record.write(&mut tx).await?;
        put(&mut tx, keys::ENABLED, "true").await?;
        tx.commit().await?;

        let mut inner = self.inner.write().await;
        inner.key = Some(data_key);
        inner.failed_attempts = 0;
        inner.locked_until = None;
        Ok(())
    }

    /// Отказать, если идёт пауза после неудачных попыток.
    async fn ensure_not_locked_out(&self) -> Result<()> {
        let inner = self.inner.read().await;
        if let Some(until) = inner.locked_until {
            // Сколько осталось ждать, интерфейс знает из
            // `MasterStatus.lockoutSecondsLeft` — дублировать число в
            // тексте значит завести второй источник правды.
            if until.checked_duration_since(Instant::now()).is_some() {
                bail!(codes::MASTER_LOCKED_OUT);
            }
        }
        Ok(())
    }

    /// Засчитать неудачную попытку и отказать нужным кодом.
    async fn fail_attempt(&self, code: &'static str) -> anyhow::Error {
        let mut inner = self.inner.write().await;
        inner.failed_attempts += 1;
        if inner.failed_attempts >= MAX_ATTEMPTS {
            inner.locked_until = Some(Instant::now() + LOCKOUT);
            inner.failed_attempts = 0;
            return anyhow!(codes::MASTER_LOCKED_OUT);
        }
        anyhow!(code)
    }

    /// Проверить пароль и вернуть ключ из него.
    async fn password_kek(&self, db: &SqlitePool, password: &str) -> Result<DerivedKey> {
        self.ensure_not_locked_out().await?;

        let kek = self.derive_from_settings(db, password).await?;
        let nonce = setting_bytes(db, keys::VERIFIER_NONCE)
            .await?
            .context("verifier nonce отсутствует")?;
        let verifier = setting_bytes(db, keys::VERIFIER)
            .await?
            .context("verifier отсутствует")?;

        match decrypt(&kek, &nonce, &verifier) {
            Ok(plain) if plain == VERIFIER_PLAINTEXT => Ok(kek),
            Ok(plain) if plain == LEGACY_VERIFIER_PLAINTEXT => {
                // Пароль верен, verifier прежнего образца: переписываем
                // новым. Не вышло записать — не страшно, повторим при
                // следующем вводе, а человека впускаем в любом случае.
                if let Err(e) = self.rewrite_verifier(db, &kek).await {
                    tracing::warn!(target: "master", "verifier не обновлён: {e}");
                }
                Ok(kek)
            }
            _ => Err(self.fail_attempt(codes::MASTER_WRONG_PASSWORD).await),
        }
    }

    async fn rewrite_verifier(&self, db: &SqlitePool, kek: &DerivedKey) -> Result<()> {
        let (nonce, verifier) = encrypt(kek, VERIFIER_PLAINTEXT)?;
        let mut tx = db.begin().await?;
        put(&mut tx, keys::VERIFIER_NONCE, &B64.encode(&nonce)).await?;
        put(&mut tx, keys::VERIFIER, &B64.encode(&verifier)).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Получить ключ данных по проверенному ключу из пароля.
    ///
    /// Хранилище первой версии здесь же переводится во вторую.
    async fn data_key_by_password(&self, db: &SqlitePool, kek: &DerivedKey) -> Result<DerivedKey> {
        if setting_get(db, keys::VAULT_VERSION).await?.as_deref() != Some(VAULT_VERSION_CURRENT) {
            return self.upgrade_vault(db, kek).await;
        }
        let nonce = setting_bytes(db, keys::DATA_KEY_BY_PASSWORD_NONCE)
            .await?
            .context("обёртка ключа данных отсутствует")?;
        let wrapped = setting_bytes(db, keys::DATA_KEY_BY_PASSWORD)
            .await?
            .context("обёртка ключа данных отсутствует")?;
        unwrap(kek, &nonce, &wrapped)
    }

    /// Перевести хранилище первой версии во вторую.
    ///
    /// Каждый секрет расшифровывается ключом из пароля и шифруется новым
    /// ключом данных — всё одной транзакцией. Не расшифровался хоть один —
    /// транзакция откатывается, и хранилище остаётся первой версии целиком:
    /// разблокировка сообщит об ошибке, но ничего не потеряно.
    async fn upgrade_vault(&self, db: &SqlitePool, kek: &DerivedKey) -> Result<DerivedKey> {
        let data_key = DerivedKey(random_bytes::<32>()?);

        let rows: Vec<(String, i64, Vec<u8>, Vec<u8>)> =
            sqlx::query_as("SELECT kind, ref_id, nonce, ciphertext FROM vault")
                .fetch_all(db)
                .await?;
        let (wrap_nonce, wrapped) = wrap(kek, &data_key)?;

        let mut tx = db.begin().await?;
        for (kind, ref_id, nonce, ciphertext) in rows {
            let plain = Zeroizing::new(decrypt(kek, &nonce, &ciphertext).with_context(|| {
                format!("секрет {kind}/{ref_id} не расшифровался при переводе хранилища")
            })?);
            let (new_nonce, new_ct) = encrypt(&data_key, &plain)?;
            sqlx::query(
                "UPDATE vault SET nonce = ?3, ciphertext = ?4, updated_at = CURRENT_TIMESTAMP \
                 WHERE kind = ?1 AND ref_id = ?2",
            )
            .bind(&kind)
            .bind(ref_id)
            .bind(new_nonce)
            .bind(new_ct)
            .execute(&mut *tx)
            .await?;
        }
        put(
            &mut tx,
            keys::DATA_KEY_BY_PASSWORD_NONCE,
            &B64.encode(&wrap_nonce),
        )
        .await?;
        put(&mut tx, keys::DATA_KEY_BY_PASSWORD, &B64.encode(&wrapped)).await?;
        put(&mut tx, keys::VAULT_VERSION, VAULT_VERSION_CURRENT).await?;
        tx.commit().await?;

        tracing::info!(target: "master", "хранилище переведено во вторую версию");
        Ok(data_key)
    }

    /// Разблокировать паролем и, если включён, вторым фактором.
    ///
    /// Порядок важен: сперва пароль, потом код. Иначе по реакции на код можно
    /// было бы перебирать его, не зная пароля.
    pub async fn unlock(&self, db: &SqlitePool, password: &str, code: Option<&str>) -> Result<()> {
        if !Self::is_enabled(db).await? {
            bail!(codes::MASTER_NOT_ENABLED);
        }

        let kek = self.password_kek(db, password).await?;
        let data_key = self.data_key_by_password(db, &kek).await?;

        // Пароль верен. Если заведён второй фактор — спрашиваем код,
        // и до его проверки ключ в память не кладём.
        //
        // Исключение — хранилище уже открыто: тогда пароль проверяют
        // повторно (смена пароля, выключение), а второй фактор охраняет
        // *получение* доступа, который уже есть. Требовать код снова
        // значило бы спрашивать его на каждое действие в настройках.
        let already_open = self.inner.read().await.key.is_some();
        if !already_open && Self::is_totp_enabled(db).await? {
            let secret = self.load_totp_with_key(db, &data_key).await?;
            let code = code.map(|c| c.trim().to_string()).unwrap_or_default();
            if code.is_empty() {
                bail!(codes::MASTER_TOTP_REQUIRED);
            }
            if !crate::totp::verify(&secret, &code)? {
                let mut inner = self.inner.write().await;
                inner.failed_attempts += 1;
                bail!(codes::MASTER_TOTP_WRONG);
            }
        }

        let mut inner = self.inner.write().await;
        inner.key = Some(data_key);
        inner.failed_attempts = 0;
        inner.locked_until = None;
        Ok(())
    }

    async fn derive_from_settings(&self, db: &SqlitePool, password: &str) -> Result<DerivedKey> {
        let salt = setting_bytes(db, keys::SALT)
            .await?
            .context("соль отсутствует — мастер-пароль настроен не полностью")?;
        let m = setting_get(db, keys::KDF_M_COST)
            .await?
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_M_COST);
        let t = setting_get(db, keys::KDF_T_COST)
            .await?
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_T_COST);
        let p = setting_get(db, keys::KDF_P_COST)
            .await?
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_P_COST);
        derive_key(password, &salt, m, t, p)
    }

    /// Заблокировать: затереть ключ из памяти. Секреты станут недоступны
    /// до следующего ввода пароля.
    pub async fn lock(&self) {
        let mut inner = self.inner.write().await;
        inner.key = None;
    }

    /// Полностью выключить мастер-пароль. Требует ввода текущего пароля:
    /// иначе выключить защиту смог бы кто угодно с доступом к открытому окну.
    pub async fn disable(&self, db: &SqlitePool, password: &str) -> Result<()> {
        // Код второго фактора здесь не спрашиваем: выключить можно только
        // из разблокированного состояния, а его уже открывали с кодом.
        self.unlock(db, password, None).await?;

        let mut tx = db.begin().await?;
        sqlx::query("DELETE FROM vault").execute(&mut *tx).await?;
        for key in keys::ALL {
            sqlx::query("DELETE FROM settings WHERE key = ?1")
                .bind(key)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;

        let mut inner = self.inner.write().await;
        inner.key = None;
        inner.failed_attempts = 0;
        inner.locked_until = None;
        Ok(())
    }

    /// Сменить пароль.
    ///
    /// Во второй версии хранилища меняется только обёртка ключа данных:
    /// секреты не перешифровываются, а ключ восстановления остаётся
    /// действительным. Именно ради этого и заведён отдельный ключ данных.
    pub async fn change_password(&self, db: &SqlitePool, old: &str, new: &str) -> Result<()> {
        check_new_password(new)?;
        self.unlock(db, old, None).await?;
        let data_key = self
            .inner
            .read()
            .await
            .key
            .clone()
            .context("хранилище неожиданно заблокировано")?;

        let record = PasswordRecord::new(new, &data_key)?;
        let mut tx = db.begin().await?;
        record.write(&mut tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Выпустить ключ восстановления (#180b).
    ///
    /// Требует разблокированного хранилища — а оно после разблокировки всегда
    /// второй версии. Прежний ключ, если был, **перестаёт работать**: его
    /// обёртка перезаписывается. Иначе потерянная бумажка оставалась бы
    /// действительной навсегда — ключ не истекает, и отозвать его можно только
    /// так.
    ///
    /// Возвращает ключ для человека. Больше его не узнать ниоткуда: в базе лежит
    /// только обёртка, а сам ключ не сохраняется нигде.
    pub async fn issue_recovery_key(&self, db: &SqlitePool) -> Result<String> {
        if !Self::is_enabled(db).await? {
            bail!(codes::MASTER_NOT_ENABLED);
        }
        let data_key = self
            .inner
            .read()
            .await
            .key
            .clone()
            .context("хранилище заблокировано — введите мастер-пароль")?;

        let (printed, nonce, wrapped) = new_recovery_key(&data_key)?;
        let mut tx = db.begin().await?;
        write_recovery(&mut tx, &nonce, &wrapped).await?;
        tx.commit().await?;
        Ok(printed)
    }

    /// Восстановить доступ ключом восстановления и задать новый пароль (#180b).
    ///
    /// **Второй фактор здесь не спрашивается** — сознательно. Ключ
    /// восстановления нужен ровно тогда, когда потеряно что-то из привычного, и
    /// часто это телефон вместе с памятью о пароле. Требовать код значило бы
    /// сделать бумажку бесполезной в самом частом случае. Цена названа прямо:
    /// ключ восстановления равносилен паролю и коду вместе, и хранить его надо
    /// соответственно.
    ///
    /// Использованный ключ **заменяется новым**, и новый возвращается: однажды
    /// набранный, он мог быть подсмотрен или остаться в буфере обмена.
    pub async fn recover(
        &self,
        db: &SqlitePool,
        recovery_key: &str,
        new_password: &str,
    ) -> Result<String> {
        if !Self::is_enabled(db).await? {
            bail!(codes::MASTER_NOT_ENABLED);
        }
        check_new_password(new_password)?;
        self.ensure_not_locked_out().await?;

        // Опечатка в длине — не попытка подбора, и засчитывать её незачем:
        // человек, пропустивший знак, не должен ждать паузу.
        let raw = Zeroizing::new(
            crate::recovery_key::parse(recovery_key)
                .ok_or_else(|| anyhow!(codes::MASTER_RECOVERY_MALFORMED))?,
        );

        let (Some(nonce), Some(wrapped)) = (
            setting_bytes(db, keys::DATA_KEY_BY_RECOVERY_NONCE).await?,
            setting_bytes(db, keys::DATA_KEY_BY_RECOVERY).await?,
        ) else {
            bail!(codes::MASTER_RECOVERY_NOT_SET);
        };

        let kek = recovery_kek(&raw)?;
        let data_key = match unwrap(&kek, &nonce, &wrapped) {
            Ok(key) => key,
            Err(_) => return Err(self.fail_attempt(codes::MASTER_RECOVERY_WRONG).await),
        };

        let record = PasswordRecord::new(new_password, &data_key)?;
        let (printed, rk_nonce, rk_wrapped) = new_recovery_key(&data_key)?;

        // Новый пароль и замена ключа — одной транзакцией. Порознь сбой между
        // ними оставил бы человека либо со старым паролем, который он забыл,
        // либо с использованным ключом, который уже засвечен.
        let mut tx = db.begin().await?;
        record.write(&mut tx).await?;
        write_recovery(&mut tx, &rk_nonce, &rk_wrapped).await?;
        tx.commit().await?;

        let mut inner = self.inner.write().await;
        inner.key = Some(data_key);
        inner.failed_attempts = 0;
        inner.locked_until = None;
        Ok(printed)
    }

    /// Положить секрет в зашифрованное хранилище.
    pub async fn store_secret(
        &self,
        db: &SqlitePool,
        kind: &str,
        ref_id: i64,
        secret: &str,
    ) -> Result<()> {
        let key = self
            .inner
            .read()
            .await
            .key
            .clone()
            .context("хранилище заблокировано — введите мастер-пароль")?;
        let (nonce, ciphertext) = encrypt(&key, secret.as_bytes())?;
        sqlx::query(
            "INSERT INTO vault (kind, ref_id, nonce, ciphertext) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(kind, ref_id) DO UPDATE SET \
                nonce = excluded.nonce, ciphertext = excluded.ciphertext, \
                updated_at = CURRENT_TIMESTAMP",
        )
        .bind(kind)
        .bind(ref_id)
        .bind(nonce)
        .bind(ciphertext)
        .execute(db)
        .await?;
        Ok(())
    }

    /// Достать секрет. `Ok(None)` — записи нет; ошибка — заблокировано или не расшифровалось.
    pub async fn load_secret(
        &self,
        db: &SqlitePool,
        kind: &str,
        ref_id: i64,
    ) -> Result<Option<String>> {
        let key = self
            .inner
            .read()
            .await
            .key
            .clone()
            .context("хранилище заблокировано — введите мастер-пароль")?;

        let row: Option<(Vec<u8>, Vec<u8>)> =
            sqlx::query_as("SELECT nonce, ciphertext FROM vault WHERE kind = ?1 AND ref_id = ?2")
                .bind(kind)
                .bind(ref_id)
                .fetch_optional(db)
                .await?;

        match row {
            None => Ok(None),
            Some((nonce, ciphertext)) => {
                let plain = Zeroizing::new(decrypt(&key, &nonce, &ciphertext)?);
                Ok(Some(
                    String::from_utf8(plain.to_vec())
                        .context("секрет в хранилище повреждён: не UTF-8")?,
                ))
            }
        }
    }

    pub async fn delete_secret(&self, db: &SqlitePool, kind: &str, ref_id: i64) -> Result<()> {
        sqlx::query("DELETE FROM vault WHERE kind = ?1 AND ref_id = ?2")
            .bind(kind)
            .bind(ref_id)
            .execute(db)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_from(password: &str, salt: &[u8]) -> DerivedKey {
        // В тестах берём минимальные параметры: проверяем логику, а не стойкость.
        derive_key(password, salt, 8 * 1024, 1, 1).expect("derive")
    }

    #[test]
    fn roundtrip_encrypt_decrypt() {
        let key = key_from("correct horse", b"0123456789abcdef");
        let (nonce, ct) = encrypt(&key, b"s3cret").expect("encrypt");
        let plain = decrypt(&key, &nonce, &ct).expect("decrypt");
        assert_eq!(plain, b"s3cret");
    }

    #[test]
    fn wrong_password_does_not_decrypt() {
        let salt = b"0123456789abcdef";
        let key = key_from("correct horse", salt);
        let (nonce, ct) = encrypt(&key, b"s3cret").expect("encrypt");

        let other = key_from("wrong horse", salt);
        assert!(
            decrypt(&other, &nonce, &ct).is_err(),
            "чужой ключ не должен расшифровывать"
        );
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        let key = key_from("pw", b"0123456789abcdef");
        let (nonce, mut ct) = encrypt(&key, b"s3cret").expect("encrypt");
        // Портим один байт — GCM обязан заметить по тегу аутентификации.
        ct[0] ^= 0xFF;
        assert!(decrypt(&key, &nonce, &ct).is_err());
    }

    #[test]
    fn nonce_differs_between_encryptions() {
        let key = key_from("pw", b"0123456789abcdef");
        let (n1, c1) = encrypt(&key, b"same").expect("encrypt");
        let (n2, c2) = encrypt(&key, b"same").expect("encrypt");
        // Повтор nonce в GCM ломает конфиденциальность — проверяем явно.
        assert_ne!(n1, n2, "nonce обязан быть уникальным");
        assert_ne!(c1, c2, "одинаковый шифртекст выдал бы одинаковый nonce");
    }

    #[test]
    fn different_salt_gives_different_key() {
        let a = key_from("same password", b"0123456789abcdef");
        let b = key_from("same password", b"fedcba9876543210");
        assert_ne!(a.0, b.0, "соль обязана разводить ключи");
    }

    // Запись ключа для человека и её разбор проверяются в
    // `crate::recovery_key`: там же, где живут, и одним набором на оба ключа
    // восстановления — от мастер-пароля и от пароля аккаунта.
}

/// Сквозные проверки хранилища на настоящей базе.
///
/// Отдельно от криптографических тестов выше: те проверяют шифрование в
/// памяти, а ошибки здесь живут на стыке с базой — в том, как байты кладутся
/// и как читаются обратно. Такое тест в памяти не видит в принципе.
#[cfg(test)]
mod vault_tests {
    use super::*;

    async fn fresh_db() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("подключение к памяти");
        crate::db::MIGRATOR.run(&pool).await.expect("миграции");
        pool
    }

    /// Код ошибки, которым закончилась операция.
    fn code<T: std::fmt::Debug>(result: Result<T>) -> String {
        result
            .expect_err("операция обязана была отказать")
            .to_string()
    }

    /// Включённый мастер-пароль с одним сохранённым секретом.
    async fn vault_with_secret(db: &SqlitePool) -> MasterKeyring {
        let master = MasterKeyring::new();
        master.enable(db, "пароль-мастер-12").await.expect("включение");
        master
            .store_secret(db, "password", 1, "пароль-от-сервера")
            .await
            .expect("секрет");
        master
    }

    async fn secret(master: &MasterKeyring, db: &SqlitePool) -> Option<String> {
        master
            .load_secret(db, "password", 1)
            .await
            .expect("чтение секрета")
    }

    fn any_recovery_key() -> String {
        crate::recovery_key::format(&crate::recovery_key::generate().expect("случайность"))
    }

    /// Вход со вторым фактором обязан работать на настоящей базе.
    ///
    /// Секрет второго фактора кладётся `store_secret` — сырыми байтами в
    /// колонки BLOB, — а при разблокировке читался как base64-строка. Если
    /// эти два пути разошлись, вход с включённым вторым фактором невозможен
    /// вовсе, а e2e этого не покажут: там backend подменён.
    #[tokio::test]
    async fn unlock_with_second_factor_works_against_a_real_database() {
        let db = fresh_db().await;
        let master = MasterKeyring::new();
        master.enable(&db, "пароль-мастер-12").await.expect("включение");

        let secret = crate::totp::generate_secret().expect("секрет");
        let config = crate::totp::TotpConfig {
            secret,
            ..Default::default()
        };
        master.set_totp(&db, &config).await.expect("второй фактор");
        master.lock().await;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("время")
            .as_secs();
        let code = crate::totp::code_at(&config, now).expect("код");

        master
            .unlock(&db, "пароль-мастер-12", Some(&code))
            .await
            .expect("вход с верным паролем и верным кодом обязан проходить");
        assert!(master.is_unlocked().await);
    }

    /// Хранилище, заведённое 0.1.0, открывается и после этого переходит на
    /// новый verifier. Неверный пароль по-прежнему не проходит.
    #[tokio::test]
    async fn a_0_1_0_vault_opens_and_moves_to_the_new_verifier() {
        let db = fresh_db().await;
        let master = vault_with_secret(&db).await;

        // Состояние 0.1.0: verifier зашифрован старым открытым текстом.
        let kek = master
            .derive_from_settings(&db, "пароль-мастер-12")
            .await
            .expect("ключ");
        let (nonce, legacy) = encrypt(&kek, LEGACY_VERIFIER_PLAINTEXT).expect("verifier");
        let mut tx = db.begin().await.expect("транзакция");
        put(&mut tx, keys::VERIFIER_NONCE, &B64.encode(&nonce)).await.expect("nonce");
        put(&mut tx, keys::VERIFIER, &B64.encode(&legacy)).await.expect("verifier");
        tx.commit().await.expect("запись");
        master.lock().await;

        assert!(master.unlock(&db, "чужой-пароль-12!", None).await.is_err());
        master
            .unlock(&db, "пароль-мастер-12", None)
            .await
            .expect("хранилище 0.1.0 обязано открываться");
        assert_eq!(secret(&master, &db).await.as_deref(), Some("пароль-от-сервера"));

        let nonce = setting_bytes(&db, keys::VERIFIER_NONCE).await.expect("чтение").expect("nonce");
        let stored = setting_bytes(&db, keys::VERIFIER).await.expect("чтение").expect("verifier");
        assert_eq!(
            decrypt(&kek, &nonce, &stored).expect("расшифровка"),
            VERIFIER_PLAINTEXT,
            "после успешного ввода verifier обязан стать новым"
        );

        master.lock().await;
        master.unlock(&db, "пароль-мастер-12", None).await.expect("повторный вход");
    }

    /// Главный путь: забыли пароль, ввели ключ, задали новый.
    #[tokio::test]
    async fn a_recovery_key_sets_a_new_password_and_keeps_the_secrets() {
        let db = fresh_db().await;
        let master = vault_with_secret(&db).await;
        let paper = master
            .issue_recovery_key(&db)
            .await
            .expect("ключ восстановления");
        master.lock().await;

        let replacement = master
            .recover(&db, &paper, "новый пароль-12!")
            .await
            .expect("восстановление");

        assert_eq!(
            secret(&master, &db).await.as_deref(),
            Some("пароль-от-сервера"),
            "секреты обязаны пережить восстановление"
        );

        master.lock().await;
        master
            .unlock(&db, "новый пароль-12!", None)
            .await
            .expect("новый пароль обязан открывать хранилище");

        master.lock().await;
        assert_eq!(
            code(master.unlock(&db, "пароль-мастер-12", None).await),
            codes::MASTER_WRONG_PASSWORD,
            "забытый пароль больше не открывает ничего"
        );

        assert_ne!(paper, replacement, "использованный ключ заменяется новым");
        assert_eq!(
            code(master.recover(&db, &paper, "ещё один пароль!").await),
            codes::MASTER_RECOVERY_WRONG,
            "использованный ключ однажды набирали — он обязан перестать работать"
        );
        master
            .recover(&db, &replacement, "ещё один пароль!")
            .await
            .expect("выданный взамен ключ обязан работать");
    }

    /// Ради этого и заведён отдельный ключ данных.
    ///
    /// Положи под бумажку копию ключа из пароля — и после первой же смены
    /// пароля копия устареет, а бумажка молча перестанет работать ровно тогда,
    /// когда понадобится.
    #[tokio::test]
    async fn changing_the_password_keeps_the_recovery_key_valid() {
        let db = fresh_db().await;
        let master = vault_with_secret(&db).await;
        let paper = master
            .issue_recovery_key(&db)
            .await
            .expect("ключ восстановления");

        master
            .change_password(&db, "пароль-мастер-12", "сменённый-пароль")
            .await
            .expect("смена пароля");
        master.lock().await;

        master
            .recover(&db, &paper, "после смены пароля")
            .await
            .expect("ключ, выпущенный до смены пароля, обязан работать после неё");
        assert_eq!(
            secret(&master, &db).await.as_deref(),
            Some("пароль-от-сервера")
        );
    }

    /// Новый выпуск отзывает прежний ключ: так отзывают потерянную бумажку.
    #[tokio::test]
    async fn issuing_a_new_recovery_key_revokes_the_old_one() {
        let db = fresh_db().await;
        let master = vault_with_secret(&db).await;
        let lost = master.issue_recovery_key(&db).await.expect("первый ключ");
        let current = master.issue_recovery_key(&db).await.expect("второй ключ");
        master.lock().await;

        assert_eq!(
            code(master.recover(&db, &lost, "новый-пароль-12").await),
            codes::MASTER_RECOVERY_WRONG,
            "потерянная бумажка после перевыпуска не открывает ничего"
        );
        master
            .recover(&db, &current, "новый-пароль-12")
            .await
            .expect("действующий ключ обязан работать");
    }

    /// Подбор ключа упирается в ту же паузу, что и подбор пароля.
    #[tokio::test]
    async fn guessing_recovery_keys_leads_to_a_pause() {
        let db = fresh_db().await;
        let master = vault_with_secret(&db).await;
        master.issue_recovery_key(&db).await.expect("ключ");
        master.lock().await;

        let wrong = any_recovery_key();
        for _ in 0..MAX_ATTEMPTS - 1 {
            assert_eq!(
                code(master.recover(&db, &wrong, "x-длинный-пароль").await),
                codes::MASTER_RECOVERY_WRONG
            );
        }
        assert_eq!(
            code(master.recover(&db, &wrong, "x-длинный-пароль").await),
            codes::MASTER_LOCKED_OUT,
            "после предела попыток — пауза"
        );
    }

    /// Опечатка в длине — не подбор: она не засчитывается и не приближает паузу.
    #[tokio::test]
    async fn a_malformed_recovery_key_does_not_count_as_an_attempt() {
        let db = fresh_db().await;
        let master = vault_with_secret(&db).await;
        master.issue_recovery_key(&db).await.expect("ключ");
        master.lock().await;

        for _ in 0..MAX_ATTEMPTS * 2 {
            assert_eq!(
                code(master.recover(&db, "ABCD-EFGH", "x-длинный-пароль").await),
                codes::MASTER_RECOVERY_MALFORMED
            );
        }
        assert_eq!(master.status(&db).await.expect("статус").failed_attempts, 0);
    }

    #[tokio::test]
    async fn status_reports_the_kdf_parameters_it_really_uses() {
        // Подпись под полем пароля берётся отсюда. Число, вписанное в
        // разметку, расходилось бы с настоящим молча.
        let db = fresh_db().await;
        let before = MasterKeyring::new().status(&db).await.expect("статус").kdf;
        assert_eq!(before.memory_kib, DEFAULT_M_COST);

        sqlx::query("INSERT INTO settings (key, value) VALUES (?1, '12345')")
            .bind(keys::KDF_M_COST)
            .execute(&db)
            .await
            .expect("записано");
        let after = MasterKeyring::new().status(&db).await.expect("статус").kdf;
        assert_eq!(after.memory_kib, 12345);
    }

    #[tokio::test]
    async fn recovery_without_an_issued_key_says_so() {
        let db = fresh_db().await;
        let master = vault_with_secret(&db).await;
        master.lock().await;

        assert_eq!(
            code(master.recover(&db, &any_recovery_key(), "x-длинный-пароль").await),
            codes::MASTER_RECOVERY_NOT_SET
        );
        assert!(!master.status(&db).await.expect("статус").recovery_key_set);
    }

    /// Выключение убирает и ключ восстановления.
    ///
    /// Иначе после выключения и повторного включения старая обёртка лежала бы
    /// рядом с новым хранилищем и не открывала бы ничего — а интерфейс
    /// показывал бы, что ключ выпущен.
    #[tokio::test]
    async fn disabling_forgets_the_recovery_key() {
        let db = fresh_db().await;
        let master = vault_with_secret(&db).await;
        master.issue_recovery_key(&db).await.expect("ключ");

        master.disable(&db, "пароль-мастер-12").await.expect("выключение");
        for key in keys::ALL {
            assert!(
                setting_get(&db, key).await.expect("чтение").is_none(),
                "после выключения осталась настройка {key}"
            );
        }
    }

    /// Собрать хранилище первой версии — так, как его заводил прежний код.
    ///
    /// Секреты там зашифрованы прямо ключом из пароля, обёртки ключа данных и
    /// номера версии нет.
    async fn first_version_vault(db: &SqlitePool, password: &str, secrets: &[(i64, &[u8])]) {
        let salt = random_bytes::<16>().expect("соль");
        let kek = derive_key(
            password,
            &salt,
            DEFAULT_M_COST,
            DEFAULT_T_COST,
            DEFAULT_P_COST,
        )
        .expect("ключ");
        let (v_nonce, verifier) = encrypt(&kek, VERIFIER_PLAINTEXT).expect("verifier");

        for (key, value) in [
            (keys::SALT, B64.encode(salt)),
            (keys::VERIFIER_NONCE, B64.encode(&v_nonce)),
            (keys::VERIFIER, B64.encode(&verifier)),
            (keys::KDF_M_COST, DEFAULT_M_COST.to_string()),
            (keys::KDF_T_COST, DEFAULT_T_COST.to_string()),
            (keys::KDF_P_COST, DEFAULT_P_COST.to_string()),
            (keys::ENABLED, "true".to_string()),
        ] {
            sqlx::query("INSERT INTO settings (key, value) VALUES (?1, ?2)")
                .bind(key)
                .bind(value)
                .execute(db)
                .await
                .expect("настройка");
        }

        for (ref_id, plain) in secrets {
            let (nonce, ct) = encrypt(&kek, plain).expect("шифрование");
            sqlx::query(
                "INSERT INTO vault (kind, ref_id, nonce, ciphertext) VALUES ('password', ?1, ?2, ?3)",
            )
            .bind(ref_id)
            .bind(nonce)
            .bind(ct)
            .execute(db)
            .await
            .expect("секрет");
        }
    }

    /// У кого мастер-пароль включён давно, хранилище первой версии — и после
    /// обновления приложения оно обязано открываться тем же паролем.
    #[tokio::test]
    async fn a_first_version_vault_is_upgraded_on_unlock() {
        let db = fresh_db().await;
        first_version_vault(&db, "старый", &[(1, "пароль-от-сервера".as_bytes())]).await;

        let master = MasterKeyring::new();
        master
            .unlock(&db, "старый", None)
            .await
            .expect("хранилище первой версии обязано открываться прежним паролем");

        assert_eq!(
            secret(&master, &db).await.as_deref(),
            Some("пароль-от-сервера")
        );
        assert_eq!(
            setting_get(&db, keys::VAULT_VERSION)
                .await
                .expect("версия")
                .as_deref(),
            Some(VAULT_VERSION_CURRENT),
            "после разблокировки хранилище обязано быть второй версии"
        );

        // И после перевода на нём работает всё, ради чего переводили.
        let paper = master
            .issue_recovery_key(&db)
            .await
            .expect("ключ восстановления");
        master.lock().await;
        master
            .recover(&db, &paper, "новый-пароль-12")
            .await
            .expect("восстановление");
        assert_eq!(
            secret(&master, &db).await.as_deref(),
            Some("пароль-от-сервера")
        );
    }

    /// Перевод — всё или ничего.
    ///
    /// Один испорченный секрет обязан откатить весь перевод: хранилище,
    /// переведённое наполовину, не открывается ни старой схемой, ни новой.
    #[tokio::test]
    async fn a_failed_upgrade_leaves_the_vault_untouched() {
        let db = fresh_db().await;
        first_version_vault(&db, "старый", &[(1, b"whole"), (2, b"to be damaged")]).await;
        sqlx::query("UPDATE vault SET ciphertext = x'00' WHERE ref_id = 2")
            .execute(&db)
            .await
            .expect("порча");
        let before: Vec<(i64, Vec<u8>)> =
            sqlx::query_as("SELECT ref_id, ciphertext FROM vault ORDER BY ref_id")
                .fetch_all(&db)
                .await
                .expect("до");

        let master = MasterKeyring::new();
        assert!(master.unlock(&db, "старый", None).await.is_err());

        let after: Vec<(i64, Vec<u8>)> =
            sqlx::query_as("SELECT ref_id, ciphertext FROM vault ORDER BY ref_id")
                .fetch_all(&db)
                .await
                .expect("после");
        assert_eq!(
            before, after,
            "неудачный перевод не должен трогать ни один секрет"
        );
        assert!(
            setting_get(&db, keys::VAULT_VERSION)
                .await
                .expect("версия")
                .is_none(),
            "неудачный перевод не должен помечать хранилище второй версией"
        );
        assert!(!master.is_unlocked().await);
    }

    #[tokio::test]
    async fn a_new_password_shorter_than_the_minimum_is_refused_everywhere() {
        let db = fresh_db().await;
        let master = MasterKeyring::new();
        let short = "а".repeat(MIN_PASSWORD_CHARS - 1);
        assert_eq!(code(master.enable(&db, &short).await), codes::MASTER_PASSWORD_TOO_SHORT);

        master.enable(&db, "пароль-мастер-12").await.expect("включение");
        assert_eq!(
            code(master.change_password(&db, "пароль-мастер-12", &short).await),
            codes::MASTER_PASSWORD_TOO_SHORT
        );
        let paper = master.issue_recovery_key(&db).await.expect("ключ");
        assert_eq!(
            code(master.recover(&db, &paper, &short).await),
            codes::MASTER_PASSWORD_TOO_SHORT
        );
        // Ровно минимум проходит; считаются знаки, а не байты.
        let exact = "я".repeat(MIN_PASSWORD_CHARS);
        master.change_password(&db, "пароль-мастер-12", &exact).await.expect("граница");
        assert_eq!(master.status(&db).await.unwrap().min_length, MIN_PASSWORD_CHARS);
    }
}
