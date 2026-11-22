//! Срок перед временем и явный день недели в тексте с заглавными буквами.

use jiff::civil::date;
use tally_when::{When, split_title};

#[test]
fn a_deadline_before_a_clock_is_preserved_in_mixed_case() {
    let today = date(2026, 11, 19);
    for text in [
        "СДАТЬ ОТЧЁТ К 18:00 ЗАВТРА",
        "СДАТЬ ОТЧЁТ К 18:00 завтра",
        "СДАТЬ ОТЧЁТ К 9:30 в пятницу",
        "Сдать отчёт к 18:00 завтра",
    ] {
        let entry = split_title(text, today);
        assert_eq!((entry.title, entry.when), (text, None), "{text}");
    }
}

#[test]
fn an_explicit_weekday_or_day_keeps_names_and_meeting_times() {
    let today = date(2026, 11, 19);
    let friday = Some(When::On {
        date: date(2026, 11, 20),
        evening: false,
    });
    for (text, title) in [
        ("ВИТАМИНЫ ГРУППЫ В пт", "ВИТАМИНЫ ГРУППЫ В"),
        ("ФОРМА К завтра", "ФОРМА К"),
        ("СОЗВОН в 18:00 завтра", "СОЗВОН в 18:00"),
    ] {
        let entry = split_title(text, today);
        assert_eq!((entry.title, entry.when), (title, friday), "{text}");
    }
}
