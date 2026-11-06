-- Migration #6: зашифрованное хранилище секретов под мастер-паролем (#110)
--
-- Используется вместо системного keyring, когда мастер-пароль включён:
-- секрет шифруется AES-256-GCM ключом, выведенным из пароля через Argon2id.
-- Ключ нигде не хранится — только в памяти процесса после разблокировки.

CREATE TABLE IF NOT EXISTS vault (
    kind        TEXT    NOT NULL,   -- 'password' | 'passphrase' — как в keyring
    ref_id      INTEGER NOT NULL,   -- id identity
    nonce       BLOB    NOT NULL,   -- 96-битный nonce AES-GCM, уникален на каждую запись
    ciphertext  BLOB    NOT NULL,   -- шифртекст вместе с тегом аутентификации
    updated_at  DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (kind, ref_id)
);
