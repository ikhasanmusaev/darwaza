//! Ошибки, которые видит человек (#177).
//!
//! Наружу из команд уходит **код**, а не текст: `errors.duplicateName`, а не
//! «UNIQUE constraint failed: hosts.name». Перевод лежит в локалях рядом со
//! всеми остальными надписями, потому что языков шесть, и текст, написанный
//! в Rust, был бы русским во всех шести.
//!
//! Полная техническая причина при этом не теряется — она уходит в журнал,
//! где нужна разработчику. Человеку она не нужна никогда: 29 августа он
//! увидел «14 values for 15 columns» и не смог понять из этого ничего, кроме
//! того, что приложение сломалось.
//!
//! Правило: **ни один текст ошибки от базы, SSH или файловой системы не
//! попадает в интерфейс дословно.** Всё проходит через этот модуль.

use std::fmt::Display;

/// Коды, которые понимает интерфейс. Каждому соответствует ключ в локалях.
pub mod codes {
    /// Сказать нечего — подробности в журнале.
    pub const UNEXPECTED: &str = "errors.unexpected";
    /// Не удалось сохранить.
    pub const SAVE_FAILED: &str = "errors.saveFailed";
    /// Не удалось прочитать.
    pub const LOAD_FAILED: &str = "errors.loadFailed";
    /// Такая запись уже есть.
    ///
    /// Достижимо по-настоящему: `known_hosts` уникальны по паре
    /// (адрес, тип ключа), и импорт чужого `known_hosts` в это упирается.
    pub const DUPLICATE: &str = "errors.duplicate";
    /// Ссылка ведёт на запись, которой больше нет.
    pub const MISSING_REF: &str = "errors.missingRef";
    /// Запись, которую открыли или правят, уже удалена.
    pub const NOT_FOUND: &str = "errors.notFound";
    /// Порт вне допустимого диапазона.
    pub const INVALID_PORT: &str = "errors.invalidPort";
    /// Адрес хоста пуст (после обрезки пробелов) или содержит пробел внутри.
    pub const INVALID_HOSTNAME: &str = "errors.invalidHostname";
    /// Имя пользователя пусто (после обрезки пробелов) или содержит пробел внутри.
    pub const INVALID_USERNAME: &str = "errors.invalidUsername";
    /// Название хоста пусто после обрезки пробелов.
    pub const NAME_REQUIRED: &str = "errors.nameRequired";
    /// Интервал keep-alive вне диапазона.
    pub const INVALID_KEEP_ALIVE: &str = "errors.invalidKeepAlive";
    /// Ключ настройки не из списка разрешённых для записи из окна.
    pub const SETTING_NOT_ALLOWED: &str = "errors.settingNotAllowed";
    /// Пробросу нужен адрес назначения.
    pub const REMOTE_HOST_REQUIRED: &str = "errors.remoteHostRequired";
    /// Пробросу нужен порт назначения.
    pub const REMOTE_PORT_REQUIRED: &str = "errors.remotePortRequired";

    // ── Мастер-пароль ────────────────────────────────────────────────────
    //
    // У него свои коды, потому что почти всё здесь человек исправляет сам:
    // неверный пароль, забытый код, пауза после неудач. Общее «что-то пошло
    // не так» на месте «неверного пароля» — худшее, что можно показать.
    pub const MASTER_WRONG_PASSWORD: &str = "errors.masterWrongPassword";
    pub const MASTER_LOCKED_OUT: &str = "errors.masterLockedOut";
    pub const MASTER_TOTP_REQUIRED: &str = "errors.masterTotpRequired";
    pub const MASTER_TOTP_WRONG: &str = "errors.masterTotpWrong";
    pub const MASTER_NOT_ENABLED: &str = "errors.masterNotEnabled";
    pub const MASTER_ALREADY_ENABLED: &str = "errors.masterAlreadyEnabled";
    pub const MASTER_EMPTY_PASSWORD: &str = "errors.masterEmptyPassword";
    /// Новый мастер-пароль короче `master::MIN_PASSWORD_CHARS`.
    pub const MASTER_PASSWORD_TOO_SHORT: &str = "errors.masterPasswordTooShort";
    /// Ключ восстановления не подошёл (#180b). Засчитывается как попытка.
    pub const MASTER_RECOVERY_WRONG: &str = "errors.masterRecoveryWrong";
    /// Ключ набран не полностью или со знаком не из алфавита. Не
    /// засчитывается: опечатка в длине — не подбор.
    pub const MASTER_RECOVERY_MALFORMED: &str = "errors.masterRecoveryMalformed";
    /// Ключ восстановления не выпускали — восстанавливать нечем.
    pub const MASTER_RECOVERY_NOT_SET: &str = "errors.masterRecoveryNotSet";

    /// Файл для сохранения ключа уже существует. Затирать его нельзя: человек
    /// мог выбрать чужой файл.
    pub const FILE_EXISTS: &str = "errors.fileExists";

    /// Адрес не похож на адрес: узнать об опечатке при первой синхронизации —
    /// худший момент.
    pub const INVALID_URL: &str = "errors.invalidUrl";

    /// Обёртка ключа записей с сервера не открывается ключом из пароля
    /// (#180c). Отказываем, а не берём ключ из пароля втихую: тихая подмена
    /// ключа означала бы расхождение с остальными устройствами.
    pub const SYNC_PROTECTED_KEY_UNREADABLE: &str = "errors.syncProtectedKeyUnreadable";

    /// Действие требует входа в аккаунт, а его нет.
    pub const SYNC_NOT_SIGNED_IN: &str = "errors.syncNotSignedIn";

    // ── Подключение по SSH (#178) ────────────────────────────────────────
    //
    // Отдельный словарь, потому что здесь почти каждая ошибка — про внешний
    // мир, и человек может с ней что-то сделать: включить сервер, поправить
    // порт, ввести пароль. Общее «что-то пошло не так» на месте «сервер не
    // отвечает» оставляет его без единой зацепки.
    pub const SSH_UNREACHABLE: &str = "errors.sshUnreachable";
    pub const SSH_REFUSED: &str = "errors.sshRefused";
    pub const SSH_DNS_FAILED: &str = "errors.sshDnsFailed";
    pub const SSH_TIMED_OUT: &str = "errors.sshTimedOut";
    pub const SSH_HANDSHAKE_FAILED: &str = "errors.sshHandshakeFailed";
    pub const SSH_AUTH_FAILED: &str = "errors.sshAuthFailed";
    pub const SSH_NO_PASSWORD: &str = "errors.sshNoPassword";
    pub const SSH_NO_PASSPHRASE: &str = "errors.sshNoPassphrase";
    pub const SSH_NO_KEY_PATH: &str = "errors.sshNoKeyPath";
    pub const SSH_KEY_UNREADABLE: &str = "errors.sshKeyUnreadable";
    pub const SSH_TOO_MANY_ROUNDS: &str = "errors.sshTooManyRounds";
    pub const SSH_COMMAND_TIMEOUT: &str = "errors.sshCommandTimeout";
    pub const SSH_JUMP_LOOP: &str = "errors.sshJumpLoop";
    pub const SSH_JUMP_TOO_DEEP: &str = "errors.sshJumpTooDeep";
    pub const SSH_TUNNEL_FAILED: &str = "errors.sshTunnelFailed";
    /// Значение из закрытого списка, которого в списке нет. Через интерфейс
    /// недостижимо — значит, это наша ошибка, а не человека.
    pub const INVALID_VALUE: &str = "errors.unexpected";
}

/// Записать причину в журнал и вернуть код.
///
/// `context` — где сломалось, в терминах кода («hosts_create»). В интерфейс
/// он не попадает, но без него запись в журнале бесполезна.
fn log_and(context: &str, e: impl Display, code: &'static str) -> String {
    tracing::error!(context, code, error = %e, "команда завершилась ошибкой");
    code.to_string()
}

/// Ошибка, о которой сказать нечего. Годится для всего, что не база.
pub fn opaque(context: &str, e: impl Display) -> String {
    log_and(context, e, codes::UNEXPECTED)
}

/// Ошибка базы при записи.
///
/// Два случая человек может исправить сам, поэтому разбираются отдельно:
/// занятое имя и ссылка на удалённую запись. Всё прочее — наша вина, и
/// сказать про неё нечего, кроме «не сохранилось».
pub fn db_save(context: &str, e: sqlx::Error) -> String {
    let code = classify(&e, codes::SAVE_FAILED);
    log_and(context, e, code)
}

/// Ошибка базы при чтении.
pub fn db_load(context: &str, e: sqlx::Error) -> String {
    let code = classify(&e, codes::LOAD_FAILED);
    log_and(context, e, code)
}

/// Пропустить готовый код наружу, всё остальное — обезличить.
///
/// Нужен там, где ошибку уже описали кодом внутри модуля: например
/// мастер-пароль сам знает, что пароль неверный, и заменять это на «что-то
/// пошло не так» было бы враньём в сторону бесполезности.
pub fn passthrough(context: &str, e: impl Display) -> String {
    let text = e.to_string();
    if text.starts_with("errors.") {
        // Код уже осмысленный — в журнал он идёт как есть.
        tracing::warn!(context, code = %text, "команда отклонена");
        return text;
    }
    opaque(context, text)
}

/// Секрета для идентификации нет — со своим кодом.
///
/// Отдельно от `not_found`, потому что код разный у пароля и ключевой фразы:
/// человеку надо сказать, что именно ввести, а не «чего-то не хватает».
pub fn not_found_secret(context: &str, what: &str, id: i64, code: &'static str) -> String {
    log_and(
        context,
        format!("{what} для identity {id} не сохранён"),
        code,
    )
}

/// Разобрать отказ подключения по SSH (#178).
///
/// Разбор идёт по **типу** ошибки, а не по её тексту: тексты меняются с
/// версией библиотеки и различаются между системами, и проверка подстроки
/// однажды молча перестанет срабатывать — причём именно там, где человеку
/// нужнее всего понятное объяснение.
///
/// Что не разобралось — «не удалось установить соединение», а полная причина
/// уходит в журнал. Врать точностью хуже, чем признать, что мы не знаем.
pub fn ssh_connect(context: &str, e: &russh::Error) -> String {
    use std::io::ErrorKind;

    let code = match e {
        russh::Error::IO(io) => match io.kind() {
            ErrorKind::ConnectionRefused => codes::SSH_REFUSED,
            ErrorKind::TimedOut => codes::SSH_TIMED_OUT,
            ErrorKind::HostUnreachable | ErrorKind::NetworkUnreachable => codes::SSH_UNREACHABLE,
            // Имя не разрешилось. Отдельного вида ошибки для этого в
            // стандартной библиотеке нет — `getaddrinfo` приходит как
            // «прочее», — поэтому здесь единственное место, где приходится
            // смотреть на текст. Ошибиться не страшно: не угадали — человек
            // получит «не удалось соединиться», а не враньё.
            _ if io.to_string().contains("lookup") => codes::SSH_DNS_FAILED,
            _ => codes::SSH_HANDSHAKE_FAILED,
        },
        // Договориться об алгоритмах не вышло — это не сеть и не пароль.
        russh::Error::NoCommonAlgo { .. }
        | russh::Error::UnknownAlgo
        | russh::Error::Kex
        | russh::Error::KexInit
        | russh::Error::Version => codes::SSH_HANDSHAKE_FAILED,
        russh::Error::NotAuthenticated => codes::SSH_AUTH_FAILED,
        _ => codes::SSH_HANDSHAKE_FAILED,
    };
    log_and(context, e, code)
}

/// Значение, которое человек может исправить сам, — со своим кодом.
pub fn invalid(context: &str, why: impl Display, code: &'static str) -> String {
    log_and(context, why, code)
}

/// Записи нет — её удалили, пока форма была открыта.
///
/// В журнал уходит, о какой именно записи речь: без номера разбирать жалобу
/// невозможно.
pub fn not_found(context: &str, what: &str, id: i64) -> String {
    log_and(context, format!("{what} {id} не найдена"), codes::NOT_FOUND)
}

fn classify(e: &sqlx::Error, fallback: &'static str) -> &'static str {
    match e {
        sqlx::Error::Database(db) if db.is_unique_violation() => codes::DUPLICATE,
        sqlx::Error::Database(db) if db.is_foreign_key_violation() => codes::MISSING_REF,
        _ => fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn fresh() -> sqlx::SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("подключение к памяти");
        crate::db::MIGRATOR.run(&pool).await.expect("миграции");
        pool
    }

    /// Код не должен нести технических подробностей.
    ///
    /// Проверка выглядит наивной, но ловит ровно тот соблазн, ради которого
    /// всё затеяно: «добавлю причину в скобках, вдруг пригодится». Пригождается
    /// она только в журнале.
    #[test]
    fn code_carries_no_technical_text() {
        let code = opaque("test", "UNIQUE constraint failed: hosts.name");
        assert_eq!(code, codes::UNEXPECTED);
        assert!(!code.contains("UNIQUE"));
        assert!(!code.contains("constraint"));
    }

    /// Очищенные модули обязаны оставаться очищенными.
    ///
    /// Сторож, а не придирка: сырую строку возвращают не по злому умыслу, а
    /// потому что так быстрее, когда правишь соседнюю строчку. Тест назовёт
    /// файл раньше, чем это увидит человек.
    ///
    /// `ssh.rs` вошёл в список после #178: у ошибок подключения появился свой
    /// словарь, и возвращаться к сырому тексту там больше незачем.
    #[test]
    fn cleaned_modules_return_codes_not_text() {
        let modules = [
            ("hosts.rs", include_str!("commands/hosts.rs")),
            ("groups.rs", include_str!("commands/groups.rs")),
            ("identities.rs", include_str!("commands/identities.rs")),
            ("snippets.rs", include_str!("commands/snippets.rs")),
            (
                "port_forwards.rs",
                include_str!("commands/port_forwards.rs"),
            ),
            ("ssh.rs", include_str!("commands/ssh.rs")),
        ];

        for (name, source) in modules {
            for leak in ["Err(format!(", "ok_or_else(|| format!("] {
                assert!(
                    !source.contains(leak),
                    "{name} снова отдаёт человеку сырой текст через `{leak}`. \
                     Ошибка должна уходить кодом: crate::errors::*"
                );
            }
        }
    }

    /// Ссылка на удалённую запись — случай, который человек может исправить.
    ///
    /// Достижимо по-настоящему: форма хоста открыта, а идентификацию за это
    /// время удалили в другом окне. Раньше человек получал бы текст про
    /// FOREIGN KEY, из которого следует ровно ничего.
    #[tokio::test]
    async fn a_dead_reference_is_told_apart() {
        let pool = fresh().await;

        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .expect("внешние ключи");

        let e = sqlx::query(
            "INSERT INTO hosts (name, hostname, port, username, auth_method, identity_id, sync_id) \
             VALUES ('т', 'example.org', 22, 'root', 'password', 999999, ?1)",
        )
        .bind(crate::db::new_sync_id())
        .execute(&pool)
        .await
        .expect_err("идентификации 999999 не существует");

        assert_eq!(db_save("test", e), codes::MISSING_REF);
    }

    /// Обычный отказ базы при чтении — «не удалось прочитать», не «дубликат».
    #[tokio::test]
    async fn an_ordinary_failure_keeps_its_own_code() {
        let pool = fresh().await;

        let e = sqlx::query("SELECT * FROM таблицы_такой_нет")
            .execute(&pool)
            .await
            .expect_err("таблицы нет");

        assert_eq!(db_load("test", e), codes::LOAD_FAILED);

        let e = sqlx::query("INSERT INTO таблицы_такой_нет DEFAULT VALUES")
            .execute(&pool)
            .await
            .expect_err("таблицы нет");

        assert_eq!(db_save("test", e), codes::SAVE_FAILED);
    }

    /// «Записи нет» несёт номер в журнал, но не в интерфейс.
    #[test]
    fn not_found_tells_the_log_which_record_but_not_the_person() {
        let code = not_found("test", "хост", 42);
        assert_eq!(code, codes::NOT_FOUND);
        assert!(
            !code.contains("42"),
            "номер записи человеку не нужен: {code}"
        );
    }

    /// Проверки ввода отдают каждая свой код, а не общий.
    ///
    /// Иначе «порт не тот» и «интервал не тот» показали бы одинаковый текст, и
    /// человек не понял бы, какое поле чинить.
    #[test]
    fn each_input_check_has_its_own_code() {
        assert_eq!(
            invalid("test", "порт 0", codes::INVALID_PORT),
            codes::INVALID_PORT
        );
        assert_eq!(
            invalid("test", "keep_alive 99999", codes::INVALID_KEEP_ALIVE),
            codes::INVALID_KEEP_ALIVE
        );
        assert_ne!(codes::INVALID_PORT, codes::INVALID_KEEP_ALIVE);
        assert_ne!(codes::REMOTE_HOST_REQUIRED, codes::REMOTE_PORT_REQUIRED);
    }

    /// Отказ подключения разбирается по типу ошибки, а не по её тексту.
    ///
    /// Проверка нужна потому, что соблазн «поискать подстроку в сообщении»
    /// здесь особенно силён, а расплата отложенная: тексты меняются с версией
    /// библиотеки, и разбор молча перестаёт работать — ровно там, где человеку
    /// нужнее всего понятное объяснение.
    #[test]
    fn a_refused_connection_is_told_from_a_timeout() {
        use std::io::{Error as IoError, ErrorKind};

        let refused = russh::Error::IO(IoError::new(ErrorKind::ConnectionRefused, "refused"));
        assert_eq!(ssh_connect("test", &refused), codes::SSH_REFUSED);

        let timed_out = russh::Error::IO(IoError::new(ErrorKind::TimedOut, "timed out"));
        assert_eq!(ssh_connect("test", &timed_out), codes::SSH_TIMED_OUT);

        // Имя не разрешилось — единственное место, где смотрим на текст:
        // отдельного вида ошибки для этого в стандартной библиотеке нет.
        let dns = russh::Error::IO(IoError::other("failed to lookup address information"));
        assert_eq!(ssh_connect("test", &dns), codes::SSH_DNS_FAILED);

        // Незнакомое не выдаём за известное: «не удалось соединиться» честнее
        // выдуманной точности.
        let other = russh::Error::Inconsistent;
        assert_eq!(ssh_connect("test", &other), codes::SSH_HANDSHAKE_FAILED);
    }

    /// Коды подключения не совпадают между собой.
    ///
    /// Иначе «сервер не отвечает» и «пароль не подошёл» показали бы один
    /// текст, и человек чинил бы не то.
    #[test]
    fn connection_codes_are_all_different() {
        let all = [
            codes::SSH_UNREACHABLE,
            codes::SSH_REFUSED,
            codes::SSH_DNS_FAILED,
            codes::SSH_TIMED_OUT,
            codes::SSH_HANDSHAKE_FAILED,
            codes::SSH_AUTH_FAILED,
            codes::SSH_NO_PASSWORD,
            codes::SSH_NO_PASSPHRASE,
            codes::SSH_NO_KEY_PATH,
            codes::SSH_TOO_MANY_ROUNDS,
            codes::SSH_COMMAND_TIMEOUT,
            codes::SSH_JUMP_LOOP,
            codes::SSH_JUMP_TOO_DEEP,
            codes::SSH_TUNNEL_FAILED,
        ];
        let unique: std::collections::HashSet<_> = all.iter().collect();
        assert_eq!(unique.len(), all.len(), "коды подключения повторяются");
    }

    /// Уже существующую запись человек может исправить сам — говорим отдельно.
    #[tokio::test]
    async fn duplicate_is_told_apart() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("подключение к памяти");
        crate::db::MIGRATOR.run(&pool).await.expect("миграции");

        let insert = "INSERT INTO known_hosts (hostname, key_type, fingerprint, sync_id) \
                      VALUES ('example.org', 'ssh-ed25519', ?1, ?2)";

        sqlx::query(insert)
            .bind("SHA256:aaa")
            .bind("sync-1")
            .execute(&pool)
            .await
            .expect("первая запись");

        // Тот же адрес и тот же тип ключа — уникальный индекс обязан не пустить.
        let e = sqlx::query(insert)
            .bind("SHA256:bbb")
            .bind("sync-2")
            .execute(&pool)
            .await
            .expect_err("вторая запись с той же парой не должна вставиться");

        assert_eq!(db_save("test", e), codes::DUPLICATE);
    }
}
