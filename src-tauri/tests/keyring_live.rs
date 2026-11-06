//! Секреты обязаны переживать перезагрузку (2026-08-28).
//!
//! Проверка живая и потому помечена `#[ignore]`: ей нужен работающий Secret
//! Service (KWallet, GNOME Keyring), которого нет ни в контейнере сборки, ни
//! на сервере. Запуск руками:
//!
//! ```
//! cargo test --test keyring_live -- --ignored
//! ```
//!
//! Зачем она вообще нужна. Приложение собиралось с `keyring/linux-native` —
//! это память ядра (keyutils). Запись и чтение работали, тесты бы прошли, и
//! ровно поэтому обычный тест «записал — прочитал» здесь бесполезен: он
//! зелёный в обоих случаях. Разница видна только снаружи процесса: секрет,
//! лежащий в keyutils, не виден системному хранилищу и исчезает при
//! перезагрузке. Человек при этом видит «no password stored for identity N» и
//! думает, что не сохранил пароль.
//!
//! Поэтому проверяем не «читается ли», а «видит ли его система».

use std::process::Command;

const SERVICE: &str = "darwaza";

#[test]
#[ignore = "нужен работающий Secret Service (KWallet/GNOME Keyring)"]
fn secret_is_visible_to_the_system_store_not_only_to_us() {
    let account = "password::999999";
    let secret = "проверка-хранилища";

    let entry = keyring::Entry::new(SERVICE, account).expect("запись keyring");
    entry.set_password(secret).expect("сохранение секрета");

    // Своим же кодом — обязано читаться.
    assert_eq!(entry.get_password().expect("чтение"), secret);

    // А теперь снаружи, системным инструментом. Именно здесь и падает
    // keyutils: для него запись существует только внутри процесса и сеанса.
    let output = Command::new("secret-tool")
        .args(["lookup", "service", SERVICE, "username", account])
        .output()
        .expect("secret-tool должен быть установлен для этой проверки");

    let seen = String::from_utf8_lossy(&output.stdout);
    entry.delete_credential().expect("уборка за собой");

    assert_eq!(
        seen.trim(),
        secret,
        "секрет не виден системному хранилищу — значит он в памяти ядра и не переживёт перезагрузку"
    );
}

/// Проверка доступности хранилища обязана отвечать «есть», когда оно есть
/// (#169).
///
/// Живая и потому `#[ignore]`: рабочий Secret Service нужен по определению —
/// проверяется именно он.
///
/// ```bash
/// cargo test --test keyring_live -- --ignored
/// ```
#[test]
#[ignore = "нужен работающий Secret Service (KWallet/GNOME Keyring)"]
fn availability_is_true_when_the_store_answers() {
    assert!(
        darwaza_lib::keyring_store::is_available(),
        "Secret Service запущен, но проверка доступности говорит обратное — \
         значит человеку предложат мастер-пароль там, где он не нужен"
    );
}

/// Проверка не должна ничего записывать.
///
/// Это не педантизм. Запись в KWallet поднимает окно с запросом доступа, а
/// проверка вызывается при **открытии формы**: человек, начав заводить хост,
/// получал бы всплывающее окно ни с того ни с сего.
#[test]
#[ignore = "нужен работающий Secret Service (KWallet/GNOME Keyring)"]
fn availability_check_writes_nothing() {
    darwaza_lib::keyring_store::is_available();

    let probe = keyring::Entry::new(SERVICE, "probe::0").expect("запись keyring");
    assert!(
        matches!(probe.get_password(), Err(keyring::Error::NoEntry)),
        "проверка доступности оставила запись в хранилище — она обязана только читать"
    );
}

/// Запись, сохранённая 0.1.0 под прежним именем службы, находится и
/// переезжает на новое имя.
#[test]
#[ignore = "нужен работающий Secret Service (KWallet/GNOME Keyring)"]
fn a_secret_under_the_pre_0_1_1_service_name_moves_to_the_new_one() {
    // Прежнее имя службы нужно, чтобы создать запись «как у 0.1.0».
    const LEGACY: &str = "termius-analog"; // LEGACY: имя службы 0.1.0
    let account = "password::999998";
    let old = keyring::Entry::new(LEGACY, account).expect("запись keyring");
    old.set_password("старый-секрет").expect("сохранение");

    let got = darwaza_lib::keyring_store::get("password", 999998).expect("чтение");
    assert_eq!(got.as_deref(), Some("старый-секрет"));
    assert!(old.get_password().is_err(), "старая запись должна исчезнуть");

    darwaza_lib::keyring_store::delete("password", 999998).expect("уборка");
}
