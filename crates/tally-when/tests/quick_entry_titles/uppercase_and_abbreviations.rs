//! Заглавные буквы в сроке и времени, сокращение дня недели, составной
//! интервал перед «завтра» и неподдерживаемая дробь.

use jiff::civil::date;
use tally_when::{ParseError, When, parse, split_title};

#[test]
fn uppercasing_a_deadline_or_clock_keeps_the_title_intact() {
    let today = date(2026, 11, 19);
    for original in ["Созвон в 18.05", "Сдать отчёт к 18:00 завтра"] {
        for text in [original.to_owned(), original.to_uppercase()] {
            let entry = split_title(&text, today);
            assert_eq!((entry.title, entry.when), (text.as_str(), None), "{text}");
        }
    }
}

#[test]
fn a_weekday_abbreviation_keeps_the_vitamin_name_intact() {
    let today = date(2026, 11, 19);
    let friday = Some(When::On {
        date: date(2026, 11, 20),
        evening: false,
    });
    let title = "Купить витамины группы В";
    for suffix in ["пт", "в пятницу", "завтра"] {
        let text = format!("{title} {suffix}");
        let entry = split_title(&text, today);
        assert_eq!((entry.title, entry.when), (title, friday), "{text}");
    }
}

#[test]
fn a_compound_duration_before_tomorrow_keeps_the_title_intact() {
    let today = date(2026, 11, 19);
    for text in [
        "Тренировка два с половиной часа завтра",
        "Занятие три с четвертью часа завтра",
        "Тренировка полтора часа завтра",
    ] {
        let entry = split_title(text, today);
        assert_eq!((entry.title, entry.when), (text, None), "{text}");
    }
}

#[test]
fn an_unsupported_fraction_does_not_schedule_this_evening() {
    let today = date(2026, 11, 19);
    for phrase in ["через 1/2 дня", "через половину часа", "через 1,5 дня"]
    {
        assert!(
            matches!(parse(phrase, today), Err(ParseError::Unsupported(_))),
            "{phrase}"
        );
        let text = format!("Напомнить {phrase} вечером");
        let entry = split_title(&text, today);
        assert_eq!((entry.title, entry.when), (text.as_str(), None), "{text}");
    }
}
