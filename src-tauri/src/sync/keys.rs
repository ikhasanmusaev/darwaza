//! Вывод ключей аккаунта по спецификации `docs/06-web/02-auth-and-crypto.md`.
//!
//! Это **единственная** копия спецификации в приложении. До сих пор она жила
//! только в живом тесте (`tests/sync_live.rs`), и тот проверял сам себя:
//! написанное в тесте сходилось с сервером, а приложение об этом не знало
//! ничего. Зелёный тест доказывал сервер и протокол, но не клиента.
//!
//! Главное свойство: **мастер-ключ не покидает устройство.** На сервер уходит
//! `auth_hash` — производная, из которой мастер-ключ не восстановить. Ключ
//! шифрования выводится из мастер-ключа и тоже остаётся здесь.
//!
//! Расхождение с сервером хотя бы в одном байте означает «не могу войти» без
//! единой подсказки, поэтому каждое решение ниже объяснено: менять их вслепую
//! нельзя.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::{Context, Result};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use unicode_normalization::UnicodeNormalization;

/// Параметры Argon2id. Обязаны совпадать с серверными до последнего числа.
///
/// Открыты наружу намеренно: при регистрации клиент **сообщает их серверу**,
/// и брать их оттуда же, откуда выводится ключ, — единственный способ не дать
/// им разойтись. Продублированные в вызывающем коде, они однажды разойдутся.
pub const KDF_MEMORY_KIB: u32 = 65536;
pub const KDF_ITERATIONS: u32 = 3;
/// Параллелизм **единица**, а не число ядер: иначе ключ зависел бы от машины,
/// и вход с другого компьютера давал бы другой результат.
pub const KDF_PARALLELISM: u32 = 1;

/// Длина nonce у AES-GCM.
const NONCE_LEN: usize = 12;

/// Нормализация почты: обрезка пробелов и нижний регистр.
///
/// Без обрезки адрес с пробелом на конце даёт другую соль — то есть другой
/// ключ. Сервер однажды на этом уже спотыкался: регистрация принимала адрес с
/// пробелами, а вход отвергал.
pub fn normalize_email(raw: &str) -> String {
    raw.trim().to_lowercase()
}

/// Нормализация пароля — NFC по RFC 8265, **не** NFKC.
///
/// NFKC склеивает визуально различные символы, молча снижая энтропию пароля.
pub fn normalize_password(raw: &str) -> String {
    raw.nfc().collect()
}

fn argon2id(password: &[u8], salt: &[u8]) -> Result<[u8; 32]> {
    let params = Params::new(KDF_MEMORY_KIB, KDF_ITERATIONS, KDF_PARALLELISM, Some(32))
        .map_err(|e| anyhow::anyhow!("параметры Argon2id: {e}"))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; 32];
    argon
        .hash_password_into(password, salt, &mut out)
        .map_err(|e| anyhow::anyhow!("вывод ключа: {e}"))?;
    Ok(out)
}

/// Мастер-ключ. **Остаётся на устройстве и никогда не уходит на сервер.**
///
/// Соль строится по безусловному правилу: постоянный префикс гарантирует
/// минимум восемь байт даже для очень короткого адреса. Ветвления «если
/// короче — дополнить» нет намеренно: такое ветвление однажды разойдётся
/// между клиентом и сервером, и разойдётся молча.
pub fn master_key(email: &str, password: &str) -> Result<[u8; 32]> {
    let salt = format!("darwaza:{}", normalize_email(email));
    argon2id(normalize_password(password).as_bytes(), salt.as_bytes())
}

/// То, что уходит на сервер вместо пароля.
///
/// Base64-**строкой**: сервер хеширует именно строку. Отправить сырые байты
/// значило бы получить другой хеш на той стороне.
pub fn auth_hash(master: &[u8; 32], password: &str) -> Result<String> {
    let salt = normalize_password(password);
    Ok(B64.encode(argon2id(master, salt.as_bytes())?))
}

/// Ключ шифрования записей.
///
/// HKDF-**Expand без Extract**: мастер-ключ уже равномерен, это выход
/// Argon2id. Лишний Extract дал бы ДРУГИЕ байты, и стороны разошлись бы, не
/// сообщив об этом ничем, кроме «не расшифровывается».
pub fn encryption_key(master: &[u8; 32]) -> Result<[u8; 32]> {
    let mut mac = Hmac::<Sha256>::new_from_slice(master).context("ключ HMAC")?;
    mac.update(b"enc");
    // Единственный блок HKDF-Expand.
    mac.update(&[0x01]);
    let out = mac.finalize().into_bytes();
    let mut key = [0u8; 32];
    key.copy_from_slice(&out[..32]);
    Ok(key)
}

/// Запечатать для отправки: `base64(nonce ‖ ciphertext ‖ tag)`.
///
/// Формат отличается от локального хранения, где nonce и шифротекст лежат
/// раздельными полями. Здесь **nonce впереди** и всё склеено — так описано в
/// спецификации, и так это разбирает сервер.
pub fn seal_for_wire(key: &[u8; 32], plaintext: &[u8]) -> Result<String> {
    let cipher = Aes256Gcm::new(&Key::<Aes256Gcm>::from(*key));
    let mut nonce_bytes = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce_bytes).map_err(|e| anyhow::anyhow!("нет случайности: {e}"))?;
    let ct = cipher
        .encrypt(
            &Nonce::try_from(&nonce_bytes[..]).map_err(|_| anyhow::anyhow!("длина nonce"))?,
            plaintext,
        )
        .map_err(|_| anyhow::anyhow!("шифрование не удалось"))?;

    let mut blob = nonce_bytes.to_vec();
    blob.extend_from_slice(&ct);
    Ok(B64.encode(blob))
}

/// Распечатать полученное с сервера.
pub fn open_from_wire(key: &[u8; 32], blob_b64: &str) -> Result<Vec<u8>> {
    let blob = B64
        .decode(blob_b64)
        .context("блоб не разбирается как base64")?;
    if blob.len() <= NONCE_LEN {
        anyhow::bail!("блоб короче nonce: {} байт", blob.len());
    }
    let (nonce, ct) = blob.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new(&Key::<Aes256Gcm>::from(*key));
    cipher
        .decrypt(
            &Nonce::try_from(nonce).map_err(|_| anyhow::anyhow!("длина nonce"))?,
            ct,
        )
        // Причину не уточняем: «неверный ключ» и «испорченные данные»
        // различить нельзя, а догадка в сообщении увела бы разбор не туда.
        .map_err(|_| anyhow::anyhow!("не расшифровывается: неверный ключ или испорченные данные"))
}

/// Какими ключами шифровать и читать записи после входа (#180c).
///
/// Возвращает пару: которым писать и которым ещё уметь читать.
///
/// **Почему это не просто ключ из пароля.** По спецификации записи шифруются
/// случайным `symmetricKey`, а он лежит на сервере под обёрткой, снимаемой
/// ключом из пароля (шаг 5 входа в `02-auth-and-crypto.md`). Приложение до
/// 2026-09-22 обёртку не снимало и шифровало прямо ключом из пароля.
/// Работало это ровно до первой смены пароля: ключ становится другим, и всё
/// отправленное раньше перестаёт читаться. По той же причине не могло
/// работать и восстановление по бумаге — под обёрткой оказывался бы ключ,
/// которым никто ничего не шифровал.
///
/// Переход без потерь: новым ключом пишем, старым ещё читаем.
///
/// **Обёртки нет** — остаёмся на прежнем поведении: так отвечает сервер,
/// который эту часть ещё не умеет, а выдумать себе ключ, отличный от других
/// устройств, нельзя.
///
/// **Обёртка не открывается** — ошибка, и намеренно. Молча взять вместо неё
/// ключ из пароля значило бы разойтись с остальными устройствами и портить
/// данные тихо вместо того, чтобы сломаться громко.
pub fn record_keys(
    enc: &[u8; 32],
    protected: Option<&str>,
) -> Result<([u8; 32], Option<[u8; 32]>)> {
    let Some(protected) = protected else {
        return Ok((*enc, None));
    };

    let opened = open_from_wire(enc, protected).context("обёртка ключа записей не открывается")?;
    let symmetric: [u8; 32] = opened
        .try_into()
        .map_err(|v: Vec<u8>| anyhow::anyhow!("ключ записей длиной {} байт вместо 32", v.len()))?;

    Ok((symmetric, Some(*enc)))
}

/// Ключи, выведенные из ключа восстановления аккаунта (#180c).
///
/// ```text
/// recoveryEncKey   = HKDF-Expand(RK, "rec",  32)
/// recoveryAuthHash = HKDF-Expand(RK, "auth", 32)
/// ```
///
/// `HKDF` — **только Expand, без Extract**, ровно как для ключа шифрования:
/// на входе уже равномерные 160 бит, а лишний шаг дал бы другие байты у той
/// реализации, которая его сделает. Расхождение здесь не видно ничем, кроме
/// «ключ не подошёл».
///
/// Argon2id к ключу восстановления на устройстве **не применяется**:
/// перебирать 160 случайных бит бессмысленно, а медленная функция нужна там,
/// где на входе придуманный человеком пароль. Сервер считает Argon2id от
/// `recoveryAuthHash` своими лёгкими параметрами — ради одного пути в коде,
/// а не ради стойкости.
pub struct RecoveryKeys {
    /// Которым снимается обёртка с `symmetricKey`.
    pub enc: [u8; 32],
    /// Который уходит на сервер вместо ключа. Base64-строкой, как `authHash`.
    pub auth_hash: String,
}

/// Один блок HKDF-Expand над ключом восстановления.
fn expand(rk: &[u8; crate::recovery_key::KEY_BYTES], info: &[u8]) -> Result<[u8; 32]> {
    let mut mac = Hmac::<Sha256>::new_from_slice(rk).context("ключ HMAC")?;
    mac.update(info);
    // Счётчик единственного блока — так же, как в `encryption_key`.
    mac.update(&[0x01]);
    let out = mac.finalize().into_bytes();
    let mut key = [0u8; 32];
    key.copy_from_slice(&out[..32]);
    Ok(key)
}

pub fn recovery_keys(rk: &[u8; crate::recovery_key::KEY_BYTES]) -> Result<RecoveryKeys> {
    Ok(RecoveryKeys {
        enc: expand(rk, b"rec")?,
        auth_hash: B64.encode(expand(rk, b"auth")?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_normalization_makes_stray_spaces_harmless() {
        // Сервер на этом уже спотыкался: регистрация принимала адрес с
        // пробелами, вход — нет.
        assert_eq!(normalize_email("  User@Example.COM "), "user@example.com");
    }

    #[test]
    fn the_same_credentials_always_give_the_same_key() {
        // Иначе вход со второй машины давал бы другой ключ, и данные не
        // расшифровались бы без единой подсказки почему.
        let a = master_key("user@example.com", "пароль").unwrap();
        let b = master_key(" USER@example.com ", "пароль").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn a_different_password_gives_a_different_key() {
        let a = master_key("user@example.com", "пароль").unwrap();
        let b = master_key("user@example.com", "паролb").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn the_auth_hash_is_not_the_master_key() {
        // На сервер уходит производная. Совпади они — сервер получил бы ключ
        // расшифровки, и всё обещание рассыпалось бы.
        let master = master_key("user@example.com", "пароль").unwrap();
        let hash = auth_hash(&master, "пароль").unwrap();
        assert_ne!(B64.encode(master), hash);
    }

    #[test]
    fn the_encryption_key_differs_from_the_master_key() {
        let master = master_key("user@example.com", "пароль").unwrap();
        let enc = encryption_key(&master).unwrap();
        assert_ne!(master, enc);
    }

    #[test]
    fn sealed_data_comes_back_unchanged() {
        let key = encryption_key(&master_key("u@e.com", "пароль").unwrap()).unwrap();
        let blob = seal_for_wire(&key, b"secret payload").unwrap();
        assert_eq!(open_from_wire(&key, &blob).unwrap(), b"secret payload");
    }

    #[test]
    fn the_nonce_goes_first_and_is_twelve_bytes() {
        // Формат проверяем явно: разойдись он с сервером — тот не разберёт
        // ничего, а сообщение будет про «испорченные данные».
        let key = [7u8; 32];
        let blob = B64.decode(seal_for_wire(&key, b"x").unwrap()).unwrap();
        // nonce + шифротекст(1) + тег(16)
        assert_eq!(blob.len(), NONCE_LEN + 1 + 16);
    }

    #[test]
    fn the_same_text_seals_differently_every_time() {
        // Повторяющийся nonce у AES-GCM ломает не только скрытность, но и
        // целостность, поэтому свойство проверяем, а не подразумеваем.
        let key = [3u8; 32];
        assert_ne!(
            seal_for_wire(&key, "одно и то же".as_bytes()).unwrap(),
            seal_for_wire(&key, "одно и то же".as_bytes()).unwrap()
        );
    }

    #[test]
    fn a_wrong_key_fails_instead_of_returning_garbage() {
        let blob = seal_for_wire(&[1u8; 32], "тайна".as_bytes()).unwrap();
        assert!(open_from_wire(&[2u8; 32], &blob).is_err());
    }

    /// Ключ записей берётся из обёртки, а не из пароля.
    ///
    /// Это и есть исправление #180c. До него приложение шифровало записи
    /// ключом из пароля: смена пароля сделала бы всё отправленное раньше
    /// нечитаемым, а восстановление по бумаге не работало бы в принципе —
    /// под обёрткой лежал бы ключ, которым никто ничего не шифровал.
    #[test]
    fn the_record_key_comes_from_the_wrapper_not_from_the_password() {
        let enc = encryption_key(&master_key("u@e.com", "пароль").unwrap()).unwrap();
        let symmetric = [42u8; 32];
        let protected = seal_for_wire(&enc, &symmetric).unwrap();

        let (current, legacy) = record_keys(&enc, Some(&protected)).unwrap();

        assert_eq!(current, symmetric, "пишем ключом из обёртки");
        assert_ne!(current, enc, "ключ из пароля записям больше не ключ");
        assert_eq!(legacy, Some(enc), "прежним ключом ещё читаем");
    }

    /// Обёртки нет — остаёмся на прежнем поведении.
    ///
    /// Так отвечает сервер, который эту часть ещё не умеет. Выдумать себе
    /// ключ, отличный от других устройств, нельзя: записи разойдутся молча.
    #[test]
    fn without_a_wrapper_the_old_behaviour_stays() {
        let enc = encryption_key(&master_key("u@e.com", "пароль").unwrap()).unwrap();

        let (current, legacy) = record_keys(&enc, None).unwrap();

        assert_eq!(current, enc);
        assert_eq!(legacy, None, "читать нечем, кроме него же");
    }

    /// Обёртка, которая не открывается, — ошибка, а не тихий откат.
    ///
    /// Молча взять ключ из пароля значило бы разойтись с остальными
    /// устройствами: они пишут ключом из обёртки, мы бы писали другим, и
    /// каждый видел бы половину данных.
    #[test]
    fn an_unreadable_wrapper_fails_instead_of_falling_back() {
        let enc = encryption_key(&master_key("u@e.com", "пароль").unwrap()).unwrap();
        let foreign = seal_for_wire(&[9u8; 32], &[1u8; 32]).unwrap();

        assert!(record_keys(&enc, Some(&foreign)).is_err());
        // И длина проверяется: под обёрткой обязаны быть ровно 32 байта.
        let short = seal_for_wire(&enc, "коротко".as_bytes()).unwrap();
        assert!(record_keys(&enc, Some(&short)).is_err());
    }

    /// Известный ответ для вывода ключей из бумаги.
    ///
    /// Живой тест против сервера этого **не** проверяет, и это важно
    /// понимать: обе стороны обёртки считает наш же код, а сервер хранит
    /// `recoveryAuthHash` как непрозрачную строку. Поменяй здесь слово `rec`
    /// на `recovery` — живой тест всё равно пройдёт, потому что он проверяет
    /// согласие с сервером, а не совпадение со спецификацией. Проверено
    /// подменой: проходит.
    ///
    /// Разойтись здесь можно только с кабинетом — другой реализацией на
    /// другом языке. Поэтому единственная настоящая защита — образец,
    /// закреплённый у обеих сторон. Менять его можно только вместе с ними.
    #[test]
    fn known_recovery_derivation() {
        let rk = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13,
        ];
        let derived = recovery_keys(&rk).unwrap();

        assert_eq!(
            hex(&derived.enc),
            "0f7e8faf70921119b74778550e4431d51f855ba4519d23dee69a20236f207b0d",
            "recoveryEncKey разошёлся со спецификацией"
        );
        assert_eq!(
            derived.auth_hash, "t6CxMn0xEUsCMV1zPDrwM2NAQJz7ETUoTZWXMbxw4zM=",
            "recoveryAuthHash разошёлся со спецификацией"
        );
    }

    fn hex(bytes: &[u8; 32]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Ключ с бумаги открывает ту же обёртку, что положили под него.
    ///
    /// Это и есть всё восстановление: под двумя обёртками лежит **один**
    /// ключ записей. Положи под бумагу что-то своё — и человек, восстановив
    /// доступ, получил бы пустую базу вместо своих хостов.
    #[test]
    fn the_paper_key_opens_the_same_record_key() {
        let rk = crate::recovery_key::generate().unwrap();
        let recovery = recovery_keys(&rk).unwrap();
        let symmetric = [17u8; 32];

        let wrapped = seal_for_wire(&recovery.enc, &symmetric).unwrap();

        // Тот же ключ, набранный с бумаги заново.
        let retyped = crate::recovery_key::parse(&crate::recovery_key::format(&rk)).unwrap();
        let again = recovery_keys(&retyped).unwrap();

        assert_eq!(
            open_from_wire(&again.enc, &wrapped).unwrap(),
            symmetric.to_vec()
        );
        assert_eq!(again.auth_hash, recovery.auth_hash);
    }

    /// На сервер уходит не ключ, а производная — и не та, которой снимают
    /// обёртку.
    ///
    /// Совпади они — сервер, получив `recoveryAuthHash`, снял бы обёртку сам,
    /// и обещание «сервер не может прочитать данные» перестало бы
    /// выполняться при первом же восстановлении.
    #[test]
    fn what_goes_to_the_server_cannot_open_anything() {
        let rk = crate::recovery_key::generate().unwrap();
        let recovery = recovery_keys(&rk).unwrap();

        assert_ne!(B64.encode(recovery.enc), recovery.auth_hash);
        // И сам ключ с бумаги на сервер не уходит ни в каком виде.
        assert_ne!(B64.encode(rk), recovery.auth_hash);

        let wrapped = seal_for_wire(&recovery.enc, &[5u8; 32]).unwrap();
        let from_server = B64.decode(&recovery.auth_hash).unwrap();
        let as_key: [u8; 32] = from_server.try_into().unwrap();
        assert!(open_from_wire(&as_key, &wrapped).is_err());
    }

    /// Разные ключи с бумаги дают разные производные.
    #[test]
    fn different_paper_keys_give_different_results() {
        let a = recovery_keys(&[1u8; crate::recovery_key::KEY_BYTES]).unwrap();
        let b = recovery_keys(&[2u8; crate::recovery_key::KEY_BYTES]).unwrap();

        assert_ne!(a.enc, b.enc);
        assert_ne!(a.auth_hash, b.auth_hash);
    }

    #[test]
    fn a_truncated_blob_reports_its_length_instead_of_panicking() {
        // split_at на коротком срезе паникует, а данные приходят снаружи.
        let short = B64.encode([0u8; 5]);
        let err = open_from_wire(&[1u8; 32], &short).unwrap_err().to_string();
        assert!(err.contains("короче nonce"), "получено: {err}");
    }
}
