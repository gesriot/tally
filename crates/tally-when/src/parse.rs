//! Грамматика (слова уже нормализованы, см. README.md):
//!
//! ```text
//! выражение = [«вечером»] основа | основа «вечером» | «вечером»
//! основа    = «когда-нибудь» | «в любое время»
//!           | [«на»] день
//! день      = «сегодня» | «завтра» | «послезавтра»
//!           | «через» [число] единица
//!           | [«в»|«во»] [«следующий»] день-недели
//!           | «на следующей неделе» | «на выходных» | «в выходные»
//!           | число месяц [год] | ДД.ММ[.ГГ[ГГ]]
//! ```

use jiff::ToSpan;
use jiff::civil::Date;

use crate::token::tokenize;
use crate::words::{self, Unit};
use crate::{MAX_DATE, MIN_DATE, ParseError, When};

/// Разобрать всё выражение целиком относительно `today`.
///
/// ```
/// use jiff::civil::date;
/// use tally_when::{When, parse};
/// let today = date(2026, 11, 19); // четверг
/// assert_eq!(
///     parse("в пятницу вечером", today),
///     Ok(When::On { date: date(2026, 11, 20), evening: true })
/// );
/// ```
pub fn parse(text: &str, today: Date) -> Result<When, ParseError> {
    let tokens = tokenize(text);
    let words: Vec<&str> = tokens.iter().map(|t| t.text.as_str()).collect();
    parse_words(&words, today)
}

/// Разбор уже разбитых и нормализованных слов – для [`crate::split_title`].
pub(crate) fn parse_words_at(words: &[&str], today: Date) -> Result<When, ParseError> {
    parse_words(words, today)
}

/// Что даёт основа выражения.
enum Core {
    Day(Date),
    Anytime,
    Someday,
}

fn parse_words(words: &[&str], today: Date) -> Result<When, ParseError> {
    if words.is_empty() {
        return Err(ParseError::Empty);
    }
    // «вечером» – в начале или в конце, один раз.
    let (evening, core) = match words {
        ["вечером", rest @ ..] => (true, rest),
        [rest @ .., "вечером"] => (true, rest),
        _ => (false, words),
    };
    if evening && core.is_empty() {
        return in_range(today).map(|date| When::On {
            date,
            evening: true,
        });
    }
    match parse_core(core, today)? {
        Core::Day(date) => in_range(date).map(|date| When::On { date, evening }),
        Core::Anytime | Core::Someday if evening => {
            Err(ParseError::EveningWithoutDate(core.join(" ")))
        }
        Core::Anytime => Ok(When::Anytime),
        Core::Someday => Ok(When::Someday),
    }
}

/// День в пределах [`MIN_DATE`]..=[`MAX_DATE`].
fn in_range(date: Date) -> Result<Date, ParseError> {
    if (MIN_DATE..=MAX_DATE).contains(&date) {
        Ok(date)
    } else {
        Err(ParseError::OutOfRange)
    }
}

fn unknown(words: &[&str]) -> ParseError {
    ParseError::Unknown(words.join(" "))
}

fn parse_core(words: &[&str], today: Date) -> Result<Core, ParseError> {
    match words {
        ["когда-нибудь"] | ["когда", "нибудь"] => return Ok(Core::Someday),
        ["в", "любое", "время"] | ["любое", "время"] => {
            return Ok(Core::Anytime);
        }
        _ => {}
    }
    // «на завтра», «на пятницу», «на 15 декабря»; но «на следующей неделе»
    // и «на выходных» – цельные выражения.
    let day = match words {
        ["на", rest @ ..] if !rest.is_empty() => parse_day(words, today).or_else(|whole| {
            if !matches!(whole, ParseError::Unknown(_)) {
                return Err(whole);
            }
            parse_day(rest, today).map_err(|e| match e {
                ParseError::Unknown(_) => whole,
                other => other,
            })
        }),
        _ => parse_day(words, today),
    };
    day.map(Core::Day)
}

fn parse_day(words: &[&str], today: Date) -> Result<Date, ParseError> {
    let add_days = |n: i64| {
        today
            .checked_add(n.days())
            .map_err(|_| ParseError::OutOfRange)
    };
    match words {
        ["сегодня"] => Ok(today),
        ["завтра"] => add_days(1),
        ["послезавтра"] => add_days(2),
        ["через", rest @ ..] => {
            offset(rest, today).unwrap_or_else(|bad| Err(bad(words.join(" "))))
        }
        ["на", "следующей", "неделе"] => {
            // Понедельник следующей календарной недели.
            let monday_offset = i64::from(today.weekday().to_monday_zero_offset());
            add_days(7 - monday_offset)
        }
        ["на", "выходных"] | ["в", "выходные"] => {
            // Ближайшая суббота; в субботу и воскресенье – сегодня.
            let wd = i64::from(today.weekday().to_monday_zero_offset());
            add_days(if wd >= 5 { 0 } else { 5 - wd })
        }
        _ if is_past_phrase(words) => Err(ParseError::Past(words.join(" "))),
        _ => {
            if let Some(d) = weekday_phrase(words, today) {
                return d;
            }
            calendar(words, today).unwrap_or_else(|| Err(unknown(words)))
        }
    }
}

/// Ошибка распознанной, но неверной конструкции; ей нужен текст выражения.
type Bad = fn(String) -> ParseError;

/// «через [N] единица»: N от 1 до 999, без числа – 1.
///
/// `Err` – не дата:
/// - не «через N …» вовсе («через Москву», «через 2 дома»):
///   [`ParseError::Unknown`];
/// - целое число вне 1..=999 («через 0 дней», «через -1 день»):
///   [`ParseError::InvalidCount`];
/// - часы и минуты с любым количеством («через 2 часа», «через полтора
///   часа», «через 1,5 часа», «через четверть часа»), дробное, составное
///   или словесное количество дней, недель, месяцев, лет («через 1,5 дня»,
///   «через два с половиной дня», «через сто двадцать пять дней», «через
///   двадцать дней», «через полгода»): [`ParseError::Unsupported`].
///
/// Количество – только слова количества (`words::quantity`), любой длины:
/// в «через банк через неделю» «банк через» – не количество, и «через»
/// здесь обычное слово.
fn offset(words: &[&str], today: Date) -> Result<Result<Date, ParseError>, Bad> {
    use words::Quantity;
    let (count, u) = match words {
        [u] if words::is_time_unit(u) || words::is_half_unit(u) => {
            return Err(ParseError::Unsupported);
        }
        [u] => (Quantity::Whole(1), *u),
        [count @ .., u] => (
            words::quantity(count).ok_or(ParseError::Unknown as Bad)?,
            *u,
        ),
        [] => return Err(ParseError::Unknown),
    };
    if words::is_time_unit(u) {
        return Err(ParseError::Unsupported);
    }
    let unit = words::unit(u).ok_or(ParseError::Unknown as Bad)?;
    let n = match count {
        Quantity::Whole(n) if (1..=999).contains(&n) => n,
        Quantity::Whole(_) => return Err(ParseError::InvalidCount),
        Quantity::Other => return Err(ParseError::Unsupported),
    };
    let span = match unit {
        Unit::Day => n.days(),
        Unit::Week => (n * 7).days(),
        Unit::Month => n.months(),
        Unit::Year => n.years(),
    };
    Ok(today.checked_add(span).map_err(|_| ParseError::OutOfRange))
}

/// Прошедший день относительно сегодня: «вчера», «позавчера», «прошлый пн»,
/// «в прошлую пятницу», «на прошлой неделе», «3 дня назад», «неделю назад».
fn is_past_phrase(words: &[&str]) -> bool {
    let words = match words {
        ["в" | "во" | "на", rest @ ..] => rest,
        _ => words,
    };
    match words {
        ["вчера" | "позавчера"] => true,
        [p, what] if words::is_past(p) => {
            words::weekday(what).is_some()
                || matches!(
                    *what,
                    "неделе"
                        | "неделю"
                        | "выходных"
                        | "выходные"
                        | "месяц"
                        | "месяце"
                        | "году"
                        | "год"
                )
        }
        [u, "назад"] => words::unit(u).is_some() || words::is_time_unit(u),
        [count @ .., u, "назад"] => {
            words::quantity(count).is_some() && (words::unit(u).is_some() || words::is_time_unit(u))
        }
        _ => false,
    }
}

/// «пт», «в пятницу», «во вторник», «следующий пн», «в следующую среду».
///
/// Просто день недели – ближайший такой день после сегодня (сегодняшний –
/// через неделю). «Следующий» – этот день следующей календарной недели
/// (недели с понедельника).
fn weekday_phrase(words: &[&str], today: Date) -> Option<Result<Date, ParseError>> {
    let words = match words {
        ["в" | "во", rest @ ..] => rest,
        _ => words,
    };
    let (next, day) = match words {
        [day] => (false, words::weekday(day)?),
        [n, day] if words::is_next(n) => (true, words::weekday(day)?),
        _ => return None,
    };
    let wd = today.weekday().to_monday_zero_offset();
    let days = if next {
        // До понедельника следующей недели плюс номер дня.
        7 - wd + day
    } else {
        (day - wd - 1).rem_euclid(7) + 1
    };
    Some(
        today
            .checked_add(i64::from(days).days())
            .map_err(|_| ParseError::OutOfRange),
    )
}

/// «15 дек», «15 декабря 2027», «15 декабря 2027 г.», «15.12», «15.12.27».
fn calendar(words: &[&str], today: Date) -> Option<Result<Date, ParseError>> {
    let (day, month, year) = match words {
        [dotted] => dotted_date(dotted)?,
        [d, m] => (day_number(d)?, words::month(m)?, None),
        [d, m, y] | [d, m, y, "г" | "года"] => {
            (day_number(d)?, words::month(m)?, Some(year_number(y)?))
        }
        _ => return None,
    };
    Some(resolve(day, month, year, today))
}

fn day_number(word: &str) -> Option<u8> {
    if word.is_empty() || word.len() > 2 || !word.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    word.parse().ok().filter(|d| (1..=31).contains(d))
}

/// Год: четыре цифры, можно с «г» слитно («2027г»). Год до 1000-го
/// («0999») разбирается, но даёт [`ParseError::OutOfRange`].
fn year_number(word: &str) -> Option<i16> {
    let digits = word.strip_suffix('г').unwrap_or(word);
    if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn dotted_date(word: &str) -> Option<(u8, u8, Option<i16>)> {
    let mut parts = word.split('.');
    let day = day_number(parts.next()?)?;
    let month_text = parts.next()?;
    if month_text.is_empty()
        || month_text.len() > 2
        || !month_text.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let month: u8 = month_text.parse().ok().filter(|m| (1..=12).contains(m))?;
    let year = match parts.next() {
        None => None,
        Some(y) if y.len() == 2 && y.bytes().all(|b| b.is_ascii_digit()) => {
            Some(2000 + y.parse::<i16>().ok()?)
        }
        Some(y) => Some(year_number(y)?),
    };
    if parts.next().is_some() {
        return None;
    }
    Some((day, month, year))
}

/// День и месяц без года – ближайший такой день, начиная с сегодня
/// (прошедший в этом году – в следующем; 29 февраля – в ближайшем
/// високосном). С годом – ровно этот день, даже прошедший.
fn resolve(day: u8, month: u8, year: Option<i16>, today: Date) -> Result<Date, ParseError> {
    let invalid = ParseError::InvalidDate { day, month, year };
    let (d, m) = (day as i8, month as i8);
    match year {
        Some(y) if !(MIN_DATE.year()..=MAX_DATE.year()).contains(&y) => Err(ParseError::OutOfRange),
        Some(y) => Date::new(y, m, d).map_err(|_| invalid),
        None => (today.year()..=today.year().saturating_add(8))
            .filter_map(|y| Date::new(y, m, d).ok())
            .find(|date| *date >= today)
            // Такой день бывает (в високосном 2000-м есть и 29 февраля), но
            // ближайший – после `MAX_DATE`.
            .ok_or(if Date::new(2000, m, d).is_ok() {
                ParseError::OutOfRange
            } else {
                invalid
            }),
    }
}
