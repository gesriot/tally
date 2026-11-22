//! «Сегодня»: задачи и проекты с датой не позже сегодняшней или со сроком сегодня/раньше.

use jiff::civil::Date;

use super::Containers;
use super::item::Item;
use super::placement::List;
use crate::area::Area;
use crate::heading::Heading;
use crate::project::Project;
use crate::task::Task;

/// «Сегодня» в два блока, каждый в порядке `today_index`.
#[derive(Debug, Default)]
pub struct TodayView<'a> {
    pub day: Vec<Item<'a>>,
    pub evening: Vec<Item<'a>>,
}

impl TodayView<'_> {
    pub fn len(&self) -> usize {
        self.day.len() + self.evening.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub fn today<'a>(
    tasks: &'a [Task],
    projects: &'a [Project],
    areas: &'a [Area],
    headings: &'a [Heading],
    today: Date,
) -> TodayView<'a> {
    let c = Containers::new(projects, areas, headings);
    let mut view = TodayView::default();
    let task_items = tasks.iter().filter_map(|t| {
        let placement = c.place(t, today);
        match placement.list {
            List::Today { evening } if placement.is_own() => Some((Item::Task(t), evening)),
            _ => None,
        }
    });
    let project_items = projects
        .iter()
        .filter_map(|p| match c.place_project(p, today) {
            List::Today { evening } => Some((Item::Project(p), evening)),
            _ => None,
        });
    for (item, evening) in task_items.chain(project_items) {
        if evening {
            view.evening.push(item);
        } else {
            view.day.push(item);
        }
    }
    view.day.sort_by(Item::cmp_today);
    view.evening.sort_by(Item::cmp_today);
    view
}
