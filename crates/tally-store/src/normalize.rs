//! Нормализация текста для поиска.
//!
//! `LIKE` и `lower()` в SQLite понимают регистр только у ASCII, поэтому текст
//! для поиска (`search_text`) готовит Rust: разложение (NFD), Unicode-нижний
//! регистр, «ё» → «е», сборка (NFC), пробелы схлопнуты, по краям обрезаны. Запрос нормализуется
//! той же функцией, и дальше достаточно побайтового поиска подстроки.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::canonical_combining_class;

/// Нормализованный текст: «  Ёлка  ЗЕЛЁНАЯ » → «елка зеленая».
///
/// Идемпотентна: `normalize(normalize(x)) == normalize(x)`.
pub fn normalize(text: &str) -> String {
    // Нижний регистр – на разложенном тексте, и разложить ещё раз: строчная
    // буква может дать знаки не в каноническом порядке («J» + U+030C → «ǰ»).
    let lower: String = text.nfd().flat_map(char::to_lowercase).nfd().collect();
    // «ё» → «е» до сборки, а не после: иначе «Ё» + U+0300 станет «е» + U+0300,
    // а повторный вызов соберёт из них «ѐ».
    let composed: String = fold_yo(&lower).nfc().collect();
    let mut out = String::with_capacity(composed.len());
    for word in composed.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

/// В разложенном тексте убрать U+0308, которые NFC собрал бы с «е» в «ё»:
/// знаки после «е», не заслонённые знаком с классом ≥ 230.
fn fold_yo(decomposed: &str) -> String {
    let mut out = String::with_capacity(decomposed.len());
    let mut after_e = false;
    let mut max_class = 0;
    for c in decomposed.chars() {
        let class = canonical_combining_class(c);
        if class == 0 {
            after_e = c == 'е';
            max_class = 0;
        } else if c == '\u{308}' && after_e && max_class < 230 {
            // Все такие знаки: после удаления первого NFC собрал бы «ё» из второго.
            continue;
        } else {
            max_class = max_class.max(class);
        }
        out.push(c);
    }
    out
}

/// Слова запроса: каждое должно встретиться в тексте (в любом порядке).
pub fn query_words(query: &str) -> Vec<String> {
    normalize(query)
        .split(' ')
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

/// `search_text` из нескольких полей: каждое нормализуется отдельно,
/// между полями – перевод строки, чтобы слова разных полей не склеивались.
pub(crate) fn search_text<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    parts
        .into_iter()
        .map(normalize)
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cyrillic_case_and_yo() {
        assert_eq!(normalize("  Ёлка  ЗЕЛЁНАЯ "), "елка зеленая");
        assert_eq!(normalize("ПРИВЕТ, Мир"), "привет, мир");
        assert_eq!(normalize("Straße ÄÖÜ"), "straße äöü");
    }

    #[test]
    fn decomposed_letters_are_composed() {
        // «й» и «ё» из двух кодовых точек (так бывает при вставке из macOS).
        let decomposed = "и\u{306}од е\u{308}ж";
        assert_eq!(normalize(decomposed), "йод еж");
    }

    #[test]
    fn lowercase_then_compose_is_idempotent() {
        // «J» с гачеком не имеет составной формы, а строчная «ǰ» – имеет.
        let once = normalize("J\u{30c}");
        assert_eq!(once, "\u{1f0}");
        assert_eq!(normalize(&once), once);
    }

    #[test]
    fn yo_with_another_mark_is_idempotent() {
        // «Ё» + U+0300: «ё» сворачивается в «е» до сборки, получается «ѐ» сразу.
        let once = normalize("\u{401}\u{300}");
        assert_eq!(once, "\u{450}");
        assert_eq!(normalize(&once), once);
        // Разложенная «ё» и знак под буквой: «ё» всё равно сворачивается.
        assert_eq!(normalize("Е\u{323}\u{308}"), normalize("е\u{323}"));
        // Две точки подряд: после первой NFC собрал бы «ё» из второй.
        assert_eq!(normalize("ё\u{308}"), "е");
        // «й» не трогаем.
        assert_eq!(normalize("И\u{306}"), "й");
    }

    #[test]
    fn whitespace_collapses() {
        assert_eq!(normalize("a\t\n  b\u{a0}c"), "a b c");
        assert_eq!(normalize("   "), "");
    }

    #[test]
    fn query_splits_into_words() {
        assert_eq!(query_words(" Купить  МОЛОКО "), ["купить", "молоко"]);
        assert!(query_words("  ").is_empty());
    }

    #[test]
    fn fields_are_separated() {
        assert_eq!(
            search_text(["Заголовок", "", "Заметка  ТУТ"]),
            "заголовок\nзаметка тут"
        );
    }
}
