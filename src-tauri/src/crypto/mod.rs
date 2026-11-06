//! Общие криптографические примитивы.
//!
//! Вынесены сюда, потому что ключей в приложении два и живут они по-разному:
//! ключ мастер-пароля выводится из пароля и стирается при блокировке, а ключ
//! синхронизации приходит с аккаунта. Алгоритм при этом один и тот же, и
//! дублировать его в двух местах — верный способ получить два расходящихся
//! формата.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::{anyhow, Result};

/// Длина nonce для AES-GCM.
pub const NONCE_LEN: usize = 12;

fn cipher(key: &[u8; 32]) -> Aes256Gcm {
    // Длина проверена типом: ровно 32 байта, поэтому преобразование не может
    // не сойтись.
    Aes256Gcm::new(&Key::<Aes256Gcm>::from(*key))
}

fn to_nonce(bytes: &[u8]) -> Result<Nonce<<Aes256Gcm as aes_gcm::AeadCore>::NonceSize>> {
    Nonce::try_from(bytes).map_err(|_| {
        anyhow!(
            "nonce должен быть {NONCE_LEN} байт, получено {}",
            bytes.len()
        )
    })
}

/// Случайный nonce.
pub fn random_nonce() -> Result<[u8; NONCE_LEN]> {
    let mut buf = [0u8; NONCE_LEN];
    getrandom::fill(&mut buf).map_err(|e| anyhow!("нет случайности: {e}"))?;
    Ok(buf)
}

/// Зашифровать. Возвращает `(nonce, ciphertext)`.
pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    // Nonce обязан быть уникальным на каждое шифрование этим ключом:
    // повтор в GCM ломает конфиденциальность. Поэтому случайный, а не счётчик —
    // счётчик пришлось бы где-то хранить и синхронизировать между устройствами.
    let nonce_bytes = random_nonce()?;
    let ct = cipher(key)
        .encrypt(&to_nonce(&nonce_bytes)?, plaintext)
        .map_err(|_| anyhow!("не удалось зашифровать"))?;
    Ok((nonce_bytes.to_vec(), ct))
}

pub fn decrypt(key: &[u8; 32], nonce: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>> {
    cipher(key)
        .decrypt(&to_nonce(nonce)?, ciphertext)
        // Единственная причина отказа, которую можно показать наружу, —
        // «не сходится». Отличать «не тот ключ» от «данные испорчены» нельзя:
        // в GCM это одна и та же проверка тега.
        .map_err(|_| anyhow!("не удалось расшифровать: неверный ключ или повреждённые данные"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_returns_the_same_bytes() {
        let key = [7u8; 32];
        let (nonce, ct) = encrypt(&key, b"hello").expect("шифрование");
        assert_eq!(decrypt(&key, &nonce, &ct).expect("расшифровка"), b"hello");
    }

    #[test]
    fn wrong_key_is_rejected() {
        let (nonce, ct) = encrypt(&[1u8; 32], b"secret").expect("шифрование");
        assert!(decrypt(&[2u8; 32], &nonce, &ct).is_err());
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        // Тег аутентификации существует именно для этого: подмена не должна
        // проходить молча.
        let key = [3u8; 32];
        let (nonce, mut ct) = encrypt(&key, b"payload").expect("шифрование");
        ct[0] ^= 0xff;
        assert!(decrypt(&key, &nonce, &ct).is_err());
    }

    #[test]
    fn nonce_is_never_repeated() {
        // Повтор nonce в GCM ломает конфиденциальность — проверяем, что
        // случайность действительно случайна.
        let key = [4u8; 32];
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            let (nonce, _) = encrypt(&key, b"x").expect("шифрование");
            assert!(seen.insert(nonce), "nonce повторился");
        }
    }

    #[test]
    fn wrong_nonce_length_is_refused() {
        let key = [5u8; 32];
        let (_, ct) = encrypt(&key, b"x").expect("шифрование");
        assert!(decrypt(&key, &[0u8; 8], &ct).is_err(), "короткий nonce");
        assert!(decrypt(&key, &[0u8; 16], &ct).is_err(), "длинный nonce");
    }

    #[test]
    fn binary_payloads_survive_intact() {
        // Данные синхронизации — произвольные байты, а не текст. Урок из
        // веб-части: ровные данные прячут то, что ломается на настоящих.
        let key = [6u8; 32];
        let payload: Vec<u8> = (0..=255u8).chain(0..=255u8).collect();
        let (nonce, ct) = encrypt(&key, &payload).expect("шифрование");
        assert_eq!(decrypt(&key, &nonce, &ct).expect("расшифровка"), payload);
    }
}
