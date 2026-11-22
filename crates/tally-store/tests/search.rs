//! Поиск по кириллице: регистр, «ё», несколько слов, заметки и чек-листы.

mod common;

use common::*;
use proptest::prelude::*;
use tally_core::{ChecklistItem, ChecklistItemId, Status};
use tally_store::{Found, RecordId, Store};

fn seeded() -> Store {
    let mut s = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
    let home = area(1, "Ёлочные игрушки");
    let shop = tag(2, "МАГАЗИН");
    let mut p = project(10, "Новогодняя ёлка");
    p.notes = "Купить гирлянду".into();
    let trashed = project(11, "Старая ёлка");
    let mut a = task(100, "Купить ЁЛКУ");
    a.notes = "живую, не меньше двух метров".into();
    let mut b = task(101, "Позвонить Алёне");
    b.checklist = vec![ChecklistItem::new(
        ChecklistItemId::from_u128(1),
        "Спросить про ёлку",
        rank("a0"),
        t0(),
    )];
    let mut c = task(102, "Ёлка: украсить");
    c.status = Status::Completed { at: t0() };
    let mut d = task(103, "Выбросить ёлку");
    d.project = Some(trashed.id);
    let e = task(104, "Ёлка в корзине");
    let ok = s.write(|w| {
        w.insert_area(&home)?;
        w.insert_tag(&shop)?;
        w.insert_project(&p)?;
        w.insert_project(&trashed)?;
        for t in [&a, &b, &c, &d, &e] {
            w.insert_task(t)?;
        }
        w.delete(RecordId::Project(trashed.id), t0())?;
        w.delete(RecordId::Task(e.id), t0())
    });
    assert!(ok.is_ok(), "{ok:?}");
    s
}

fn titles(s: &Store, q: &str) -> Vec<String> {
    s.read()
        .search(q)
        .unwrap_or_default()
        .into_iter()
        .map(|h| h.title)
        .collect()
}

#[test]
fn case_and_yo_do_not_matter() {
    let s = seeded();
    for q in ["елк", "ЁЛК", "Ёлк", "еЛк"] {
        assert_eq!(
            titles(&s, q),
            [
                "Новогодняя ёлка",
                "Купить ЁЛКУ",
                "Ёлка: украсить",
                "Позвонить Алёне", // «ёлку» – в чек-листе
            ],
            "{q}"
        );
    }
}

#[test]
fn title_matches_come_before_notes_and_checklists() {
    let s = seeded();
    // «ёлку»: в названии у «Купить ЁЛКУ», в чек-листе у «Позвонить Алёне».
    let hits = s.read().search("ёлку").unwrap_or_default();
    let found: Vec<(&str, bool)> = hits
        .iter()
        .map(|h| (h.title.as_str(), h.in_title))
        .collect();
    assert_eq!(found, [("Купить ЁЛКУ", true), ("Позвонить Алёне", false)]);
    assert_eq!(titles(&s, "гирлянду"), ["Новогодняя ёлка"]);
    assert_eq!(titles(&s, "ДВУХ метров"), ["Купить ЁЛКУ"]);
    assert_eq!(titles(&s, "АЛЕН"), ["Позвонить Алёне"]);
    assert_eq!(titles(&s, "ёлочные"), ["Ёлочные игрушки"]);
}

#[test]
fn all_words_must_match_in_any_order() {
    let s = seeded();
    assert_eq!(titles(&s, "ёлку купить"), ["Купить ЁЛКУ"]);
    assert!(titles(&s, "ёлку продать").is_empty());
}

#[test]
fn completed_after_open_and_deleted_not_found() {
    let s = seeded();
    let hits = s.read().search("ёлка").unwrap_or_default();
    let flags: Vec<(&str, bool)> = hits.iter().map(|h| (h.title.as_str(), h.open)).collect();
    // «Выбросить ёлку» – в удалённом проекте, «Ёлка в корзине» удалена сама.
    assert_eq!(
        flags,
        [("Новогодняя ёлка", true), ("Ёлка: украсить", false)]
    );
}

#[test]
fn every_kind_is_searchable() {
    let s = seeded();
    let kinds = |q: &str| -> Vec<Found> {
        s.read()
            .search(q)
            .unwrap_or_default()
            .into_iter()
            .map(|h| h.found)
            .collect()
    };
    assert!(matches!(kinds("игрушки")[..], [Found::Area(_)]));
    assert!(matches!(kinds("магазин")[..], [Found::Tag(_)]));
    assert!(matches!(kinds("новогодняя")[..], [Found::Project(_)]));
    assert!(matches!(kinds("украсить")[..], [Found::Task(_)]));
}

#[test]
fn blank_query_finds_nothing_and_symbols_are_literal() {
    let s = seeded();
    assert!(titles(&s, "   ").is_empty());
    assert!(titles(&s, "%").is_empty());
    assert!(titles(&s, "_").is_empty());
    assert_eq!(titles(&s, "ёлка:"), ["Ёлка: украсить"]);
}

#[test]
fn search_follows_updates() {
    let mut s = seeded();
    let mut t = s
        .read()
        .task(tally_core::TaskId::from_u128(100))
        .ok()
        .flatten()
        .unwrap_or_else(|| task(0, ""));
    t.title = "Купить сосну".into();
    t.notes.clear();
    assert!(s.write(|w| w.update_task(&t)).is_ok());
    assert!(titles(&s, "метров").is_empty());
    assert_eq!(titles(&s, "СОСНУ"), ["Купить сосну"]);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn any_cyrillic_title_round_trips_and_is_found(
        title in "[а-яёА-ЯЁ]{1,12}( [а-яёА-ЯЁ0-9]{1,8}){0,3}",
    ) {
        let mut s = Store::open_in_memory().map_err(|e| TestCaseError::fail(e.to_string()))?;
        let t = task(1, &title);
        s.write(|w| w.insert_task(&t)).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(s.read().task(t.id).ok().flatten(), Some(t));
        let shouted = title.to_uppercase();
        let plain_e = title.replace('ё', "е").replace('Ё', "Е");
        for q in [title.as_str(), shouted.as_str(), plain_e.as_str()] {
            prop_assert_eq!(titles(&s, q), vec![title.clone()]);
        }
    }

    #[test]
    fn normalize_is_idempotent(text in "\\PC{0,40}") {
        let once = tally_store::normalize(&text);
        prop_assert_eq!(tally_store::normalize(&once), once.clone());
    }

}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4096))]

    /// Кириллица, латиница и комбинируемые знаки вперемешку – там, где
    /// случайный `\PC` почти не попадает в сочетания вроде «Ё» + U+0300.
    #[test]
    fn normalize_is_idempotent_with_combining_marks(
        text in "[а-яА-ЯёЁйЙіїІЇa-zA-ZİJǰ \u{300}-\u{36F}\u{483}-\u{489}\u{1DC0}-\u{1DFF}]{0,30}"
    ) {
        let once = tally_store::normalize(&text);
        prop_assert_eq!(tally_store::normalize(&once), once.clone());
        prop_assert!(!once.contains('ё'));
    }
}
