//! Элемент списка: задача, проект или (только в «Корзине») раздел и область.

use std::cmp::Ordering;

use tally_rank::Rank;

use crate::area::Area;
use crate::heading::Heading;
use crate::project::Project;
use crate::task::Task;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item<'a> {
    Task(&'a Task),
    Project(&'a Project),
    /// Удалённый раздел – одной строкой вместо всех его задач.
    Heading(&'a Heading),
    /// Удалённая область – одной строкой вместо всего её содержимого.
    Area(&'a Area),
}

impl<'a> Item<'a> {
    pub fn title(&self) -> &'a str {
        match self {
            Self::Task(t) => &t.title,
            Self::Project(p) => &p.title,
            Self::Heading(h) => &h.title,
            Self::Area(a) => &a.title,
        }
    }

    pub fn index(&self) -> &'a Rank {
        match self {
            Self::Task(t) => &t.index,
            Self::Project(p) => &p.index,
            Self::Heading(h) => &h.index,
            Self::Area(a) => &a.index,
        }
    }

    pub fn today_index(&self) -> &'a Rank {
        match self {
            Self::Task(t) => &t.today_index,
            Self::Project(p) => &p.today_index,
            // У раздела и области нет своего порядка в «Сегодня» (они туда не попадают).
            Self::Heading(h) => &h.index,
            Self::Area(a) => &a.index,
        }
    }

    /// При совпадающих ключах порядок не зависит от обычного `index`
    /// и от порядка чтения записей. Задачи при равенстве идут перед проектами.
    pub fn cmp_today(&self, other: &Self) -> Ordering {
        self.today_index()
            .cmp(other.today_index())
            .then_with(|| self.today_tiebreaker().cmp(&other.today_tiebreaker()))
    }

    fn today_tiebreaker(&self) -> (u8, jiff::Timestamp, uuid::Uuid) {
        match self {
            Self::Task(t) => (0, t.sync.created_at, t.id.0),
            Self::Project(p) => (1, p.sync.created_at, p.id.0),
            Self::Heading(h) => (2, h.sync.created_at, h.id.0),
            Self::Area(a) => (3, a.sync.created_at, a.id.0),
        }
    }

    pub fn evening(&self) -> bool {
        match self {
            Self::Task(t) => t.evening,
            Self::Project(p) => p.evening,
            Self::Heading(_) | Self::Area(_) => false,
        }
    }
}
