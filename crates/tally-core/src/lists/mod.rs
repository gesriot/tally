//! Вычисляемые списки в духе Things 3. Ничего из этого не хранится.
//!
//! Всё зависит от параметра `today: Date`, поэтому смена суток – это просто
//! вызов с другой датой.
//!
//! # Правила (порядок проверки важен)
//!
//! Для задачи ([`place_task`]); `project` – её проект, `heading` – раздел,
//! `area` – её область (своя или область проекта):
//! 1. задача удалена → «Корзина»; удалён её проект → «Корзина» (через проект);
//!    удалён её раздел → «Корзина» (через раздел); удалена её область →
//!    «Корзина» (через область). Правило одно для списков и поиска – [`Deleted`];
//! 2. задача выполнена или отменена → «Журнал»; закрыт её проект → «Журнал» (через проект);
//! 3. есть дата: `date <= today` → «Сегодня» (в том числе просроченная дата начала),
//!    `date > today` → «Планы», если срок ещё не наступил (правило 4). Дата
//!    важнее `start`: задача из «Входящих» или «Когда-нибудь» с датой уходит
//!    в «Сегодня»/«Планы»;
//! 4. срок сегодня или прошёл → «Сегодня», дневной блок (задача сама приходит
//!    в «Сегодня» в день срока), даже если она «Когда-нибудь» и даже если
//!    дата начала ещё впереди: наступивший срок важнее будущего старта.
//!    `start` при этом не меняется: перенесли срок – задача вернулась туда, где была;
//! 5. проект сам в «Планах» или «Когда-нибудь» ([`place_project`]) → туда же
//!    (через проект);
//! 6. иначе по `start`: «Входящие» (только если нет проекта и области,
//!    иначе считается «В любое время»), «В любое время», «Когда-нибудь».
//!
//! «Вечер» учитывается только в «Сегодня»: `List::Today { evening }`.
//! Записи «через проект», «через раздел» или «через область» в представлениях
//! не показываются по отдельности – вместо них виден сам проект (как в Things)
//! или, в «Корзине», сам раздел или область.
//!
//! Проект ([`place_project`]) – по тем же правилам 1–4 и 6, но без «Входящих».
//!
//! «В любое время» – задачи из «В любое время» и «Сегодня», кроме задач
//! отложенного проекта (см. [`anytime`]). Очистка даты – [`Task::clear_date`]:
//! запись уходит во «В любое время».

mod item;
mod placement;
mod progress;
mod today;
mod upcoming;
mod views;

use std::collections::HashMap;

use jiff::civil::Date;

pub use item::Item;
pub use placement::{
    Deleted, List, Placement, Via, list_of, place_project, place_project_via, place_task,
};
pub use progress::{Progress, project_progress};
pub use today::{TodayView, today};
pub use upcoming::{DAY_GROUPS, Group, Reason, UpcomingEntry, UpcomingGroup, group_of, upcoming};
pub use views::{AnytimeEntry, SomedayView, anytime, inbox, logbook, someday, trash};

use crate::area::Area;
use crate::heading::Heading;
use crate::ids::{AreaId, HeadingId, ProjectId};
use crate::project::Project;
use crate::task::Task;

/// Быстрый поиск контейнеров записи: проекта, раздела и области.
pub(crate) struct Containers<'a> {
    projects: HashMap<ProjectId, &'a Project>,
    headings: HashMap<HeadingId, &'a Heading>,
    areas: HashMap<AreaId, &'a Area>,
}

impl<'a> Containers<'a> {
    pub(crate) fn new(projects: &'a [Project], areas: &'a [Area], headings: &'a [Heading]) -> Self {
        Self {
            projects: projects.iter().map(|p| (p.id, p)).collect(),
            headings: headings.iter().map(|h| (h.id, h)).collect(),
            areas: areas.iter().map(|a| (a.id, a)).collect(),
        }
    }

    pub(crate) fn heading_of(&self, task: &Task) -> Option<&'a Heading> {
        task.heading.and_then(|id| self.headings.get(&id).copied())
    }

    pub(crate) fn project_of(&self, task: &Task) -> Option<&'a Project> {
        task.project.and_then(|id| self.projects.get(&id).copied())
    }

    /// Область задачи: у задачи в проекте – область проекта (своя не
    /// учитывается), у задачи вне проекта – своя.
    pub(crate) fn area_of(&self, task: &Task) -> Option<&'a Area> {
        let id = if task.project.is_some() {
            self.project_of(task).and_then(|p| p.area)
        } else {
            task.area
        };
        id.and_then(|id| self.areas.get(&id).copied())
    }

    pub(crate) fn area_of_project(&self, project: &Project) -> Option<&'a Area> {
        project.area.and_then(|id| self.areas.get(&id).copied())
    }

    pub(crate) fn place(&self, task: &Task, today: Date) -> Placement {
        place_task(
            task,
            self.project_of(task),
            self.heading_of(task),
            self.area_of(task),
            today,
        )
    }

    pub(crate) fn place_project(&self, project: &Project, today: Date) -> List {
        place_project(project, self.area_of_project(project), today)
    }
}

/// Все списки сразу – для отображения и демонстрации.
#[derive(Debug)]
pub struct Lists<'a> {
    pub inbox: Vec<&'a Task>,
    pub today: TodayView<'a>,
    pub upcoming: Vec<UpcomingGroup<'a>>,
    pub anytime: Vec<AnytimeEntry<'a>>,
    pub someday: SomedayView<'a>,
    pub logbook: Vec<Item<'a>>,
    pub trash: Vec<Item<'a>>,
}

impl<'a> Lists<'a> {
    pub fn compute(
        tasks: &'a [Task],
        projects: &'a [Project],
        areas: &'a [Area],
        headings: &'a [Heading],
        today_date: Date,
    ) -> Self {
        Self {
            inbox: inbox(tasks, projects, areas, headings, today_date),
            today: today(tasks, projects, areas, headings, today_date),
            upcoming: upcoming(tasks, projects, areas, headings, today_date),
            anytime: anytime(tasks, projects, areas, headings, today_date),
            someday: someday(tasks, projects, areas, headings, today_date),
            logbook: logbook(tasks, projects, areas, headings),
            trash: trash(tasks, projects, areas, headings),
        }
    }
}
