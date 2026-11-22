//! Ядро Tally: модели и правила в духе Things 3.
//!
//! Здесь нет ввода-вывода и нет `now()`: текущая дата (`today`) и время
//! всегда приходят параметром. Поэтому смену суток при открытом приложении
//! можно проверить, просто подставив другую дату.
//!
//! Модели – [`Task`], [`Project`], [`Area`], [`Heading`], [`Tag`],
//! [`ChecklistItem`]. Списки («Входящие», «Сегодня», «Планы» и т. д.) не
//! хранятся, а вычисляются функциями из модуля [`lists`].

pub mod area;
pub mod checklist;
pub mod heading;
pub mod ids;
pub mod lists;
pub mod meta;
pub mod project;
pub mod repeat;
pub mod schedule;
pub mod status;
pub mod tag;
pub mod task;

pub use area::Area;
pub use checklist::ChecklistItem;
pub use heading::Heading;
pub use ids::{AreaId, ChecklistItemId, HeadingId, ProjectId, TagId, TaskId};
pub use jiff::Timestamp;
pub use jiff::civil::Date;
pub use meta::SyncMeta;
pub use project::Project;
pub use repeat::{RepeatFrom, RepeatRule, RepeatUnit};
pub use schedule::{DeadlineState, Start};
pub use status::Status;
pub use tag::Tag;
pub use tally_rank::Rank;
pub use task::Task;
