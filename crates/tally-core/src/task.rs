//! Задача.

use jiff::Timestamp;
use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use tally_rank::Rank;

use crate::checklist::ChecklistItem;
use crate::ids::{AreaId, HeadingId, ProjectId, TagId, TaskId};
use crate::meta::SyncMeta;
use crate::repeat::{RepeatFrom, RepeatRule};
use crate::schedule::Start;
use crate::status::Status;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    pub notes: String,

    /// Где задача живёт без даты.
    pub start: Start,
    /// Дата начала («когда»); в JSON и БД – `YYYY-MM-DD`.
    pub date: Option<Date>,
    /// Блок «Вечер» в «Сегодня». Имеет смысл только когда задача в «Сегодня».
    pub evening: bool,
    /// Крайний срок.
    pub deadline: Option<Date>,
    /// Момент напоминания об этом экземпляре задачи.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reminder_at: Option<Timestamp>,

    pub status: Status,

    pub project: Option<ProjectId>,
    /// Раздел – всегда раздел проекта `project`.
    pub heading: Option<HeadingId>,
    /// Своя область – только у задачи вне проекта. Задача в проекте
    /// наследует область проекта, и это поле у неё `None` (хранилище
    /// отклоняет оба поля сразу).
    pub area: Option<AreaId>,
    pub tags: Vec<TagId>,
    pub checklist: Vec<ChecklistItem>,

    /// Порядок внутри проекта или списка.
    pub index: Rank,
    /// Собственный порядок в «Сегодня».
    pub today_index: Rank,

    pub repeat_rule: Option<RepeatRule>,
    pub repeat_from: RepeatFrom,
    /// Исходная дата для календарного повтора.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_anchor: Option<Date>,
    /// Этот экземпляр уже передал правило следующему.
    #[serde(default)]
    pub repeat_advanced: bool,

    pub sync: SyncMeta,
}

impl Task {
    /// Новая задача во «Входящих», создана в момент `at`.
    pub fn new(id: TaskId, title: impl Into<String>, index: Rank, at: Timestamp) -> Self {
        Self {
            id,
            title: title.into(),
            notes: String::new(),
            start: Start::Inbox,
            date: None,
            evening: false,
            deadline: None,
            reminder_at: None,
            status: Status::Open,
            project: None,
            heading: None,
            area: None,
            tags: Vec::new(),
            checklist: Vec::new(),
            today_index: index.clone(),
            index,
            repeat_rule: None,
            repeat_from: RepeatFrom::Date,
            repeat_anchor: None,
            repeat_advanced: false,
            sync: SyncMeta::new(at),
        }
    }

    /// Убрать дату начала («Когда»), как в Things: запись уходит
    /// во «В любое время», а не возвращается в прежние «Входящие»
    /// или «Когда-нибудь». «Вечер» без даты смысла не имеет и тоже снимается.
    /// Срок не трогается.
    ///
    /// Будущая команда «очистить дату» в tally-app должна вызывать именно это.
    pub fn clear_date(&mut self, at: Timestamp) {
        self.date = None;
        self.evening = false;
        self.start = Start::Anytime;
        self.sync.touch(at);
    }
}
