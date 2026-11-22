//! Модель отображения: боковая панель и строки списка из снимка базы.
//! Чистые функции без Slint – их проверяют модульные тесты; `.slint`
//! только рисует готовые строки.
//!
//! Списки считает `tally-core` по тем же правилам, что и `tally list` в
//! консоли: «Сегодня» с блоком «Вечер», «Планы» по дням и месяцам,
//! «В любое время» с пометкой задач из «Сегодня» и т. д.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use jiff::{Timestamp, civil::Date};
use tally_app::{Index, TaskLane, candidate_id_len, short};
use tally_core::lists::{Deleted, Group, Item, List, project_progress};
use tally_core::schedule::DeadlineState;
use tally_core::{AreaId, Project, ProjectId, Status, Tag, TagId, Task, TaskId};
use tally_store::{Found, Hit, RecordId, Snapshot, normalize};
use uuid::Uuid;

use crate::strings;

/// Что открыто в правой части окна.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    Inbox,
    Today,
    Upcoming,
    Anytime,
    Someday,
    Logbook,
    Trash,
    Area(AreaId),
    Project(ProjectId),
    Tag(TagId),
}

/// Стандартные списки в порядке боковой панели.
const LISTS: [Selection; 7] = [
    Selection::Inbox,
    Selection::Today,
    Selection::Upcoming,
    Selection::Anytime,
    Selection::Someday,
    Selection::Logbook,
    Selection::Trash,
];

impl Selection {
    /// Ключ для `.slint`: «today», «project:<uuid>».
    pub fn key(self) -> String {
        match self {
            Self::Inbox => "inbox".into(),
            Self::Today => "today".into(),
            Self::Upcoming => "upcoming".into(),
            Self::Anytime => "anytime".into(),
            Self::Someday => "someday".into(),
            Self::Logbook => "logbook".into(),
            Self::Trash => "trash".into(),
            Self::Area(id) => format!("area:{id}"),
            Self::Project(id) => format!("project:{id}"),
            Self::Tag(id) => format!("tag:{id}"),
        }
    }

    pub fn parse(key: &str) -> Option<Self> {
        if let Some(id) = key.strip_prefix("area:") {
            return Uuid::parse_str(id).ok().map(|u| Self::Area(AreaId(u)));
        }
        if let Some(id) = key.strip_prefix("project:") {
            return Uuid::parse_str(id)
                .ok()
                .map(|u| Self::Project(ProjectId(u)));
        }
        if let Some(id) = key.strip_prefix("tag:") {
            return Uuid::parse_str(id).ok().map(|u| Self::Tag(TagId(u)));
        }
        LISTS.into_iter().find(|s| s.key() == key)
    }

    pub(crate) fn drop_lane(self) -> Option<TaskLane> {
        Some(match self {
            Self::Inbox => TaskLane::Inbox,
            Self::Today => TaskLane::Today { evening: false },
            Self::Anytime => TaskLane::Anytime,
            Self::Someday => TaskLane::Someday,
            Self::Project(id) => TaskLane::Project(id, None),
            Self::Area(id) => TaskLane::Area(id),
            Self::Upcoming | Self::Logbook | Self::Trash | Self::Tag(_) => return None,
        })
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Inbox => strings::INBOX,
            Self::Today => strings::TODAY,
            Self::Upcoming => strings::UPCOMING,
            Self::Anytime => strings::ANYTIME,
            Self::Someday => strings::SOMEDAY,
            Self::Logbook => strings::LOGBOOK,
            Self::Trash => strings::TRASH,
            Self::Area(_) | Self::Project(_) | Self::Tag(_) => "",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Self::Inbox => Icon::Inbox,
            Self::Today => Icon::Today,
            Self::Upcoming => Icon::Upcoming,
            Self::Anytime => Icon::Anytime,
            Self::Someday => Icon::Someday,
            Self::Logbook => Icon::Logbook,
            Self::Trash => Icon::Trash,
            Self::Area(_) | Self::Tag(_) => Icon::Area,
            Self::Project(_) => Icon::Project,
        }
    }
}

/// Временная иконка (итоговые рисует автор).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Inbox,
    Today,
    Upcoming,
    Anytime,
    Someday,
    Logbook,
    Trash,
    Area,
    Project,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideKind {
    /// Стандартный список.
    List,
    /// Пустая строка-разделитель.
    Gap,
    Area,
    Project,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SideRow {
    pub kind: SideKind,
    pub nested: bool,
    pub key: String,
    pub title: String,
    /// Число задач справа («Входящие», «Сегодня»); пусто – не показывать.
    pub count: String,
    /// В «Сегодня» есть задачи со сроком сегодня или просроченные.
    pub due: bool,
    pub selected: bool,
    pub icon: Icon,
    /// Доля выполненного у проекта, 0–1.
    pub progress: f32,
}

impl SideRow {
    fn gap() -> Self {
        Self {
            kind: SideKind::Gap,
            nested: false,
            key: String::new(),
            title: String::new(),
            count: String::new(),
            due: false,
            selected: false,
            icon: Icon::Inbox,
            progress: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Task,
    Project,
    Link,
    /// Заголовок блока: «Вечер», раздел проекта, день «Планов».
    Section,
    /// Строка «Корзины» без отметки: удалённый раздел или область.
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Open,
    Completed,
    Canceled,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub kind: RowKind,
    /// UUID задачи или ключ проекта ([`Selection::key`]).
    pub key: String,
    /// Ключ вхождения в списке. В «Планах» у старта и срока разные строки.
    pub row_key: String,
    pub title: String,
    /// Когда, если это не ясно из списка: «пт, 20 ноября», «вечер».
    pub when: String,
    /// Проект (и раздел) или область, теги: «Ремонт / Кухня · #срочно».
    pub meta: String,
    /// Срок: «сегодня», «пт, 27 ноября»; пусто – нет.
    pub deadline: String,
    /// Срок сегодня или прошёл.
    pub due: bool,
    pub mark: Mark,
    /// Отметку можно нажать – по [`toggle`], тому же правилу, что и для
    /// щелчка.
    pub checkable: bool,
    /// Строку проекта можно открыть: проект в работе ([`valid`] его
    /// примет). Закрытый проект в «Журнале» и удалённый в «Корзине» – нет.
    pub opens: bool,
    pub editable: bool,
    pub reopenable: bool,
    /// Задача «В любое время», которая сейчас и в «Сегодня».
    pub star: bool,
    pub progress: f32,
    pub(crate) drop_lane: Option<TaskLane>,
    pub(crate) drag_task: Option<(TaskId, Timestamp)>,
    pub(crate) trash_record: Option<(RecordId, Timestamp)>,
}

impl Row {
    fn link(title: &str, selection: Selection, kind: &str) -> Self {
        let mut row = Self::section(title);
        row.kind = RowKind::Link;
        row.key = selection.key();
        row.row_key.clone_from(&row.key);
        row.when = kind.into();
        row.opens = true;
        row
    }

    fn section(title: impl Into<String>) -> Self {
        Self {
            kind: RowKind::Section,
            key: String::new(),
            row_key: String::new(),
            title: title.into(),
            when: String::new(),
            meta: String::new(),
            deadline: String::new(),
            due: false,
            mark: Mark::Open,
            checkable: false,
            opens: false,
            editable: false,
            reopenable: false,
            star: false,
            progress: 0.0,
            drop_lane: None,
            drag_task: None,
            trash_record: None,
        }
    }

    fn drop_section(title: impl Into<String>, lane: TaskLane) -> Self {
        let mut row = Self::section(title);
        row.drop_lane = Some(lane);
        row
    }
}

/// Страница: заголовок и строки.
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub title: String,
    pub progress: Option<tally_core::lists::Progress>,
    pub rows: Vec<Row>,
}

/// Проект в работе: открыт и не в корзине (сам или через область).
fn live_project(s: &Snapshot, p: &Project) -> bool {
    let area = p.area.and_then(|a| s.areas.iter().find(|x| x.id == a));
    p.status.is_open() && !Deleted::of_project(p, area).is_trashed()
}

/// Можно ли открыть выбор: стандартный список – всегда, область – если она
/// не в корзине, проект – если он в работе. Одно правило для строк проекта
/// ([`Row::opens`]), перехода ([`crate::controller::Controller::select`]) и
/// [`valid`].
pub fn can_open(s: &Snapshot, selection: Selection) -> bool {
    match selection {
        Selection::Area(id) => s.areas.iter().any(|a| a.id == id && !a.sync.is_deleted()),
        Selection::Project(id) => s.projects.iter().any(|p| p.id == id && live_project(s, p)),
        Selection::Tag(id) => s.tags.iter().any(|t| t.id == id && !t.sync.is_deleted()),
        _ => true,
    }
}

/// Проекты в работе: открыты и не в корзине (сами или через область), в
/// порядке `index` – как `tally list projects`.
fn open_projects<'a>(ix: &Index<'a>) -> Vec<&'a Project> {
    let mut out: Vec<&Project> = ix
        .snapshot()
        .projects
        .iter()
        .filter(|p| live_project(ix.snapshot(), p))
        .collect();
    out.sort_by(|a, b| a.index.cmp(&b.index));
    out
}

fn progress_of(s: &Snapshot, p: &Project) -> f32 {
    project_progress(p.id, &s.tasks, &s.headings).fraction() as f32
}

/// Выбор, которого больше нет (проект закрыли или удалили из консоли), –
/// «Сегодня».
pub fn valid(s: &Snapshot, _today: Date, selection: Selection) -> Selection {
    if can_open(s, selection) {
        selection
    } else {
        Selection::Today
    }
}

/// Боковая панель: стандартные списки, затем проекты без области, затем
/// области со своими проектами.
pub fn sidebar(s: &Snapshot, today: Date, selection: Selection) -> Vec<SideRow> {
    let ix = Index::new(s, today);
    let lists = s.lists(today);
    let due = lists
        .today
        .day
        .iter()
        .chain(&lists.today.evening)
        .any(|i| match i {
            Item::Task(t) => t
                .deadline
                .is_some_and(|d| DeadlineState::of(d, today).is_due()),
            Item::Project(p) => p
                .deadline
                .is_some_and(|d| DeadlineState::of(d, today).is_due()),
            _ => false,
        });
    let count = |n: usize| if n == 0 { String::new() } else { n.to_string() };
    let mut rows = Vec::new();
    for sel in LISTS {
        if sel == Selection::Logbook {
            rows.push(SideRow::gap());
        }
        let (n, d) = match sel {
            Selection::Inbox => (count(lists.inbox.len()), false),
            Selection::Today => (count(lists.today.len()), due),
            _ => (String::new(), false),
        };
        rows.push(SideRow {
            kind: SideKind::List,
            nested: false,
            key: sel.key(),
            title: sel.title().to_owned(),
            count: n,
            due: d,
            selected: sel == selection,
            icon: sel.icon(),
            progress: 0.0,
        });
    }
    let projects = open_projects(&ix);
    let project_row = |p: &Project| SideRow {
        kind: SideKind::Project,
        nested: p.area.is_some(),
        key: Selection::Project(p.id).key(),
        title: p.title.clone(),
        count: p
            .date
            .filter(|_| p.repeat_rule.is_some() || p.repeat_advanced)
            .map_or_else(String::new, |day| {
                if day.year() == today.year() {
                    format!("{:02}.{:02}", day.day(), day.month())
                } else {
                    format!("{:02}.{:02}.{}", day.day(), day.month(), day.year())
                }
            }),
        due: false,
        selected: selection == Selection::Project(p.id),
        icon: Icon::Project,
        progress: progress_of(s, p),
    };
    let loose: Vec<&&Project> = projects.iter().filter(|p| p.area.is_none()).collect();
    if !loose.is_empty() {
        rows.push(SideRow::gap());
        rows.extend(loose.into_iter().map(|p| project_row(p)));
    }
    let mut areas: Vec<_> = s.areas.iter().filter(|a| !a.sync.is_deleted()).collect();
    areas.sort_by(|a, b| a.index.cmp(&b.index));
    for a in areas {
        rows.push(SideRow::gap());
        rows.push(SideRow {
            kind: SideKind::Area,
            nested: false,
            key: Selection::Area(a.id).key(),
            title: a.title.clone(),
            count: String::new(),
            due: false,
            selected: selection == Selection::Area(a.id),
            icon: Icon::Area,
            progress: 0.0,
        });
        rows.extend(
            projects
                .iter()
                .filter(|p| p.area == Some(a.id))
                .map(|p| project_row(p)),
        );
    }
    rows
}

/// Снимок, в котором задачи `kept` (закрытые в этом окне, пока открыт тот
/// же список) снова открыты: так они остаются на своём месте – с
/// отметкой – до перехода в другой список, как в Things.
fn with_kept<'a>(s: &'a Snapshot, originals: &HashMap<TaskId, &Task>) -> Cow<'a, Snapshot> {
    if originals.is_empty() {
        return Cow::Borrowed(s);
    }
    let mut shown = s.clone();
    for task in &mut shown.tasks {
        if originals.contains_key(&task.id) {
            task.status = Status::Open;
        }
    }
    Cow::Owned(shown)
}

/// Что сделает щелчок по отметке задачи.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggle {
    /// Выполнить открытую задачу.
    Close,
    /// Снять отметку: отменить закрытие задачи.
    Reopen,
}

/// Единственное правило отметки: и для вида строки (`checkable` – указатель,
/// доступность для экранного диктора), и для самого щелчка
/// ([`crate::controller::Controller::toggle`]). Задача в корзине (сама или
/// через проект, раздел, область) не отмечается. Открытую можно выполнить,
/// если `allow(id, true)`; закрытую – снять отметку, если `allow(id,
/// false)`. `allow` решает контроллер: закрытие ещё в пути, снять можно
/// только последнее закрытие окна.
pub fn toggle(s: &Snapshot, id: TaskId, allow: &dyn Fn(TaskId, bool) -> bool) -> Option<Toggle> {
    let t = s.tasks.iter().find(|t| t.id == id)?;
    let project = t
        .project
        .and_then(|p| s.projects.iter().find(|x| x.id == p));
    let heading = t
        .heading
        .and_then(|h| s.headings.iter().find(|x| x.id == h));
    // Область – как `Index::area_of`: у задачи в проекте – область проекта.
    let area = match project {
        Some(p) => p.area,
        None => t.area,
    }
    .and_then(|a| s.areas.iter().find(|x| x.id == a));
    row_toggle(
        t,
        Deleted::of_task(t, project, heading, area).is_trashed(),
        allow,
    )
}

fn row_toggle(task: &Task, trashed: bool, allow: &dyn Fn(TaskId, bool) -> bool) -> Option<Toggle> {
    let open = task.status.is_open();
    if trashed || !allow(task.id, open) {
        return None;
    }
    Some(if open { Toggle::Close } else { Toggle::Reopen })
}

/// Страница выбранного списка, проекта или области. Отметку можно снять
/// у задач `kept`; см. [`page_with`].
pub fn page(s: &Snapshot, today: Date, selection: Selection, kept: &[TaskId]) -> Page {
    page_with(s, today, selection, kept, &|id, open| {
        open || kept.contains(&id)
    })
}

/// Страница: задачи `kept` (закрытые в этом окне на этой странице) стоят на
/// своих местах с отметкой; нажимаемость отметок – по [`toggle`] с `allow`.
pub fn page_with(
    s: &Snapshot,
    today: Date,
    selection: Selection,
    kept: &[TaskId],
    allow: &dyn Fn(TaskId, bool) -> bool,
) -> Page {
    page_filtered(s, today, selection, kept, allow, None)
}

pub(crate) fn page_filtered(
    s: &Snapshot,
    today: Date,
    selection: Selection,
    kept: &[TaskId],
    allow: &dyn Fn(TaskId, bool) -> bool,
    tag: Option<TagId>,
) -> Page {
    let filter = match selection {
        Selection::Tag(id) => Some(id),
        Selection::Trash => None,
        _ => tag,
    }
    .map(|id| TagFilter::new(s, id));
    let kept: HashSet<_> = kept.iter().copied().collect();
    let originals = if kept.is_empty() {
        HashMap::new()
    } else {
        s.tasks
            .iter()
            .filter(|t| !t.status.is_open() && kept.contains(&t.id))
            .map(|t| (t.id, t))
            .collect()
    };
    let shown = with_kept(s, &originals);
    let ix = Index::new(&shown, today);
    let mut rows = Rows {
        ix: &ix,
        real: s,
        originals,
        here: None,
        allow,
    };
    let task = |t: &Task| filter.as_ref().is_none_or(|f| f.task(&ix, t));
    let item = |i: Item<'_>| filter.as_ref().is_none_or(|f| f.item(&ix, i));
    let lists = shown.lists(today);
    let mut out = Vec::new();
    let title = match selection {
        Selection::Inbox => {
            out.extend(
                lists
                    .inbox
                    .iter()
                    .filter(|t| task(t))
                    .map(|t| rows.task(t, "", false)),
            );
            strings::INBOX.to_owned()
        }
        Selection::Today => {
            out.extend(
                lists
                    .today
                    .day
                    .iter()
                    .copied()
                    .filter(|&i| item(i))
                    .map(|i| rows.item(i, "")),
            );
            let mut evening = lists
                .today
                .evening
                .iter()
                .copied()
                .filter(|&i| item(i))
                .peekable();
            if evening.peek().is_some() {
                out.push(Row::drop_section(
                    strings::EVENING,
                    TaskLane::Today { evening: true },
                ));
                out.extend(evening.map(|i| rows.item(i, "")));
            }
            strings::TODAY.to_owned()
        }
        Selection::Upcoming => {
            for g in &lists.upcoming {
                // Семь дней с завтрашнего – всегда, как в консоли.
                let mut entries = g.entries.iter().filter(|e| item(e.item)).peekable();
                if entries.peek().is_none() && !matches!(g.group, Group::Day(_)) {
                    continue;
                }
                out.push(Row::section(strings::group(g.group, today)));
                let mut last_day = None;
                for e in entries {
                    if !matches!(g.group, Group::Day(_)) && last_day != Some(e.date) {
                        out.push(Row::section(strings::date(e.date, today)));
                        last_day = Some(e.date);
                    }
                    let evening = e.reason.is_scheduled() && e.item.evening();
                    let when = if evening { strings::EVENING_SHORT } else { "" };
                    let mut row = rows.item(e.item, when);
                    if matches!(row.kind, RowKind::Task | RowKind::Project) {
                        let occurrence = if e.reason.is_scheduled() {
                            "start"
                        } else {
                            "deadline"
                        };
                        row.row_key = format!("{}:{occurrence}", row.key);
                    }
                    if e.reason.has_deadline() {
                        row.deadline = strings::DEADLINE_DAY.to_owned();
                        row.due = false;
                    }
                    out.push(row);
                }
            }
            strings::UPCOMING.to_owned()
        }
        Selection::Anytime => {
            out.extend(
                lists
                    .anytime
                    .iter()
                    .filter(|e| task(e.task))
                    .map(|e| rows.task(e.task, "", e.in_today)),
            );
            strings::ANYTIME.to_owned()
        }
        Selection::Someday => {
            out.extend(
                lists
                    .someday
                    .tasks
                    .iter()
                    .filter(|t| task(t))
                    .map(|t| rows.task(t, "", false)),
            );
            out.extend(
                lists
                    .someday
                    .projects
                    .iter()
                    .filter(|p| item(Item::Project(p)))
                    .map(|p| rows.item(Item::Project(p), "")),
            );
            strings::SOMEDAY.to_owned()
        }
        Selection::Logbook => {
            let zone = jiff::tz::TimeZone::system();
            out.extend(
                lists
                    .logbook
                    .iter()
                    .copied()
                    .filter(|&i| item(i))
                    .map(|item| {
                        let closed = match item {
                            Item::Task(t) => t.status.closed_at(),
                            Item::Project(p) => p.status.closed_at(),
                            _ => None,
                        };
                        let day = closed.map(|at| at.to_zoned_datetime(zone.clone()).date());
                        let when = day.map_or_else(String::new, |d| strings::date(d, today));
                        rows.item(item, &when)
                    }),
            );
            strings::LOGBOOK.to_owned()
        }
        Selection::Trash => {
            out.extend(lists.trash.iter().map(|i| rows.item(*i, "")));
            strings::TRASH.to_owned()
        }
        Selection::Project(id) => match ix.project(id) {
            Some(p) => {
                rows.here = Some(Here::Project(id));
                project_page(&rows, p, filter.as_ref(), &mut out);
                p.title.clone()
            }
            None => String::new(),
        },
        Selection::Area(id) => match ix.area(id) {
            Some(a) => {
                rows.here = Some(Here::Area(id));
                for p in open_projects(&ix)
                    .into_iter()
                    .filter(|p| p.area == Some(id) && item(Item::Project(p)))
                {
                    out.push(rows.item(Item::Project(p), ""));
                }
                let mut tasks: Vec<&Task> = shown
                    .tasks
                    .iter()
                    .filter(|t| t.project.is_none() && t.area == Some(id) && task(t))
                    .filter(|t| !matches!(ix.list_of(t), List::Logbook | List::Trash))
                    .collect();
                tasks.sort_by(|a, b| a.index.cmp(&b.index));
                out.extend(
                    tasks
                        .into_iter()
                        .map(|t| rows.task(t, &rows.when(t), false)),
                );
                a.title.clone()
            }
            None => String::new(),
        },
        Selection::Tag(id) => {
            let mut areas: Vec<_> = shown
                .areas
                .iter()
                .filter(|a| !a.sync.is_deleted() && item(Item::Area(a)))
                .collect();
            areas.sort_by(|a, b| a.index.cmp(&b.index));
            out.extend(
                areas
                    .into_iter()
                    .map(|a| Row::link(&a.title, Selection::Area(a.id), strings::SEARCH_AREA)),
            );
            out.extend(
                open_projects(&ix)
                    .into_iter()
                    .filter(|p| item(Item::Project(p)))
                    .map(|p| rows.item(Item::Project(p), "")),
            );
            for open in [true, false] {
                if !open {
                    out.extend(
                        lists
                            .logbook
                            .iter()
                            .copied()
                            .filter(|&entry| matches!(entry, Item::Project(_)) && item(entry))
                            .map(|entry| rows.item(entry, "")),
                    );
                }
                out.extend(
                    shown
                        .tasks
                        .iter()
                        .filter(|t| {
                            !ix.trashed(t) && (ix.list_of(t) != List::Logbook) == open && task(t)
                        })
                        .map(|t| rows.task(t, &rows.when(t), false)),
                );
            }
            tag_labels(&s.tags).remove(&id).unwrap_or_default()
        }
    };
    let mut lane = selection.drop_lane();
    for row in &mut out {
        match row.kind {
            RowKind::Section => lane = row.drop_lane,
            RowKind::Task => row.drop_lane = lane,
            RowKind::Project if matches!(lane, Some(TaskLane::Today { .. })) => {
                row.drop_lane = lane
            }
            _ => {}
        }
    }
    let progress = match selection {
        Selection::Project(id) if ix.project(id).is_some() => {
            Some(project_progress(id, &s.tasks, &s.headings))
        }
        _ => None,
    };
    Page {
        title,
        progress,
        rows: out,
    }
}

pub(crate) fn search_page(
    s: &Snapshot,
    today: Date,
    hits: &[Hit],
    allow: &dyn Fn(TaskId, bool) -> bool,
) -> Page {
    if hits.is_empty() {
        return Page {
            title: strings::SEARCH.to_owned(),
            progress: None,
            rows: Vec::new(),
        };
    }
    let ix = Index::new(s, today);
    let rows = Rows {
        ix: &ix,
        real: s,
        originals: HashMap::new(),
        here: None,
        allow,
    };
    let tasks: HashMap<_, _> = s.tasks.iter().map(|t| (t.id, t)).collect();
    let tags = tag_labels(&s.tags);
    let rows = hits
        .iter()
        .filter_map(|hit| match hit.found {
            Found::Task(id) => tasks
                .get(&id)
                .filter(|t| !ix.trashed(t))
                .map(|t| rows.task(t, &rows.when(t), false)),
            Found::Project(id) => ix
                .project(id)
                .filter(|p| !Deleted::of_project(p, ix.area_of_project(p)).is_trashed())
                .map(|p| rows.item(Item::Project(p), strings::SEARCH_PROJECT)),
            Found::Area(id) => ix
                .area(id)
                .filter(|a| !a.sync.is_deleted())
                .map(|a| Row::link(&a.title, Selection::Area(id), strings::SEARCH_AREA)),
            Found::Tag(id) => tags
                .get(&id)
                .map(|label| Row::link(label, Selection::Tag(id), strings::SEARCH_TAG)),
        })
        .collect();
    Page {
        title: strings::SEARCH.to_owned(),
        progress: None,
        rows,
    }
}

/// Одинаковые названия различаем путём; совпавшие пути – номером тега.
pub(crate) fn tag_labels(tags: &[Tag]) -> HashMap<TagId, String> {
    let live: HashMap<_, _> = tags
        .iter()
        .filter(|t| !t.sync.is_deleted())
        .map(|t| (t.id, t))
        .collect();
    let mut title_count = HashMap::new();
    for tag in live.values() {
        *title_count.entry(normalize(&tag.title)).or_insert(0usize) += 1;
    }
    let mut labels: HashMap<_, _> = live
        .values()
        .map(|tag| {
            let mut parts = vec![tag.title.as_str()];
            if title_count[&normalize(&tag.title)] > 1 {
                let mut visited = HashSet::from([tag.id]);
                let mut parent = tag.parent;
                while let Some(id) = parent.filter(|id| visited.insert(*id)) {
                    let Some(ancestor) = live.get(&id) else {
                        break;
                    };
                    parts.push(&ancestor.title);
                    parent = ancestor.parent;
                }
                parts.reverse();
            }
            (tag.id, parts.join(" / "))
        })
        .collect();
    let mut matching_paths = HashMap::<_, Vec<_>>::new();
    for (&id, label) in &labels {
        matching_paths.entry(normalize(label)).or_default().push(id);
    }
    for ids in matching_paths.values().filter(|ids| ids.len() > 1) {
        let length = candidate_id_len(ids.iter().map(|id| id.0));
        for id in ids {
            if let Some(label) = labels.get_mut(id) {
                label.push_str(&format!(" [{}]", short(id.0, length)));
            }
        }
    }
    labels
}

struct TagFilter(HashSet<TagId>);

impl TagFilter {
    fn new(s: &Snapshot, root: TagId) -> Self {
        let mut children = HashMap::<TagId, Vec<TagId>>::new();
        for tag in s.tags.iter().filter(|t| !t.sync.is_deleted()) {
            if let Some(parent) = tag.parent {
                children.entry(parent).or_default().push(tag.id);
            }
        }
        let mut matched = HashSet::new();
        let mut pending = vec![root];
        while let Some(id) = pending.pop() {
            if matched.insert(id)
                && let Some(children) = children.get(&id)
            {
                pending.extend(children);
            }
        }
        Self(matched)
    }

    fn tagged(&self, tags: &[TagId]) -> bool {
        tags.iter().any(|id| self.0.contains(id))
    }

    fn task(&self, ix: &Index<'_>, task: &Task) -> bool {
        self.tagged(&task.tags)
            || ix.project_of(task).is_some_and(|p| self.tagged(&p.tags))
            || ix.area_of(task).is_some_and(|a| self.tagged(&a.tags))
    }

    fn item(&self, ix: &Index<'_>, item: Item<'_>) -> bool {
        match item {
            Item::Task(t) => self.task(ix, t),
            Item::Project(p) => {
                self.tagged(&p.tags) || ix.area_of_project(p).is_some_and(|a| self.tagged(&a.tags))
            }
            Item::Area(a) => self.tagged(&a.tags),
            Item::Heading(_) => false,
        }
    }
}

/// Страница проекта: задачи без раздела, затем разделы по порядку (без
/// архивных и удалённых), в каждом – его задачи в порядке `index`.
/// Выполненные и удалённые задачи – в «Журнале» и «Корзине».
fn project_page(rows: &Rows<'_, '_>, p: &Project, filter: Option<&TagFilter>, out: &mut Vec<Row>) {
    let s = rows.ix.snapshot();
    let mut tasks: Vec<&Task> = s
        .tasks
        .iter()
        .filter(|t| t.project == Some(p.id) && filter.is_none_or(|f| f.task(rows.ix, t)))
        .filter(|t| !matches!(rows.ix.list_of(t), List::Logbook | List::Trash))
        .collect();
    tasks.sort_by(|a, b| a.index.cmp(&b.index));
    let mut headings: Vec<_> = s
        .headings
        .iter()
        .filter(|h| h.project == p.id && !h.archived && !h.sync.is_deleted())
        .collect();
    headings.sort_by(|a, b| a.index.cmp(&b.index));
    let row = |t: &&Task| rows.task(t, &rows.when(t), false);
    out.extend(tasks.iter().filter(|t| t.heading.is_none()).map(row));
    for h in headings {
        if filter.is_some() && !tasks.iter().any(|t| t.heading == Some(h.id)) {
            continue;
        }
        let mut section = Row::drop_section(h.title.clone(), TaskLane::Project(p.id, Some(h.id)));
        section.trash_record = Some((RecordId::Heading(h.id), h.sync.updated_at));
        out.push(section);
        out.extend(tasks.iter().filter(|t| t.heading == Some(h.id)).map(row));
    }
}

/// Страница проекта или области: сам контейнер в строках не повторяется.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Here {
    Project(ProjectId),
    Area(AreaId),
}

/// Строки для одного снимка. `ix` – снимок для показа (с «удержанными»
/// задачами), `real` – как в базе: по нему ставится отметка.
struct Rows<'i, 'a> {
    ix: &'i Index<'a>,
    real: &'a Snapshot,
    originals: HashMap<TaskId, &'a Task>,
    here: Option<Here>,
    allow: &'i dyn Fn(TaskId, bool) -> bool,
}

impl Rows<'_, '_> {
    fn today(&self) -> Date {
        self.ix.today()
    }

    /// Когда задача, если это не «В любое время» – для страниц проекта и
    /// области, где списки смешаны.
    fn when(&self, t: &Task) -> String {
        match self.ix.list_of(t) {
            List::Today { evening } => strings::when_today(evening),
            List::Upcoming => match t.date {
                Some(d) if t.evening => strings::with_evening(&strings::date(d, self.today())),
                Some(d) => strings::date(d, self.today()),
                None => strings::UPCOMING.to_owned(),
            },
            List::Someday => strings::SOMEDAY.to_owned(),
            List::Inbox => strings::INBOX.to_owned(),
            List::Anytime | List::Logbook | List::Trash => String::new(),
        }
    }

    /// Проект (и раздел) или область, затем теги.
    fn meta(&self, t: &Task) -> String {
        let mut parts = Vec::new();
        match (self.ix.project_of(t), self.ix.heading_of(t)) {
            // На странице проекта разделы – заголовки блоков.
            (Some(p), _) if self.here == Some(Here::Project(p.id)) => {}
            (Some(p), Some(h)) => parts.push(format!("{} / {}", p.title, h.title)),
            (Some(p), None) => parts.push(p.title.clone()),
            (None, _) => {
                if let Some(a) = self.ix.area_of(t)
                    && self.here != Some(Here::Area(a.id))
                {
                    parts.push(a.title.clone());
                }
            }
        }
        for id in &t.tags {
            // Тег в корзине не показывается, как в консоли.
            if let Some(tag) = self
                .ix
                .snapshot()
                .tags
                .iter()
                .find(|x| x.id == *id && !x.sync.is_deleted())
            {
                parts.push(format!("#{}", tag.title));
            }
        }
        parts.join(" · ")
    }

    fn task(&self, t: &Task, when: &str, star: bool) -> Row {
        let real = self.originals.get(&t.id).copied().unwrap_or(t);
        let trashed = self.ix.trashed(t);
        let mark = match real.status {
            Status::Open => Mark::Open,
            Status::Completed { .. } => Mark::Completed,
            Status::Canceled { .. } => Mark::Canceled,
        };
        let open = mark == Mark::Open;
        let deadline = t.deadline.filter(|_| open);
        Row {
            kind: RowKind::Task,
            key: t.id.to_string(),
            row_key: t.id.to_string(),
            title: t.title.clone(),
            when: when.to_owned(),
            meta: self.meta(t),
            deadline: deadline.map_or_else(String::new, |d| strings::deadline(d, self.today())),
            due: deadline.is_some_and(|d| DeadlineState::of(d, self.today()).is_due()),
            mark,
            checkable: row_toggle(real, trashed, self.allow).is_some(),
            opens: false,
            editable: !trashed,
            reopenable: self.ix.can_reopen(real),
            star,
            progress: 0.0,
            drop_lane: None,
            trash_record: (!trashed || real.sync.is_deleted())
                .then_some((RecordId::Task(real.id), real.sync.updated_at)),
            drag_task: (open && !matches!(self.ix.list_of(real), List::Trash | List::Logbook))
                .then_some((real.id, real.sync.updated_at)),
        }
    }

    fn item(&self, item: Item<'_>, when: &str) -> Row {
        match item {
            Item::Task(t) => self.task(t, when, false),
            Item::Project(p) => {
                let progress = project_progress(p.id, &self.real.tasks, &self.real.headings);
                let deadline = p.deadline.filter(|_| p.status.is_open());
                Row {
                    kind: RowKind::Project,
                    key: Selection::Project(p.id).key(),
                    row_key: Selection::Project(p.id).key(),
                    title: p.title.clone(),
                    when: when.to_owned(),
                    meta: strings::progress(progress.done, progress.total),
                    deadline: deadline
                        .map_or_else(String::new, |d| strings::deadline(d, self.today())),
                    due: deadline.is_some_and(|d| DeadlineState::of(d, self.today()).is_due()),
                    mark: match p.status {
                        Status::Open => Mark::Open,
                        Status::Completed { .. } => Mark::Completed,
                        Status::Canceled { .. } => Mark::Canceled,
                    },
                    checkable: false,
                    opens: can_open(self.real, Selection::Project(p.id)),
                    editable: false,
                    reopenable: false,
                    star: false,
                    progress: progress.fraction() as f32,
                    drop_lane: None,
                    drag_task: None,
                    trash_record: Some((RecordId::Project(p.id), p.sync.updated_at)),
                }
            }
            Item::Heading(h) => {
                let project = self.ix.project(h.project).map_or("", |p| p.title.as_str());
                let mut row = Row::section(strings::trashed_heading(&h.title, project));
                row.kind = RowKind::Other;
                row.trash_record = Some((RecordId::Heading(h.id), h.sync.updated_at));
                row
            }
            Item::Area(a) => {
                let mut row = Row::section(strings::trashed_area(&a.title));
                row.kind = RowKind::Other;
                row.trash_record = Some((RecordId::Area(a.id), a.sync.updated_at));
                row
            }
        }
    }
}
