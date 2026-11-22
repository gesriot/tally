//! Property-тесты: случайные последовательности вставок и удалений.

use proptest::prelude::*;
use tally_rank::Rank;

/// Операция над списком: вставка в позицию (доля от длины) или удаление.
#[derive(Debug, Clone)]
enum Op {
    Insert(f64),
    Remove(f64),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (0.0..=1.0f64).prop_map(Op::Insert),
        1 => (0.0..1.0f64).prop_map(Op::Remove),
    ]
}

fn position(fraction: f64, len: usize) -> usize {
    ((fraction * len as f64) as usize).min(len)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn random_sequences_stay_sorted(ops in prop::collection::vec(op(), 1..400)) {
        let mut keys: Vec<Rank> = Vec::new();
        for op in ops {
            match op {
                Op::Insert(f) => {
                    let i = position(f, keys.len());
                    let k = Rank::between(i.checked_sub(1).map(|j| &keys[j]), keys.get(i));
                    keys.insert(i, k);
                }
                Op::Remove(f) if !keys.is_empty() => {
                    let i = position(f, keys.len() - 1);
                    keys.remove(i);
                }
                Op::Remove(_) => {}
            }
            prop_assert!(keys.windows(2).all(|w| w[0] < w[1]));
        }
        for k in &keys {
            prop_assert!(k.as_str().parse::<Rank>().ok().as_ref() == Some(k));
        }
    }

    #[test]
    fn between_is_strictly_inside(seed in prop::collection::vec(0.0..=1.0f64, 2..60)) {
        // Строим набор разнообразных ключей, затем проверяем каждую пару соседей.
        let mut keys: Vec<Rank> = Vec::new();
        for f in seed {
            let i = position(f, keys.len());
            let k = Rank::between(i.checked_sub(1).map(|j| &keys[j]), keys.get(i));
            keys.insert(i, k);
        }
        for w in keys.windows(2) {
            let m = Rank::between(Some(&w[0]), Some(&w[1]));
            prop_assert!(w[0] < m && m < w[1]);
        }
        let first = &keys[0];
        let last = &keys[keys.len() - 1];
        prop_assert!(first.before() < *first);
        prop_assert!(last.after() > *last);
    }

    #[test]
    fn n_between_is_sorted_and_inside(n in 0usize..300, gap in 1u32..5) {
        let mut a = Rank::initial();
        for _ in 0..gap {
            a = Rank::between(Some(&a), Some(&a.after()));
        }
        let b = a.after();
        let keys = Rank::n_between(Some(&a), Some(&b), n);
        prop_assert_eq!(keys.len(), n);
        prop_assert!(keys.windows(2).all(|w| w[0] < w[1]));
        prop_assert!(keys.iter().all(|k| a < *k && *k < b));
    }

    #[test]
    fn keys_near_smallest_round_trip(low in 1usize..62 * 62, steps in 1usize..8) {
        // Целые части у самой нижней границы: «A», 24 нуля и две цифры.
        const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
        let mut key = String::from("A");
        key.push_str(&"0".repeat(24));
        key.push(char::from(DIGITS[low / 62]));
        key.push(char::from(DIGITS[low % 62]));
        let mut k: Rank = key.parse().map_err(|e| TestCaseError::fail(format!("{key}: {e}")))?;
        for _ in 0..steps {
            let prev = k.before();
            prop_assert!(prev < k);
            prop_assert_eq!(prev.as_str().parse::<Rank>().ok(), Some(prev.clone()));
            let mid = Rank::between(Some(&prev), Some(&k));
            prop_assert_eq!(mid.as_str().parse::<Rank>().ok(), Some(mid.clone()));
            k = prev;
        }
    }
}
