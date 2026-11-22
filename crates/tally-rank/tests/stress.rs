//! Тысячи вставок в одно и то же место: ключи остаются строго упорядоченными.

use tally_rank::Rank;

const N: usize = 5_000;

fn assert_sorted(keys: &[Rank]) {
    for w in keys.windows(2) {
        assert!(w[0] < w[1], "{} !< {}", w[0], w[1]);
        // Порядок Rust совпадает с побайтовым порядком строк (как в SQLite BINARY).
        assert!(w[0].as_str().as_bytes() < w[1].as_str().as_bytes());
    }
}

fn max_len(keys: &[Rank]) -> usize {
    keys.iter().map(|k| k.as_str().len()).max().unwrap_or(0)
}

fn reparsed(keys: &[Rank]) -> bool {
    keys.iter()
        .all(|k| k.as_str().parse::<Rank>().as_ref() == Ok(k))
}

#[test]
fn always_at_end() {
    let mut keys = vec![Rank::initial()];
    for _ in 0..N {
        let next = keys.last().map(Rank::after).unwrap_or_else(Rank::initial);
        keys.push(next);
    }
    assert_sorted(&keys);
    assert!(reparsed(&keys));
    assert!(max_len(&keys) <= 4, "длина {}", max_len(&keys));
}

#[test]
fn always_at_front() {
    let mut keys = vec![Rank::initial()];
    for _ in 0..N {
        let first = keys[0].before();
        keys.insert(0, first);
    }
    assert_sorted(&keys);
    assert!(reparsed(&keys));
    assert!(max_len(&keys) <= 4, "длина {}", max_len(&keys));
}

#[test]
fn always_right_after_the_same_neighbour() {
    // Список a, b; каждая новая задача встаёт сразу после a.
    let a = Rank::initial();
    let b = a.after();
    let mut keys = vec![a.clone(), b];
    for _ in 0..N {
        let k = Rank::between(Some(&keys[0]), Some(&keys[1]));
        keys.insert(1, k);
    }
    assert_sorted(&keys);
    assert!(reparsed(&keys));
    // Деление пополам: примерно символ на log2(62) ≈ 6 вставок.
    assert!(max_len(&keys) <= N / 5 + 4, "длина {}", max_len(&keys));
}

#[test]
fn always_right_before_the_same_neighbour() {
    let a = Rank::initial();
    let b = a.after();
    let mut keys = vec![a, b];
    for _ in 0..N {
        let n = keys.len();
        let k = Rank::between(Some(&keys[n - 2]), Some(&keys[n - 1]));
        keys.insert(n - 1, k);
    }
    assert_sorted(&keys);
    assert!(reparsed(&keys));
    assert!(max_len(&keys) <= N / 5 + 4, "длина {}", max_len(&keys));
}

#[test]
fn rebalance_after_crowding() {
    // После тысяч вставок в одно место список перенумеровывается короткими ключами.
    let keys = Rank::spread(N + 2);
    assert_sorted(&keys);
    // a0..az (62 ключа), b00..bzz (3844), дальше c000…
    assert!(max_len(&keys) <= 4, "длина {}", max_len(&keys));
}

#[test]
fn n_between_neighbours() {
    let a = Rank::initial();
    let b = a.after();
    let keys = Rank::n_between(Some(&a), Some(&b), 1_000);
    assert_eq!(keys.len(), 1_000);
    assert!(a < keys[0] && keys[999] < b);
    assert_sorted(&keys);
    // Равномерное деление: длина ~ log62(1000) + 2.
    assert!(max_len(&keys) <= 5, "длина {}", max_len(&keys));
}
