//! Свойства разбора на случайной опорной дате.

use jiff::ToSpan;
use jiff::civil::{Date, date};
use proptest::prelude::*;
use tally_when::{MAX_DATE, MIN_DATE, ParseError, When, format, parse, split_title};

/// Дней от `MIN_DATE` до `MAX_DATE`.
const SPAN: i64 = 3_287_181;

fn from_min(n: i64) -> Date {
    MIN_DATE.checked_add(n.days()).unwrap_or(MAX_DATE)
}

/// Опорная дата: чаще с 1990 по 2100 год, но и во всём диапазоне
/// `MIN_DATE..=MAX_DATE`, и у самых его краёв.
fn any_today() -> impl Strategy<Value = Date> {
    prop_oneof![
        3 => (0i64..40_500).prop_map(|n| date(1990, 1, 1).checked_add(n.days()).unwrap_or(MIN_DATE)),
        1 => (0..SPAN).prop_map(from_min),
        1 => (0i64..800).prop_map(from_min),
        1 => (0i64..800).prop_map(|n| from_min(SPAN - 1 - n)),
    ]
}

/// Опорная дата, от которой до `MAX_DATE` больше 8000 дней: «через 999
/// недель» ещё в диапазоне.
fn early_today() -> impl Strategy<Value = Date> {
    any_today().prop_map(|t| t.min(from_min(SPAN - 8001)))
}

/// День для `format`: рядом с опорной датой, где угодно в диапазоне или у
/// его краёв.
fn any_day(today: Date) -> impl Strategy<Value = Date> {
    prop_oneof![
        3 => (-400i64..800).prop_map(move |o| {
            today.checked_add(o.days()).unwrap_or(today).clamp(MIN_DATE, MAX_DATE)
        }),
        1 => (0..SPAN).prop_map(from_min),
        1 => (0i64..400).prop_map(from_min),
        1 => (0i64..400).prop_map(|n| from_min(SPAN - 1 - n)),
    ]
}

fn days(from: Date, to: Date) -> i32 {
    from.until(to).map(|s| s.get_days()).unwrap_or(i32::MIN)
}

const WEEKDAY_FORMS: [(&str, i8); 14] = [
    ("пн", 0),
    ("понедельник", 0),
    ("вт", 1),
    ("во вторник", 1),
    ("ср", 2),
    ("в среду", 2),
    ("чт", 3),
    ("четверг", 3),
    ("пт", 4),
    ("в пятницу", 4),
    ("сб", 5),
    ("в субботу", 5),
    ("вс", 6),
    ("воскресенье", 6),
];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// «через N дней/недель» – ровно N дней (N×7) вперёд, всегда после сегодня.
    #[test]
    fn through_n_days_and_weeks(today in early_today(), n in 1u32..=999) {
        let d = parse(&format!("через {n} дней"), today).ok().and_then(When::date);
        prop_assert_eq!(d.map(|d| days(today, d)), Some(n as i32));
        let w = parse(&format!("через {n} недель"), today).ok().and_then(When::date);
        prop_assert_eq!(w.map(|d| days(today, d)), Some(7 * n as i32));
    }

    /// «через N месяцев» – не раньше сегодня + N×28 дней и не позже + N×31.
    #[test]
    fn through_n_months(today in early_today(), n in 1u32..=120) {
        let d = parse(&format!("через {n} месяцев"), today).ok().and_then(When::date);
        let ahead = d.map(|d| days(today, d)).unwrap_or(i32::MIN);
        prop_assert!(ahead >= 28 * n as i32 && ahead <= 31 * n as i32, "{ahead}");
    }

    /// День недели – ближайший такой день строго после сегодня (1–7 дней);
    /// «следующий» – этот день следующей календарной недели.
    #[test]
    fn weekdays(today in early_today(), form in 0usize..WEEKDAY_FORMS.len()) {
        let (text, wd) = WEEKDAY_FORMS[form];
        let d = parse(text, today).ok().and_then(When::date);
        prop_assert!(d.is_some(), "«{}»", text);
        let d = d.unwrap_or(today);
        prop_assert_eq!(d.weekday().to_monday_zero_offset(), wd);
        prop_assert!((1..=7).contains(&days(today, d)));

        let name = text.trim_start_matches("в ").trim_start_matches("во ");
        let next = parse(&format!("следующий {name}"), today).ok().and_then(When::date);
        let next = next.unwrap_or(today);
        let monday = |x: Date| x.checked_sub(i64::from(x.weekday().to_monday_zero_offset()).days());
        prop_assert_eq!(next.weekday().to_monday_zero_offset(), wd);
        prop_assert_eq!(
            monday(next).ok(),
            monday(today).ok().and_then(|m| m.checked_add(7.days()).ok())
        );
    }

    /// День и месяц без года – ближайший такой день, начиная с сегодня:
    /// не раньше сегодня и не дальше чем через 365 дней.
    #[test]
    fn day_month_is_the_next_occurrence(today in early_today(), n in 0i64..365) {
        // Любой день невисокосного года (29 февраля – отдельный тест в таблице).
        let day = date(2001, 1, 1).checked_add(n.days()).unwrap_or(date(2001, 1, 1));
        let (m, d) = (day.month(), day.day());
        for text in [format!("{d}.{m}"), format!("{d}.{m:02}")] {
            let got = parse(&text, today).ok().and_then(When::date);
            prop_assert!(got.is_some(), "«{}»", text);
            let got = got.unwrap_or(today);
            prop_assert_eq!((got.month(), got.day()), (m, d));
            prop_assert!((0..=365).contains(&days(today, got)));
        }
    }

    /// Текст из `format` разбирается обратно в то же значение – и в любом
    /// регистре, с лишними пробелами. Опорные даты и дни – по всему
    /// диапазону и у его краёв.
    #[test]
    fn format_round_trips(
        (today, day) in any_today().prop_flat_map(|t| (Just(t), any_day(t))),
        evening: bool,
        kind in 0u8..10,
        upper: bool,
    ) {
        let when = match kind {
            0 => When::Someday,
            1 => When::Anytime,
            _ => When::On { date: day, evening },
        };
        let text = format(when, today);
        prop_assert!(text.is_some(), "{:?}", when);
        let text = text.unwrap_or_default();
        prop_assert_eq!(parse(&text, today), Ok(when), "«{}»", text);
        let messy = format!("  {}  ", text.replace(' ', "   "));
        let messy = if upper { messy.to_uppercase() } else { messy };
        prop_assert_eq!(parse(&messy, today), Ok(when), "«{}»", messy);
    }

    /// День раньше `MIN_DATE` не пишется, а разбор никогда не даёт день
    /// вне диапазона.
    #[test]
    fn out_of_range_is_never_written_or_read(
        today in any_today(),
        back in 1i64..400_000,
        n in 1u32..=999,
        unit in prop::sample::select(vec!["дней", "недель", "месяцев", "лет"]),
    ) {
        let early = MIN_DATE.checked_sub(back.days()).unwrap_or(MIN_DATE);
        prop_assert_eq!(format(When::On { date: early, evening: false }, today), None);
        if let Ok(w) = parse(&format!("через {n} {unit}"), today) {
            let d = w.date().unwrap_or(MIN_DATE);
            prop_assert!((MIN_DATE..=MAX_DATE).contains(&d));
        }
    }

    /// Быстрый ввод: «название + выражение» делится обратно на название и дату.
    #[test]
    fn quick_entry_splits_formatted_suffix(
        (today, day) in any_today().prop_flat_map(|t| (Just(t), any_day(t))),
        words in prop::collection::vec(
            prop::sample::select(vec!["Купить", "хлеб", "позвонить", "маме", "отчёт", "книгу", "3", "яблока"]),
            1..5,
        ),
        evening: bool,
        kind in 0u8..10,
    ) {
        let when = match kind {
            0 => When::Someday,
            1 => When::Anytime,
            _ => When::On { date: day, evening },
        };
        let title = words.join(" ");
        let text = format!("{title} {}", format(when, today).unwrap_or_default());
        let q = split_title(&text, today);
        prop_assert_eq!((q.title, q.when), (title.as_str(), Some(when)), "«{}»", text);
    }

    /// «название + предлог + выражение»: дату с предлогом («до завтра»,
    /// «к 15 декабря») быстрый ввод не отрезает.
    #[test]
    fn quick_entry_keeps_preposition_phrase(
        today in date_today(),
        prep in prop::sample::select(vec!["до", "к", "ко", "по", "с", "со", "после", "от", "за", "не", "или"]),
        offset in 0i64..400,
        evening: bool,
    ) {
        let when = When::On { date: today.checked_add(offset.days()).unwrap_or(today), evening };
        let text = format!("Сдать отчёт {prep} {}", format(when, today).unwrap_or_default());
        let q = split_title(&text, today);
        prop_assert_eq!((q.title, q.when), (text.as_str(), None), "«{}»", text);
    }

    /// Выражение в кавычках – название; дата после кавычек отделяется.
    #[test]
    fn quick_entry_ignores_quoted_phrase(
        today in date_today(),
        offset in 0i64..400,
        evening: bool,
        quotes in prop::sample::select(vec![("«", "»"), ("„", "“"), ("“", "”"), ("\"", "\"")]),
    ) {
        let when = When::On { date: today.checked_add(offset.days()).unwrap_or(today), evening };
        let phrase = format(when, today).unwrap_or_default();
        let title = format!("Посмотреть фильм {}{phrase}{}", quotes.0, quotes.1);
        let q = split_title(&title, today);
        prop_assert_eq!((q.title, q.when), (title.as_str(), None), "«{}»", title);
        let text = format!("{title} завтра");
        let q = split_title(&text, today);
        let tomorrow = today.checked_add(1.days()).ok().map(|date| When::On { date, evening: false });
        prop_assert_eq!((q.title, q.when), (title.as_str(), tomorrow), "«{}»", text);
    }

    /// Время между предлогом и датой не прячет предлог: «до 18:00 завтра» –
    /// название целиком; без предлога («в 18:00 завтра») дата отделяется.
    #[test]
    fn quick_entry_time_before_date(
        today in date_today(),
        prep in prop::sample::select(vec!["до", "к", "ко", "с", "после", "не"]),
        (h, m) in (0u8..24, 0u8..60),
        offset in 0i64..400,
        evening: bool,
    ) {
        let when = When::On { date: today.checked_add(offset.days()).unwrap_or(today), evening };
        let phrase = format(when, today).unwrap_or_default();
        let text = format!("Сдать отчёт {prep} {h}:{m:02} {phrase}");
        let q = split_title(&text, today);
        prop_assert_eq!((q.title, q.when), (text.as_str(), None), "«{}»", text);
        let title = format!("Созвон в {h}:{m:02}");
        let text = format!("{title} {phrase}");
        let q = split_title(&text, today);
        prop_assert_eq!((q.title, q.when), (title.as_str(), Some(when)), "«{}»", text);
    }

    /// «через N …» с N вне 1..=999 – ошибка количества, и быстрый ввод не
    /// отрезает от неё «вечером».
    #[test]
    fn bad_count_is_never_a_date(
        today in date_today(),
        n in prop_oneof![-1000i64..=0, 1000i64..100_000],
        unit in prop::sample::select(vec!["дней", "недель", "месяцев", "лет"]),
    ) {
        let phrase = format!("через {n} {unit}");
        prop_assert_eq!(parse(&phrase, today), Err(ParseError::InvalidCount(phrase.clone())));
        let text = format!("Позвонить {phrase} вечером");
        let q = split_title(&text, today);
        prop_assert_eq!((q.title, q.when), (text.as_str(), None), "«{}»", text);
    }

    /// Промежуток времени после «с»: с любым тире и любыми пробелами вокруг
    /// него «с 18:00 – 19:00 завтра» остаётся названием; без «с» дата
    /// отделяется, а промежуток остаётся в названии.
    #[test]
    fn quick_entry_time_range(
        today in date_today(),
        dash in prop::sample::select(vec!["–", "\u{2014}", "-", "−", "‒", "\u{2015}"]),
        (left, right) in (0usize..3, 0usize..3),
        (h1, h2) in (0u8..24, 0u8..24),
        offset in 0i64..400,
        evening: bool,
    ) {
        let when = When::On { date: today.checked_add(offset.days()).unwrap_or(today), evening };
        let phrase = format(when, today).unwrap_or_default();
        let range = format!("{h1}:00{}{dash}{}{h2}:30", " ".repeat(left), " ".repeat(right));
        let text = format!("Дежурство с {range} {phrase}");
        let q = split_title(&text, today);
        prop_assert_eq!((q.title, q.when), (text.as_str(), None), "«{}»", text);
        let title = format!("Дежурство {range}");
        let text = format!("{title} {phrase}");
        let q = split_title(&text, today);
        prop_assert_eq!((q.title, q.when), (title.as_str(), Some(when)), "«{}»", text);
    }

    /// «через» с дробным, словесным или половинным количеством или с часами
    /// и минутами – Unsupported, и быстрый ввод не отрезает «вечером».
    #[test]
    fn fractional_or_time_offset_is_unsupported(
        today in date_today(),
        count in prop::sample::select(vec![
            "1,5", "2.5", "0,5", "полтора", "полторы", "пол", "четверть", "двадцать", "двадцать пять", "",
        ]),
        unit in prop::sample::select(vec![
            "час", "часа", "часов", "минут", "минуты", "секунд", "дня", "недели", "месяца", "года",
        ]),
    ) {
        let is_date_unit = ["дня", "недели", "месяца", "года"].contains(&unit);
        // «через дня» – не выражение вовсе: у единицы даты нужно количество.
        let count = if count.is_empty() && is_date_unit { "полтора" } else { count };
        let phrase = if count.is_empty() { format!("через {unit}") } else { format!("через {count} {unit}") };
        prop_assert_eq!(parse(&phrase, today), Err(ParseError::Unsupported(phrase.clone())));
        let text = format!("Позвонить {phrase} вечером");
        let q = split_title(&text, today);
        prop_assert_eq!((q.title, q.when), (text.as_str(), None), "«{}»", text);
    }

    /// Составное количество словами любой длины («сто двадцать пять»,
    /// «две тысячи триста», «два с половиной») – Unsupported, и быстрый ввод
    /// сохраняет «… вечером» целиком.
    #[test]
    fn compound_quantity_is_unsupported(
        today in date_today(),
        parts in prop::collection::vec(
            prop::sample::select(vec![
                "сто", "двести", "тысяча", "тысячи", "двадцать", "тридцать", "сорок",
                "девяносто", "одиннадцать", "пятнадцать", "пять", "две", "три", "полтора",
            ]),
            2..7,
        ),
        half: bool,
        unit in prop::sample::select(vec![
            "дней", "дня", "недель", "месяцев", "лет", "часов", "минут",
        ]),
    ) {
        let mut count = parts.join(" ");
        if half {
            count.push_str(" с половиной");
        }
        let phrase = format!("через {count} {unit}");
        prop_assert_eq!(parse(&phrase, today), Err(ParseError::Unsupported(phrase.clone())));
        let text = format!("Напомнить {phrase} вечером");
        let q = split_title(&text, today);
        prop_assert_eq!((q.title, q.when), (text.as_str(), None), "«{}»", text);
    }

    /// Обычное слово после «через» – часть названия; настоящий «через N …»
    /// после него отделяется.
    #[test]
    fn ordinary_word_after_through_stays_in_title(
        today in early_today(),
        noun in prop::sample::select(vec![
            "банк", "Москву", "парк", "мост", "пол", "сайт", "Госуслуги", "окно",
        ]),
        n in 1i64..=999,
        unit in prop::sample::select(vec!["дней", "недель", "месяцев"]),
    ) {
        // До `MAX_DATE` от `early_today` – больше 8000 дней: 240 месяцев влезают.
        let n = if unit == "месяцев" { n % 240 + 1 } else { n };
        let title = format!("Оплатить через {noun}");
        let text = format!("{title} через {n} {unit}");
        let span = match unit {
            "дней" => n.days(),
            "недель" => (n * 7).days(),
            _ => n.months(),
        };
        let want = today.checked_add(span).ok().map(|date| When::On { date, evening: false });
        let q = split_title(&text, today);
        prop_assert_eq!((q.title, q.when), (title.as_str(), want), "«{}»", text);
    }

    /// Любое слово количества перед единицей («1/2», «½», «2-3», «3-х»,
    /// «половину», «две трети», «два с половиной») – и после «через», и как
    /// длительность в часах и минутах – не даёт отрезать «вечером»,
    /// «завтра» или «в пятницу». Для «через» – после любой единицы,
    /// календарной тоже.
    #[test]
    fn quantity_before_unit_never_splits(
        today in date_today(),
        count in prop_oneof![
            (1u32..100, 1u32..100).prop_map(|(a, b)| format!("{a}/{b}")),
            (0u32..100, 1u32..10).prop_map(|(a, b)| format!("{a},{b}")),
            (1u32..20, 1u32..20).prop_map(|(a, b)| format!("{a}-{b}")),
            (1u32..20, 1u32..20).prop_map(|(a, b)| format!("{a}\u{2013}{b}")),
            (1u32..20).prop_map(|a| format!("{a}-х")),
            prop::sample::select(vec![
                "½", "¼", "¾", "⅓", "половину", "две трети", "треть", "четверть",
                "два с половиной", "три с четвертью", "одну-две", "сто двадцать пять",
                "полтора", "несколько",
            ]).prop_map(str::to_owned),
        ],
        unit in prop::sample::select(vec![
            "дня", "дней", "недели", "месяца", "лет", "часа", "часов", "минут", "минуты",
        ]),
        tail in prop::sample::select(vec!["вечером", "завтра", "в пятницу"]),
    ) {
        let phrase = format!("через {count} {unit}");
        prop_assert!(matches!(parse(&phrase, today), Err(ParseError::Unsupported(_))), "«{}»", phrase);
        for tail in ["вечером", "завтра", "в пятницу"] {
            let text = format!("Напомнить {phrase} {tail}");
            let q = split_title(&text, today);
            prop_assert_eq!((q.title, q.when), (text.as_str(), None), "«{}»", text);
        }
        if ["часа", "часов", "минут", "минуты"].contains(&unit) {
            let text = format!("Тренировка {count} {unit} {tail}");
            let q = split_title(&text, today);
            prop_assert_eq!((q.title, q.when), (text.as_str(), None), "«{}»", text);
        }
    }
}

/// Опорная дата для быстрого ввода: до 400 дней от неё есть в диапазоне.
fn date_today() -> impl Strategy<Value = Date> {
    (0i64..SPAN - 401).prop_map(from_min)
}
