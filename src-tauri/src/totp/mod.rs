//! TOTP — одноразовые коды по RFC 6238 (#153a).
//!
//! Код считается **у нас**: секрет плюс текущее время дают шесть цифр, сервер
//! только проверяет присланное. Поэтому окно в 30 секунд приходится отслеживать
//! на клиенте — это суть алгоритма, а не наша выдумка.
//!
//! Секрет хранится там же, где пароли и passphrase — через фасад `secrets`,
//! то есть в keyring или в зашифрованном хранилище, смотря что включено.

use anyhow::{bail, Context, Result};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Sha256, Sha512};
use std::time::{SystemTime, UNIX_EPOCH};

/// Алгоритм хеша. Подавляющее большинство сервисов использует SHA-1; остальные
/// поддерживаем, потому что они встречаются в `otpauth://`-ссылках.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Algorithm {
    Sha1,
    Sha256,
    Sha512,
}

impl Algorithm {
    fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_uppercase().as_str() {
            "SHA1" => Some(Self::Sha1),
            "SHA256" => Some(Self::Sha256),
            "SHA512" => Some(Self::Sha512),
            _ => None,
        }
    }
}

/// Разобранные параметры одноразового кода.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TotpConfig {
    /// Секрет в base32 — в том виде, в каком его даёт сервис.
    pub secret: String,
    pub algorithm: Algorithm,
    pub digits: u32,
    /// Длина окна в секундах.
    pub period: u64,
    /// Подпись из ссылки, если была — показываем пользователю для сверки.
    pub label: Option<String>,
    pub issuer: Option<String>,
}

impl Default for TotpConfig {
    fn default() -> Self {
        Self {
            secret: String::new(),
            algorithm: Algorithm::Sha1,
            digits: 6,
            period: 30,
            label: None,
            issuer: None,
        }
    }
}

/// Раскодировать base32 (RFC 4648) без учёта регистра, пробелов и padding'а.
///
/// Сервисы печатают секрет по-разному: группами через пробел, строчными,
/// с `=` на конце и без. Всё это должно приниматься — иначе пользователь
/// получит «неверный секрет» на совершенно валидной строке.
pub fn decode_base32(input: &str) -> Result<Vec<u8>> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    let mut out = Vec::new();

    for ch in input.chars() {
        if ch == '=' || ch.is_whitespace() || ch == '-' {
            continue;
        }
        let up = ch.to_ascii_uppercase() as u8;
        let value = match ALPHABET.iter().position(|&c| c == up) {
            Some(v) => v as u32,
            None => bail!("недопустимый символ в base32: {ch:?}"),
        };
        buffer = (buffer << 5) | value;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }

    if out.is_empty() {
        bail!("пустой секрет");
    }
    Ok(out)
}

/// Посчитать код для конкретного момента времени.
pub fn code_at(config: &TotpConfig, unix_seconds: u64) -> Result<String> {
    let key = decode_base32(&config.secret)?;
    let counter = unix_seconds / config.period.max(1);
    let msg = counter.to_be_bytes();

    let digest: Vec<u8> = match config.algorithm {
        Algorithm::Sha1 => {
            let mut mac = Hmac::<Sha1>::new_from_slice(&key).context("ключ HMAC")?;
            mac.update(&msg);
            mac.finalize().into_bytes().to_vec()
        }
        Algorithm::Sha256 => {
            let mut mac = Hmac::<Sha256>::new_from_slice(&key).context("ключ HMAC")?;
            mac.update(&msg);
            mac.finalize().into_bytes().to_vec()
        }
        Algorithm::Sha512 => {
            let mut mac = Hmac::<Sha512>::new_from_slice(&key).context("ключ HMAC")?;
            mac.update(&msg);
            mac.finalize().into_bytes().to_vec()
        }
    };

    // Динамическая усечка из RFC 4226: младший полубайт последнего байта
    // указывает, откуда брать четыре байта результата.
    let offset = (digest[digest.len() - 1] & 0x0f) as usize;
    let binary = ((u32::from(digest[offset]) & 0x7f) << 24)
        | (u32::from(digest[offset + 1]) << 16)
        | (u32::from(digest[offset + 2]) << 8)
        | u32::from(digest[offset + 3]);

    let modulo = 10u32.pow(config.digits);
    Ok(format!(
        "{:0width$}",
        binary % modulo,
        width = config.digits as usize
    ))
}

/// Сгенерировать новый секрет (160 бит, как рекомендует RFC 4226).
pub fn generate_secret() -> Result<String> {
    let mut bytes = [0u8; 20];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("нет случайности: {e}"))?;
    Ok(encode_base32(&bytes))
}

/// Ссылка для аутентификатора — то же, что кодируют в QR.
pub fn build_uri(secret: &str, label: &str) -> String {
    let encoded_label = label.replace(' ', "%20");
    format!("otpauth://totp/{encoded_label}?secret={secret}&issuer={encoded_label}")
}

/// Кодирование в base32 — нужно при выдаче свежего секрета.
pub fn encode_base32(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::new();
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for &byte in data {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// Сколько соседних окон принимаем помимо текущего.
///
/// Часы на телефоне и на компьютере почти всегда расходятся на секунды, а
/// пока человек читает и печатает код, окно может смениться. Без допуска
/// это выглядит как «код не подходит», хотя всё верно. Одно окно в обе
/// стороны — общепринятый компромисс: удобство есть, перебор не облегчается
/// сколько-нибудь заметно.
pub const DRIFT_WINDOWS: i64 = 1;

/// Проверить код с учётом расхождения часов.
pub fn verify(config: &TotpConfig, code: &str) -> Result<bool> {
    let entered = code.trim().replace(' ', "");
    if entered.is_empty() {
        return Ok(false);
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("системные часы раньше эпохи")?
        .as_secs();
    let period = config.period.max(1);

    for shift in -DRIFT_WINDOWS..=DRIFT_WINDOWS {
        let moment = (now as i64) + shift * (period as i64);
        if moment < 0 {
            continue;
        }
        let expected = code_at(config, moment as u64)?;
        // Сравнение постоянного времени здесь не нужно: код живёт 30 секунд
        // и подбирается по сети, а не по таймингу локальной функции.
        if expected == entered {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Принять то, что пользователь вставил: либо голый секрет, либо `otpauth://`.
///
/// Ссылка — это ровно то, что зашито в QR-код, поэтому «вставить ссылку»
/// заменяет сканирование картинки.
pub fn parse_input(input: &str) -> Result<TotpConfig> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        bail!("пустой секрет");
    }

    if !trimmed.to_ascii_lowercase().starts_with("otpauth://") {
        // Голый секрет: проверяем, что он вообще раскодируется.
        decode_base32(trimmed)?;
        return Ok(TotpConfig {
            secret: trimmed.to_string(),
            ..Default::default()
        });
    }

    let without_scheme = &trimmed["otpauth://".len()..];
    let (kind, rest) = without_scheme
        .split_once('/')
        .context("в ссылке нет типа (ожидался otpauth://totp/...)")?;
    if !kind.eq_ignore_ascii_case("totp") {
        bail!("поддерживается только otpauth://totp/, а не {kind}");
    }

    let (label_part, query) = match rest.split_once('?') {
        Some((l, q)) => (l, q),
        None => (rest, ""),
    };

    let mut config = TotpConfig {
        label: percent_decode(label_part).filter(|s| !s.is_empty()),
        ..Default::default()
    };

    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = match pair.split_once('=') {
            Some(kv) => kv,
            None => continue,
        };
        let value = percent_decode(value).unwrap_or_default();
        match key.to_ascii_lowercase().as_str() {
            "secret" => config.secret = value,
            "issuer" => config.issuer = Some(value).filter(|s| !s.is_empty()),
            "algorithm" => {
                if let Some(a) = Algorithm::parse(&value) {
                    config.algorithm = a;
                }
            }
            // Неизвестные и неразбираемые параметры игнорируем: лучше выдать
            // код по умолчанию, чем отказать из-за лишнего поля в ссылке.
            "digits" => {
                if let Ok(d) = value.parse::<u32>() {
                    if (6..=10).contains(&d) {
                        config.digits = d;
                    }
                }
            }
            "period" => {
                if let Ok(p) = value.parse::<u64>() {
                    if p > 0 {
                        config.period = p;
                    }
                }
            }
            _ => {}
        }
    }

    if config.secret.is_empty() {
        bail!("в ссылке нет параметра secret");
    }
    decode_base32(&config.secret)?;
    Ok(config)
}

/// Минимальный percent-decoding: в метках сплошь и рядом `%20` и `%40`.
fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&input[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Тестовые векторы из RFC 6238, приложение B. Семя — ASCII-строка
    /// "12345678901234567890", повторённая до нужной длины ключа.
    fn vector_config(algorithm: Algorithm) -> TotpConfig {
        let seed = match algorithm {
            Algorithm::Sha1 => "12345678901234567890".to_string(),
            Algorithm::Sha256 => "12345678901234567890123456789012".to_string(),
            Algorithm::Sha512 => {
                "1234567890123456789012345678901234567890123456789012345678901234".to_string()
            }
        };
        TotpConfig {
            secret: base32_encode(seed.as_bytes()),
            algorithm,
            digits: 8,
            period: 30,
            ..Default::default()
        }
    }

    /// Кодировщик нужен только тестам — RFC даёт семя в ASCII, а наш вход base32.
    fn base32_encode(data: &[u8]) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
        let mut out = String::new();
        let mut buffer: u32 = 0;
        let mut bits: u32 = 0;
        for &byte in data {
            buffer = (buffer << 8) | u32::from(byte);
            bits += 8;
            while bits >= 5 {
                bits -= 5;
                out.push(ALPHABET[((buffer >> bits) & 31) as usize] as char);
            }
        }
        if bits > 0 {
            out.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
        }
        out
    }

    #[test]
    fn matches_rfc6238_sha1_vectors() {
        let cfg = vector_config(Algorithm::Sha1);
        for (time, expected) in [
            (59u64, "94287082"),
            (1_111_111_109, "07081804"),
            (1_111_111_111, "14050471"),
            (1_234_567_890, "89005924"),
            (2_000_000_000, "69279037"),
            (20_000_000_000, "65353130"),
        ] {
            assert_eq!(code_at(&cfg, time).unwrap(), expected, "T={time}");
        }
    }

    #[test]
    fn matches_rfc6238_sha256_vectors() {
        let cfg = vector_config(Algorithm::Sha256);
        for (time, expected) in [
            (59u64, "46119246"),
            (1_111_111_109, "68084774"),
            (2_000_000_000, "90698825"),
        ] {
            assert_eq!(code_at(&cfg, time).unwrap(), expected, "T={time}");
        }
    }

    #[test]
    fn matches_rfc6238_sha512_vectors() {
        let cfg = vector_config(Algorithm::Sha512);
        for (time, expected) in [
            (59u64, "90693936"),
            (1_111_111_109, "25091201"),
            (2_000_000_000, "38618901"),
        ] {
            assert_eq!(code_at(&cfg, time).unwrap(), expected, "T={time}");
        }
    }

    #[test]
    fn accepts_secret_in_any_common_formatting() {
        // Сервисы печатают секрет группами, строчными и с padding'ом —
        // всё это одна и та же строка.
        let canonical = decode_base32("JBSWY3DPEHPK3PXP").unwrap();
        for variant in [
            "jbswy3dpehpk3pxp",
            "JBSW Y3DP EHPK 3PXP",
            "JBSWY3DPEHPK3PXP======",
            "JBSW-Y3DP-EHPK-3PXP",
        ] {
            assert_eq!(decode_base32(variant).unwrap(), canonical, "{variant}");
        }
    }

    #[test]
    fn rejects_garbage_secret() {
        assert!(decode_base32("не base32!").is_err());
        assert!(decode_base32("").is_err());
    }

    #[test]
    fn parses_otpauth_uri_with_all_parameters() {
        let uri = "otpauth://totp/Example%3Aalice%40example.com\
                   ?secret=JBSWY3DPEHPK3PXP&issuer=Example&algorithm=SHA256&digits=8&period=60";
        let cfg = parse_input(uri).unwrap();
        assert_eq!(cfg.secret, "JBSWY3DPEHPK3PXP");
        assert_eq!(cfg.algorithm, Algorithm::Sha256);
        assert_eq!(cfg.digits, 8);
        assert_eq!(cfg.period, 60);
        assert_eq!(cfg.issuer.as_deref(), Some("Example"));
        assert_eq!(cfg.label.as_deref(), Some("Example:alice@example.com"));
    }

    #[test]
    fn plain_secret_gets_standard_defaults() {
        let cfg = parse_input("  JBSWY3DPEHPK3PXP  ").unwrap();
        assert_eq!(cfg.secret, "JBSWY3DPEHPK3PXP");
        assert_eq!(cfg.algorithm, Algorithm::Sha1);
        assert_eq!(cfg.digits, 6);
        assert_eq!(cfg.period, 30);
    }

    #[test]
    fn rejects_uri_without_secret() {
        let err = parse_input("otpauth://totp/Example?issuer=Example").unwrap_err();
        assert!(err.to_string().contains("secret"), "{err}");
    }

    #[test]
    fn rejects_hotp_uri() {
        // HOTP считается по счётчику, а не по времени — молча выдавать
        // неверные коды хуже, чем отказать.
        assert!(parse_input("otpauth://hotp/X?secret=JBSWY3DPEHPK3PXP").is_err());
    }

    #[test]
    fn verify_accepts_the_current_code() {
        let cfg = parse_input("JBSWY3DPEHPK3PXP").unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let code = code_at(&cfg, now).unwrap();
        assert!(verify(&cfg, &code).unwrap());
        // Пробелы и лишние края пользователь ставит постоянно.
        assert!(verify(&cfg, &format!(" {code} ")).unwrap());
    }

    #[test]
    fn verify_tolerates_clock_drift_by_one_window() {
        // Часы телефона и компьютера расходятся; без допуска верный код
        // выглядел бы неверным.
        let cfg = parse_input("JBSWY3DPEHPK3PXP").unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let previous = code_at(&cfg, now - cfg.period).unwrap();
        let next = code_at(&cfg, now + cfg.period).unwrap();
        assert!(verify(&cfg, &previous).unwrap(), "предыдущее окно");
        assert!(verify(&cfg, &next).unwrap(), "следующее окно");
    }

    #[test]
    fn verify_rejects_distant_and_empty_codes() {
        let cfg = parse_input("JBSWY3DPEHPK3PXP").unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        // Далёкое окно принимать нельзя: это уже не расхождение часов.
        let far = code_at(&cfg, now + cfg.period * 10).unwrap();
        assert!(!verify(&cfg, &far).unwrap());
        assert!(!verify(&cfg, "").unwrap());

        // Код, посчитанный по другому секрету, приниматься не должен —
        // это и есть основная проверка, ради которой verify существует.
        let other = parse_input("MFRGGZDFMZTWQ2LK").unwrap();
        let foreign = code_at(&other, now).unwrap();
        assert!(!verify(&cfg, &foreign).unwrap());
    }
}
