//! Установленные моноширинные шрифты для настроек терминала (#233).

/// Разобрать вывод `fc-list : family`.
///
/// Строка — одно или несколько семейств через запятую (`DejaVu Sans Mono,DejaVu
/// Sans Mono Book`), а у файла с несколькими начертаниями она повторяется.
/// Берём первое имя в строке, убираем повторы и сортируем без учёта регистра.
fn parse(output: &str) -> Vec<String> {
    let mut families: Vec<String> = output
        .lines()
        .filter_map(|line| line.split(',').next())
        .map(|name| name.trim().to_string())
        // Управляющие символы и пустые имена в выпадающий список не нужны, а
        // имя из чужого файла шрифта — это чужой ввод.
        .filter(|name| !name.is_empty() && !name.chars().any(char::is_control))
        .collect();
    families.sort_by_key(|name| name.to_lowercase());
    families.dedup_by_key(|name| name.to_lowercase());
    families
}

/// Моноширинные шрифты системы.
///
/// Нет `fc-list` или он упал — пустой список, а не ошибка: настройки обязаны
/// открываться и без него, шрифт тогда вводят вручную.
#[tauri::command]
pub async fn system_fonts() -> Vec<String> {
    let output = tokio::process::Command::new("fc-list")
        .args([":spacing=mono", "family"])
        .output()
        .await;
    match output {
        Ok(out) if out.status.success() => parse(&String::from_utf8_lossy(&out.stdout)),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_are_unique_sorted_and_clean() {
        let raw = "JetBrains Mono\nDejaVu Sans Mono,DejaVu Sans Mono Book\njetbrains mono\n\n  \nFira Code\nDejaVu Sans Mono\n";
        assert_eq!(
            parse(raw),
            ["DejaVu Sans Mono", "Fira Code", "JetBrains Mono"]
        );
        assert!(parse("").is_empty());
        assert!(parse("Evil\u{1b}[31m Font\n").is_empty());
    }
}
