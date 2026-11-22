//! Слова выражения: нормализованный текст и место в исходной строке.

use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Token {
    /// Нижний регистр, «ё» → «е», без знаков препинания по краям.
    pub(crate) text: String,
    /// Байтовые границы слова в исходной строке.
    pub(crate) start: usize,
    /// Слово в кавычках или рядом с ними («Послезавтра», "фильм") – часть
    /// названия, а не выражения даты (см. [`crate::split_title`]).
    pub(crate) quoted: bool,
    /// Слово – одна заглавная буква («В» в «группы В», «Д» в «Витамин Д»):
    /// не в начале текста это название буквы, а не предлог.
    pub(crate) capital_letter: bool,
    /// Слово из двух и больше букв, все заглавные («СОЗВОН»): рядом с ним
    /// одна заглавная буква может быть предлогом текста капсом (см. `quick::is_letter`).
    pub(crate) caps: bool,
    /// Есть строчная буква («пт», «Купить»): после одной заглавной буквы
    /// текст продолжается не капсом – буква остаётся названием буквы.
    pub(crate) lower: bool,
}

/// Кавычки: «ёлочки», „лапки“, “английские”, "прямые".
fn is_quote(c: char) -> bool {
    matches!(c, '«' | '»' | '„' | '“' | '”' | '"')
}

/// Закрывает ли `c` кавычку, открытую `open`.
fn closes(open: char, c: char) -> bool {
    match open {
        '«' => c == '»',
        '„' => c == '“' || c == '”',
        '“' => c == '”',
        _ => c == open,
    }
}

/// Знаки по краям слова, которые не меняют смысла: «пт.», «дек.», «завтра,».
fn is_edge_punct(c: char) -> bool {
    matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | '(' | ')' | '\'') || is_quote(c)
}

pub(crate) fn normalize_word(word: &str) -> String {
    word.trim_matches(is_edge_punct)
        .nfc()
        .flat_map(char::to_lowercase)
        .map(|c| if c == 'ё' { 'е' } else { c })
        .collect()
}

/// Слова строки; слова из одних знаков препинания пропускаются.
///
/// Слово помечается `quoted`, если в нём есть кавычка или оно внутри
/// кавычек. Незакрытая кавычка действует до конца строки.
pub(crate) fn tokenize(text: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut start = None;
    // Открытые кавычки: вложенные «…„…“…» закрываются по очереди.
    let mut open: Vec<char> = Vec::new();
    let mut quoted = false;
    for (i, c) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        if is_quote(c) {
            quoted = true;
            if open.last().is_some_and(|&o| closes(o, c)) {
                open.pop();
            } else if c != '»' && c != '”' {
                open.push(c);
            }
        }
        match (c.is_whitespace(), start) {
            (false, None) => {
                start = Some(i);
                quoted = quoted || !open.is_empty();
            }
            (true, Some(s)) => {
                let raw = &text[s..i];
                let norm = normalize_word(raw);
                if !norm.is_empty() {
                    let mut letters = raw.trim_matches(is_edge_punct).chars();
                    let capital_letter = matches!(
                        (letters.next(), letters.next()),
                        (Some(c), None) if c.is_uppercase()
                    );
                    let alpha = raw.chars().filter(|c| c.is_alphabetic());
                    let caps = alpha.clone().count() >= 2 && alpha.clone().all(char::is_uppercase);
                    let lower = alpha.clone().any(char::is_lowercase);
                    out.push(Token {
                        text: norm,
                        start: s,
                        quoted,
                        capital_letter,
                        caps,
                        lower,
                    });
                }
                start = None;
                quoted = false;
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_normalized_with_positions() {
        let t = tokenize("  Купить  ЁЛКУ, в Пт. ");
        let words: Vec<(&str, usize)> = t.iter().map(|t| (t.text.as_str(), t.start)).collect();
        assert_eq!(words, [("купить", 2), ("елку", 16), ("в", 26), ("пт", 29)]);
        // Слово из одних знаков препинания пропускается, тире остаётся словом.
        let only: Vec<String> = tokenize(" , – ").into_iter().map(|t| t.text).collect();
        assert_eq!(only, ["–"]);
    }

    #[test]
    fn quoted_words_are_marked() {
        let marks = |text: &str| -> Vec<(String, bool)> {
            tokenize(text)
                .into_iter()
                .map(|t| (t.text, t.quoted))
                .collect()
        };
        let pairs = |list: &[(&str, bool)]| -> Vec<(String, bool)> {
            list.iter().map(|(w, q)| ((*w).to_owned(), *q)).collect()
        };
        assert_eq!(
            marks("фильм «Война и мир» завтра"),
            pairs(&[
                ("фильм", false),
                ("война", true),
                ("и", true),
                ("мир", true),
                ("завтра", false)
            ])
        );
        assert_eq!(
            marks("„Послезавтра“ “Ещё раз” \"Завтра\" потом"),
            pairs(&[
                ("послезавтра", true),
                ("еще", true),
                ("раз", true),
                ("завтра", true),
                ("потом", false)
            ])
        );
        // Незакрытая кавычка – до конца строки.
        assert_eq!(
            marks("книга «Завтра никогда"),
            pairs(&[("книга", false), ("завтра", true), ("никогда", true)])
        );
    }
}
