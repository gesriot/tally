//! Быстрый ввод: выражение «когда» в конце названия задачи.

use jiff::civil::Date;

use crate::parse::parse_words_at;
use crate::token::{Token, tokenize};
use crate::words::{
    blocks_date_after, count, is_counted_time_unit, is_dangling_preposition, is_dash_word,
    is_date_word, is_half_unit, is_quantity_word, is_time, is_time_unit, unit,
};
use crate::{ParseError, When};

/// Название без выражения даты и само выражение, если оно найдено.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuickEntry<'a> {
    pub title: &'a str,
    pub when: Option<When>,
}

/// Найти выражение «когда» в конце `text`: «Купить хлеб завтра» →
/// «Купить хлеб» + завтра. Берётся самое длинное подходящее окончание.
///
/// Если быстрый ввод сомневается, текст целиком остаётся названием и
/// `when = None` – дату пользователь назначит сам:
/// - весь текст – выражение даты («завтра», «завтра вечером»): задача не
///   может остаться без названия;
/// - окончание – ошибочная дата («31 ноября», «31 ноября вечером»,
///   «когда-нибудь вечером»): более короткие окончания не пробуются;
/// - окончание – распознанная, но неверная конструкция: «через 0 дней»,
///   «вчера», «в прошлую пятницу», «через 2 часа»;
/// - такая конструкция стоит прямо перед найденным выражением: «через
///   1/2 дня завтра», «через 0 дней в пятницу»;
/// - перед найденным выражением стоит предлог или слово, с которым это
///   не дата начала («до завтра», «к 15 декабря», «32 декабря вечером»,
///   см. `words::blocks_date_after`); время между ними пропускается
///   («до 18:00 завтра»);
/// - слова в кавычках и рядом с ними – часть названия: «Посмотреть фильм
///   «Послезавтра»» не содержит даты, а «Завтра» вечером – название
///   «Завтра» и сегодня вечером.
///
/// Разделители перед выражением («хлеб, завтра») отбрасываются.
///
/// Время линейно по числу слов: каждое слово просматривается постоянное
/// число раз (см. [`SHORT`] и [`long_starts`]).
pub fn split_title(text: &str, today: Date) -> QuickEntry<'_> {
    let whole = QuickEntry {
        title: text.trim(),
        when: None,
    };
    let tokens = tokenize(text);
    let words: Vec<&str> = tokens.iter().map(|t| t.text.as_str()).collect();
    let n = tokens.len();
    // Разбиение на слова и список слов – по одному просмотру каждого.
    visit(2 * n);
    let letters = letters(&tokens);
    // Выражение – только после последнего слова в кавычках и последнего
    // названия буквы: цитата и буква – часть названия, и с ними название не
    // пустое («Завтра» вечером, «группы В пт»).
    let boundary = (0..n)
        .rev()
        .inspect(|_| visit(1))
        .find(|&i| tokens[i].quoted || letters[i])
        .map_or(0, |i| i + 1);
    // Выражение состоит только из слов даты: начало ищется внутри последней
    // цепочки таких слов – один проход с конца.
    let mut tail = n;
    while tail > boundary && is_date_word(look(words[tail - 1])) {
        tail -= 1;
    }
    // Без кавычек и букв текст целиком – выражение даты («завтра вечером»)
    // или ошибочная дата: это название.
    if boundary == 0 && tail == 0 {
        count_candidate();
        if !matches!(parse(&words, today), Err(ParseError::Unknown(_))) {
            return whole;
        }
    }
    for i in starts(&words, n, tail.max(1)) {
        count_candidate();
        let when = match parse(&words[i..], today) {
            Ok(when) => when,
            Err(ParseError::Unknown(_)) => continue,
            // Окончание – распознанная, но неверная конструкция («31 ноября»,
            // «через 0 дней», «вчера»): её кусок – не дата.
            Err(_) => return whole,
        };
        if invalid_before(&words, tail, i, today) || blocked(&tokens[..i], &letters) {
            return whole;
        }
        let title = text[..tokens[i].start]
            .trim_end_matches(|c: char| c.is_whitespace() || ",;:–-".contains(c))
            .trim_start();
        if title.is_empty() {
            return whole;
        }
        return QuickEntry {
            title,
            when: Some(when),
        };
    }
    whole
}

/// Разбор окончания; каждое его слово – одно посещение.
fn parse(words: &[&str], today: Date) -> Result<When, ParseError> {
    visit(words.len());
    parse_words_at(words, today)
}

/// Обычное выражение – не длиннее этого числа слов («15 декабря 2027 года
/// вечером», «в следующую пятницу вечером»). Длиннее бывают только «через
/// <количество> <единица>» и «<количество> <единица> назад» – количество
/// любой длины.
const SHORT: usize = 8;

/// Начала окончаний `words[..end]` не раньше `lo` по возрастанию: длинные
/// по форме ([`long_starts`]) и все короткие – не длиннее [`SHORT`].
fn starts(words: &[&str], end: usize, lo: usize) -> Vec<usize> {
    let short = lo.max(end.saturating_sub(SHORT));
    let mut out = long_starts(words, end, lo);
    out.retain(|&i| i < short);
    out.extend(short..end);
    out
}

/// Начала длинных окончаний `words[..end]` не раньше `lo`: «через
/// <количество> <единица>» и «<количество> <единица> назад», перед ними –
/// не больше двух слов из «вечером», «в», «во», «на», после них – может
/// быть «вечером». Окончание кончается в `end`, поэтому форма одна: проход
/// назад от `end` по количеству – каждое слово один раз, начал не больше
/// трёх. По возрастанию.
fn long_starts(words: &[&str], end: usize, lo: usize) -> Vec<usize> {
    let is_unit = |w: &str| unit(w).is_some() || is_time_unit(w) || is_half_unit(w);
    let mut e = end;
    if e > lo && look(words[e - 1]) == "вечером" {
        e -= 1;
    }
    // Слова количества перед `to` – назад до `lo`.
    let run = |to: usize| {
        let mut j = to;
        while j > lo && is_quantity_part(look(words[j - 1])) {
            j -= 1;
        }
        j
    };
    let core = if e > lo && is_unit(look(words[e - 1])) {
        let j = run(e - 1);
        (j > lo && look(words[j - 1]) == "через").then(|| j - 1)
    } else if e >= lo + 2 && look(words[e - 1]) == "назад" && is_unit(look(words[e - 2])) {
        let j = run(e - 2);
        // Количество целиком: с его начала, а не с середины.
        (j < e - 2 && !(j > 0 && is_quantity_part(look(words[j - 1])))).then_some(j)
    } else {
        None
    };
    let Some(mut s) = core else {
        return Vec::new();
    };
    let mut out = vec![s];
    while out.len() < 3 && s > lo && matches!(look(words[s - 1]), "вечером" | "в" | "во" | "на")
    {
        s -= 1;
        out.push(s);
    }
    out.reverse();
    out
}

/// Перед выражением `words[i..]` – распознанная, но неверная конструкция,
/// которая кончается прямо перед ним: «через 1/2 дня» в «через 1/2 дня
/// завтра», «через 0 дней», «вчера». Тогда выражение – не отдельная дата,
/// а продолжение этой конструкции. Пробуются окончания `words[tail..i]`
/// теми же правилами, что и окончания текста.
fn invalid_before(words: &[&str], tail: usize, i: usize, today: Date) -> bool {
    starts(words, i, tail).into_iter().any(|k| {
        !matches!(
            parse(&words[k..i], today),
            Ok(_) | Err(ParseError::Unknown(_) | ParseError::Empty)
        )
    })
}

/// Названия букв ([`is_letter`]) – один проход с конца.
fn letters(tokens: &[Token]) -> Vec<bool> {
    let mut out = vec![false; tokens.len()];
    // Следующее слово с буквами написано капсом или его нет.
    let mut caps_after = true;
    for i in (0..tokens.len()).rev() {
        visit(1);
        out[i] = is_letter(tokens, i, caps_after);
        if tokens[i].lower {
            caps_after = false;
        } else if tokens[i].text.chars().any(char::is_alphabetic) {
            caps_after = true;
        }
    }
    out
}

/// Одна заглавная буква не в начале текста – название буквы («витамины
/// группы В», «Витамин С»), а не предлог. Сначала – роль в самой дате:
/// «Г.» после числа – «год» («15 ДЕКАБРЯ 2027 Г.»), «С» перед «половиной»
/// или «четвертью» – часть количества. Затем контекст: после слова капсом
/// («СОЗВОН В 18.05», «ОТЧЁТ К 18:00 ЗАВТРА») буква – предлог, если и
/// дальше текст капсом или без букв; если дальше строчные буквы
/// («ВИТАМИНЫ ГРУППЫ В пт») – капсом набрано только название, и буква –
/// его часть. «К» и «С» прямо перед временем ЧЧ:ММ – предлоги срока и
/// промежутка в любом регистре: «СДАТЬ ОТЧЁТ К 18:00 завтра» остаётся
/// целиком, как и «… к 18:00 завтра»; «ФОРМА К завтра» – буква.
fn is_letter(tokens: &[Token], i: usize, caps_after: bool) -> bool {
    let t = &tokens[i];
    if i == 0 || !t.capital_letter {
        return false;
    }
    // Соседи – предыдущее и следующее слово.
    visit(2);
    let next = tokens.get(i + 1).map(|t| t.text.as_str());
    match t.text.as_str() {
        "г" if count(&tokens[i - 1].text).is_some() => return false,
        "с" if matches!(next, Some("половиной" | "четвертью")) => return false,
        "к" | "с" if next.is_some_and(is_time) => return false,
        _ => {}
    }
    !tokens[i - 1].caps || !caps_after
}

#[cfg(test)]
thread_local! {
    /// Сколько окончаний разобрано – для теста на линейность.
    static CANDIDATES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Сколько раз просмотрены слова – во всех проходах и разборах.
    static VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn count_candidate() {
    #[cfg(test)]
    CANDIDATES.with(|c| c.set(c.get() + 1));
}

#[cfg_attr(not(test), allow(unused_variables))]
fn visit(words: usize) {
    #[cfg(test)]
    VISITS.with(|c| c.set(c.get() + words));
}

/// Просмотр одного слова: каждое чтение слова поиском – одно посещение.
fn look(word: &str) -> &str {
    visit(1);
    word
}

/// Слова перед выражением не дают считать его датой начала.
///
/// - Предлог непосредственно перед выражением, который разбор не принял,
///   оставил бы его висеть в конце названия («Созвон в» от «Созвон в
///   18.05»): выражение не отделяется. Так «в 18.05» – время или дата –
///   остаётся в названии целиком, а «Отчёт 18.05» – 18 мая.
/// - Время и тире между словом и выражением пропускаются, и проверяется
///   слово перед ними: в «до 18:00 завтра» и «с 18:00 – 19:00 завтра» –
///   «до» и «с»; «Созвон в 18:00 завтра» делится.
/// - Время словами целиком – количество (по той же грамматике, что в
///   «через»: «7», «семь», «два с половиной», «1/2») и единица («часа»,
///   «минут») или «час», «полчаса» – тоже не дата начала. Одно слово
///   единицы – предмет: «Купить ремешок для часов завтра» делится.
/// - Название буквы ([`is_letter`]) – не предлог и не союз: «Купить
///   витамины группы В завтра», «Витамин С завтра» делятся.
fn blocked(before: &[Token], letters: &[bool]) -> bool {
    let Some(last) = before.last() else {
        return false;
    };
    let n = before.len() - 1;
    visit(1);
    if !last.quoted && !letters[n] && is_dangling_preposition(&last.text) {
        return true;
    }
    let mut i = n;
    loop {
        visit(1);
        let word = &before[i];
        if word.quoted {
            return false;
        }
        if is_time(&word.text) || is_dash_word(&word.text) {
            if i == 0 {
                return false;
            }
            i -= 1;
            continue;
        }
        if is_time_unit(&word.text)
            && (is_counted_time_unit(&word.text) || quantity_ends_at(&before[..i]))
        {
            return true;
        }
        return !letters[i] && blocks_date_after(&word.text);
    }
}

/// Кончаются ли слова количеством: «7», «семь», «два с половиной», «1/2».
/// Та же грамматика, что у `words::quantity`, но для всех начал сразу:
/// проход с конца, `whole[k]` – слова с `k` до конца разбираются целиком.
/// Каждое слово – один раз.
fn quantity_ends_at(before: &[Token]) -> bool {
    let end = before.len();
    let word = |k: usize| look(&before[k].text);
    // whole[k - j]: слова `k..end` – последовательность слов количества и
    // пар «с половиной», «с четвертью».
    let mut j = end;
    while j > 0 && is_quantity_part(word(j - 1)) {
        j -= 1;
    }
    let mut whole = vec![false; end - j + 1];
    whole[end - j] = true;
    let mut found = false;
    for k in (j..end).rev() {
        let w = word(k);
        let pair = w == "с"
            && k + 2 <= end
            && matches!(word(k + 1), "половиной" | "четвертью")
            && whole[k + 2 - j];
        whole[k - j] = pair || (w != "с" && is_quantity_word(w) && whole[k + 1 - j]);
        let ok = if k + 1 == end {
            count(w).is_some() || is_quantity_word(w)
        } else {
            whole[k - j] && w != "с"
        };
        found |= ok;
    }
    found
}

fn is_quantity_part(word: &str) -> bool {
    matches!(word, "с" | "половиной" | "четвертью") || is_quantity_word(word)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;
    use proptest::prelude::*;

    const TODAY: Date = date(2026, 11, 19);

    /// Название буквы – отдельная от [`letters`] запись того же правила:
    /// для каждого слова заново ищет следующее слово с буквами.
    fn oracle_letters(tokens: &[Token]) -> Vec<bool> {
        (0..tokens.len())
            .map(|i| {
                let t = &tokens[i];
                if i == 0 || !t.capital_letter {
                    return false;
                }
                let prev = tokens[i - 1].text.as_str();
                let next = tokens.get(i + 1).map_or("", |t| t.text.as_str());
                if t.text == "г" && count(prev).is_some() {
                    return false;
                }
                if t.text == "с" && (next == "половиной" || next == "четвертью")
                {
                    return false;
                }
                // «К 18:00», «С 9:30», «С 18:00–19:00» – предлоги перед
                // временем; «С 2:1» – пропорция, «С» – буква.
                let clock = |p: &str| {
                    let Some((h, m)) = p.split_once(':') else {
                        return false;
                    };
                    let h: Option<u32> = (h.len() <= 2).then(|| h.parse().ok()).flatten();
                    let m: Option<u32> = (m.len() == 2).then(|| m.parse().ok()).flatten();
                    h.is_some_and(|h| h < 24) && m.is_some_and(|m| m < 60)
                };
                let dashes: &[char] = &['-', '‐', '‑', '‒', '–', '\u{2014}', '\u{2015}', '−'];
                let core = next.trim_matches(dashes);
                let clock = !core.is_empty()
                    && match core.split_once(dashes) {
                        Some((a, b)) => clock(a) && clock(b),
                        None => clock(core),
                    };
                if (t.text == "к" || t.text == "с") && clock {
                    return false;
                }
                if !tokens[i - 1].caps {
                    return true;
                }
                tokens[i + 1..]
                    .iter()
                    .find(|t| t.text.chars().any(char::is_alphabetic))
                    .is_some_and(|t| t.text.chars().any(char::is_alphabetic) && t.lower)
            })
            .collect()
    }

    /// Прежний перебор всех окончаний – эталон для одного прохода: все
    /// окончания и все конструкции перед выражением.
    fn split_title_exhaustive(text: &str, today: Date) -> QuickEntry<'_> {
        let whole = QuickEntry {
            title: text.trim(),
            when: None,
        };
        let tokens = tokenize(text);
        let words: Vec<&str> = tokens.iter().map(|t| t.text.as_str()).collect();
        let letters = oracle_letters(&tokens);
        let boundary = (0..tokens.len())
            .rev()
            .find(|&i| tokens[i].quoted || letters[i])
            .map_or(0, |i| i + 1);
        if boundary == 0 && !matches!(parse_words_at(&words, today), Err(ParseError::Unknown(_))) {
            return whole;
        }
        for i in boundary.max(1)..tokens.len() {
            let when = match parse_words_at(&words[i..], today) {
                Ok(when) => when,
                Err(ParseError::Unknown(_)) => continue,
                Err(_) => return whole,
            };
            let invalid = (boundary..i).any(|k| {
                !matches!(
                    parse_words_at(&words[k..i], today),
                    Ok(_) | Err(ParseError::Unknown(_))
                )
            });
            if invalid || blocked(&tokens[..i], &letters) {
                return whole;
            }
            let title = text[..tokens[i].start]
                .trim_end_matches(|c: char| c.is_whitespace() || ",;:–-".contains(c))
                .trim_start();
            if title.is_empty() {
                return whole;
            }
            return QuickEntry {
                title,
                when: Some(when),
            };
        }
        whole
    }

    fn candidates(text: &str) -> (QuickEntry<'_>, usize) {
        CANDIDATES.with(|c| c.set(0));
        let q = split_title(text, TODAY);
        (q, CANDIDATES.with(std::cell::Cell::get))
    }

    /// 10 000 обычных слов: ни одного разбора; с датой в конце – один.
    #[test]
    fn long_title_is_one_pass() {
        let title = (0..10_000)
            .map(|i| format!("слово{}", if i % 2 == 0 { "" } else { "а" }))
            .collect::<Vec<_>>()
            .join(" ");
        let (q, n) = candidates(&title);
        assert_eq!((q.when, n), (None, 0));
        let text = format!("{title} завтра вечером");
        let (q, n) = candidates(&text);
        assert!(q.when.is_some());
        assert_eq!(n, 1);
        // Длинное количество целиком – окончаний не больше длины цепочки.
        let text = format!(
            "{title} через {} дней вечером",
            "сто ".repeat(50).trim_end()
        );
        let (q, n) = candidates(&text);
        assert_eq!(q.when, None);
        assert_eq!(n, 1);
        // Одни слова даты подряд: каждое длинное окончание проверяется по
        // форме, разборов – не больше `SHORT` коротких и одного длинного.
        for chain in [
            vec!["через"; 10_000].join(" "),
            vec!["15"; 10_000].join(" "),
            format!("{} дней назад", vec!["15"; 10_000].join(" ")),
            format!("Было {} дней назад вечером", vec!["сто"; 10_000].join(" ")),
        ] {
            let (_, n) = candidates(&chain);
            assert!(n <= SHORT + 2, "{n} разборов");
        }
    }

    fn visits(text: &str) -> (QuickEntry<'_>, usize) {
        VISITS.with(|c| c.set(0));
        let q = split_title(text, TODAY);
        (q, VISITS.with(std::cell::Cell::get))
    }

    /// Неудобные для поиска тексты из `n` слов (примерно).
    pub(super) fn adversarial(n: usize) -> Vec<(&'static str, String)> {
        let rep = |w: &str| vec![w; n].join(" ");
        let plain = (0..n)
            .map(|i| format!("слово{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        vec![
            ("обычные слова", plain.clone()),
            ("обычные слова + «завтра»", format!("{plain} завтра")),
            (
                "вводные слова + «через 3 дня»",
                format!("Позвонить {} через 3 дня", rep("в")),
            ),
            (
                "вводные слова + длинное «через»",
                format!(
                    "Позвонить {} через {} дней",
                    rep("в"),
                    vec!["сто"; 20].join(" ")
                ),
            ),
            (
                "длинное количество с ошибкой в конце + «часа завтра»",
                format!("Тренировка {} с часа завтра", rep("сто")),
            ),
            (
                "длинное количество + «часа завтра»",
                format!("Тренировка {} часа завтра", rep("сто")),
            ),
            ("«через» подряд", rep("через")),
            (
                "«через» подряд + «завтра»",
                format!("Купить {} завтра", rep("через")),
            ),
            ("числительные подряд", rep("сто")),
            (
                "«15» подряд + «дней назад»",
                format!("{} дней назад", rep("15")),
            ),
            (
                "«через» + числительные + «дней вечером»",
                format!("Напомнить через {} дней вечером", rep("сто")),
            ),
            (
                "«через» + числительные + «дней завтра»",
                format!("Напомнить через {} дней завтра", rep("сто")),
            ),
            (
                "смесь слов даты",
                (0..n)
                    .map(|i| {
                        [
                            "в",
                            "через",
                            "сто",
                            "с",
                            "половиной",
                            "дней",
                            "назад",
                            "вечером",
                            "15",
                            "декабря",
                        ][i % 10]
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            (
                "заглавные буквы",
                (0..n)
                    .map(|i| if i % 2 == 0 { "В" } else { "ГРУППЫ" })
                    .collect::<Vec<_>>()
                    .join(" ")
                    + " пт",
            ),
        ]
    }

    /// Каждое слово просматривается постоянное число раз: посещений слов
    /// во всех проходах и разборах – не больше `12 n`. Посещение – каждое
    /// чтение слова поиском ([`look`], соседи в [`is_letter`], разбиение на
    /// слова) и каждое слово, отданное разбору; на деле – 4–8 на слово.
    #[test]
    fn word_visits_are_linear() {
        for n in [1_000, 10_000] {
            for (name, text) in adversarial(n) {
                let words = tokenize(&text).len();
                let (_, v) = visits(&text);
                assert!(v <= 12 * words, "{name}: {v} посещений на {words} слов");
                println!("{name}: {:.2} посещений на слово", v as f64 / words as f64);
            }
        }
    }

    /// Время суток – часы 0–23 и две цифры минут, можно промежуток;
    /// пропорция – не время.
    #[test]
    fn clock_times_differ_from_ratios() {
        use crate::words::is_time;
        for w in [
            "18:00",
            "9:30",
            "0:05",
            "23:59",
            "18:00–19:00",
            "18:00-19:00",
            "-19:00",
            "18:00-",
        ] {
            assert!(is_time(w), "{w}");
        }
        for w in [
            "2:1",
            "18:7",
            "24:00",
            "25:00",
            "18:60",
            "1:2:3",
            "18:00–19:00–20:00",
            "–",
            ":",
            "18:00––19:00",
            "123:00",
        ] {
            assert!(!is_time(w), "{w}");
        }
    }

    /// Каждое слово любого распознанного выражения – слово даты.
    #[test]
    fn date_vocabulary_covers_every_phrase() {
        for phrase in [
            "сегодня вечером",
            "вечером завтра",
            "послезавтра",
            "на завтра",
            "через 3 дня",
            "через пару недель",
            "через месяц",
            "через год",
            "в следующую пятницу",
            "во вторник",
            "след пн",
            "на следующей неделе",
            "на выходных",
            "в выходные",
            "15 дек",
            "15 декабря 2027 г",
            "15 декабря 2027 года",
            "2027г",
            "15.12.27",
            "когда-нибудь",
            "когда нибудь",
            "в любое время",
            "любое время",
            "вчера вечером",
            "позавчера",
            "в прошлую пятницу",
            "на прошлой неделе",
            "в прошлом месяце",
            "в прошлом году",
            "3 дня назад",
            "неделю назад",
            "через 0 дней",
            "через 2 часа",
            "через полтора часа",
            "через 1,5 часа",
            "через два с половиной дня",
            "через сто двадцать пять дней",
            "через полгода",
            "через 1/2 дня",
            "через половину часа",
            "через две трети часа",
            "через ½ дня",
            "через 2-3 дня",
            "через одну-две недели",
            "31 ноября",
            "29 февраля 2027",
            "31.12.0999",
            "когда-нибудь вечером",
            "в любое время вечером",
        ] {
            for w in tokenize(phrase) {
                assert!(is_date_word(&w.text), "«{}» в «{phrase}»", w.text);
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(4000))]

        /// Один проход даёт то же, что перебор всех окончаний.
        #[test]
        fn one_pass_matches_exhaustive(
            words in prop::collection::vec(
                prop::sample::select(vec![
                    "Купить", "хлеб", "через", "банк", "в", "В", "СОЗВОН", "на", "до", "с",
                    "завтра", "вечером", "пт", "15", "декабря", "18.05", "18:00", "–", "два",
                    "половиной", "часа", "дней", "неделю", "сто", "1/2", "«Завтра»", "группы",
                    "когда-нибудь", "вчера", "назад", "следующий", "любое", "время", "31",
                    "ноября", "полтора", "2027", "г", "часов", "для", "сто", "сто", "двадцать",
                    "через", "через", "назад", "лет", "минут", "Г.", "ГРУППЫ", "пятницу", "С",
                    "ВЕЧЕРОМ", "ЗАВТРА", "К", "0", "½", "2–3", "четвертью", "ДЕКАБРЯ", "9:30",
                    "ОТЧЁТ", "ФОРМА", "2:1", "18:00–19:00", "25:00", "9:5", "-19:00",
                ]),
                0..24,
            ),
        ) {
            let text = words.join(" ");
            prop_assert_eq!(split_title(&text, TODAY), split_title_exhaustive(&text, TODAY), "«{}»", text);
        }
    }
}
