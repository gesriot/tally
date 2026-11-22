//! Маршрут, количество из нескольких слов, обычные слова и время в конце
//! названия сохраняют запись целиком.

use jiff::civil::date;
use tally_when::{ParseError, When, parse, split_title};

#[test]
fn a_route_in_the_title_can_precede_a_valid_offset() {
    let today = date(2026, 11, 19);
    let entry = split_title("Оплатить через банк через неделю", today);
    assert_eq!(entry.title, "Оплатить через банк");
    assert_eq!(
        entry.when,
        Some(When::On {
            date: date(2026, 11, 26),
            evening: false,
        })
    );
}

#[test]
fn a_multiword_quantity_preserves_the_whole_entry() {
    let today = date(2026, 11, 19);
    for phrase in ["через сто двадцать пять дней", "через два с половиной дня"]
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

#[test]
fn ordinary_words_and_letter_names_can_end_a_title() {
    let today = date(2026, 11, 19);
    let tomorrow = Some(When::On {
        date: date(2026, 11, 20),
        evening: false,
    });
    for title in ["Купить ремешок для часов", "Купить витамины группы В"]
    {
        let text = format!("{title} завтра");
        let entry = split_title(&text, today);
        assert_eq!((entry.title, entry.when), (title, tomorrow), "{text}");
    }
}

#[test]
fn actual_time_expressions_and_prepositions_preserve_the_whole_entry() {
    let today = date(2026, 11, 19);
    for text in [
        "Созвон в 7 часов вечером",
        "Тренировка 2 часа завтра",
        "Подарок для 15.12",
        "Созвон в 18.05",
        "Позвонить через полтора часа вечером",
    ] {
        let entry = split_title(text, today);
        assert_eq!((entry.title, entry.when), (text, None), "{text}");
    }
}
