//! Проект: задачи с разделами, своя дата, срок и состояние.

use jiff::Timestamp;
use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use tally_rank::Rank;

use crate::ids::{AreaId, ProjectId, TagId};
use crate::meta::SyncMeta;
use crate::repeat::{RepeatFrom, RepeatRule};
use crate::schedule::Start;
use crate::status::Status;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub title: String,
    pub notes: String,
    pub area: Option<AreaId>,

    /// У проекта нет «Входящих»: `Start::Inbox` считается «В любое время».
    pub start: Start,
    pub date: Option<Date>,
    pub evening: bool,
    pub deadline: Option<Date>,

    pub status: Status,
    pub tags: Vec<TagId>,
    pub index: Rank,
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

impl Project {
    /// Новый активный проект («В любое время»), создан в момент `at`.
    pub fn new(id: ProjectId, title: impl Into<String>, index: Rank, at: Timestamp) -> Self {
        Self {
            id,
            title: title.into(),
            notes: String::new(),
            area: None,
            start: Start::Anytime,
            date: None,
            evening: false,
            deadline: None,
            status: Status::Open,
            tags: Vec::new(),
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
