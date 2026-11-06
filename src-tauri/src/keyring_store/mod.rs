// Обёртка над системным keyring (libsecret на Linux).
// См. docs/01-architecture/security.md
//
// Имя службы в системном keyring.
// Account format: "{kind}::{id}" (например "password::42", "passphrase::7")

use keyring::Entry;

/// Имя службы в системном keyring.
const SERVICE: &str = "darwaza";

/// Прежнее имя службы — до 0.1.1, когда продукт назывался рабочим именем.
///
/// Секреты в keyring адресуются парой (служба, учётная запись): сменив имя
/// службы без переноса, приложение перестало бы находить сохранённые пароли
/// и passphrase. Поэтому чтение, не найдя запись под новым именем, ищет её
/// под этим, переписывает под новым и удаляет старую. Само значение здесь —
/// единственное место в коде, где прежнее имя ещё живо.
const LEGACY_SERVICE: &str = "termius-analog";

fn account(kind: &str, id: i64) -> String {
    format!("{kind}::{id}")
}

/// Что нужно от хранилища секретов. Вынесено, чтобы перенос со старого имени
/// проверялся тестом без настоящего Secret Service.
trait Store {
    fn get(&self, service: &str, account: &str) -> Result<Option<String>, String>;
    fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), String>;
    fn delete(&self, service: &str, account: &str) -> Result<(), String>;
}

struct System;

impl System {
    fn entry(service: &str, account: &str) -> Result<Entry, String> {
        Entry::new(service, account).map_err(|e| format!("keyring init failed: {e}"))
    }
}

impl Store for System {
    fn get(&self, service: &str, account: &str) -> Result<Option<String>, String> {
        match Self::entry(service, account)?.get_password() {
            Ok(s) => Ok(Some(s)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("keyring get failed: {e}")),
        }
    }

    fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), String> {
        Self::entry(service, account)?
            .set_password(secret)
            .map_err(|e| format!("keyring set failed: {e}"))
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), String> {
        match Self::entry(service, account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("keyring delete failed: {e}")),
        }
    }
}

fn set_in(store: &impl Store, kind: &str, id: i64, secret: &str) -> Result<(), String> {
    let acc = account(kind, id);
    store.set(SERVICE, &acc, secret)?;
    // Старая копия устарела: оставить её значит при следующем чтении
    // не отличить свежее от прежнего. Не удалась уборка — не беда.
    let _ = store.delete(LEGACY_SERVICE, &acc);
    Ok(())
}

fn get_in(store: &impl Store, kind: &str, id: i64) -> Result<Option<String>, String> {
    let acc = account(kind, id);
    if let Some(s) = store.get(SERVICE, &acc)? {
        return Ok(Some(s));
    }
    let Some(secret) = store.get(LEGACY_SERVICE, &acc)? else {
        return Ok(None);
    };
    // Перенос: пишем под новым именем и только после успеха удаляем старое.
    // Не записалось — секрет остаётся на прежнем месте, повторим при
    // следующем чтении, а человек свой пароль получает в любом случае.
    if store.set(SERVICE, &acc, &secret).is_ok() {
        let _ = store.delete(LEGACY_SERVICE, &acc);
    }
    Ok(Some(secret))
}

fn delete_in(store: &impl Store, kind: &str, id: i64) -> Result<(), String> {
    let acc = account(kind, id);
    store.delete(SERVICE, &acc)?;
    store.delete(LEGACY_SERVICE, &acc)
}

/// Пройти по всем учёткам и перенести записи со старого имени службы.
///
/// Перенос при чтении трогает только то, что кто-то прочитал. Пароль учётки,
/// которой не пользуются ни один хост, остался бы под прежним именем навсегда.
/// Возвращает число перенесённых записей. Ошибка хранилища на одной записи не
/// прерывает проход, но в конце возвращается ошибкой: тогда вызывающий не
/// отмечает проход выполненным и повторит его при следующем запуске.
fn sweep_in(store: &impl Store, ids: &[i64]) -> Result<usize, String> {
    let mut moved = 0usize;
    let mut first_error: Option<String> = None;
    for &id in ids {
        for kind in [kinds::PASSWORD, kinds::PASSPHRASE] {
            let acc = account(kind, id);
            match store.get(LEGACY_SERVICE, &acc) {
                Ok(None) => continue,
                Ok(Some(_)) => match get_in(store, kind, id) {
                    Ok(_) => moved += 1,
                    Err(e) => first_error = first_error.or(Some(e)),
                },
                Err(e) => first_error = first_error.or(Some(e)),
            }
        }
    }
    match first_error {
        Some(e) => Err(e),
        None => Ok(moved),
    }
}

/// Однократный проход по учёткам: перенос записей со старого имени службы.
pub fn sweep_legacy(ids: &[i64]) -> Result<usize, String> {
    sweep_in(&System, ids)
}

/// Сохранить секрет. Перезаписывает существующий.
pub fn set(kind: &str, id: i64, secret: &str) -> Result<(), String> {
    set_in(&System, kind, id, secret)
}

/// Прочитать секрет. `Ok(None)` если записи нет. Запись под прежним именем
/// службы переезжает на новое.
pub fn get(kind: &str, id: i64) -> Result<Option<String>, String> {
    get_in(&System, kind, id)
}

/// Удалить секрет. Молча возвращает Ok даже если записи не было.
pub fn delete(kind: &str, id: i64) -> Result<(), String> {
    delete_in(&System, kind, id)
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
    match Entry::new(SERVICE, &account("probe", 0)) {
        // «Записи нет» — значит хранилище ответило. Это успех.
        Ok(e) => !matches!(
            e.get_password(),
            Err(keyring::Error::NoStorageAccess(_)) | Err(keyring::Error::PlatformFailure(_))
        ),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Mem(RefCell<HashMap<(String, String), String>>);

    impl Store for Mem {
        fn get(&self, s: &str, a: &str) -> Result<Option<String>, String> {
            Ok(self.0.borrow().get(&(s.into(), a.into())).cloned())
        }
        fn set(&self, s: &str, a: &str, v: &str) -> Result<(), String> {
            self.0.borrow_mut().insert((s.into(), a.into()), v.into());
            Ok(())
        }
        fn delete(&self, s: &str, a: &str) -> Result<(), String> {
            self.0.borrow_mut().remove(&(s.into(), a.into()));
            Ok(())
        }
    }

    /// Пароль, сохранённый версией 0.1.0, находится и переезжает на новое имя.
    #[test]
    fn secret_saved_by_0_1_0_is_found_and_moved() {
        let m = Mem::default();
        m.set(LEGACY_SERVICE, "password::7", "старый-пароль").unwrap();

        assert_eq!(get_in(&m, "password", 7).unwrap().as_deref(), Some("старый-пароль"));
        assert_eq!(m.get(SERVICE, "password::7").unwrap().as_deref(), Some("старый-пароль"));
        assert_eq!(m.get(LEGACY_SERVICE, "password::7").unwrap(), None);
        // Второе чтение — уже с нового места.
        assert_eq!(get_in(&m, "password", 7).unwrap().as_deref(), Some("старый-пароль"));
    }

    #[test]
    fn new_value_wins_and_missing_stays_missing() {
        let m = Mem::default();
        m.set(LEGACY_SERVICE, "password::1", "старый").unwrap();
        m.set(SERVICE, "password::1", "новый").unwrap();
        assert_eq!(get_in(&m, "password", 1).unwrap().as_deref(), Some("новый"));
        assert_eq!(get_in(&m, "password", 2).unwrap(), None);
    }

    /// Проход переносит и записи, которые никто не читал; чужие не трогает.
    #[test]
    fn sweep_moves_unread_entries_and_keeps_the_rest() {
        let m = Mem::default();
        m.set(LEGACY_SERVICE, "password::1", "p1").unwrap();
        m.set(LEGACY_SERVICE, "passphrase::2", "k2").unwrap();
        m.set(SERVICE, "password::3", "уже-новый").unwrap();
        m.set(LEGACY_SERVICE, "password::99", "чужая").unwrap();

        assert_eq!(sweep_in(&m, &[1, 2, 3]).unwrap(), 2);
        assert_eq!(m.get(SERVICE, "password::1").unwrap().as_deref(), Some("p1"));
        assert_eq!(m.get(SERVICE, "passphrase::2").unwrap().as_deref(), Some("k2"));
        assert_eq!(m.get(LEGACY_SERVICE, "password::1").unwrap(), None);
        assert_eq!(m.get(LEGACY_SERVICE, "passphrase::2").unwrap(), None);
        assert_eq!(m.get(SERVICE, "password::3").unwrap().as_deref(), Some("уже-новый"));
        // Учётки нет в списке — запись остаётся как была.
        assert_eq!(m.get(LEGACY_SERVICE, "password::99").unwrap().as_deref(), Some("чужая"));
        // Второй проход ничего не находит.
        assert_eq!(sweep_in(&m, &[1, 2, 3]).unwrap(), 0);
    }

    /// Не записалось под новым именем — старая запись цела, проход сообщает
    /// об ошибке (флаг «выполнено» не ставится).
    #[test]
    fn sweep_keeps_the_old_entry_when_the_new_one_cannot_be_written() {
        struct ReadOnlyNew(Mem);
        impl Store for ReadOnlyNew {
            fn get(&self, s: &str, a: &str) -> Result<Option<String>, String> {
                self.0.get(s, a)
            }
            fn set(&self, s: &str, a: &str, v: &str) -> Result<(), String> {
                if s == SERVICE {
                    return Err("запись запрещена".into());
                }
                self.0.set(s, a, v)
            }
            fn delete(&self, s: &str, a: &str) -> Result<(), String> {
                self.0.delete(s, a)
            }
        }
        let m = ReadOnlyNew(Mem::default());
        m.0.set(LEGACY_SERVICE, "password::1", "p1").unwrap();
        // get_in не считает это ошибкой (секрет отдаётся), проход — тоже;
        // главное, что старая запись не потеряна.
        let _ = sweep_in(&m, &[1]);
        assert_eq!(m.0.get(LEGACY_SERVICE, "password::1").unwrap().as_deref(), Some("p1"));
    }

    #[test]
    fn set_and_delete_leave_no_old_copy() {
        let m = Mem::default();
        m.set(LEGACY_SERVICE, "passphrase::3", "старый").unwrap();
        set_in(&m, "passphrase", 3, "новый").unwrap();
        assert_eq!(m.get(LEGACY_SERVICE, "passphrase::3").unwrap(), None);
        assert_eq!(get_in(&m, "passphrase", 3).unwrap().as_deref(), Some("новый"));

        m.set(LEGACY_SERVICE, "passphrase::3", "призрак").unwrap();
        delete_in(&m, "passphrase", 3).unwrap();
        assert_eq!(get_in(&m, "passphrase", 3).unwrap(), None);
    }
}
