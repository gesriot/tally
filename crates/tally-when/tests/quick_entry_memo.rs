//! Примеры памятки quick-entry-ru.md: каждый пример из её таблиц и из
//! текста даёт то, что в ней написано.

use jiff::civil::{Date, date};
use tally_when::{When, parse, split_title};

const TODAY: Date = date(2026, 11, 19);

fn on(y: i16, m: i8, d: i8) -> Option<When> {
    Some(When::On {
        date: date(y, m, d),
        evening: false,
    })
}

fn eve(y: i16, m: i8, d: i8) -> Option<When> {
    Some(When::On {
        date: date(y, m, d),
        evening: true,
    })
}

/// Что написать, название, дата – по строкам таблиц памятки. `None` вместо
/// названия – вся строка остаётся названием.
fn table() -> Vec<(&'static str, Option<&'static str>, Option<When>)> {
    vec![
        (
            "Позвонить маме сегодня",
            Some("Позвонить маме"),
            on(2026, 11, 19),
        ),
        (
            "Купить хлеб завтра вечером",
            Some("Купить хлеб"),
            eve(2026, 11, 20),
        ),
        (
            "Заказать ужин вечером",
            Some("Заказать ужин"),
            eve(2026, 11, 19),
        ),
        (
            "Забрать заказ послезавтра",
            Some("Забрать заказ"),
            on(2026, 11, 21),
        ),
        (
            "Полить цветы через 3 дня",
            Some("Полить цветы"),
            on(2026, 11, 22),
        ),
        ("Сверить смету пт", Some("Сверить смету"), on(2026, 11, 20)),
        (
            "Сверить смету в пятницу",
            Some("Сверить смету"),
            on(2026, 11, 20),
        ),
        (
            "Сверить смету в следующую пятницу",
            Some("Сверить смету"),
            on(2026, 11, 27),
        ),
        (
            "Съездить за город на выходных",
            Some("Съездить за город"),
            on(2026, 11, 21),
        ),
        (
            "Записаться к врачу на следующей неделе",
            Some("Записаться к врачу"),
            on(2026, 11, 23),
        ),
        ("Сдать отчёт 15 дек", Some("Сдать отчёт"), on(2026, 12, 15)),
        ("Сдать отчёт 15.12", Some("Сдать отчёт"), on(2026, 12, 15)),
        (
            "Сдать отчёт 15 декабря 2027 г. вечером",
            Some("Сдать отчёт"),
            eve(2027, 12, 15),
        ),
        (
            "Разобрать фотографии в любое время",
            Some("Разобрать фотографии"),
            Some(When::Anytime),
        ),
        (
            "Научиться плавать когда-нибудь",
            Some("Научиться плавать"),
            Some(When::Someday),
        ),
        ("Завтра", None, None),
        (
            "Посмотреть фильм «Послезавтра» завтра",
            Some("Посмотреть фильм «Послезавтра»"),
            on(2026, 11, 20),
        ),
        ("Сдать отчёт до пятницы", None, None),
        (
            "Созвон в 18:00 завтра",
            Some("Созвон в 18:00"),
            on(2026, 11, 20),
        ),
        ("Позвонить через два часа", None, None),
        ("Позвонить через 1/2 дня завтра", None, None),
        ("Купить подарок 31 ноября завтра", None, None),
        (
            "Сделать за 2 дня завтра",
            Some("Сделать за 2 дня"),
            on(2026, 11, 20),
        ),
        // Последний абзац.
        (
            "Витамины группы В пт",
            Some("Витамины группы В"),
            on(2026, 11, 20),
        ),
        ("СОЗВОН в пятницу", Some("СОЗВОН"), on(2026, 11, 20)),
        ("СОЗВОН В пятницу", Some("СОЗВОН В"), on(2026, 11, 20)),
        (
            "СОЗВОН в 18:05 завтра",
            Some("СОЗВОН в 18:05"),
            on(2026, 11, 20),
        ),
    ]
}

#[test]
fn every_memo_example_behaves_as_written() {
    for (text, title, when) in table() {
        let entry = split_title(text, TODAY);
        let title = title.unwrap_or(text);
        assert_eq!((entry.title, entry.when), (title, when), "{text}");
    }
}

/// Каждый пример в `…` из таблиц памятки есть в [`table`]: памятка и тест
/// не расходятся.
#[test]
fn the_memo_has_no_untested_examples() {
    let memo = include_str!("../quick-entry-ru.md");
    let table = table();
    let mut checked = 0;
    for line in memo.lines().filter(|l| l.starts_with("| `")) {
        let first = line.split('|').nth(1).unwrap_or_default();
        for example in first.split('`').skip(1).step_by(2) {
            assert!(
                table.iter().any(|(text, _, _)| *text == example),
                "пример «{example}» памятки не проверен"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 23);
}

/// Правила из текста памятки.
#[test]
fn memo_rules_hold() {
    let day = |text: &str, today: Date| parse(text, today).ok();
    let plain = |y, m, d| on(y, m, d);
    // Дни недели – ближайший после сегодняшнего: в четверг «чт» – через неделю.
    for (text, d) in [
        ("пн", 23),
        ("вт", 24),
        ("ср", 25),
        ("чт", 26),
        ("пт", 20),
        ("сб", 21),
        ("вс", 22),
    ] {
        assert_eq!(day(text, TODAY), plain(2026, 11, d), "{text}");
    }
    // «Следующий» – день следующей календарной недели.
    assert_eq!(day("следующий пн", TODAY), plain(2026, 11, 23));
    assert_eq!(day("следующий чт", TODAY), plain(2026, 11, 26));
    assert_eq!(day("в следующую пятницу", TODAY), plain(2026, 11, 27));
    // «На выходных» – ближайшая суббота, в выходной – сегодня.
    assert_eq!(day("на выходных", TODAY), plain(2026, 11, 21));
    assert_eq!(day("на выходных", date(2026, 11, 21)), plain(2026, 11, 21));
    assert_eq!(day("на выходных", date(2026, 11, 22)), plain(2026, 11, 22));
    // «Через»: цифры 1–999, слова «один» – «десять», «пару», «месяц».
    assert_eq!(day("через 999 дней", TODAY), plain(2029, 8, 14));
    assert_eq!(day("через десять дней", TODAY), plain(2026, 11, 29));
    assert_eq!(day("через один день", TODAY), plain(2026, 11, 20));
    assert_eq!(day("через пару недель", TODAY), plain(2026, 12, 3));
    assert_eq!(day("через месяц", TODAY), plain(2026, 12, 19));
    assert_eq!(day("через месяц", date(2027, 1, 31)), plain(2027, 2, 28));
    assert_eq!(day("через 1000 дней", TODAY), None);
    // Дата без года – ближайшая, включая сегодня; с годом – любая.
    assert_eq!(day("19 ноября", TODAY), plain(2026, 11, 19));
    assert_eq!(day("18 ноября", TODAY), plain(2027, 11, 18));
    for text in ["15 декабря 2027", "15.12.2027", "15.12.27"] {
        assert_eq!(day(text, TODAY), plain(2027, 12, 15), "{text}");
    }
    assert_eq!(day("1 мая 2026", TODAY), plain(2026, 5, 1));
    // Остаются текстом.
    for text in [
        "Позвонить первое декабря",
        "Позвонить утром",
        "Позвонить днём",
    ] {
        assert_eq!(split_title(text, TODAY).when, None, "{text}");
    }
    assert_eq!(day("1 декабря", TODAY), plain(2026, 12, 1));
}
