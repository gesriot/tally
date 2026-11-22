//! Словари: месяцы, дни недели, числительные, единицы.

/// Месяц по слову: полное (любой падеж из употребительных) или сокращение.
pub(crate) fn month(word: &str) -> Option<u8> {
    const MONTHS: [&[&str]; 12] = [
        &["январь", "января", "янв"],
        &["февраль", "февраля", "фев", "февр"],
        &["март", "марта", "мар"],
        &["апрель", "апреля", "апр"],
        &["май", "мая"],
        &["июнь", "июня", "июн"],
        &["июль", "июля", "июл"],
        &["август", "августа", "авг"],
        &["сентябрь", "сентября", "сен", "сент"],
        &["октябрь", "октября", "окт"],
        &["ноябрь", "ноября", "ноя", "нояб"],
        &["декабрь", "декабря", "дек"],
    ];
    (1u8..)
        .zip(MONTHS)
        .find_map(|(n, forms)| forms.contains(&word).then_some(n))
}

/// Месяц в родительном падеже – для [`crate::format`].
pub(crate) fn month_genitive(month: u8) -> &'static str {
    const NAMES: [&str; 12] = [
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
    NAMES
        .get(usize::from(month).saturating_sub(1))
        .copied()
        .unwrap_or("")
}

/// День недели: 0 – понедельник … 6 – воскресенье. Именительный и
/// винительный падежи («в пятницу») и сокращения.
pub(crate) fn weekday(word: &str) -> Option<i8> {
    const DAYS: [&[&str]; 7] = [
        &["понедельник", "пн", "пон"],
        &["вторник", "вт"],
        &["среда", "среду", "ср"],
        &["четверг", "чт"],
        &["пятница", "пятницу", "пт"],
        &["суббота", "субботу", "сб"],
        &["воскресенье", "вс", "вск"],
    ];
    (0i8..)
        .zip(DAYS)
        .find_map(|(n, forms)| forms.contains(&word).then_some(n))
}

/// «Следующий» в любом роде и сокращение.
pub(crate) fn is_next(word: &str) -> bool {
    matches!(
        word,
        "следующий" | "следующая" | "следующую" | "следующее" | "след"
    )
}

/// Число словом или цифрами: «3», «три», «пару».
pub(crate) fn number(word: &str) -> Option<u32> {
    if !word.is_empty() && word.len() <= 4 && word.bytes().all(|b| b.is_ascii_digit()) {
        return word.parse().ok();
    }
    Some(match word {
        "один" | "одну" | "одна" | "одно" => 1,
        "два" | "две" | "пару" | "пара" => 2,
        "три" => 3,
        "четыре" => 4,
        "пять" => 5,
        "шесть" => 6,
        "семь" => 7,
        "восемь" => 8,
        "девять" => 9,
        "десять" => 10,
        _ => return None,
    })
}

/// Количество в «через N …» – и неверное: «0», «1000», «-1» (для
/// [`crate::ParseError::InvalidCount`]). Слишком длинное число – `i64::MAX`.
pub(crate) fn count(word: &str) -> Option<i64> {
    let (sign, digits) = match word.strip_prefix(['-', '−', '+']) {
        Some(rest) if word.starts_with('+') => (1, rest),
        Some(rest) => (-1, rest),
        None => (1, word),
    };
    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        return Some(sign * digits.parse::<i64>().unwrap_or(i64::MAX));
    }
    if sign < 0 {
        return None;
    }
    number(word).map(i64::from)
}

/// Единицы времени суток в любом падеже – «через 2 часа», «через
/// полтора часа» распознаются, но не поддержаны.
pub(crate) fn is_time_unit(word: &str) -> bool {
    matches!(
        word,
        "час"
            | "часа"
            | "часов"
            | "часу"
            | "часам"
            | "часик"
            | "часика"
            | "ч"
            | "полчаса"
            | "полчасика"
            | "минута"
            | "минуту"
            | "минуты"
            | "минут"
            | "минутку"
            | "минутки"
            | "минуток"
            | "мин"
            | "полминуты"
            | "секунда"
            | "секунду"
            | "секунды"
            | "секунд"
            | "сек"
    )
}

/// Количество перед единицей в «через N …» и «N … назад».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Quantity {
    /// Целое число, которое понимает разбор: цифры («3», «0», «-1») или
    /// одно слово от «один» до «десять», «пару».
    Whole(i64),
    /// Количество, которое разбор не считает: дробь («1,5», «полтора»,
    /// «два с половиной»), составное или большое числительное («сто двадцать
    /// пять», «двадцать»), «несколько».
    Other,
}

/// Слово-числительное любого размера (кроме тех, что понимает [`number`]).
fn is_number_word(word: &str) -> bool {
    const WORDS: [&str; 55] = [
        "ноль",
        "нуль",
        "одиннадцать",
        "двенадцать",
        "тринадцать",
        "четырнадцать",
        "пятнадцать",
        "шестнадцать",
        "семнадцать",
        "восемнадцать",
        "девятнадцать",
        "двадцать",
        "тридцать",
        "сорок",
        "пятьдесят",
        "шестьдесят",
        "семьдесят",
        "восемьдесят",
        "девяносто",
        "сто",
        "двести",
        "триста",
        "четыреста",
        "пятьсот",
        "шестьсот",
        "семьсот",
        "восемьсот",
        "девятьсот",
        "тысяча",
        "тысячу",
        "тысячи",
        "тысяч",
        "миллион",
        "миллиона",
        "миллионов",
        "полтора",
        "полторы",
        "пол",
        "четверть",
        "несколько",
        "пара",
        "пару",
        "один",
        "одну",
        "одна",
        "одно",
        "два",
        "две",
        "три",
        "четыре",
        "пять",
        "шесть",
        "семь",
        "восемь",
        "девять",
    ];
    const MORE: [&str; 14] = [
        "десять",
        "половину",
        "половина",
        "половинку",
        "треть",
        "трети",
        "третью",
        "четверти",
        "десяток",
        "десятка",
        "дюжину",
        "дюжина",
        "тройку",
        "пяток",
    ];
    WORDS.contains(&word) || MORE.contains(&word)
}

/// Число на вид: цифры или дробь знаком с разделителями («1/2», «2-3»,
/// «1,5–2», «½») и, может быть, падежным окончанием («3-х», «5-ти»).
/// Год слитно с «г» («2027г») – тоже. «слово5» и «отчёт2.pdf» – не числа.
fn is_numeric_like(word: &str) -> bool {
    let body = ["-х", "х", "-ти", "-ми", "-и", "г"]
        .iter()
        .find_map(|end| word.strip_suffix(end))
        .unwrap_or(word);
    body.chars()
        .any(|c| c.is_ascii_digit() || is_vulgar_fraction(c))
        && body.chars().all(|c| {
            c.is_ascii_digit()
                || is_vulgar_fraction(c)
                || is_dash(c)
                || matches!(c, ',' | '.' | '/' | '+')
        })
}

/// Дробь знаком: «½», «¼», «⅔» …
fn is_vulgar_fraction(c: char) -> bool {
    matches!(c, '\u{BC}'..='\u{BE}' | '\u{2150}'..='\u{215F}' | '\u{2189}')
}

/// Число цифрами, в том числе дробное: «3», «-1», «1,5», «2.5».
fn is_digit_number(word: &str) -> bool {
    let body = word.strip_prefix(['-', '−', '+']).unwrap_or(word);
    let mut parts = body.splitn(2, [',', '.']);
    let whole = parts.next().unwrap_or("");
    let ok = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    ok(whole) && parts.next().is_none_or(ok)
}

/// Слово количества: число цифрами или числительное; число на вид с
/// дробью или промежутком («1/2», «2-3», «3-х», «½») и промежуток
/// числительных через дефис («одну-две»).
pub(crate) fn is_quantity_word(word: &str) -> bool {
    is_digit_number(word)
        || is_number_word(word)
        || is_numeric_like(word)
        || (word.contains(is_dash)
            && word
                .split(is_dash)
                .all(|part| !part.is_empty() && (is_number_word(part) || number(part).is_some())))
}

/// Количество из слов: только слова количества ([`is_quantity_word`]),
/// между ними – «с половиной», «с четвертью» («два с половиной», «сто
/// двадцать пять», «1,5», «1/2», «две трети»). Любое другое слово – не
/// количество («банк через»).
pub(crate) fn quantity(words: &[&str]) -> Option<Quantity> {
    match words {
        [] => return None,
        [w] => {
            if let Some(n) = count(w) {
                return Some(Quantity::Whole(n));
            }
            return is_quantity_word(w).then_some(Quantity::Other);
        }
        _ => {}
    }
    let mut rest = words;
    while let [w, tail @ ..] = rest {
        rest = match (*w, tail) {
            ("с", ["половиной" | "четвертью", more @ ..]) => more,
            (w, _) if is_quantity_word(w) => tail,
            _ => return None,
        };
    }
    // «с половиной» в начале – не количество.
    (words[0] != "с").then_some(Quantity::Other)
}

/// Время суток как длительность или момент с самим количеством:
/// «час», «полчаса», «минуту» – и без числа означают «один».
pub(crate) fn is_counted_time_unit(word: &str) -> bool {
    matches!(
        word,
        "час"
            | "часик"
            | "полчаса"
            | "полчасика"
            | "минуту"
            | "минутку"
            | "полминуты"
            | "секунду"
    )
}

/// Половина дня, недели, месяца или года одним словом: «через полгода».
pub(crate) fn is_half_unit(word: &str) -> bool {
    matches!(word, "полдня" | "полнедели" | "полмесяца" | "полгода")
}

/// «прошлый» в любом роде и падеже.
pub(crate) fn is_past(word: &str) -> bool {
    word.strip_prefix("прошл").is_some_and(|end| {
        matches!(
            end,
            "ый" | "ая" | "ую" | "ое" | "ой" | "ого" | "ом" | "ые" | "ых" | "ым" | "ому"
        )
    })
}

/// Тире или дефис: «18:00 – 19:00».
pub(crate) fn is_dash(c: char) -> bool {
    matches!(
        c,
        '-' | '‐' | '‑' | '‒' | '–' | '\u{2014}' | '\u{2015}' | '−'
    )
}

/// Время суток или промежуток: «18:00», «9:30», «18:00–19:00», «-19:00».
/// Часы 0–23, минуты – две цифры 00–59. Пропорция «2:1» или «18:7» –
/// не время.
pub(crate) fn is_time(word: &str) -> bool {
    let mut clocks = 0;
    let parts: Vec<&str> = word.split(is_dash).collect();
    for (k, part) in parts.iter().enumerate() {
        if part.is_empty() {
            // Тире – только в начале или в конце («-19:00», «18:00-»).
            if k != 0 && k != parts.len() - 1 {
                return false;
            }
            continue;
        }
        if !is_clock(part) {
            return false;
        }
        clocks += 1;
    }
    (1..=2).contains(&clocks)
}

/// «18:00», «9:30», «0:05».
fn is_clock(part: &str) -> bool {
    let Some((h, m)) = part.split_once(':') else {
        return false;
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    digits(h)
        && h.len() <= 2
        && digits(m)
        && m.len() == 2
        && h.parse::<u8>().is_ok_and(|h| h <= 23)
        && m.parse::<u8>().is_ok_and(|m| m <= 59)
}

/// Слово из одних тире: «–» в «18:00 – 19:00».
pub(crate) fn is_dash_word(word: &str) -> bool {
    !word.is_empty() && word.chars().all(is_dash)
}

/// Предлоги, которыми название не может кончаться: если перед найденным
/// выражением стоит такой предлог, а разбор его не принял («в 18.05»,
/// «о 15 декабря»), предлог – часть того же выражения.
pub(crate) fn is_dangling_preposition(word: &str) -> bool {
    matches!(
        word,
        "в" | "во"
            | "на"
            | "о"
            | "об"
            | "обо"
            | "у"
            | "из"
            | "при"
            | "для"
            | "про"
            | "без"
            | "под"
            | "над"
    )
}

/// Единица «через N …».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unit {
    Day,
    Week,
    Month,
    Year,
}

/// Падеж не проверяется: «через 5 день» тоже понятно.
pub(crate) fn unit(word: &str) -> Option<Unit> {
    Some(match word {
        "день" | "дня" | "дней" => Unit::Day,
        "неделю" | "неделя" | "недели" | "недель" => Unit::Week,
        "месяц" | "месяца" | "месяцев" => Unit::Month,
        "год" | "года" | "лет" => Unit::Year,
        _ => return None,
    })
}

/// Слова, после которых найденное в конце названия выражение – не дата
/// начала, а часть другой конструкции: срок («до завтра», «к 15 декабря»),
/// промежуток («с пятницы», «после 15 декабря»), перечисление («завтра или
/// послезавтра»), отрицание («не завтра»), «за пятницу», обрывок даты
/// («32 декабря вечером», «до пятницы вечером»).
pub(crate) fn blocks_date_after(word: &str) -> bool {
    const BEFORE: [&str; 21] = [
        "до",
        "к",
        "ко",
        "по",
        "с",
        "со",
        "после",
        "от",
        "ото",
        "перед",
        "передо",
        "между",
        "за",
        "около",
        "не",
        "и",
        "или",
        "либо",
        "через",
        "кроме",
        "вместо",
    ];
    BEFORE.contains(&word)
        || is_next(word)
        || is_month_form(word)
        || is_weekday_form(word)
        || is_dotted_number(word)
}

/// «32.11», «15.12.2026», «1/12»: только цифры с точками или дробью –
/// похоже на дату. «отчёт2.pdf» и «v1.2» – обычные слова.
fn is_dotted_number(word: &str) -> bool {
    word.bytes().any(|b| b.is_ascii_digit())
        && word.bytes().any(|b| b == b'.' || b == b'/')
        && word
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b'.' || b == b'/')
}

/// Слово с основой из `stems` и окончанием из `endings`.
fn has_form(word: &str, stems: &[&str], endings: &[&str]) -> bool {
    stems.iter().any(|stem| {
        word.strip_prefix(stem)
            .is_some_and(|end| endings.contains(&end))
    })
}

/// Месяц в любом падеже: «декабрь», «декабре», «декабрем», «марте», «мае».
fn is_month_form(word: &str) -> bool {
    month(word).is_some()
        || has_form(
            word,
            &[
                "январ",
                "феврал",
                "апрел",
                "июн",
                "июл",
                "сентябр",
                "октябр",
                "ноябр",
                "декабр",
            ],
            &["ь", "я", "ю", "е", "ем"],
        )
        || has_form(word, &["март", "август"], &["", "а", "у", "е", "ом"])
        || matches!(word, "мае" | "маю" | "маем")
}

/// День недели в любом падеже и числе: «пятницы», «пятнице», «понедельника»,
/// «воскресенья», «субботам».
fn is_weekday_form(word: &str) -> bool {
    weekday(word).is_some()
        || has_form(
            word,
            &["понедельник", "вторник", "четверг"],
            &["", "а", "у", "ом", "е", "и", "ов", "ам", "ами", "ах"],
        )
        || has_form(
            word,
            &["сред", "пятниц", "суббот"],
            &["а", "ы", "е", "у", "ой", "ою", "ам", "ами", "ах"],
        )
        || has_form(
            word,
            &["воскресен"],
            &["ье", "ья", "ью", "ьем", "ьям", "ьями", "ьях", "ий"],
        )
}

/// Слова, из которых состоит любое выражение «когда» – и верное, и
/// распознанное с ошибкой. Окончание, в котором есть другое слово, разбор
/// всегда отвергает как [`crate::ParseError::Unknown`], поэтому быстрый ввод
/// ищет начало выражения только внутри последней цепочки таких слов.
pub(crate) fn is_date_word(word: &str) -> bool {
    matches!(
        word,
        "вечером"
            | "сегодня"
            | "завтра"
            | "послезавтра"
            | "вчера"
            | "позавчера"
            | "когда-нибудь"
            | "когда"
            | "нибудь"
            | "в"
            | "во"
            | "на"
            | "любое"
            | "время"
            | "через"
            | "следующей"
            | "неделе"
            | "выходных"
            | "выходные"
            | "месяце"
            | "году"
            | "г"
            | "года"
            | "назад"
            | "с"
            | "половиной"
            | "четвертью"
    ) || is_next(word)
        || is_past(word)
        || weekday(word).is_some()
        || month(word).is_some()
        || unit(word).is_some()
        || is_time_unit(word)
        || is_half_unit(word)
        || number(word).is_some()
        || is_quantity_word(word)
}
