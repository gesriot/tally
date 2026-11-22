//! Поиск записей в снимке базы: контейнеры задачи, короткие номера, задача
//! по номеру или названию, проекты, разделы, области и теги по названию.

use std::collections::HashMap;

use jiff::civil::Date;
use tally_core::lists::{Deleted, List, place_task};
use tally_core::{Area, AreaId, Heading, HeadingId, Project, ProjectId, Tag, Task};
use tally_store::{Snapshot, normalize};
use uuid::Uuid;

use crate::error::{Result, reject, rejected};

/// Короткий номер не бывает короче.
pub const MIN_ID: usize = 6;

/// Длина коротких номеров: не меньше [`MIN_ID`] знаков и столько, чтобы
/// номера всех `ids` различались.
pub fn id_len(ids: impl IntoIterator<Item = Uuid>) -> usize {
    let mut hex: Vec<String> = ids.into_iter().map(hex).collect();
    hex.sort_unstable();
    let common = hex
        .windows(2)
        .map(|w| {
            w[0].bytes()
                .zip(w[1].bytes())
                .take_while(|(a, b)| a == b)
                .count()
        })
        .max()
        .unwrap_or(0);
    (common + 1).clamp(MIN_ID, 32)
}

/// 32 шестнадцатеричных знака без дефисов.
pub fn hex(id: Uuid) -> String {
    id.simple().to_string()
}

/// Короткий номер: первые `len` знаков.
pub fn short(id: Uuid, len: usize) -> String {
    hex(id).chars().take(len).collect()
}

/// Номер одноимённой записи в диагностике: не короче 8 знаков, а если у
/// кандидатов начала совпадают – до первого различия, чтобы строки
/// различались.
pub fn candidate_id_len(ids: impl IntoIterator<Item = Uuid>) -> usize {
    id_len(ids).max(8)
}

/// Снимок с быстрым поиском контейнеров и длиной коротких номеров задач.
pub struct Index<'a> {
    s: &'a Snapshot,
    today: Date,
    projects: HashMap<ProjectId, &'a Project>,
    headings: HashMap<HeadingId, &'a Heading>,
    areas: HashMap<AreaId, &'a Area>,
    id_len: usize,
}

impl<'a> Index<'a> {
    pub fn new(s: &'a Snapshot, today: Date) -> Self {
        Self {
            s,
            today,
            projects: s.projects.iter().map(|p| (p.id, p)).collect(),
            headings: s.headings.iter().map(|h| (h.id, h)).collect(),
            areas: s.areas.iter().map(|a| (a.id, a)).collect(),
            id_len: id_len(s.tasks.iter().map(|t| t.id.0)),
        }
    }

    pub fn snapshot(&self) -> &'a Snapshot {
        self.s
    }

    pub fn today(&self) -> Date {
        self.today
    }

    /// Длина коротких номеров задач.
    pub fn id_len(&self) -> usize {
        self.id_len
    }

    pub fn project(&self, id: ProjectId) -> Option<&'a Project> {
        self.projects.get(&id).copied()
    }

    pub fn heading(&self, id: HeadingId) -> Option<&'a Heading> {
        self.headings.get(&id).copied()
    }

    pub fn area(&self, id: AreaId) -> Option<&'a Area> {
        self.areas.get(&id).copied()
    }

    pub fn project_of(&self, t: &Task) -> Option<&'a Project> {
        t.project.and_then(|id| self.project(id))
    }

    /// Область задачи: своя или её проекта.
    pub fn area_of(&self, t: &Task) -> Option<&'a Area> {
        let id = match self.project_of(t) {
            Some(p) => p.area,
            None => t.area,
        };
        id.and_then(|id| self.area(id))
    }

    pub fn area_of_project(&self, p: &Project) -> Option<&'a Area> {
        p.area.and_then(|id| self.area(id))
    }

    pub fn heading_of(&self, t: &Task) -> Option<&'a Heading> {
        t.heading.and_then(|id| self.heading(id))
    }

    /// Список задачи по правилам `tally-core`.
    pub fn list_of(&self, t: &Task) -> List {
        place_task(
            t,
            self.project_of(t),
            self.heading_of(t),
            self.area_of(t),
            self.today,
        )
        .list
    }

    /// В корзине – сама или через проект, раздел или область.
    pub fn trashed(&self, t: &Task) -> bool {
        Deleted::of_task(t, self.project_of(t), self.heading_of(t), self.area_of(t)).is_trashed()
    }

    /// Закрытую задачу можно вернуть в работу, если её контейнеры доступны.
    pub fn can_reopen(&self, task: &Task) -> bool {
        !task.status.is_open()
            && !self.trashed(task)
            && self
                .project_of(task)
                .is_none_or(|project| project.status.is_open())
    }

    /// «[3f9a2c] Купить хлеб».
    pub fn label(&self, t: &Task) -> String {
        format!("[{}] {}", short(t.id.0, self.id_len), t.title)
    }

    /// Задача по началу номера (от 4 знаков, дефисы не важны) или по части
    /// названия. Номер точнее: если он подходит, название не ищется.
    /// `open_titles` – по названию искать только открытые задачи.
    pub fn resolve(&self, arg: &str, open_titles: bool) -> Result<&'a Task> {
        let live: Vec<&'a Task> = self.s.tasks.iter().filter(|t| !self.trashed(t)).collect();
        let key: String = arg
            .trim()
            .to_lowercase()
            .chars()
            .filter(|&c| c != '-')
            .collect();
        if key.len() >= 4 && key.chars().all(|c| c.is_ascii_hexdigit()) {
            let found: Vec<&'a Task> = live
                .iter()
                .copied()
                .filter(|t| hex(t.id.0).starts_with(&key))
                .collect();
            if !found.is_empty() {
                return self.only(found, arg);
            }
        }
        let query = normalize(arg);
        if query.is_empty() {
            reject!("пустой номер задачи");
        }
        let found: Vec<&'a Task> = live
            .into_iter()
            // По названию – только задачи в работе: закрытая сама или через
            // закрытый проект (в «Журнале», как в tally-core) не в счёт.
            .filter(|t| !open_titles || !matches!(self.list_of(t), List::Logbook | List::Trash))
            .filter(|t| normalize(&t.title).contains(&query))
            .collect();
        // Точное совпадение названия важнее части.
        let exact: Vec<&'a Task> = found
            .iter()
            .copied()
            .filter(|t| normalize(&t.title) == query)
            .collect();
        // Несколько совпали целиком – выбирать только из них.
        if exact.is_empty() {
            self.only(found, arg)
        } else {
            self.only(exact, arg)
        }
    }

    fn only(&self, found: Vec<&'a Task>, arg: &str) -> Result<&'a Task> {
        match found.as_slice() {
            [one] => Ok(one),
            [] => reject!("задача «{arg}» не найдена"),
            many => {
                let list: Vec<String> = many
                    .iter()
                    .map(|t| format!("  {}", self.label(t)))
                    .collect();
                Err(rejected!(
                    "«{arg}» подходит к нескольким задачам:\n{}",
                    list.join("\n")
                ))
            }
        }
    }
}

/// Записи с названием `name` без учёта регистра и «ё».
pub(crate) fn by_name<'a, T>(
    items: impl Iterator<Item = &'a T>,
    title: impl Fn(&T) -> &str,
    name: &str,
) -> Vec<&'a T>
where
    T: 'a,
{
    let key = normalize(name);
    items.filter(|x| normalize(title(x)) == key).collect()
}

/// Живая запись с названием `name`: нет – `None`, одна – она, несколько
/// (например, после восстановления одноимённой из корзины) – ошибка со
/// списком, чтобы не выбрать наугад и не создать ещё одну.
fn unique<'a, T>(
    found: Vec<&'a T>,
    what: &str,
    name: &str,
    key: impl Fn(&T) -> (Uuid, &str),
) -> Result<Option<&'a T>> {
    if found.len() > 1 {
        let len = candidate_id_len(found.iter().map(|x| key(x).0));
        let list: Vec<String> = found
            .iter()
            .map(|x| {
                let (id, title) = key(x);
                format!("\n  [{}] {title}", short(id, len))
            })
            .collect();
        reject!("{what} «{name}» несколько:{}", list.concat());
    }
    Ok(found.into_iter().next())
}

pub(crate) fn area_named<'a>(s: &'a Snapshot, name: &str) -> Result<Option<&'a Area>> {
    let live = s.areas.iter().filter(|a| !a.sync.is_deleted());
    unique(
        by_name(live, |a| &a.title, name),
        "областей",
        name,
        |a| (a.id.0, a.title.as_str()),
    )
}

pub(crate) fn find_area<'a>(s: &'a Snapshot, name: &str) -> Result<&'a Area> {
    area_named(s, name)?.ok_or_else(|| rejected!("области «{name}» нет"))
}

/// Проект в работе: открыт и не в корзине (сам или через область).
pub(crate) fn project_named<'a>(s: &'a Snapshot, name: &str) -> Result<Option<&'a Project>> {
    let areas: HashMap<AreaId, &Area> = s.areas.iter().map(|a| (a.id, a)).collect();
    let live = s.projects.iter().filter(|p| {
        let area = p.area.and_then(|id| areas.get(&id).copied());
        p.status.is_open() && !Deleted::of_project(p, area).is_trashed()
    });
    unique(
        by_name(live, |p| &p.title, name),
        "проектов",
        name,
        |p| (p.id.0, p.title.as_str()),
    )
}

pub(crate) fn find_project<'a>(s: &'a Snapshot, name: &str) -> Result<&'a Project> {
    project_named(s, name)?.ok_or_else(|| rejected!("проекта «{name}» нет"))
}

pub(crate) fn heading_named<'a>(
    s: &'a Snapshot,
    project: &Project,
    name: &str,
) -> Result<Option<&'a Heading>> {
    heading_named_if(s, project, name, |_| true)
}

/// Живые разделы проекта `project` с названием `name` (не в корзине и не в
/// архиве), без учёта доступности самого проекта.
pub(crate) fn headings_named<'a>(
    s: &'a Snapshot,
    project: &Project,
    name: &str,
) -> Vec<&'a Heading> {
    let live = s
        .headings
        .iter()
        .filter(|h| h.project == project.id && !h.sync.is_deleted() && !h.archived);
    by_name(live, |h| &h.title, name)
}

/// Как [`heading_named`], но среди кандидатов только те, что проходят
/// `keep`: отбор идёт до проверки на неоднозначность.
pub(crate) fn heading_named_if<'a>(
    s: &'a Snapshot,
    project: &Project,
    name: &str,
    keep: impl Fn(&Heading) -> bool,
) -> Result<Option<&'a Heading>> {
    let found = headings_named(s, project, name)
        .into_iter()
        .filter(|h| keep(h))
        .collect();
    let what = format!("в проекте «{}» разделов", project.title);
    unique(found, &what, name, |h| (h.id.0, h.title.as_str()))
}

pub(crate) fn find_heading<'a>(
    s: &'a Snapshot,
    project: &Project,
    name: &str,
) -> Result<&'a Heading> {
    heading_named(s, project, name)?
        .ok_or_else(|| rejected!("раздела «{name}» в проекте «{}» нет", project.title))
}

pub(crate) fn find_tag<'a>(s: &'a Snapshot, name: &str) -> Result<Option<&'a Tag>> {
    let live = s.tags.iter().filter(|t| !t.sync.is_deleted());
    unique(by_name(live, |t| &t.title, name), "тегов", name, |t| {
        (t.id.0, t.title.as_str())
    })
}
