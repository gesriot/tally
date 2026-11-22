//! «Когда»: где задача живёт без даты, и состояние срока.

use jiff::civil::Date;
use serde::{Deserialize, Serialize};

/// Куда относится задача, пока у неё нет даты.
///
/// Дата (`date`) и «Вечер» (`evening`) хранятся отдельно, а «Сегодня» и «Планы»
/// вычисляются: см. [`crate::lists`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Start {
    /// Входящие: ещё не разобрано.
    #[default]
    Inbox,
    /// В любое время.
    Anytime,
    /// Когда-нибудь.
    Someday,
}

/// Состояние срока относительно `today`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeadlineState {
    /// Срок впереди, через `days` дней (≥ 1).
    Ahead { days: i32 },
    /// Срок сегодня.
    Today,
    /// Срок прошёл `days` дней назад (≥ 1).
    Overdue { days: i32 },
}

impl DeadlineState {
    pub fn of(deadline: Date, today: Date) -> Self {
        let days = days_between(today, deadline);
        match days {
            0 => Self::Today,
            d if d > 0 => Self::Ahead { days: d },
            d => Self::Overdue { days: -d },
        }
    }

    /// Срок сегодня или уже прошёл – задача должна быть в «Сегодня».
    pub fn is_due(self) -> bool {
        !matches!(self, Self::Ahead { .. })
    }
}

/// Число дней от `from` до `to` (отрицательное, если `to` раньше).
pub fn days_between(from: Date, to: Date) -> i32 {
    from.until(to).map_or(0, |span| span.get_days())
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn deadline_states() {
        let today = date(2026, 11, 19);
        assert_eq!(DeadlineState::of(today, today), DeadlineState::Today);
        assert_eq!(
            DeadlineState::of(date(2026, 11, 22), today),
            DeadlineState::Ahead { days: 3 }
        );
        assert_eq!(
            DeadlineState::of(date(2026, 10, 31), today),
            DeadlineState::Overdue { days: 19 }
        );
        assert!(DeadlineState::of(date(2026, 11, 18), today).is_due());
        assert!(!DeadlineState::of(date(2026, 11, 20), today).is_due());
    }

    #[test]
    fn start_serializes_as_snake_case() {
        let json = serde_json::to_string(&Start::Someday).unwrap_or_default();
        assert_eq!(json, "\"someday\"");
    }
}
