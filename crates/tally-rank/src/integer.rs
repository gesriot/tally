//! Целая часть ключа: символ-«голова» задаёт длину, дальше цифры base-62.
//!
//! Голова `a..=z` – неотрицательные числа длиной 2..=27 символов,
//! `A..=Z` – отрицательные длиной 27..=2. Голова сравнивается первой,
//! поэтому побайтовый порядок совпадает с числовым.

use crate::digits::{DIGITS, MAX, ZERO, value_of};

/// Длина целой части по её голове.
pub(crate) fn length(head: u8) -> Option<usize> {
    match head {
        b'a'..=b'z' => Some(usize::from(head - b'a') + 2),
        b'A'..=b'Z' => Some(usize::from(b'Z' - head) + 2),
        _ => None,
    }
}

/// Самое маленькое целое: `A` и 26 нулей. Меньше его ключ не уменьшить.
pub(crate) fn smallest() -> Vec<u8> {
    let mut v = vec![b'A'];
    v.extend(std::iter::repeat_n(ZERO, 26));
    v
}

/// Следующее целое или `None`, если места больше нет (голова `z`, все `z`).
pub(crate) fn increment(x: &[u8]) -> Option<Vec<u8>> {
    let (&head, digits) = x.split_first()?;
    let mut digits = digits.to_vec();
    let mut carry = true;
    for d in digits.iter_mut().rev() {
        let v = value_of(*d) + 1;
        if v == DIGITS.len() {
            *d = ZERO;
        } else {
            *d = DIGITS[v];
            carry = false;
            break;
        }
    }
    if !carry {
        return Some(prepend(head, digits));
    }
    match head {
        b'Z' => Some(vec![b'a', ZERO]),
        b'z' => None,
        _ => {
            let next = head + 1;
            if next > b'a' {
                digits.push(ZERO);
            } else {
                digits.pop();
            }
            Some(prepend(next, digits))
        }
    }
}

/// Предыдущее целое или `None`, если это [`smallest`].
pub(crate) fn decrement(x: &[u8]) -> Option<Vec<u8>> {
    let (&head, digits) = x.split_first()?;
    let mut digits = digits.to_vec();
    let mut borrow = true;
    for d in digits.iter_mut().rev() {
        let v = value_of(*d);
        if v == 0 {
            *d = MAX;
        } else {
            *d = DIGITS[v - 1];
            borrow = false;
            break;
        }
    }
    if !borrow {
        return Some(prepend(head, digits));
    }
    match head {
        b'a' => Some(vec![b'Z', MAX]),
        b'A' => None,
        _ => {
            let prev = head - 1;
            if prev < b'Z' {
                digits.push(MAX);
            } else {
                digits.pop();
            }
            Some(prepend(prev, digits))
        }
    }
}

fn prepend(head: u8, digits: Vec<u8>) -> Vec<u8> {
    let mut v = Vec::with_capacity(digits.len() + 1);
    v.push(head);
    v.extend(digits);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: Option<Vec<u8>>) -> String {
        v.and_then(|v| String::from_utf8(v).ok())
            .unwrap_or_default()
    }

    #[test]
    fn increments() {
        assert_eq!(s(increment(b"a0")), "a1");
        assert_eq!(s(increment(b"az")), "b00");
        assert_eq!(s(increment(b"Zz")), "a0");
        assert_eq!(s(increment(b"Yzz")), "Z0");
    }

    #[test]
    fn decrements() {
        assert_eq!(s(decrement(b"a1")), "a0");
        assert_eq!(s(decrement(b"a0")), "Zz");
        assert_eq!(s(decrement(b"b00")), "az");
        assert_eq!(s(decrement(b"Z0")), "Yzz");
        assert_eq!(decrement(&smallest()), None);
    }

    #[test]
    fn length_matches_head() {
        for x in [&b"a0"[..], b"b00", b"Zz", b"Yzz"] {
            assert_eq!(length(x[0]), Some(x.len()));
        }
    }
}
