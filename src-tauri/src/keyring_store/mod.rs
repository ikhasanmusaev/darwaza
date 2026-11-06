// Обёртка над системным keyring (libsecret на Linux).
// См. docs/01-architecture/security.md
//
// Имя службы в системном keyring.
// Account format: "{kind}::{id}" (например "password::42", "passphrase::7")

use keyring::Entry;

/// Имя службы в системном keyring.
///
/// **Намеренно осталось прежним после переименования продукта в Darwaza.**
/// Секреты в keyring адресуются парой (служба, учётная запись): сменив имя
/// службы, приложение перестало бы находить сохранённые пароли и passphrase —
/// для человека это выглядит как «все пароли пропали», хотя они на месте.
///
/// Переименовать можно, но только вместе с переносом: перечислить identity из
/// базы, прочитать каждый секрет под старым именем, записать под новым,
/// удалить старый. Пока выгода чисто косметическая — имя видно лишь в
/// менеджерах ключей, — этот риск не окупается.
const SERVICE: &str = "termius-analog";

fn account(kind: &str, id: i64) -> String {
    format!("{kind}::{id}")
}

fn entry(kind: &str, id: i64) -> Result<Entry, String> {
    Entry::new(SERVICE, &account(kind, id)).map_err(|e| format!("keyring init failed: {e}"))
}

/// Сохранить секрет. Перезаписывает существующий.
pub fn set(kind: &str, id: i64, secret: &str) -> Result<(), String> {
    entry(kind, id)?
        .set_password(secret)
        .map_err(|e| format!("keyring set failed: {e}"))
}

/// Прочитать секрет. `Ok(None)` если записи нет.
pub fn get(kind: &str, id: i64) -> Result<Option<String>, String> {
    match entry(kind, id)?.get_password() {
        Ok(s) => Ok(Some(s)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("keyring get failed: {e}")),
    }
}

/// Удалить секрет. Молча возвращает Ok даже если записи не было.
pub fn delete(kind: &str, id: i64) -> Result<(), String> {
    match entry(kind, id)?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("keyring delete failed: {e}")),
    }
}

pub mod kinds {
    pub const PASSWORD: &str = "password";
    pub const PASSPHRASE: &str = "passphrase";
}

/// Есть ли на этой машине рабочее системное хранилище (#169).
///
/// Спрашиваем **до** того, как человек начал набирать пароль. Раньше он
/// заполнял форму целиком и получал техническую ошибку в конце — а сделать с
/// ней ничего не мог, потому что альтернативу ему никто не предлагал.
///
/// Проверка **читающая**: берём заведомо отсутствующую запись. Если хранилище
/// работает, оно ответит «нет такой записи» — этого достаточно. Писать пробную
/// запись нельзя: на KWallet это всплывающее окно с запросом доступа, и
/// показывать его при открытии формы недопустимо.
pub fn is_available() -> bool {
    match entry("probe", 0) {
        // «Записи нет» — значит хранилище ответило. Это успех.
        Ok(e) => !matches!(
            e.get_password(),
            Err(keyring::Error::NoStorageAccess(_)) | Err(keyring::Error::PlatformFailure(_))
        ),
        Err(_) => false,
    }
}
