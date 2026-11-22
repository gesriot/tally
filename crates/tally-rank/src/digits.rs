//! Алфавит base-62 в порядке ASCII: `0-9`, `A-Z`, `a-z`.

pub(crate) const DIGITS: &[u8; 62] =
    b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
pub(crate) const BASE: usize = DIGITS.len();
pub(crate) const ZERO: u8 = DIGITS[0];
pub(crate) const MAX: u8 = DIGITS[BASE - 1];

/// Значение цифры или `None`, если символ не из алфавита.
pub(crate) fn value(c: u8) -> Option<usize> {
    match c {
        b'0'..=b'9' => Some(usize::from(c - b'0')),
        b'A'..=b'Z' => Some(usize::from(c - b'A') + 10),
        b'a'..=b'z' => Some(usize::from(c - b'a') + 36),
        _ => None,
    }
}

/// Значение цифры, о которой уже известно, что она из алфавита.
pub(crate) fn value_of(c: u8) -> usize {
    value(c).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_are_sorted_and_round_trip() {
        assert!(DIGITS.windows(2).all(|w| w[0] < w[1]));
        for (i, &c) in DIGITS.iter().enumerate() {
            assert_eq!(value(c), Some(i));
        }
        assert_eq!(value(b'-'), None);
    }
}
