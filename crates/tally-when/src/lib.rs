//! Разбор русских выражений «когда» для Tally: «сегодня», «завтра вечером»,
//! «через 3 дня», «в пятницу», «15 дек», «когда-нибудь».
//!
//! Опорная дата `today` – всегда параметр: крейт не читает часы.
//! Результат – [`When`], который ложится на поля `tally-core`
//! (`start`, `date`, `evening`); срок (`deadline`) не трогается.
//!
//! - [`parse`] – всё выражение целиком («завтра вечером»);
//! - [`split_title`] – быстрый ввод: выражение в конце названия
//!   («Купить хлеб завтра» → «Купить хлеб» + завтра);
//! - [`format`] – обратно в текст; `parse(format(w)) == w`.
//!
//! Даты – от [`MIN_DATE`] до [`MAX_DATE`]: только в этих пределах
//! [`parse`] возвращает день, а [`format`] пишет его текстом.
//!
//! Грамматика и правила неоднозначностей – в README.md.

mod format;
mod parse;
mod quick;
mod token;
mod words;

use jiff::civil::Date;
use tally_core::{Project, Start, Task, Timestamp};

pub use format::format;
pub use parse::parse;
pub use quick::{QuickEntry, split_title};

/// Самый ранний день, который понимают [`parse`] и [`format`]: год пишется
/// четырьмя цифрами, поэтому раньше 1000 года дат нет.
pub const MIN_DATE: Date = Date::constant(1000, 1, 1);
/// Самый поздний день – последний день календаря `jiff`.
pub const MAX_DATE: Date = Date::MAX;

/// Что означает выражение.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum When {
    /// Конкретный день; `evening` – блок «Вечер» в этот день.
    On { date: Date, evening: bool },
    /// «В любое время»: без даты.
    Anytime,
    /// «Когда-нибудь»: без даты.
    Someday,
}

/// Поля «когда» в `tally-core`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Schedule {
    pub start: Start,
    pub date: Option<Date>,
    pub evening: bool,
}

impl When {
    /// Значения полей `start`, `date`, `evening`. У датированной записи
    /// `start` – «В любое время»: туда она попадёт, если дату потом снять
    /// (как `Task::clear_date` в `tally-core`).
    pub fn schedule(self) -> Schedule {
        match self {
            Self::On { date, evening } => Schedule {
                start: Start::Anytime,
                date: Some(date),
                evening,
            },
            Self::Anytime => Schedule {
                start: Start::Anytime,
                date: None,
                evening: false,
            },
            Self::Someday => Schedule {
                start: Start::Someday,
                date: None,
                evening: false,
            },
        }
    }

    /// Записать в задачу и отметить изменение в момент `at`. Срок не меняется.
    pub fn apply_to_task(self, task: &mut Task, at: Timestamp) {
        let s = self.schedule();
        task.start = s.start;
        task.date = s.date;
        task.evening = s.evening;
        task.sync.touch(at);
    }

    /// То же для проекта.
    pub fn apply_to_project(self, project: &mut Project, at: Timestamp) {
        let s = self.schedule();
        project.start = s.start;
        project.date = s.date;
        project.evening = s.evening;
        project.sync.touch(at);
    }

    /// День, если он есть.
    pub fn date(self) -> Option<Date> {
        match self {
            Self::On { date, .. } => Some(date),
            Self::Anytime | Self::Someday => None,
        }
    }
}

/// Почему выражение не разобрано.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("пустое выражение")]
    Empty,
    #[error("не понимаю «{0}»")]
    Unknown(String),
    #[error("такого дня нет: {day}.{month:02}{}", .year.map(|y| format!(".{y}")).unwrap_or_default())]
    InvalidDate {
        day: u8,
        month: u8,
        year: Option<i16>,
    },
    /// «через» с числом вне 1..=999: «через 0 дней», «через -1 день».
    #[error("«через» – от 1 до 999: «{0}»")]
    InvalidCount(String),
    /// Прошедший день относительно сегодня: «вчера», «позавчера»,
    /// «в прошлую пятницу», «3 дня назад». Прошедшая дата задаётся числом
    /// («18 ноября 2026»).
    #[error("«{0}» – в прошлом; прошедший день задаётся датой")]
    Past(String),
    /// Распознанная, но не поддержанная конструкция: «через 2 часа».
    #[error("не поддерживается: «{0}»")]
    Unsupported(String),
    #[error("«вечером» бывает только у дня, не у «{0}»")]
    EveningWithoutDate(String),
    /// День раньше [`MIN_DATE`] или позже [`MAX_DATE`].
    #[error("дата за пределами календаря")]
    OutOfRange,
}
