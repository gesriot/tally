//! Вывод: даты по-русски, короткие номера задач, строки списков.

use jiff::civil::Date;
use tally_core::lists::{Group, List};

const WEEKDAYS: [&str; 7] = ["пн", "вт", "ср", "чт", "пт", "сб", "вс"];
const MONTHS_GENITIVE: [&str; 12] = [
    "января",
    "февраля",
    "марта",
    "апреля",
    "мая",
    "июня",
    "июля",
    "августа",
    "сентября",
    "октября",
    "ноября",
    "декабря",
];
const MONTHS: [&str; 12] = [
    "Январь",
    "Февраль",
    "Март",
    "Апрель",
    "Май",
    "Июнь",
    "Июль",
    "Август",
    "Сентябрь",
    "Октябрь",
    "Ноябрь",
    "Декабрь",
];

fn month_index(month: i8) -> usize {
    usize::try_from(month - 1).unwrap_or(0).min(11)
}

/// «пт, 20 ноября»; год – если он не текущий: «ср, 15 декабря 2027».
pub fn date(d: Date, today: Date) -> String {
    let wd = usize::try_from(d.weekday().to_monday_zero_offset()).unwrap_or(0);
    let month = MONTHS_GENITIVE[month_index(d.month())];
    let mut out = format!("{}, {} {month}", WEEKDAYS[wd.min(6)], d.day());
    if d.year() != today.year() {
        out.push_str(&format!(" {}", d.year()));
    }
    out
}

/// Название группы «Планов».
pub fn group(g: Group, today: Date) -> String {
    match g {
        Group::Day(d) => date(d, today),
        Group::RestOfMonth { month, .. } => {
            format!("До конца {}", MONTHS_GENITIVE[month_index(month)])
        }
        Group::Month { year, month } => {
            let name = MONTHS[month_index(month)];
            if year == today.year() {
                name.to_owned()
            } else {
                format!("{name} {year}")
            }
        }
        Group::Year(year) => format!("{year} год"),
    }
}

/// Где запись: «Сегодня», «Сегодня, вечер», «пт, 20 ноября», «Входящие» …
/// `evening` – вечер дня `date`: «пт, 20 ноября, вечер».
pub fn place(list: List, date: Option<Date>, evening: bool, today: Date) -> String {
    match list {
        List::Inbox => "Входящие".to_owned(),
        List::Today { evening: false } => "Сегодня".to_owned(),
        List::Today { evening: true } => "Сегодня, вечер".to_owned(),
        List::Upcoming => match date {
            Some(d) if evening => format!("{}, вечер", self::date(d, today)),
            Some(d) => self::date(d, today),
            None => "Планы".to_owned(),
        },
        List::Anytime => "В любое время".to_owned(),
        List::Someday => "Когда-нибудь".to_owned(),
        List::Logbook => "Журнал".to_owned(),
        List::Trash => "Корзина".to_owned(),
    }
}

/// Пометка срока: «⚑ сегодня», «⚑ пт, 20 ноября», «⚑ просрочено: вт, 17 ноября».
pub fn deadline(d: Date, today: Date) -> String {
    if d == today {
        "⚑ сегодня".to_owned()
    } else if d < today {
        format!("⚑ просрочено: {}", date(d, today))
    } else {
        format!("⚑ {}", date(d, today))
    }
}

/// Короткие номера – общие с `tally-app` (по ним же он ищет задачи).
pub use tally_app::{MIN_ID, hex, id_len, short};

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date as d;
    use uuid::Uuid;

    const TODAY: Date = d(2026, 11, 19);

    #[test]
    fn places_name_the_list_or_the_day() {
        let fri = Some(d(2026, 11, 20));
        assert_eq!(place(List::Upcoming, fri, false, TODAY), "пт, 20 ноября");
        assert_eq!(
            place(List::Upcoming, fri, true, TODAY),
            "пт, 20 ноября, вечер"
        );
        assert_eq!(
            place(List::Today { evening: true }, Some(TODAY), true, TODAY),
            "Сегодня, вечер"
        );
        assert_eq!(place(List::Inbox, None, false, TODAY), "Входящие");
        assert_eq!(place(List::Someday, None, false, TODAY), "Когда-нибудь");
    }

    #[test]
    fn dates_in_russian() {
        assert_eq!(date(d(2026, 11, 20), TODAY), "пт, 20 ноября");
        assert_eq!(date(d(2027, 12, 15), TODAY), "ср, 15 декабря 2027");
        assert_eq!(deadline(TODAY, TODAY), "⚑ сегодня");
        assert_eq!(
            deadline(d(2026, 11, 17), TODAY),
            "⚑ просрочено: вт, 17 ноября"
        );
    }

    #[test]
    fn upcoming_groups() {
        assert_eq!(group(Group::Day(d(2026, 11, 21)), TODAY), "сб, 21 ноября");
        assert_eq!(
            group(
                Group::RestOfMonth {
                    year: 2026,
                    month: 11
                },
                TODAY
            ),
            "До конца ноября"
        );
        assert_eq!(
            group(
                Group::Month {
                    year: 2026,
                    month: 12
                },
                TODAY
            ),
            "Декабрь"
        );
        assert_eq!(
            group(
                Group::Month {
                    year: 2027,
                    month: 1
                },
                TODAY
            ),
            "Январь 2027"
        );
        assert_eq!(group(Group::Year(2028), TODAY), "2028 год");
    }

    #[test]
    fn short_ids_stay_unique() {
        let a = Uuid::from_u128(0xabcdef12_0000_0000_0000_000000000000);
        let b = Uuid::from_u128(0xabcdef13_0000_0000_0000_000000000000);
        let c = Uuid::from_u128(0x12345678_0000_0000_0000_000000000000);
        assert_eq!(id_len([c]), MIN_ID);
        assert_eq!(id_len([a, c]), MIN_ID);
        assert_eq!(id_len([a, b, c]), 8);
        assert_eq!(short(a, 8), "abcdef12");
    }
}
