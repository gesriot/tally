//! Прогресс проекта (кружок рядом с названием).

use std::collections::HashMap;

use crate::heading::Heading;
use crate::ids::ProjectId;
use crate::lists::Deleted;
use crate::status::Status;
use crate::task::Task;

/// `done` из `total`. Удалённые задачи – сами или вместе с разделом – не
/// считаются (правило [`Deleted`], как у списков) и возвращаются в расчёт
/// при восстановлении раздела; отменённые не входят
/// ни в `done`, ни в `total` – это работа, которую решили не делать.
///
/// Это собственное правило Tally: точная формула индикатора в Things
/// не задокументирована. Пример: 2 выполнены, 1 открыта, 1 отменена,
/// 1 удалена → 2/3.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
}

impl Progress {
    /// Доля выполненного от 0 до 1; пустой проект – 0.
    pub fn fraction(self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.done as f64 / self.total as f64
        }
    }
}

pub fn project_progress(project: ProjectId, tasks: &[Task], headings: &[Heading]) -> Progress {
    let deleted_headings: HashMap<_, bool> = headings
        .iter()
        .filter(|h| h.project == project)
        .map(|h| (h.id, h.sync.is_deleted()))
        .collect();
    tasks
        .iter()
        .filter(|t| t.project == Some(project))
        .filter(|t| {
            !Deleted {
                own: t.sync.is_deleted(),
                heading: t
                    .heading
                    .is_some_and(|h| deleted_headings.get(&h).copied().unwrap_or(false)),
                ..Deleted::default()
            }
            .is_trashed()
        })
        .fold(Progress::default(), |mut p, t| {
            match t.status {
                Status::Open => p.total += 1,
                Status::Completed { .. } => {
                    p.total += 1;
                    p.done += 1;
                }
                Status::Canceled { .. } => {}
            }
            p
        })
}
