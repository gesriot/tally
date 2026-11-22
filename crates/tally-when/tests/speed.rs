//! Быстрый ввод на очень длинных названиях – за один проход.

use std::time::{Duration, Instant};

use jiff::civil::date;
use tally_when::split_title;

/// 10 000 слов без даты и с датой в конце, а также длинные цепочки слов
/// даты, вводных слов и количества. Прежний перебор всех окончаний на 10 000 словах занимал секунды в
/// отладочной сборке; один проход – десятки миллисекунд. Порог щедрый,
/// чтобы тест не зависел от машины, но квадратичный перебор его не проходит.
#[test]
fn ten_thousand_words_are_fast() {
    let today = date(2026, 11, 19);
    let plain = (0..10_000)
        .map(|i| format!("слово{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let rep = |w: &str| vec![w; 10_000].join(" ");
    let cases = [
        (plain.clone(), false),
        (format!("{plain} завтра"), true),
        (rep("через"), false),
        (rep("15"), false),
        (format!("{} дней назад", rep("15")), false),
        (
            format!("Напомнить через {} дней вечером", rep("сто")),
            false,
        ),
        (format!("Напомнить через {} дней завтра", rep("сто")), false),
        // Вводные слова перед «через» и длинное количество, которое
        // перестаёт быть количеством в самом конце, – прежде квадратичные.
        (format!("Позвонить {} через 3 дня", rep("в")), false),
        (format!("Тренировка {} с часа завтра", rep("сто")), true),
        (format!("Тренировка {} часа завтра", rep("сто")), false),
    ];
    for (text, dated) in &cases {
        let start = Instant::now();
        let q = split_title(text, today);
        let took = start.elapsed();
        assert_eq!(q.when.is_some(), *dated);
        assert!(took < Duration::from_secs(1), "{took:?}");
    }
}
