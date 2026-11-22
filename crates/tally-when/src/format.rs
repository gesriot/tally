//! [`When`] обратно в текст, который [`crate::parse`] разберёт в то же значение.

use jiff::civil::Date;

use crate::parse::parse;
use crate::words::month_genitive;
use crate::{MAX_DATE, MIN_DATE, When};

/// Короткий текст для `when` относительно `today`: «сегодня вечером»,
/// «завтра», «15 декабря», «15 декабря 2027», «когда-нибудь».
/// Год пишется, только если без него получился бы другой день.
///
/// Контракт: если `format(w, today)` вернул `Some(s)`, то
/// `parse(&s, today) == Ok(w)` – с той же опорной датой.
///
/// - `When::Someday` и `When::Anytime` – всегда `Some` («когда-нибудь»,
///   «в любое время»);
/// - `When::On` – `Some`, если день в [`MIN_DATE`]..=[`MAX_DATE`], иначе
///   `None`: такой текст [`parse`] не разобрал бы обратно («1 января 999»).
pub fn format(when: When, today: Date) -> Option<String> {
    Some(match when {
        When::Someday => "когда-нибудь".to_owned(),
        When::Anytime => "в любое время".to_owned(),
        When::On { date, .. } if !(MIN_DATE..=MAX_DATE).contains(&date) => return None,
        When::On { date, evening } => {
            let day = day_text(date, today);
            if evening {
                format!("{day} вечером")
            } else {
                day
            }
        }
    })
}

fn day_text(date: Date, today: Date) -> String {
    let ahead = today.until(date).map(|s| s.get_days()).unwrap_or(i32::MIN);
    match ahead {
        0 => return "сегодня".to_owned(),
        1 => return "завтра".to_owned(),
        2 => return "послезавтра".to_owned(),
        _ => {}
    }
    // `until` у дат считает в днях, поэтому сравнение выше точное.
    let short = format!("{} {}", date.day(), month_genitive(date.month() as u8));
    let same = parse(&short, today).is_ok_and(|w| w.date() == Some(date));
    if same {
        short
    } else {
        format!("{short} {}", date.year())
    }
}
