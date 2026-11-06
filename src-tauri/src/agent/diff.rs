//! Показ разницы при правке файла агентом (#206).
//!
//! Агент не выполняет `sed -i`, а присылает **новое содержимое файла**.
//! Приложение показывает человеку, что именно меняется.
//!
//! Разница принципиальная: `sed -i 's/80/443/g'` выглядит безобидно, пока не
//! окажется, что в файле было ещё три восьмидесятых. Строки «было/стало»
//! читаются глазами и не требуют мысленно выполнять команду.

use serde::Serialize;

/// Одна строка в показе разницы.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DiffLine {
    /// Без изменений — нужна для понимания места.
    Context {
        text: String,
    },
    Removed {
        text: String,
    },
    Added {
        text: String,
    },
}

/// Сколько неизменных строк показывать вокруг правки.
const CONTEXT: usize = 3;

/// Построить показ разницы двух версий файла.
///
/// Намеренно простой построчный алгоритм: общее начало, общий конец,
/// изменённая середина. Настоящий алгоритм различий дал бы более аккуратный
/// результат на перестановках, но здесь важнее, чтобы человек **доверял**
/// показанному, а не чтобы оно было минимальным. Правка конфига — это почти
/// всегда несколько строк подряд.
pub fn diff(before: &str, after: &str) -> Vec<DiffLine> {
    let old: Vec<&str> = before.lines().collect();
    let new: Vec<&str> = after.lines().collect();

    // Общее начало.
    let mut head = 0;
    while head < old.len() && head < new.len() && old[head] == new[head] {
        head += 1;
    }

    // Общий конец, не залезая в уже совпавшее начало.
    let mut tail = 0;
    while tail < old.len() - head.min(old.len())
        && tail < new.len() - head.min(new.len())
        && old[old.len() - 1 - tail] == new[new.len() - 1 - tail]
    {
        tail += 1;
    }

    let mut lines = Vec::new();

    let context_start = head.saturating_sub(CONTEXT);
    for line in old.iter().take(head).skip(context_start) {
        lines.push(DiffLine::Context {
            text: (*line).to_string(),
        });
    }

    for line in old.iter().take(old.len() - tail).skip(head) {
        lines.push(DiffLine::Removed {
            text: (*line).to_string(),
        });
    }
    for line in new.iter().take(new.len() - tail).skip(head) {
        lines.push(DiffLine::Added {
            text: (*line).to_string(),
        });
    }

    for line in old.iter().skip(old.len() - tail).take(CONTEXT) {
        lines.push(DiffLine::Context {
            text: (*line).to_string(),
        });
    }

    lines
}

/// Есть ли вообще изменения.
///
/// Отдельная проверка: показывать пустой список и просить подтвердить
/// «ничего» — верный способ приучить нажимать «да» не глядя.
pub fn has_changes(lines: &[DiffLine]) -> bool {
    lines
        .iter()
        .any(|l| matches!(l, DiffLine::Added { .. } | DiffLine::Removed { .. }))
}

/// Команда проверки конфигурации перед перезапуском службы.
///
/// Приложение добавляет то, о чём агент забудет: сломанный конфиг не должен
/// доехать до перезапуска. Возвращаем `None`, если проверять нечем — тогда
/// перезапуск идёт как есть, но об этом стоит сказать человеку.
pub fn validation_command(path: &str) -> Option<&'static str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    if path.contains("/nginx/") || name == "nginx.conf" {
        return Some("nginx -t");
    }
    if path.contains("/ssh/sshd_config") || name == "sshd_config" {
        return Some("sshd -t");
    }
    if name.ends_with(".service") || path.contains("/systemd/") {
        return Some("systemd-analyze verify");
    }
    None
}

/// Чем проверяется конфигурация службы перед её перезапуском (#215).
///
/// Отдельно от `validation_command`: там вход — путь файла, здесь — имя
/// службы. Совпадение неполное намеренно: `nginx.service` проверяется
/// `nginx -t`, но `nginx -t` проверяет не файл, а всю конфигурацию целиком.
///
/// `None` — проверять нечем. Это не ошибка: у большинства служб проверки
/// конфигурации просто нет. Но сказать об этом человеку в плане надо, иначе
/// он решит, что проверка была.
pub fn service_check(unit: &str) -> Option<&'static str> {
    // `nginx.service` и `nginx` — одно и то же.
    let name = unit.strip_suffix(".service").unwrap_or(unit);
    match name {
        "nginx" => Some("nginx -t"),
        "sshd" | "ssh" => Some("sshd -t"),
        "apache2" | "httpd" => Some("apachectl configtest"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn added(lines: &[DiffLine]) -> Vec<&str> {
        lines
            .iter()
            .filter_map(|l| match l {
                DiffLine::Added { text } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    fn removed(lines: &[DiffLine]) -> Vec<&str> {
        lines
            .iter()
            .filter_map(|l| match l {
                DiffLine::Removed { text } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn shows_exactly_what_changed() {
        let before = "server {\n    listen 80;\n    root /srv;\n}";
        let after = "server {\n    listen 443 ssl;\n    root /srv;\n}";
        let lines = diff(before, after);

        assert_eq!(removed(&lines), ["    listen 80;"]);
        assert_eq!(added(&lines), ["    listen 443 ssl;"]);
        assert!(has_changes(&lines));
    }

    #[test]
    fn identical_files_produce_no_changes() {
        // Просить подтвердить «ничего» — верный способ приучить нажимать «да»
        // не глядя.
        let text = "a\nb\nc";
        let lines = diff(text, text);
        assert!(!has_changes(&lines), "изменений быть не должно: {lines:?}");
    }

    #[test]
    fn addition_at_the_end_is_visible() {
        let lines = diff("a\nb", "a\nb\nc");
        assert_eq!(added(&lines), ["c"]);
        assert!(removed(&lines).is_empty());
    }

    #[test]
    fn deletion_is_visible() {
        let lines = diff("a\nb\nc", "a\nc");
        assert_eq!(removed(&lines), ["b"]);
        assert!(added(&lines).is_empty());
    }

    #[test]
    fn context_lines_surround_the_change() {
        // Без окружения человек не понимает, ГДЕ меняется, и подтверждает
        // вслепую.
        let before = (1..=10)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let after = before.replace("\n5\n", "\nпять\n");
        let lines = diff(&before, &after);

        let context: Vec<&str> = lines
            .iter()
            .filter_map(|l| match l {
                DiffLine::Context { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(context.contains(&"4"), "строка до правки должна быть видна");
        assert!(
            context.contains(&"6"),
            "строка после правки должна быть видна"
        );
    }

    #[test]
    fn config_gets_its_validation_command() {
        // То, о чём агент забудет: сломанный конфиг не должен доехать до
        // перезапуска службы.
        assert_eq!(
            validation_command("/etc/nginx/nginx.conf"),
            Some("nginx -t")
        );
        assert_eq!(
            validation_command("/etc/nginx/sites-enabled/site"),
            Some("nginx -t")
        );
        assert_eq!(validation_command("/etc/ssh/sshd_config"), Some("sshd -t"));
        assert_eq!(
            validation_command("/srv/app/config.yml"),
            None,
            "проверять нечем"
        );
    }

    #[test]
    fn empty_file_becoming_non_empty_is_handled() {
        let lines = diff("", "первая строка");
        assert_eq!(added(&lines), ["первая строка"]);
    }
}

#[cfg(test)]
mod service_check_tests {
    use super::*;

    #[test]
    fn known_services_have_a_configuration_check() {
        assert_eq!(service_check("nginx"), Some("nginx -t"));
        // Имя с суффиксом и без — одна и та же служба.
        assert_eq!(service_check("nginx.service"), Some("nginx -t"));
        assert_eq!(service_check("sshd"), Some("sshd -t"));
        assert_eq!(service_check("ssh"), Some("sshd -t"));
    }

    /// Проверять нечем — это не ошибка, а то, что надо сказать вслух.
    #[test]
    fn an_unknown_service_has_no_check() {
        assert_eq!(service_check("my-app"), None);
        assert_eq!(service_check("postgresql"), None);
    }

    /// Похожее имя — не та же служба: `nginx-exporter` перезапускают отдельно,
    /// и `nginx -t` про него ничего не знает.
    #[test]
    fn a_similar_name_is_not_the_same_service() {
        assert_eq!(service_check("nginx-exporter"), None);
        assert_eq!(service_check("sshd-keygen"), None);
    }
}
