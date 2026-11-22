//! Дробная часть: строка цифр без завершающего нуля, значение в `[0, 1)`.

use crate::digits::{BASE, DIGITS, ZERO, value_of};

/// Дробная часть строго между `a` и `b` (`b = None` – единица).
///
/// Требования: `a < b`, ни `a`, ни `b` не оканчиваются на `0`.
/// Результат тоже не оканчивается на `0`.
pub(crate) fn midpoint(a: &[u8], b: Option<&[u8]>) -> Vec<u8> {
    if let Some(b) = b {
        // Общий префикс (короткая `a` дополняется нулями) переносим как есть.
        let n = b
            .iter()
            .enumerate()
            .take_while(|&(i, &bc)| a.get(i).copied().unwrap_or(ZERO) == bc)
            .count();
        if n > 0 {
            let rest_a = a.get(n..).unwrap_or(&[]);
            let mut out = b[..n].to_vec();
            out.extend(midpoint(rest_a, Some(&b[n..])));
            return out;
        }
    }
    // Первые цифры (или их отсутствие) различаются.
    let digit_a = a.first().map_or(0, |&c| value_of(c));
    let digit_b = b.and_then(|b| b.first()).map_or(BASE, |&c| value_of(c));
    if digit_b - digit_a > 1 {
        return vec![DIGITS[(digit_a + digit_b).div_ceil(2)]];
    }
    // Цифры соседние.
    match b {
        Some(b) if b.len() > 1 => vec![b[0]],
        _ => {
            let mut out = vec![DIGITS[digit_a]];
            out.extend(midpoint(a.get(1..).unwrap_or(&[]), None));
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::midpoint;

    fn mid(a: &str, b: Option<&str>) -> String {
        String::from_utf8(midpoint(a.as_bytes(), b.map(str::as_bytes))).unwrap_or_default()
    }

    #[test]
    fn known_values() {
        assert_eq!(mid("", None), "V");
        assert_eq!(mid("V", None), "l");
        assert_eq!(mid("", Some("V")), "G");
        assert_eq!(mid("1", Some("2")), "1V");
        assert_eq!(mid("49", Some("5")), "4a");
        assert_eq!(mid("z", None), "zV");
        assert_eq!(mid("0001", Some("0002")), "0001V");
    }
}
