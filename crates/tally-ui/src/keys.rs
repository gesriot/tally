//! Сочетания клавиш окна. Решение – здесь, `.slint` только передаёт
//! нажатие.
//!
//! Slint на macOS сам меняет модификаторы местами: `control` там – клавиша
//! Command, а `meta` – Control. Поэтому Ctrl+Z на Windows и Linux и Cmd+Z на
//! macOS приходят одинаково – с `control`.

use jiff::civil::Date;
use slint::SharedString;
use slint::platform::Key;
use tally_when::When;

/// Действие по нажатию.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Undo,
    Redo,
    /// Перечитать базу (её могла изменить консоль).
    Refresh,
}

/// Нажатие, как его передаёт Slint.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Press<'a> {
    pub text: &'a str,
    pub control: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
}

/// Нажата ли особая клавиша Slint `key`: её текст – код из таблицы Slint
/// ([`Key`]), а не символ, поэтому сравнение – только с этой константой.
pub(crate) fn is(text: &str, key: Key) -> bool {
    SharedString::from(key).as_str() == text
}

/// Ctrl+Z – отменить, Ctrl+Shift+Z – повторить (на macOS – Cmd), F5 –
/// перечитать базу. Буква – по раскладке: в русской Ctrl+Z приходит как
/// «я» (Slint передаёт символ клавиши без учёта Ctrl).
pub fn action(p: Press<'_>) -> Option<Action> {
    if is(p.text, Key::F5) && !p.control && !p.alt && !p.meta && !p.shift {
        return Some(Action::Refresh);
    }
    if !p.control || p.alt || p.meta {
        return None;
    }
    let z = matches!(p.text, "z" | "Z" | "я" | "Я" | "\u{001a}");
    match (z, p.shift) {
        (true, false) => Some(Action::Undo),
        (true, true) => Some(Action::Redo),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditorAction {
    Open,
    Save,
    Dismiss,
    Previous,
    Next,
    When,
    Deadline,
    Checklist,
    Tags,
    Schedule(When),
}

pub(crate) fn editor_action(p: Press<'_>, today: Date) -> Option<EditorAction> {
    if p.alt || p.meta {
        return None;
    }
    if !p.shift && is(p.text, Key::Return) {
        return Some(if p.control {
            EditorAction::Save
        } else {
            EditorAction::Open
        });
    }
    if !p.control && !p.shift {
        if is(p.text, Key::Escape) {
            return Some(EditorAction::Dismiss);
        }
        if is(p.text, Key::UpArrow) {
            return Some(EditorAction::Previous);
        }
        if is(p.text, Key::DownArrow) {
            return Some(EditorAction::Next);
        }
    }
    if !p.control {
        return None;
    }
    match (p.shift, p.text.to_lowercase().as_str()) {
        (true, "d" | "в" | "\u{0004}") => Some(EditorAction::Deadline),
        (true, "t" | "е" | "\u{0014}") => Some(EditorAction::Tags),
        (true, "c" | "с" | "\u{0003}") => Some(EditorAction::Checklist),
        (false, "s" | "ы" | "\u{0013}") => Some(EditorAction::When),
        (false, "t" | "е" | "\u{0014}") => Some(EditorAction::Schedule(When::On {
            date: today,
            evening: false,
        })),
        (false, "e" | "у" | "\u{0005}") => Some(EditorAction::Schedule(When::On {
            date: today,
            evening: true,
        })),
        (false, "r" | "к" | "\u{0012}") => Some(EditorAction::Schedule(When::Anytime)),
        (false, "o" | "щ" | "\u{000f}") => Some(EditorAction::Schedule(When::Someday)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(text: &str, control: bool, shift: bool) -> Press<'_> {
        Press {
            text,
            control,
            shift,
            ..Press::default()
        }
    }

    #[test]
    fn ctrl_z_undoes_and_ctrl_shift_z_redoes() {
        assert_eq!(action(press("z", true, false)), Some(Action::Undo));
        assert_eq!(action(press("Z", true, true)), Some(Action::Redo));
        assert_eq!(action(press("z", true, true)), Some(Action::Redo));
    }

    #[test]
    fn russian_layout_works_too() {
        assert_eq!(action(press("я", true, false)), Some(Action::Undo));
        assert_eq!(action(press("Я", true, true)), Some(Action::Redo));
    }

    #[test]
    fn other_presses_do_nothing() {
        assert_eq!(action(press("z", false, false)), None);
        assert_eq!(action(press("x", true, false)), None);
        let alt = Press {
            alt: true,
            ..press("z", true, false)
        };
        assert_eq!(action(alt), None);
        // На macOS `meta` – клавиша Control: Ctrl+Z там не отмена.
        let meta = Press {
            meta: true,
            ..press("z", false, false)
        };
        assert_eq!(action(meta), None);
    }

    #[test]
    fn f5_refreshes() {
        let f5 = SharedString::from(Key::F5);
        assert_eq!(action(press(&f5, false, false)), Some(Action::Refresh));
        assert_eq!(action(press(&f5, true, false)), None);
        let f1 = SharedString::from(Key::F1);
        assert_eq!(action(press(&f1, false, false)), None);
    }
}
