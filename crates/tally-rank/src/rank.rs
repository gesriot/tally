//! Тип [`Rank`] и операции над ним.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::digits::{self, ZERO};
use crate::integer;
use crate::midpoint::midpoint;

/// Ключ порядка. Сравнение – побайтовое, как у строк.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Rank(String);

/// Ошибки разбора и вставки.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RankError {
    #[error("пустой ключ порядка")]
    Empty,
    #[error("недопустимый символ {0:?} в ключе порядка")]
    BadChar(char),
    #[error("неверная длина целой части ключа порядка {0:?}")]
    BadInteger(String),
    #[error("ключ порядка {0:?} оканчивается на 0")]
    TrailingZero(String),
    #[error("ключ порядка {0:?} меньше минимально допустимого")]
    Smallest(String),
    #[error("ключи порядка не по возрастанию: {0:?} ≥ {1:?}")]
    NotAscending(String, String),
    #[error("место для ключей исчерпано")]
    Exhausted,
}

impl Rank {
    /// Длина, после которой список стоит перенумеровать через [`Rank::spread`].
    ///
    /// Столько символов набирается примерно за 300 вставок подряд в одно и то же место.
    pub const REBALANCE_LEN: usize = 64;

    /// Ключ стал длинным – пора ребалансировать список.
    pub fn needs_rebalance(&self) -> bool {
        self.0.len() > Self::REBALANCE_LEN
    }

    /// Ключ первого элемента пустого списка (`a0`).
    pub fn initial() -> Self {
        Self(String::from("a0"))
    }

    /// Строка ключа – то, что пишется в базу.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Ключ строго между `a` и `b`; `None` – край списка.
    ///
    /// `between(None, None)` – [`Rank::initial`], `between(Some(x), None)` –
    /// после последнего, `between(None, Some(x))` – перед первым.
    ///
    /// # Panics
    ///
    /// Если `a >= b`: это ошибка вызывающего кода. Для проверки без паники –
    /// [`Rank::try_between`].
    pub fn between(a: Option<&Rank>, b: Option<&Rank>) -> Rank {
        match Self::try_between(a, b) {
            Ok(rank) => rank,
            Err(e) => panic!("Rank::between: {e}"),
        }
    }

    /// Как [`Rank::between`], но с ошибкой вместо паники.
    pub fn try_between(a: Option<&Rank>, b: Option<&Rank>) -> Result<Rank, RankError> {
        if let (Some(a), Some(b)) = (a, b)
            && a >= b
        {
            return Err(RankError::NotAscending(a.0.clone(), b.0.clone()));
        }
        let bytes = between_bytes(a.map(|r| r.0.as_bytes()), b.map(|r| r.0.as_bytes()))?;
        // Все байты – из алфавита ASCII.
        String::from_utf8(bytes)
            .map(Rank)
            .map_err(|_| RankError::Exhausted)
    }

    /// Ключ после `self` (вставка в конец).
    pub fn after(&self) -> Rank {
        Self::between(Some(self), None)
    }

    /// Ключ перед `self` (вставка в начало).
    pub fn before(&self) -> Rank {
        Self::between(None, Some(self))
    }

    /// `n` возрастающих ключей строго между `a` и `b`, по возможности равномерно.
    ///
    /// # Panics
    ///
    /// Если `a >= b`.
    pub fn n_between(a: Option<&Rank>, b: Option<&Rank>, n: usize) -> Vec<Rank> {
        match n {
            0 => Vec::new(),
            1 => vec![Self::between(a, b)],
            _ => match (a, b) {
                (_, None) => {
                    let mut out = Vec::with_capacity(n);
                    let mut last = Self::between(a, None);
                    for _ in 1..n {
                        let next = last.after();
                        out.push(last);
                        last = next;
                    }
                    out.push(last);
                    out
                }
                (None, Some(b)) => {
                    let mut out = Vec::with_capacity(n);
                    let mut first = b.before();
                    for _ in 1..n {
                        let prev = first.before();
                        out.push(first);
                        first = prev;
                    }
                    out.push(first);
                    out.reverse();
                    out
                }
                (Some(_), Some(_)) => {
                    let mid = n / 2;
                    let c = Self::between(a, b);
                    let mut out = Self::n_between(a, Some(&c), mid);
                    let right = Self::n_between(Some(&c), b, n - mid - 1);
                    out.push(c);
                    out.extend(right);
                    out
                }
            },
        }
    }

    /// `n` коротких равномерных ключей для ребалансировки всего списка.
    pub fn spread(n: usize) -> Vec<Rank> {
        Self::n_between(None, None, n)
    }
}

fn between_bytes(a: Option<&[u8]>, b: Option<&[u8]>) -> Result<Vec<u8>, RankError> {
    let Some(a) = a else {
        let Some(b) = b else {
            return Ok(b"a0".to_vec());
        };
        let (ib, fb) = split(b)?;
        if ib == integer::smallest().as_slice() {
            return Ok(concat(ib, &midpoint(&[], Some(fb))));
        }
        if !fb.is_empty() {
            // Голая целая часть меньше ключа с дробной частью.
            return Ok(ib.to_vec());
        }
        let prev = integer::decrement(ib).ok_or(RankError::Exhausted)?;
        if prev == integer::smallest() {
            // Голое наименьшее целое – запрещённый ключ (его не прочитать
            // обратно), поэтому берём его с дробной частью: он всё ещё меньше `b`.
            return Ok(concat(&prev, &midpoint(&[], None)));
        }
        return Ok(prev);
    };
    let (ia, fa) = split(a)?;
    let Some(b) = b else {
        return Ok(match integer::increment(ia) {
            Some(i) => i,
            None => concat(ia, &midpoint(fa, None)),
        });
    };
    let (ib, fb) = split(b)?;
    if ia == ib {
        return Ok(concat(ia, &midpoint(fa, Some(fb))));
    }
    let i = integer::increment(ia).ok_or(RankError::Exhausted)?;
    if i.as_slice() < b {
        Ok(i)
    } else {
        Ok(concat(ia, &midpoint(fa, None)))
    }
}

/// Делит ключ на целую и дробную части (ключ уже проверен при создании).
fn split(key: &[u8]) -> Result<(&[u8], &[u8]), RankError> {
    let head = *key.first().ok_or(RankError::Empty)?;
    let len = integer::length(head).ok_or_else(|| bad_integer(key))?;
    if len > key.len() {
        return Err(bad_integer(key));
    }
    Ok(key.split_at(len))
}

fn bad_integer(key: &[u8]) -> RankError {
    RankError::BadInteger(String::from_utf8_lossy(key).into_owned())
}

fn concat(a: &[u8], b: &[u8]) -> Vec<u8> {
    let mut v = a.to_vec();
    v.extend_from_slice(b);
    v
}

impl FromStr for Rank {
    type Err = RankError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = s.as_bytes();
        if let Some(c) = s
            .chars()
            .find(|&c| !c.is_ascii() || digits::value(c as u8).is_none())
        {
            return Err(RankError::BadChar(c));
        }
        let (_, fraction) = split(bytes)?;
        if fraction.last() == Some(&ZERO) {
            return Err(RankError::TrailingZero(s.to_owned()));
        }
        if bytes == integer::smallest().as_slice() {
            return Err(RankError::Smallest(s.to_owned()));
        }
        Ok(Self(s.to_owned()))
    }
}

impl TryFrom<String> for Rank {
    type Error = RankError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<Rank> for String {
    fn from(r: Rank) -> Self {
        r.0
    }
}

impl fmt::Display for Rank {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(s: &str) -> Rank {
        s.parse().unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    #[test]
    fn basics() {
        assert_eq!(Rank::initial().as_str(), "a0");
        assert_eq!(Rank::between(None, None), Rank::initial());
        assert_eq!(r("a0").after().as_str(), "a1");
        assert_eq!(r("a0").before().as_str(), "Zz");
        assert_eq!(
            Rank::between(Some(&r("a0")), Some(&r("a1"))).as_str(),
            "a0V"
        );
        assert_eq!(
            Rank::between(Some(&r("a0")), Some(&r("a0V"))).as_str(),
            "a0G"
        );
        assert_eq!(
            Rank::between(Some(&r("az")), Some(&r("b00"))).as_str(),
            "azV"
        );
    }

    #[test]
    fn parse_rejects_invalid() {
        assert_eq!("".parse::<Rank>(), Err(RankError::Empty));
        assert_eq!("a-".parse::<Rank>(), Err(RankError::BadChar('-')));
        assert_eq!("aё".parse::<Rank>(), Err(RankError::BadChar('ё')));
        assert!(matches!(
            "b0".parse::<Rank>(),
            Err(RankError::BadInteger(_))
        ));
        assert!(matches!(
            "a00".parse::<Rank>(),
            Err(RankError::TrailingZero(_))
        ));
        assert!(matches!(
            "A00000000000000000000000000".parse::<Rank>(),
            Err(RankError::Smallest(_))
        ));
        assert!("a0V".parse::<Rank>().is_ok());
    }

    #[test]
    fn not_ascending_is_error() {
        let a = r("a1");
        assert!(matches!(
            Rank::try_between(Some(&a), Some(&a)),
            Err(RankError::NotAscending(..))
        ));
        assert!(Rank::try_between(Some(&r("a2")), Some(&a)).is_err());
    }

    #[test]
    fn before_next_to_smallest_round_trips() {
        let mut key = String::from("A");
        key.push_str(&"0".repeat(25));
        key.push('1');
        let b = r(&key);
        let before = b.before();
        assert!(before < b);
        assert_eq!(before.as_str().parse::<Rank>().as_ref(), Ok(&before));
        assert_eq!(before.as_str(), format!("A{}V", "0".repeat(26)));
        // И дальше влево – тоже читаемые ключи.
        let further = before.before();
        assert!(further < before);
        assert_eq!(further.as_str().parse::<Rank>().as_ref(), Ok(&further));
    }

    #[test]
    fn serde_is_plain_string() {
        let json = serde_json::to_string(&r("a0V")).unwrap_or_default();
        assert_eq!(json, "\"a0V\"");
        assert_eq!(serde_json::from_str::<Rank>("\"a0V\"").ok(), Some(r("a0V")));
        assert!(serde_json::from_str::<Rank>("\"a00\"").is_err());
    }

    #[test]
    fn long_keys_need_rebalance() {
        assert!(!Rank::initial().needs_rebalance());
        let a = Rank::initial();
        let mut k = a.after();
        for _ in 0..1_000 {
            k = Rank::between(Some(&a), Some(&k));
        }
        assert!(k.needs_rebalance());
    }

    #[test]
    fn spread_is_short_and_sorted() {
        let keys = Rank::spread(100);
        assert_eq!(keys.len(), 100);
        assert!(keys.windows(2).all(|w| w[0] < w[1]));
        assert!(keys.iter().all(|k| k.as_str().len() <= 3));
    }
}
