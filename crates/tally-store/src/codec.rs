//! Преобразование значений модели в колонки SQLite и обратно.

use std::str::FromStr;

use jiff::Timestamp;
use jiff::civil::Date;
use tally_core::{RepeatFrom, RepeatRule, RepeatUnit, Start, Status};

use crate::error::{Result, StoreError};

/// Момент в UTC фиксированной ширины: `2026-11-19T09:00:00.000000000Z`.
/// Строки такого вида сравниваются так же, как сами моменты.
pub(crate) fn ts(t: Timestamp) -> String {
    t.strftime("%Y-%m-%dT%H:%M:%S.%9fZ").to_string()
}

pub(crate) fn opt_ts(t: Option<Timestamp>) -> Option<String> {
    t.map(ts)
}

/// Дата как `YYYY-MM-DD`.
pub(crate) fn date(d: Date) -> String {
    d.to_string()
}

pub(crate) fn opt_date(d: Option<Date>) -> Option<String> {
    d.map(date)
}

/// Разбор колонки из таблицы `table`; ошибка – [`StoreError::Corrupt`].
#[derive(Clone, Copy)]
pub(crate) struct Decoder {
    pub table: &'static str,
}

impl Decoder {
    pub(crate) fn corrupt(self, column: &'static str, value: impl Into<String>) -> StoreError {
        StoreError::Corrupt {
            table: self.table,
            column,
            value: value.into(),
        }
    }

    pub(crate) fn parse<T: FromStr>(self, column: &'static str, value: &str) -> Result<T> {
        value.parse().map_err(|_| self.corrupt(column, value))
    }

    pub(crate) fn parse_opt<T: FromStr>(
        self,
        column: &'static str,
        value: Option<&str>,
    ) -> Result<Option<T>> {
        value.map(|v| self.parse(column, v)).transpose()
    }

    pub(crate) fn start(self, value: &str) -> Result<Start> {
        match value {
            "inbox" => Ok(Start::Inbox),
            "anytime" => Ok(Start::Anytime),
            "someday" => Ok(Start::Someday),
            other => Err(self.corrupt("start", other)),
        }
    }

    pub(crate) fn status(self, status: &str, closed_at: Option<&str>) -> Result<Status> {
        let at = self.parse_opt::<Timestamp>("closed_at", closed_at)?;
        match (status, at) {
            ("open", None) => Ok(Status::Open),
            ("completed", Some(at)) => Ok(Status::Completed { at }),
            ("canceled", Some(at)) => Ok(Status::Canceled { at }),
            (other, _) => Err(self.corrupt("status", other)),
        }
    }

    pub(crate) fn repeat_rule(
        self,
        interval: Option<i64>,
        unit: Option<&str>,
    ) -> Result<Option<RepeatRule>> {
        let (Some(interval), Some(unit)) = (interval, unit) else {
            return match (interval, unit) {
                (None, None) => Ok(None),
                (Some(i), None) => Err(self.corrupt("repeat_unit", format!("NULL при {i}"))),
                (_, Some(u)) => Err(self.corrupt("repeat_interval", format!("NULL при {u}"))),
            };
        };
        let interval = u16::try_from(interval)
            .map_err(|_| self.corrupt("repeat_interval", interval.to_string()))?;
        let unit = match unit {
            "day" => RepeatUnit::Day,
            "week" => RepeatUnit::Week,
            "month" => RepeatUnit::Month,
            "year" => RepeatUnit::Year,
            other => return Err(self.corrupt("repeat_unit", other)),
        };
        Ok(Some(RepeatRule { interval, unit }))
    }

    pub(crate) fn repeat_from(self, value: &str) -> Result<RepeatFrom> {
        match value {
            "date" => Ok(RepeatFrom::Date),
            "completion" => Ok(RepeatFrom::Completion),
            other => Err(self.corrupt("repeat_from", other)),
        }
    }
}

pub(crate) fn start(s: Start) -> &'static str {
    match s {
        Start::Inbox => "inbox",
        Start::Anytime => "anytime",
        Start::Someday => "someday",
    }
}

/// `(status, closed_at)`.
pub(crate) fn status(s: Status) -> (&'static str, Option<String>) {
    match s {
        Status::Open => ("open", None),
        Status::Completed { at } => ("completed", Some(ts(at))),
        Status::Canceled { at } => ("canceled", Some(ts(at))),
    }
}

/// `(repeat_interval, repeat_unit)`.
pub(crate) fn repeat_rule(r: Option<RepeatRule>) -> (Option<i64>, Option<&'static str>) {
    match r {
        None => (None, None),
        Some(r) => (
            Some(i64::from(r.interval)),
            Some(match r.unit {
                RepeatUnit::Day => "day",
                RepeatUnit::Week => "week",
                RepeatUnit::Month => "month",
                RepeatUnit::Year => "year",
            }),
        ),
    }
}

pub(crate) fn repeat_from(r: RepeatFrom) -> &'static str {
    match r {
        RepeatFrom::Date => "date",
        RepeatFrom::Completion => "completion",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_are_fixed_width_and_round_trip() {
        let a: Timestamp = "2026-11-19T09:00:00Z"
            .parse()
            .unwrap_or(Timestamp::UNIX_EPOCH);
        let b: Timestamp = "2026-11-19T09:00:00.5Z"
            .parse()
            .unwrap_or(Timestamp::UNIX_EPOCH);
        assert_eq!(ts(a), "2026-11-19T09:00:00.000000000Z");
        assert_eq!(ts(b), "2026-11-19T09:00:00.500000000Z");
        assert!(ts(a) < ts(b));
        let d = Decoder { table: "t" };
        assert_eq!(d.parse::<Timestamp>("c", &ts(b)).ok(), Some(b));
    }

    #[test]
    fn bad_values_are_corrupt() {
        let d = Decoder { table: "tasks" };
        assert!(matches!(
            d.start("tomorrow"),
            Err(StoreError::Corrupt {
                column: "start",
                ..
            })
        ));
        assert!(d.status("completed", None).is_err());
        assert!(d.repeat_rule(Some(1), None).is_err());
        assert!(d.parse::<Date>("date", "2026-13-01").is_err());
    }
}
