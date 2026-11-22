//! В каком списке находится задача или проект в день `today`.

use jiff::civil::Date;
use serde::{Deserialize, Serialize};

use crate::area::Area;
use crate::heading::Heading;
use crate::project::Project;
use crate::schedule::{DeadlineState, Start};
use crate::status::Status;
use crate::task::Task;

/// Основной список, где «живёт» запись.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum List {
    Inbox,
    Today { evening: bool },
    Upcoming,
    Anytime,
    Someday,
    Logbook,
    Trash,
}

/// Через что запись попала в список.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Via {
    /// По своим полям.
    Own,
    /// Через проект: проект удалён, закрыт или отложен.
    Project,
    /// Через раздел: раздел удалён.
    Heading,
    /// Через область: область удалена.
    Area,
}

/// Что удалено в цепочке «запись → раздел → проект → область».
///
/// Единое правило «в корзине ли запись»: им пользуются и списки
/// ([`place_task`], [`place_project`]), и поиск в `tally-store`, которому
/// достаточно флагов `deleted_at` из SQL.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Deleted {
    pub own: bool,
    pub heading: bool,
    pub project: bool,
    pub area: bool,
}

impl Deleted {
    /// Через что запись в «Корзине», или `None`, если она не там.
    /// Своё удаление важнее: такая запись – отдельная строка корзины.
    pub fn via(self) -> Option<Via> {
        if self.own {
            Some(Via::Own)
        } else if self.heading {
            Some(Via::Heading)
        } else if self.project {
            Some(Via::Project)
        } else if self.area {
            Some(Via::Area)
        } else {
            None
        }
    }

    pub fn is_trashed(self) -> bool {
        self.via().is_some()
    }

    /// Цепочка задачи; `area` – её область: у задачи в проекте – область
    /// проекта, вне проекта – своя.
    pub fn of_task(
        task: &Task,
        project: Option<&Project>,
        heading: Option<&Heading>,
        area: Option<&Area>,
    ) -> Self {
        Self {
            own: task.sync.is_deleted(),
            heading: heading.is_some_and(|h| h.sync.is_deleted()),
            project: project.is_some_and(|p| p.sync.is_deleted()),
            area: area.is_some_and(|a| a.sync.is_deleted()),
        }
    }

    /// Цепочка проекта.
    pub fn of_project(project: &Project, area: Option<&Area>) -> Self {
        Self {
            own: project.sync.is_deleted(),
            area: area.is_some_and(|a| a.sync.is_deleted()),
            ..Self::default()
        }
    }
}

/// Список и то, чем он определён.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub list: List,
    pub via: Via,
}

impl Placement {
    const fn own(list: List) -> Self {
        Self {
            list,
            via: Via::Own,
        }
    }

    const fn project(list: List) -> Self {
        Self {
            list,
            via: Via::Project,
        }
    }

    /// Запись видна в списке отдельной строкой (а не внутри проекта или области).
    pub fn is_own(self) -> bool {
        self.via == Via::Own
    }
}

/// Список задачи. Правила – в документации модуля [`crate::lists`].
///
/// `project` – проект задачи, `heading` – её раздел, `area` – её область: своя
/// (`task.area`) или область проекта. Не переданный контейнер считается
/// отсутствующим.
pub fn place_task(
    task: &Task,
    project: Option<&Project>,
    heading: Option<&Heading>,
    area: Option<&Area>,
    today: Date,
) -> Placement {
    if let Some(closed) = closed_placement(task, project, heading, area) {
        return closed;
    }
    if let Some(list) = dated(task.date, task.deadline, task.evening, today) {
        return Placement::own(list);
    }
    if let Some(p) = project
        && let list @ (List::Upcoming | List::Someday) = place_project(p, None, today)
    {
        return Placement::project(list);
    }
    let has_container = task.project.is_some() || task.area.is_some();
    Placement::own(match task.start {
        Start::Inbox if !has_container => List::Inbox,
        Start::Inbox | Start::Anytime => List::Anytime,
        Start::Someday => List::Someday,
    })
}

/// Правила 1–2: «Корзина» и «Журнал». От `today` не зависят, поэтому ими же
/// пользуются [`crate::lists::logbook`] и [`crate::lists::trash`].
pub(crate) fn closed_placement(
    task: &Task,
    project: Option<&Project>,
    heading: Option<&Heading>,
    area: Option<&Area>,
) -> Option<Placement> {
    if let Some(via) = Deleted::of_task(task, project, heading, area).via() {
        return Some(Placement {
            list: List::Trash,
            via,
        });
    }
    if !task.status.is_open() {
        return Some(Placement::own(List::Logbook));
    }
    if project.is_some_and(|p| !p.status.is_open()) {
        return Some(Placement::project(List::Logbook));
    }
    None
}

/// Только список задачи (без признака «через проект»).
pub fn list_of(
    task: &Task,
    project: Option<&Project>,
    heading: Option<&Heading>,
    area: Option<&Area>,
    today: Date,
) -> List {
    place_task(task, project, heading, area, today).list
}

/// Список проекта; `area` – его область. Удалённая область уносит проект
/// в «Корзину» (в корзине при этом видна одна строка – сама область).
pub fn place_project(project: &Project, area: Option<&Area>, today: Date) -> List {
    place_project_via(project, area, today).list
}

/// Как [`place_project`], но с признаком «через область».
pub fn place_project_via(project: &Project, area: Option<&Area>, today: Date) -> Placement {
    if let Some(via) = Deleted::of_project(project, area).via() {
        return Placement {
            list: List::Trash,
            via,
        };
    }
    if project.status != Status::Open {
        return Placement::own(List::Logbook);
    }
    if let Some(list) = dated(project.date, project.deadline, project.evening, today) {
        return Placement::own(list);
    }
    Placement::own(match project.start {
        Start::Inbox | Start::Anytime => List::Anytime,
        Start::Someday => List::Someday,
    })
}

/// Проект отложен: сам он в «Планах» или «Когда-нибудь». Его задачи без своей
/// даты видны только внутри проекта, а задачи со своей датой или сроком
/// попадают в «Сегодня» и «Планы», но не в «В любое время».
pub(crate) fn is_deferred(project: &Project, today: Date) -> bool {
    matches!(
        place_project(project, None, today),
        List::Upcoming | List::Someday
    )
}

/// Правила 3–4: дата начала и срок.
///
/// Наступивший срок (сегодня или просрочен) важнее будущей даты начала: запись
/// в «Сегодня», даже если начать её собирались позже. В «Сегодня» по сроку
/// запись всегда в дневном блоке: «Вечер» относится к дате начала (будущей
/// или отсутствующей), а не к сегодняшнему дню.
/// В «Планах» на день начала такая запись не повторяется: она уже в «Сегодня»
/// и останется там и в день начала (см. [`crate::lists::upcoming`]).
fn dated(date: Option<Date>, deadline: Option<Date>, evening: bool, today: Date) -> Option<List> {
    let due = deadline.is_some_and(|d| DeadlineState::of(d, today).is_due());
    match date {
        Some(d) if d <= today => Some(List::Today { evening }),
        _ if due => Some(List::Today { evening: false }),
        Some(_) => Some(List::Upcoming),
        None => None,
    }
}
