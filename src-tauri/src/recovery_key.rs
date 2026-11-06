//! Ключ восстановления: 160 бит и их запись для человека (#180b, #180c).
//!
//! Отдельным модулем, потому что таких ключей два — от мастер-пароля
//! (`master/mod.rs`) и от пароля аккаунта (`sync/keys.rs`), — а запись у них
//! обязана быть одинаковой. У человека не должно быть двух разных по виду
//! бумажек от одного продукта, и, что важнее, две копии одного алгоритма
//! однажды разойдутся.
//!
//! Эту же запись реализует кабинет на другом языке
//! (`docs/06-web/02-auth-and-crypto.md`). Расхождение там не видно ни в
//! сборке, ни в тестах одной стороны: оно проявляется бумажкой, которая
//! печатается в приложении и не принимается в кабинете — ровно тогда, когда
//! человек уже потерял пароль. Поэтому ниже лежит известный ответ, по
//! которому сверяются обе стороны.

/// Длина ключа: 160 бит.
///
/// Перебор недостижим, а набрать руками 32 знака всё ещё реально.
pub const KEY_BYTES: usize = 20;

/// Алфавит Crockford Base32: без I, L, O и U.
///
/// Выбран ради набора с бумаги: здесь нет букв, которые путаются с цифрами,
/// а при разборе I и L читаются как 1, O — как 0. Человек, переписавший «O»
/// вместо «0», не должен терять доступ к своим данным.
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Новый случайный ключ.
pub fn generate() -> Result<[u8; KEY_BYTES], String> {
    let mut raw = [0u8; KEY_BYTES];
    getrandom::fill(&mut raw).map_err(|e| format!("нет случайности: {e}"))?;
    Ok(raw)
}

/// Записать ключ для бумаги: 32 знака группами по четыре.
///
/// Биты идут **старшими вперёд**, поток непрерывный: байты складываются в
/// буфер, знаки снимаются по пять старших. 160 делится на 5 без остатка —
/// хвоста и набивки не бывает никогда.
pub fn format(bytes: &[u8; KEY_BYTES]) -> String {
    let mut out = String::with_capacity(40);
    let mut buffer: u32 = 0;
    let mut bits = 0;
    let mut written = 0;
    for &byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let index = ((buffer >> bits) & 31) as usize;
            buffer &= (1 << bits) - 1;
            if written > 0 && written % 4 == 0 {
                out.push('-');
            }
            out.push(CROCKFORD[index] as char);
            written += 1;
        }
    }
    out
}

/// Разобрать то, что человек переписал с бумаги.
///
/// Прощается всё, что делают при переписывании: регистр, дефисы, пробелы,
/// переводы строк, `O` вместо нуля, `I` и `L` вместо единицы. Не прощается
/// `U` — его в алфавите нет и ни на что он не похож.
///
/// Знаков после очистки должно остаться **ровно 32**. Не «хотя бы»: лишний
/// знак означает опечатку, а не запас.
pub fn parse(input: &str) -> Option<[u8; KEY_BYTES]> {
    let mut symbols = Vec::with_capacity(32);
    for c in input.chars() {
        let value = match c.to_ascii_uppercase() {
            '-' | ' ' | '\t' | '\n' | '\r' => continue,
            'O' => 0,
            'I' | 'L' => 1,
            other => CROCKFORD.iter().position(|&a| a as char == other)? as u32,
        };
        symbols.push(value);
    }
    if symbols.len() != 32 {
        return None;
    }

    let mut out = [0u8; KEY_BYTES];
    let mut buffer: u32 = 0;
    let mut bits = 0;
    let mut index = 0;
    for value in symbols {
        buffer = (buffer << 5) | value;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out[index] = ((buffer >> bits) & 0xFF) as u8;
            buffer &= (1 << bits) - 1;
            index += 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Известный ответ: эти байты дают ровно такую строку.
    ///
    /// Обычная проверка «записали и прочитали обратно» проходит и у двух
    /// реализаций, которые расходятся между собой: каждая читает то, что сама
    /// написала. Образец сверен с кабинетом 2026-09-22 — менять его можно
    /// только вместе с ним.
    #[test]
    fn a_known_key_prints_exactly_this() {
        let raw = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13,
        ];
        assert_eq!(format(&raw), "000G-40R4-0M30-E209-185G-R38E-1W81-24GK");
        assert_eq!(parse("000G-40R4-0M30-E209-185G-R38E-1W81-24GK"), Some(raw));

        assert_eq!(
            format(&[0x00; KEY_BYTES]),
            "0000-0000-0000-0000-0000-0000-0000-0000"
        );
        assert_eq!(
            format(&[0xff; KEY_BYTES]),
            "ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ"
        );
    }

    #[test]
    fn the_key_is_written_in_eight_groups_of_four() {
        let printed = format(&generate().expect("случайность"));
        let groups: Vec<&str> = printed.split('-').collect();
        assert_eq!(groups.len(), 8, "восемь групп: {printed}");
        assert!(groups.iter().all(|g| g.len() == 4), "{printed}");
    }

    /// Всё, что человек сделает при переписывании, не должно стоить доступа.
    #[test]
    fn sloppy_transcription_is_forgiven() {
        let raw = generate().expect("случайность");
        let printed = format(&raw);

        let sloppy = printed.to_lowercase().replace('-', " ");
        assert_eq!(parse(&sloppy), Some(raw));

        let confused = printed.replace('0', "O").replace('1', "I");
        assert_eq!(parse(&confused), Some(raw));

        assert_eq!(parse(&format!("  {printed}\n")), Some(raw));
    }

    /// Ровно 32 знака: и тридцать один, и тридцать три — опечатка.
    #[test]
    fn the_length_must_be_exact() {
        let printed = format(&generate().expect("случайность"));
        assert_eq!(parse(&printed[..printed.len() - 1]), None);
        assert_eq!(parse(&format!("{printed}0")), None);
    }

    /// `U` не прощается: его нет в алфавите и он ни на что не похож.
    #[test]
    fn a_letter_outside_the_alphabet_is_refused() {
        let printed = format(&generate().expect("случайность"));
        let broken = format!("U{}", &printed[1..]);
        assert_eq!(parse(&broken), None);
    }
}
