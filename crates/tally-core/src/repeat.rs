//! Календарный расчёт следующего экземпляра.

use jiff::{Span, civil::Date};
use serde::{Deserialize, Serialize};

use crate::schedule::days_between;

/// Единица интервала повтора.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepeatUnit {
    Day,
    Week,
    Month,
    Year,
}

/// Правило повтора: «каждые `interval` `unit`».
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RepeatRule {
    pub interval: u16,
    pub unit: RepeatUnit,
}

impl RepeatRule {
    /// Первый день цикла строго после текущего экземпляра и дня отсчёта.
    pub fn next(self, base: Date, today: Date, anchor: Date) -> Option<Date> {
        let after = base.max(today);
        let count = self.intervals(base, after)?.max(1);
        let date = self.at(base, anchor, count)?;
        if date > after {
            Some(date)
        } else {
            self.at(base, anchor, count + 1)
        }
    }

    /// Последнее наступившее повторение после базового экземпляра.
    pub fn latest(self, base: Date, today: Date, anchor: Date) -> Option<Date> {
        let count = self.intervals(base, today)?;
        if count < 1 {
            return None;
        }
        let date = self.at(base, anchor, count)?;
        if date <= today {
            Some(date)
        } else if count > 1 {
            self.at(base, anchor, count - 1)
        } else {
            None
        }
    }

    fn intervals(self, base: Date, until: Date) -> Option<i32> {
        let step = self.step();
        if step == 0 {
            return None;
        }
        let months = |date: Date| i32::from(date.year()) * 12 + i32::from(date.month());
        let elapsed = match self.unit {
            RepeatUnit::Day | RepeatUnit::Week => days_between(base, until),
            RepeatUnit::Month | RepeatUnit::Year => months(until) - months(base),
        };
        Some(elapsed / step)
    }

    fn step(self) -> i32 {
        i32::from(self.interval)
            * match self.unit {
                RepeatUnit::Day | RepeatUnit::Month => 1,
                RepeatUnit::Week => 7,
                RepeatUnit::Year => 12,
            }
    }

    fn at(self, base: Date, anchor: Date, count: i32) -> Option<Date> {
        let offset = self.step() * count;
        if matches!(self.unit, RepeatUnit::Day | RepeatUnit::Week) {
            return base.checked_add(Span::new().try_days(offset).ok()?).ok();
        }
        let first = base
            .first_of_month()
            .checked_add(Span::new().try_months(offset).ok()?)
            .ok()?;
        first
            .with()
            .day(anchor.day().min(first.days_in_month()))
            .build()
            .ok()
    }
}

/// От чего отсчитывать следующий повтор.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepeatFrom {
    /// От запланированной даты («каждый понедельник»).
    #[default]
    Date,
    /// От момента выполнения («через неделю после того, как сделал»).
    Completion,
}
