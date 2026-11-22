//! Сокращение года, неподдерживаемый календарный интервал и название группы
//! витаминов не дают ложной даты.

use jiff::civil::date;
use tally_when::{ParseError, When, parse, split_title};

#[test]
fn a_year_abbreviation_keeps_the_explicit_date_in_quick_entry() {
    let today = date(2026, 11, 19);
    for (title, phrase, evening) in [
        ("Отчёт", "15 декабря 2027 г. вечером", true),
        ("ОТЧЁТ", "15 ДЕКАБРЯ 2027 Г. ВЕЧЕРОМ", true),
        ("ОТЧЁТ", "15 ДЕКАБРЯ 2027 Г.", false),
    ] {
        let when = When::On {
            date: date(2027, 12, 15),
            evening,
        };
        assert_eq!(parse(phrase, today), Ok(when), "{phrase}");
        let text = format!("{title} {phrase}");
        let entry = split_title(&text, today);
        assert_eq!((entry.title, entry.when), (title, Some(when)), "{text}");
    }
}

#[test]
fn an_unsupported_calendar_interval_protects_the_whole_entry() {
    let today = date(2026, 11, 19);
    for phrase in ["через 1/2 дня", "через 2–3 недели", "через ½ месяца"]
    {
        assert!(matches!(
            parse(phrase, today),
            Err(ParseError::Unsupported(_))
        ));
        for suffix in ["вечером", "завтра", "в пятницу"] {
            let text = format!("Напомнить {phrase} {suffix}");
            let entry = split_title(&text, today);
            assert_eq!((entry.title, entry.when), (text.as_str(), None), "{text}");
        }
    }
    let entry = split_title("Сделать за 2 дня завтра", today);
    assert_eq!(entry.title, "Сделать за 2 дня");
    assert_eq!(
        entry.when,
        Some(When::On {
            date: date(2026, 11, 20),
            evening: false,
        })
    );
}

#[test]
fn the_vitamin_group_name_survives_uppercase_input() {
    let today = date(2026, 11, 19);
    let friday = Some(When::On {
        date: date(2026, 11, 20),
        evening: false,
    });
    for title in ["Витамины группы В", "ВИТАМИНЫ ГРУППЫ В"] {
        let text = format!("{title} пт");
        let entry = split_title(&text, today);
        assert_eq!((entry.title, entry.when), (title, friday), "{text}");
    }
    for text in ["СОЗВОН В 18.05", "СДАТЬ ОТЧЁТ К 18:00 ЗАВТРА"] {
        let entry = split_title(text, today);
        assert_eq!((entry.title, entry.when), (text, None), "{text}");
    }
}
