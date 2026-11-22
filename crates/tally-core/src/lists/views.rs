//! «Входящие», «В любое время», «Когда-нибудь», «Журнал», «Корзина».

use jiff::Timestamp;
use jiff::civil::Date;

use super::Containers;
use super::item::Item;
use super::placement::{List, closed_placement, is_deferred};
use crate::area::Area;
use crate::heading::Heading;
use crate::project::Project;
use crate::task::Task;

/// «Входящие» в порядке `index`.
pub fn inbox<'a>(
    tasks: &'a [Task],
    projects: &'a [Project],
    areas: &'a [Area],
    headings: &'a [Heading],
    today: Date,
) -> Vec<&'a Task> {
    let c = Containers::new(projects, areas, headings);
    let mut out: Vec<&Task> = tasks
        .iter()
        .filter(|t| c.place(t, today).list == List::Inbox)
        .collect();
    out.sort_by(|a, b| a.index.cmp(&b.index));
    out
}

/// Запись «В любое время». Как в Things, сюда входят и задачи из «Сегодня»
/// (с пометкой `in_today`): это всё, за что можно взяться прямо сейчас.
///
/// Исключение – задачи отложенного проекта («Когда-нибудь» или с будущей
/// датой): даже попав в «Сегодня» по своей дате или сроку, во «В любое время»
/// они не показываются (так ведёт себя Things).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnytimeEntry<'a> {
    pub task: &'a Task,
    pub in_today: bool,
}

/// «В любое время» в порядке `index`.
pub fn anytime<'a>(
    tasks: &'a [Task],
    projects: &'a [Project],
    areas: &'a [Area],
    headings: &'a [Heading],
    today: Date,
) -> Vec<AnytimeEntry<'a>> {
    let c = Containers::new(projects, areas, headings);
    let mut out: Vec<AnytimeEntry<'a>> = tasks
        .iter()
        .filter_map(|t| {
            let in_today = match c.place(t, today).list {
                List::Anytime => false,
                List::Today { .. } => true,
                _ => return None,
            };
            if c.project_of(t).is_some_and(|p| is_deferred(p, today)) {
                return None;
            }
            Some(AnytimeEntry { task: t, in_today })
        })
        .collect();
    out.sort_by(|a, b| a.task.index.cmp(&b.task.index));
    out
}

/// «Когда-нибудь»: отдельные задачи и проекты (задачи таких проектов – внутри них).
#[derive(Debug, Default)]
pub struct SomedayView<'a> {
    pub tasks: Vec<&'a Task>,
    pub projects: Vec<&'a Project>,
}

pub fn someday<'a>(
    tasks: &'a [Task],
    projects: &'a [Project],
    areas: &'a [Area],
    headings: &'a [Heading],
    today: Date,
) -> SomedayView<'a> {
    let c = Containers::new(projects, areas, headings);
    let mut view = SomedayView {
        tasks: tasks
            .iter()
            .filter(|t| {
                let p = c.place(t, today);
                p.list == List::Someday && p.is_own()
            })
            .collect(),
        projects: projects
            .iter()
            .filter(|p| c.place_project(p, today) == List::Someday)
            .collect(),
    };
    view.tasks.sort_by(|a, b| a.index.cmp(&b.index));
    view.projects.sort_by(|a, b| a.index.cmp(&b.index));
    view
}

/// Новые сверху.
fn newest_first<'a>(mut items: Vec<(Item<'a>, Option<Timestamp>)>) -> Vec<Item<'a>> {
    items.sort_by_key(|a| std::cmp::Reverse(a.1));
    items.into_iter().map(|(item, _)| item).collect()
}

/// «Журнал»: выполненные и отменённые задачи и проекты, новые сверху.
/// Не зависит от `today`. Удалённое – только в «Корзине», в том числе
/// выполненные задачи удалённого проекта, раздела или области: они вернутся сюда
/// вместе с контейнером.
pub fn logbook<'a>(
    tasks: &'a [Task],
    projects: &'a [Project],
    areas: &'a [Area],
    headings: &'a [Heading],
) -> Vec<Item<'a>> {
    let c = Containers::new(projects, areas, headings);
    let tasks = tasks
        .iter()
        .filter(|t| {
            closed_placement(t, c.project_of(t), c.heading_of(t), c.area_of(t))
                .is_some_and(|p| p.list == List::Logbook && p.is_own())
        })
        .map(|t| (Item::Task(t), t.status.closed_at()));
    let projects = projects
        .iter()
        .filter(|p| {
            !p.status.is_open()
                && !p.sync.is_deleted()
                && !c.area_of_project(p).is_some_and(|a| a.sync.is_deleted())
        })
        .map(|p| (Item::Project(p), p.status.closed_at()));
    newest_first(tasks.chain(projects).collect())
}

/// «Корзина»: мягко удалённые задачи, проекты, разделы и области, недавно удалённые
/// сверху. Содержимое удалённого проекта, раздела или области отдельными строками
/// не показывается – оно внутри них. Удалённое отдельно раньше – своей строкой.
pub fn trash<'a>(
    tasks: &'a [Task],
    projects: &'a [Project],
    areas: &'a [Area],
    headings: &'a [Heading],
) -> Vec<Item<'a>> {
    let tasks = tasks
        .iter()
        .filter(|t| t.sync.is_deleted())
        .map(|t| (Item::Task(t), t.sync.deleted_at));
    let projects = projects
        .iter()
        .filter(|p| p.sync.is_deleted())
        .map(|p| (Item::Project(p), p.sync.deleted_at));
    let headings = headings
        .iter()
        .filter(|h| h.sync.is_deleted())
        .map(|h| (Item::Heading(h), h.sync.deleted_at));
    let areas = areas
        .iter()
        .filter(|a| a.sync.is_deleted())
        .map(|a| (Item::Area(a), a.sync.deleted_at));
    newest_first(tasks.chain(projects).chain(headings).chain(areas).collect())
}
