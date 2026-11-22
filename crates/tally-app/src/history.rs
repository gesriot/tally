//! Журнал действий: отмена и повтор.
//!
//! Каждое действие хранит для каждой записи, которую оно изменило, её
//! состояние до и после (`None` – записи нет). Отмена возвращает записи в
//! состояние «до», повтор – в «после». Это и есть обратная команда: она не
//! вычисляется заново, а берётся из сохранённых состояний, поэтому
//! возвращаются все поля, которые команда тронула (проект, раздел, дата,
//! «вечер», срок, статус, теги).
//!
//! Перед записью каждая запись сверяется с тем, что оставило действие (или
//! его отмена). Если её с тех пор изменили в обход журнала, отмена или повтор
//! отклоняются целиком ([`Error::Conflict`]) и ничего не пишется.

use std::collections::HashSet;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use tally_core::{
    Area, AreaId, ChecklistItemId, Heading, HeadingId, Project, ProjectId, RepeatFrom, SyncMeta,
    Tag, TagId, Task, TaskId,
};
use tally_store::{Reader, RecordId, Snapshot, StoreError, Writer};

use crate::command::{Clock, Step};
use crate::error::{Error, Result};
use crate::exec::later;
use crate::index::Index;

/// Сколько последних действий журнал помнит.
pub const HISTORY_LIMIT: usize = 100;

/// Запись базы целиком.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Record {
    Area(Area),
    Tag(Tag),
    Project(Project),
    Heading(Heading),
    Task(Task),
}

impl Record {
    pub(crate) fn id(&self) -> RecordId {
        match self {
            Self::Area(a) => RecordId::Area(a.id),
            Self::Tag(t) => RecordId::Tag(t.id),
            Self::Project(p) => RecordId::Project(p.id),
            Self::Heading(h) => RecordId::Heading(h.id),
            Self::Task(t) => RecordId::Task(t.id),
        }
    }

    pub(crate) fn sync(&self) -> &SyncMeta {
        match self {
            Self::Area(a) => &a.sync,
            Self::Tag(t) => &t.sync,
            Self::Project(p) => &p.sync,
            Self::Heading(h) => &h.sync,
            Self::Task(t) => &t.sync,
        }
    }

    pub(crate) fn sync_mut(&mut self) -> &mut SyncMeta {
        match self {
            Self::Area(a) => &mut a.sync,
            Self::Tag(t) => &mut t.sync,
            Self::Project(p) => &mut p.sync,
            Self::Heading(h) => &mut h.sync,
            Self::Task(t) => &mut t.sync,
        }
    }

    /// Та же запись, без учёта времени правки – её самой и пунктов её
    /// чек-листа: метки отмена и повтор сами двигают вперёд, а другой клиент
    /// может обновить метку пункта, не меняя его. Поэтому после отмены одного
    /// действия запись совпадает с тем, что оставило предыдущее, по
    /// содержимому, а не по меткам. Текст, отметка и удаление пункта
    /// сравниваются.
    pub(crate) fn same(&self, other: &Self) -> bool {
        let mut other = other.clone();
        other.sync_mut().updated_at = self.sync().updated_at;
        if let (Self::Task(mine), Self::Task(theirs)) = (self, &mut other) {
            // Отмена создания пункта оставляет надгробие. Для следующего
            // шага истории важен состав видимого чек-листа.
            let mut mine = mine.clone();
            mine.checklist.retain(|item| !item.sync.is_deleted());
            theirs.checklist.retain(|item| !item.sync.is_deleted());
            for item in &mut theirs.checklist {
                if let Some(m) = mine.checklist.iter().find(|m| m.id == item.id) {
                    item.sync.updated_at = m.sync.updated_at;
                }
            }
            return mine == *theirs;
        }
        *self == other
    }

    /// Последняя метка записи.
    fn latest(&self) -> Timestamp {
        let s = self.sync();
        s.updated_at.max(s.created_at)
    }
}

/// Изменение одной записи.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Change {
    before: Option<Record>,
    after: Option<Record>,
    /// Последняя метка правки, которую журнал поставил этой записи (в том
    /// числе при удалении отменой). Новая правка – позже неё.
    stamp: Timestamp,
    /// Подтверждённые напоминания физически удалённой задачи. Храним их до
    /// восстановления задачи, пока для её id недоступна таблица доставки.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    confirmed_reminders: Vec<Timestamp>,
    /// Цепочка контейнеров записи в момент действия – до и после него:
    /// только id, связи с родителями и то, от чего зависит доступность
    /// (корзина, архив, закрытие). Контейнер, у которого всё это с тех пор
    /// не изменилось, при отмене или повторе на корзину, архив и закрытие не
    /// проверяется. В строках журнала прежних версий цепочки нет – тогда
    /// проверяется всё.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    chain_before: Vec<Around>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    chain_after: Vec<Around>,
    /// Цепочка в прежнем формате: полные записи контейнеров. Только для
    /// чтения: [`Change::upgrade`] переводит её в `chain_*`, и при первой же
    /// записи строки журнала она сохраняется уже в новом виде.
    #[serde(default, rename = "around_before", skip_serializing)]
    legacy_before: Vec<Record>,
    #[serde(default, rename = "around_after", skip_serializing)]
    legacy_after: Vec<Record>,
}

impl Change {
    /// Привести сохранённые состояния к текущей схеме. Старые полные
    /// снимки контейнеров заменяются флагами доступности.
    fn upgrade(&mut self) {
        for record in self.before.iter_mut().chain(&mut self.after) {
            let (date, from, rule, anchor) = match record {
                Record::Task(t) => (
                    t.date,
                    t.repeat_from,
                    &mut t.repeat_rule,
                    &mut t.repeat_anchor,
                ),
                Record::Project(p) => (
                    p.date,
                    p.repeat_from,
                    &mut p.repeat_rule,
                    &mut p.repeat_anchor,
                ),
                _ => continue,
            };
            // Как в миграции 0004: старое снятие даты не должно
            // восстановить неработающее правило при отмене или повторе.
            if date.is_none() && from == RepeatFrom::Date && rule.take().is_some() {
                *anchor = None;
            }
        }
        if self.chain_before.is_empty() {
            self.chain_before = self.legacy_before.iter().filter_map(Around::of).collect();
        }
        if self.chain_after.is_empty() {
            self.chain_after = self.legacy_after.iter().filter_map(Around::of).collect();
        }
        self.legacy_before.clear();
        self.legacy_after.clear();
    }

    fn id(&self) -> Option<RecordId> {
        self.before.as_ref().or(self.after.as_ref()).map(Record::id)
    }

    /// Состояние, которое сейчас должно быть в базе, и то, в которое запись
    /// перейдёт: при отмене – из «после» в «до», при повторе – наоборот.
    fn sides(&self, back: bool) -> (&Option<Record>, &Option<Record>) {
        if back {
            (&self.after, &self.before)
        } else {
            (&self.before, &self.after)
        }
    }

    fn target_mut(&mut self, back: bool) -> &mut Option<Record> {
        if back {
            &mut self.before
        } else {
            &mut self.after
        }
    }
}

pub(crate) fn find(s: &Snapshot, id: RecordId) -> Option<Record> {
    match id {
        RecordId::Area(id) => s
            .areas
            .iter()
            .find(|x| x.id == id)
            .cloned()
            .map(Record::Area),
        RecordId::Tag(id) => s.tags.iter().find(|x| x.id == id).cloned().map(Record::Tag),
        RecordId::Project(id) => s
            .projects
            .iter()
            .find(|x| x.id == id)
            .cloned()
            .map(Record::Project),
        RecordId::Heading(id) => s
            .headings
            .iter()
            .find(|x| x.id == id)
            .cloned()
            .map(Record::Heading),
        RecordId::Task(id) => s
            .tasks
            .iter()
            .find(|x| x.id == id)
            .cloned()
            .map(Record::Task),
    }
}

fn read(r: Reader<'_>, id: RecordId) -> Result<Option<Record>> {
    Ok(match id {
        RecordId::Area(id) => r.area(id)?.map(Record::Area),
        RecordId::Tag(id) => r.tag(id)?.map(Record::Tag),
        RecordId::Project(id) => r.project(id)?.map(Record::Project),
        RecordId::Heading(id) => r.heading(id)?.map(Record::Heading),
        RecordId::Task(id) => r.task(id)?.map(Record::Task),
    })
}

pub(crate) fn forget_purged(w: &Writer<'_>, before: &Snapshot, after: &Snapshot) -> Result<()> {
    let ids = |s: &Snapshot| {
        s.tasks
            .iter()
            .map(|t| RecordId::Task(t.id))
            .chain(s.projects.iter().map(|p| RecordId::Project(p.id)))
            .chain(s.headings.iter().map(|h| RecordId::Heading(h.id)))
            .chain(s.areas.iter().map(|a| RecordId::Area(a.id)))
            .chain(s.tags.iter().map(|t| RecordId::Tag(t.id)))
            .collect::<HashSet<_>>()
    };
    let live = ids(after);
    let gone: HashSet<_> = ids(before).difference(&live).copied().collect();
    let gone_checks: HashSet<_> = before
        .tasks
        .iter()
        .flat_map(|t| t.checklist.iter())
        .filter(|item| item.sync.is_deleted())
        .map(|item| item.id)
        .collect();
    if gone.is_empty() && gone_checks.is_empty() {
        return Ok(());
    }
    // Если исчезнет часть уже выполненного действия, его более ранние
    // шаги по уцелевшим записям тоже не смогут сверить исходное состояние.
    let mut interrupted = HashSet::new();
    for entry in w.read().history()?.into_iter().rev() {
        let mut changes: Vec<Change> = serde_json::from_str(&entry.changes)?;
        let purged = changes.iter().any(|c| {
            c.id()
                .is_some_and(|id| gone.contains(&id) || interrupted.contains(&id))
                || c.before
                    .iter()
                    .chain(&c.after)
                    .any(|r| references_purged(r, &gone))
        });
        let legacy = changes.iter().any(|c| {
            c.legacy_before
                .iter()
                .chain(&c.legacy_after)
                .any(|r| gone.contains(&r.id()))
        });
        if entry.undone || purged || legacy {
            if !entry.undone {
                interrupted.extend(changes.iter().filter_map(Change::id));
            }
            w.forget_history(entry.seq)?;
            continue;
        }

        // Снимок карточки хранит весь чек-лист, даже если менялись только
        // заметки. Убираем очищенные пункты с обеих сторон действия.
        let mut rewritten = false;
        changes.retain_mut(|change| {
            let scrubbed = scrub_purged_items(&mut change.before, &gone_checks)
                | scrub_purged_items(&mut change.after, &gone_checks);
            rewritten |= scrubbed;
            !scrubbed || !matches!((&change.before, &change.after), (Some(a), Some(b)) if a.same(b))
        });
        if changes.is_empty() {
            w.forget_history(entry.seq)?;
        } else if rewritten {
            changes.iter_mut().for_each(Change::upgrade);
            w.mark_history(entry.seq, entry.undone, &serde_json::to_string(&changes)?)?;
        }
    }
    Ok(())
}

fn scrub_purged_items(record: &mut Option<Record>, gone: &HashSet<ChecklistItemId>) -> bool {
    let Some(Record::Task(task)) = record else {
        return false;
    };
    let count = task.checklist.len();
    task.checklist.retain(|item| !gone.contains(&item.id));
    task.checklist.len() != count
}

fn references_purged(record: &Record, gone: &HashSet<RecordId>) -> bool {
    let has_tag = |id| gone.contains(&RecordId::Tag(id));
    let has_area = |id| gone.contains(&RecordId::Area(id));
    let has_project = |id| gone.contains(&RecordId::Project(id));
    match record {
        Record::Task(t) => {
            t.area.is_some_and(has_area)
                || t.project.is_some_and(has_project)
                || t.heading
                    .is_some_and(|id| gone.contains(&RecordId::Heading(id)))
                || t.tags.iter().copied().any(has_tag)
        }
        Record::Project(p) => p.area.is_some_and(has_area) || p.tags.iter().copied().any(has_tag),
        Record::Heading(h) => has_project(h.project),
        Record::Area(a) => a.tags.iter().copied().any(has_tag),
        Record::Tag(t) => t.parent.is_some_and(has_tag),
    }
}

pub(crate) fn save(w: &Writer<'_>, record: &Record, insert: bool) -> Result<()> {
    match (record, insert) {
        (Record::Area(x), true) => w.insert_area(x)?,
        (Record::Area(x), false) => w.update_area(x)?,
        (Record::Tag(x), true) => w.insert_tag(x)?,
        (Record::Tag(x), false) => w.update_tag(x)?,
        (Record::Project(x), true) => w.insert_project(x)?,
        (Record::Project(x), false) => w.update_project(x)?,
        (Record::Heading(x), true) => w.insert_heading(x)?,
        (Record::Heading(x), false) => w.update_heading(x)?,
        (Record::Task(x), true) => w.insert_task(x)?,
        (Record::Task(x), false) => w.update_task(x)?,
    }
    Ok(())
}

/// Записать действие `action` в журнал: состояния записей `touched` до
/// (из снимка `before`) и после (прочитаны в той же транзакции).
///
/// Запись, которая изменилась только во времени правки, – не изменение.
/// Возвращает номер новой строки журнала; нет ни одного изменения – `None`:
/// строки журнала нет.
pub(crate) fn record(
    w: &Writer<'_>,
    before: &Snapshot,
    now: Timestamp,
    action: &str,
    touched: &[RecordId],
) -> Result<Option<i64>> {
    let mut seen = HashSet::new();
    let mut changes = Vec::new();
    let after = w.read().snapshot()?;
    for &id in touched {
        if !seen.insert(id) {
            continue;
        }
        let was = find(before, id);
        let is = read(w.read(), id)?;
        let same = match (&was, &is) {
            (Some(a), Some(b)) => a.same(b),
            (None, None) => true,
            _ => false,
        };
        if same {
            continue;
        }
        let stamp = was
            .iter()
            .chain(is.iter())
            .map(Record::latest)
            .max()
            .unwrap_or(now);
        changes.push(Change {
            chain_before: was.iter().flat_map(|r| chain(before, r)).collect(),
            chain_after: is.iter().flat_map(|r| chain(&after, r)).collect(),
            legacy_before: Vec::new(),
            legacy_after: Vec::new(),
            before: was,
            after: is,
            stamp,
            confirmed_reminders: Vec::new(),
        });
    }
    if changes.is_empty() {
        return Ok(None);
    }
    let json = serde_json::to_string(&changes)?;
    Ok(Some(w.push_history(now, action, &json, HISTORY_LIMIT)?))
}

/// Как назвать запись в сообщении: именительный и винительный падежи.
fn names(ix: &Index<'_>, r: &Record) -> (String, String) {
    match r {
        Record::Task(t) => {
            let label = ix.label(t);
            (format!("задача {label}"), format!("задачу {label}"))
        }
        Record::Project(p) => {
            let n = format!("проект «{}»", p.title);
            (n.clone(), n)
        }
        Record::Area(a) => {
            let n = format!("область «{}»", a.title);
            (n.clone(), n)
        }
        Record::Heading(h) => {
            let n = format!("раздел «{}»", h.title);
            (n.clone(), n)
        }
        Record::Tag(t) => {
            let n = format!("тег «{}»", t.title);
            (n.clone(), n)
        }
    }
}

/// Кто в снимке ссылается на запись `id`, кроме записей `skip`.
fn referrer(s: &Snapshot, id: RecordId, skip: &HashSet<RecordId>) -> Option<Record> {
    let free = |r: RecordId| !skip.contains(&r);
    let task = |f: &dyn Fn(&Task) -> bool| {
        s.tasks
            .iter()
            .find(|t| free(RecordId::Task(t.id)) && f(t))
            .cloned()
            .map(Record::Task)
    };
    match id {
        RecordId::Task(_) => None,
        RecordId::Heading(h) => task(&|t| t.heading == Some(h)),
        RecordId::Project(p) => task(&|t| t.project == Some(p)).or_else(|| {
            s.headings
                .iter()
                .find(|h| free(RecordId::Heading(h.id)) && h.project == p)
                .cloned()
                .map(Record::Heading)
        }),
        RecordId::Area(a) => task(&|t| t.area == Some(a)).or_else(|| {
            s.projects
                .iter()
                .find(|p| free(RecordId::Project(p.id)) && p.area == Some(a))
                .cloned()
                .map(Record::Project)
        }),
        RecordId::Tag(g) => task(&|t| t.tags.contains(&g))
            .or_else(|| {
                s.projects
                    .iter()
                    .find(|p| free(RecordId::Project(p.id)) && p.tags.contains(&g))
                    .cloned()
                    .map(Record::Project)
            })
            .or_else(|| {
                s.areas
                    .iter()
                    .find(|a| free(RecordId::Area(a.id)) && a.tags.contains(&g))
                    .cloned()
                    .map(Record::Area)
            })
            .or_else(|| {
                s.tags
                    .iter()
                    .find(|t| free(RecordId::Tag(t.id)) && t.parent == Some(g))
                    .cloned()
                    .map(Record::Tag)
            }),
    }
}

/// База, какой она станет после шага: снимок, в котором записи шага уже
/// в целевом состоянии.
fn planned(s: &Snapshot, changes: &[Change], back: bool) -> Snapshot {
    overlay(
        s,
        changes
            .iter()
            .filter_map(|c| Some((c.id()?, c.sides(back).1.clone()))),
    )
}

/// Снимок `s`, в котором записи `targets` заменены (`None` – убраны).
fn overlay(s: &Snapshot, targets: impl Iterator<Item = (RecordId, Option<Record>)>) -> Snapshot {
    let mut p = s.clone();
    for (id, target) in targets {
        match id {
            RecordId::Area(id) => replace(
                &mut p.areas,
                |x| x.id == id,
                target,
                |r| match r {
                    Record::Area(x) => Some(x),
                    _ => None,
                },
            ),
            RecordId::Tag(id) => replace(
                &mut p.tags,
                |x| x.id == id,
                target,
                |r| match r {
                    Record::Tag(x) => Some(x),
                    _ => None,
                },
            ),
            RecordId::Project(id) => replace(
                &mut p.projects,
                |x| x.id == id,
                target,
                |r| match r {
                    Record::Project(x) => Some(x),
                    _ => None,
                },
            ),
            RecordId::Heading(id) => replace(
                &mut p.headings,
                |x| x.id == id,
                target,
                |r| match r {
                    Record::Heading(x) => Some(x),
                    _ => None,
                },
            ),
            RecordId::Task(id) => replace(
                &mut p.tasks,
                |x| x.id == id,
                target,
                |r| match r {
                    Record::Task(x) => Some(x),
                    _ => None,
                },
            ),
        }
    }
    p
}

fn replace<T>(
    items: &mut Vec<T>,
    is: impl Fn(&T) -> bool,
    target: Option<Record>,
    unwrap: impl Fn(Record) -> Option<T>,
) {
    items.retain(|x| !is(x));
    if let Some(x) = target.and_then(unwrap) {
        items.push(x);
    }
}

/// Номер записи, которой нет в базе: «[3f9a2c1b]».
fn missing(id: uuid::Uuid) -> String {
    format!("[{}]", crate::index::short(id, 8))
}

/// Не изменилась ли запись-контейнер с момента действия: её состояние на
/// корзину, архив и закрытие тогда не проверяется – действие уже застало её
/// такой (задачу переносили из закрытого проекта – отмена вернёт её туда).
/// У команды – всегда `false`: новая ссылка ведёт только на живое.
pub(crate) type Kept<'a> = &'a dyn Fn(RecordId) -> bool;

fn project_state<'s>(
    s: &'s Snapshot,
    id: ProjectId,
    kept: Kept<'_>,
) -> std::result::Result<&'s Project, String> {
    let Some(p) = s.projects.iter().find(|p| p.id == id) else {
        return Err(format!("проекта {} нет в базе", missing(id.0)));
    };
    let own = !kept(RecordId::Project(id));
    if own && p.sync.is_deleted() {
        return Err(format!("проект «{}» в корзине", p.title));
    }
    if let Some(a) = p.area.and_then(|a| s.areas.iter().find(|x| x.id == a))
        && a.sync.is_deleted()
        && !kept(RecordId::Area(a.id))
    {
        return Err(format!(
            "область «{}» проекта «{}» в корзине",
            a.title, p.title
        ));
    }
    if own && !p.status.is_open() {
        return Err(format!("проект «{}» закрыт", p.title));
    }
    Ok(p)
}

/// Раздел, на который ставится новая ссылка, – со всей цепочкой: сам он не
/// в корзине и не в архиве, его проект есть, не в корзине (сам или через
/// область) и не закрыт. Одна проверка и для команд, и для отмены и повтора.
fn heading_state(s: &Snapshot, h: &Heading, kept: Kept<'_>) -> std::result::Result<(), String> {
    if !kept(RecordId::Heading(h.id)) {
        if h.sync.is_deleted() {
            return Err(format!("раздел «{}» в корзине", h.title));
        }
        if h.archived {
            return Err(format!("раздел «{}» в архиве", h.title));
        }
    }
    project_state(s, h.project, kept)
        .map(|_| ())
        .map_err(|why| format!("раздел «{}»: {why}", h.title))
}

/// Можно ли поставить новую ссылку на раздел `h` – та же проверка, что у
/// команд и у отмены и повтора.
pub(crate) fn heading_available(s: &Snapshot, h: &Heading) -> bool {
    heading_state(s, h, &|_| false).is_ok()
}

fn area_state(s: &Snapshot, id: AreaId, kept: Kept<'_>) -> std::result::Result<(), String> {
    match s.areas.iter().find(|a| a.id == id) {
        None => Err(format!("области {} нет в базе", missing(id.0))),
        Some(a) if a.sync.is_deleted() && !kept(RecordId::Area(id)) => {
            Err(format!("область «{}» в корзине", a.title))
        }
        Some(_) => Ok(()),
    }
}

fn tags_state(
    s: &Snapshot,
    tags: &[TagId],
    old: Option<&[TagId]>,
    kept: Kept<'_>,
) -> std::result::Result<(), String> {
    for id in tags {
        if old.is_some_and(|o| o.contains(id)) {
            continue;
        }
        match s.tags.iter().find(|t| t.id == *id) {
            None => return Err(format!("тега {} нет в базе", missing(id.0))),
            Some(t) if t.sync.is_deleted() && !kept(RecordId::Tag(*id)) => {
                return Err(format!("тег «{}» в корзине", t.title));
            }
            Some(_) => {}
        }
    }
    Ok(())
}

/// Контейнеры, на которые запись `r` ссылается в базе `s`, со всей
/// цепочкой: проект, раздел и его проект, области проектов, своя область,
/// теги, родитель тега.
fn chain(s: &Snapshot, r: &Record) -> Vec<Around> {
    let mut ids: Vec<RecordId> = Vec::new();
    let project = |ids: &mut Vec<RecordId>, id: ProjectId| {
        ids.push(RecordId::Project(id));
        if let Some(a) = s.projects.iter().find(|p| p.id == id).and_then(|p| p.area) {
            ids.push(RecordId::Area(a));
        }
    };
    match r {
        Record::Task(t) => {
            if let Some(p) = t.project {
                project(&mut ids, p);
            }
            if let Some(h) = t.heading {
                ids.push(RecordId::Heading(h));
                if let Some(h) = s.headings.iter().find(|x| x.id == h) {
                    project(&mut ids, h.project);
                }
            }
            ids.extend(t.area.map(RecordId::Area));
            ids.extend(t.tags.iter().copied().map(RecordId::Tag));
        }
        Record::Project(p) => {
            ids.extend(p.area.map(RecordId::Area));
            ids.extend(p.tags.iter().copied().map(RecordId::Tag));
        }
        Record::Heading(h) => project(&mut ids, h.project),
        Record::Area(a) => ids.extend(a.tags.iter().copied().map(RecordId::Tag)),
        Record::Tag(t) => ids.extend(t.parent.map(RecordId::Tag)),
    }
    let mut seen = HashSet::new();
    ids.into_iter()
        .filter(|id| seen.insert(*id))
        .filter_map(|id| find(s, id))
        .filter_map(|r| Around::of(&r))
        .collect()
}

/// Контейнер в цепочке журнала: id, связь с родителем и доступность.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Around {
    Area {
        id: AreaId,
        deleted: bool,
    },
    Tag {
        id: TagId,
        parent: Option<TagId>,
        deleted: bool,
    },
    Project {
        id: ProjectId,
        area: Option<AreaId>,
        deleted: bool,
        open: bool,
    },
    Heading {
        id: HeadingId,
        project: ProjectId,
        deleted: bool,
        archived: bool,
    },
}

impl Around {
    fn of(r: &Record) -> Option<Self> {
        Some(match r {
            Record::Area(a) => Self::Area {
                id: a.id,
                deleted: a.sync.is_deleted(),
            },
            Record::Tag(t) => Self::Tag {
                id: t.id,
                parent: t.parent,
                deleted: t.sync.is_deleted(),
            },
            Record::Project(p) => Self::Project {
                id: p.id,
                area: p.area,
                deleted: p.sync.is_deleted(),
                open: p.status.is_open(),
            },
            Record::Heading(h) => Self::Heading {
                id: h.id,
                project: h.project,
                deleted: h.sync.is_deleted(),
                archived: h.archived,
            },
            Record::Task(_) => return None,
        })
    }

    fn id(&self) -> RecordId {
        match self {
            Self::Area { id, .. } => RecordId::Area(*id),
            Self::Tag { id, .. } => RecordId::Tag(*id),
            Self::Project { id, .. } => RecordId::Project(*id),
            Self::Heading { id, .. } => RecordId::Heading(*id),
        }
    }
}

/// Проверка команды до записи: каждая новая или изменённая ссылка записей
/// `records` (их будущие состояния) ведёт на живое – по тем же правилам,
/// что у отмены и повтора. Иначе `Rejected`: такую команду потом нельзя
/// было бы отменить.
pub(crate) fn check_planned(before: &Snapshot, records: &[Record]) -> Result<()> {
    let after = overlay(before, records.iter().map(|r| (r.id(), Some(r.clone()))));
    for r in records {
        let was = find(before, r.id());
        links(&after, r, was.as_ref(), &|_| false).map_err(Error::Rejected)?;
    }
    Ok(())
}

/// Связи записи `r` в базе после шага `s`. Связь, которая не меняется
/// (`old` – запись сейчас), не проверяется на корзину и закрытие: задача,
/// чей проект закрыли, отменяется и повторяется в нём же. Раздел задачи
/// всегда должен быть разделом её проекта.
fn links(
    s: &Snapshot,
    r: &Record,
    old: Option<&Record>,
    kept: Kept<'_>,
) -> std::result::Result<(), String> {
    match r {
        Record::Task(t) => {
            let old = match old {
                Some(Record::Task(o)) => Some(o),
                _ => None,
            };
            if let Some(p) = t.project
                && old.is_none_or(|o| o.project != t.project)
            {
                project_state(s, p, kept)?;
            }
            if let Some(hid) = t.heading {
                let Some(h) = s.headings.iter().find(|h| h.id == hid) else {
                    return Err(format!("раздела {} нет в базе", missing(hid.0)));
                };
                if Some(h.project) != t.project {
                    let owner = s
                        .projects
                        .iter()
                        .find(|p| p.id == h.project)
                        .map_or_else(|| missing(h.project.0), |p| format!("«{}»", p.title));
                    return Err(format!("раздел «{}» теперь в проекте {owner}", h.title));
                }
                if old.is_none_or(|o| o.heading != t.heading) {
                    heading_state(s, h, kept)?;
                }
            }
            if let Some(a) = t.area
                && old.is_none_or(|o| o.area != t.area)
            {
                area_state(s, a, kept)?;
            }
            tags_state(s, &t.tags, old.map(|o| o.tags.as_slice()), kept)
        }
        Record::Project(p) => {
            let old = match old {
                Some(Record::Project(o)) => Some(o),
                _ => None,
            };
            if let Some(a) = p.area
                && old.is_none_or(|o| o.area != p.area)
            {
                area_state(s, a, kept)?;
            }
            tags_state(s, &p.tags, old.map(|o| o.tags.as_slice()), kept)
        }
        Record::Heading(h) => {
            let changed = match old {
                Some(Record::Heading(o)) => o.project != h.project,
                _ => true,
            };
            if changed {
                project_state(s, h.project, kept)?;
            }
            Ok(())
        }
        Record::Area(a) => {
            let old = match old {
                Some(Record::Area(o)) => Some(o.tags.as_slice()),
                _ => None,
            };
            tags_state(s, &a.tags, old, kept)
        }
        Record::Tag(t) => {
            let changed = match old {
                Some(Record::Tag(o)) => o.parent != t.parent,
                _ => true,
            };
            match t.parent {
                Some(parent) if changed => tags_state(s, &[parent], None, kept),
                _ => Ok(()),
            }
        }
    }
}

/// Восстановить только пункты, затронутые шагом. Пропавший или удалённый
/// пункт одинаково отсутствует в видимом списке. Поэтому, например, отмена
/// заметки не восстанавливает надгробие, очищенное после редактирования.
fn preserve_untouched_items(target: &mut Record, expected: &Record, current: &Record) {
    let (Record::Task(target), Record::Task(expected), Record::Task(current)) =
        (target, expected, current)
    else {
        return;
    };
    let visible = |item: &&tally_core::ChecklistItem| !item.sync.is_deleted();
    let before: std::collections::HashMap<_, _> = expected
        .checklist
        .iter()
        .filter(visible)
        .map(|item| (item.id, item))
        .collect();
    let after: std::collections::HashMap<_, _> = target
        .checklist
        .iter()
        .filter(visible)
        .map(|item| (item.id, item))
        .collect();
    let changed: HashSet<_> = before
        .keys()
        .chain(after.keys())
        .copied()
        .filter(|id| match (before.get(id), after.get(id)) {
            (Some(a), Some(b)) => {
                let mut probe = (**a).clone();
                probe.sync.updated_at = b.sync.updated_at;
                probe != **b
            }
            _ => true,
        })
        .collect();
    target.checklist.retain(|item| changed.contains(&item.id));
    target.checklist.extend(
        current
            .checklist
            .iter()
            .filter(|item| !changed.contains(&item.id))
            .cloned(),
    );
    target.checklist.sort_by(|a, b| a.index.cmp(&b.index));
}

/// Метки пунктов чек-листа тоже идут только вперёд: пункт, который другой
/// клиент «тронул» позже, сохраняет свою метку; пункт, который шаг меняет,
/// получает метку позже обеих. Если пункт уже очистили из корзины, его
/// последняя метка берётся из той стороны шага, которая была в базе.
fn keep_item_stamps(
    target: &mut Record,
    current: &Record,
    expected: &Record,
    now: Timestamp,
) -> Result<()> {
    let (Record::Task(t), Record::Task(c), Record::Task(was)) = (target, current, expected) else {
        return Ok(());
    };
    for item in &mut t.checklist {
        let Some(cur) = c.checklist.iter().find(|x| x.id == item.id) else {
            let latest = item.sync.updated_at.max(item.sync.created_at);
            let latest = was
                .checklist
                .iter()
                .find(|x| x.id == item.id)
                .map_or(latest, |old| {
                    latest.max(old.sync.updated_at).max(old.sync.created_at)
                });
            item.sync.updated_at = later(now, latest)?;
            continue;
        };
        let latest = item.sync.updated_at.max(cur.sync.updated_at);
        let mut probe = item.clone();
        probe.sync.updated_at = cur.sync.updated_at;
        item.sync.updated_at = if probe == *cur {
            latest
        } else {
            later(now, latest)?
        };
    }
    Ok(())
}

/// Отменить последнее действие (`back`) или повторить последнее отменённое.
/// `expected` – отменить, только если последнее действие – это (с таким
/// номером журнала), иначе [`Error::NotLast`]: сверка в той же транзакции,
/// что и отмена. Возвращает шаг и номер его строки журнала.
pub(crate) fn step(
    w: &Writer<'_>,
    clock: Clock,
    back: bool,
    expected: Option<i64>,
) -> Result<(Step, i64)> {
    let entry = if back {
        let last = w.read().last_done()?;
        match (expected, last) {
            (Some(seq), Some(e)) if e.seq == seq => e,
            (Some(_), _) => return Err(Error::NotLast),
            (None, last) => last.ok_or(Error::NothingToUndo)?,
        }
    } else {
        w.read().first_undone()?.ok_or(Error::NothingToRedo)?
    };
    let mut changes: Vec<Change> = serde_json::from_str(&entry.changes)?;
    changes.iter_mut().for_each(Change::upgrade);
    let s = w.read().snapshot()?;
    let ix = Index::new(&s, clock.today);
    let verb = if back {
        "отменить"
    } else {
        "повторить"
    };
    let conflict = |why: String| Error::Conflict(format!("нельзя {verb} {}: {why}", entry.action));

    // Сверка: каждая запись – ровно такая, какой её оставило действие (или
    // его отмена).
    for c in &changes {
        let Some(id) = c.id() else { continue };
        let (expected, _) = c.sides(back);
        let current = find(&s, id);
        let why = match (&current, expected) {
            (Some(now), Some(was)) if !now.same(was) => {
                format!("{} изменили после этого действия", names(&ix, now).1)
            }
            (None, Some(was)) => {
                format!("{} удалили после этого действия", names(&ix, was).1)
            }
            (Some(now), None) => format!("{} уже есть в базе", names(&ix, now).0),
            _ => continue,
        };
        return Err(conflict(why));
    }
    let planned = planned(&s, &changes, back);
    // После шага никто не должен ссылаться на физически удалённую запись.
    // Сам шаг может сначала снять ссылку, например при отмене нового тега.
    let gone: HashSet<RecordId> = changes
        .iter()
        .filter(|c| c.sides(back).1.is_none())
        .filter_map(Change::id)
        .collect();
    for c in &changes {
        let (Some(id), Some(current)) = (c.id(), c.sides(back).0) else {
            continue;
        };
        if !gone.contains(&id) {
            continue;
        }
        if let Some(by) = referrer(&planned, id, &gone) {
            return Err(conflict(format!(
                "на {} ссылается {}",
                names(&ix, current).1,
                names(&ix, &by).0
            )));
        }
    }

    // Связи: каждая запись после шага ссылается только на то, что есть в
    // базе, не в корзине и (проект) не закрыто, а раздел задачи – раздел её
    // проекта. Проверяется состояние после шага целиком: тег, который этот же
    // повтор вернёт, задаче уже доступен.
    for c in &changes {
        let (Some(id), Some(target)) = (c.id(), c.sides(back).1) else {
            continue;
        };
        let current = find(&s, id);
        let around = if back {
            &c.chain_before
        } else {
            &c.chain_after
        };
        let kept = |id: RecordId| {
            let then = around.iter().find(|a| a.id() == id);
            let now = find(&planned, id).as_ref().and_then(Around::of);
            then.is_some() && then == now.as_ref()
        };
        links(&planned, target, current.as_ref(), &kept).map_err(conflict)?;
    }

    // Запись: отмена – с конца (задача раньше тегов, на которые ссылается),
    // повтор – по порядку действия.
    let order: Vec<usize> = if back {
        (0..changes.len()).rev().collect()
    } else {
        (0..changes.len()).collect()
    };
    // Метки – до первой записи: на пределе меток шаг отказывает, ничего не
    // начав писать.
    let mut stamps = Vec::with_capacity(changes.len());
    for c in &changes {
        let Some(id) = c.id() else {
            stamps.push(clock.now);
            continue;
        };
        let current = find(&s, id);
        let latest = current
            .iter()
            .map(Record::latest)
            .fold(c.stamp, Timestamp::max);
        stamps.push(later(clock.now, latest)?);
    }
    for i in order {
        let at = stamps[i];
        let c = &mut changes[i];
        let Some(id) = c.id() else { continue };
        let current = find(&s, id);
        let (expected, target) = c.sides(back);
        let written = match (&current, target.clone()) {
            (Some(now), Some(mut t)) => {
                t.sync_mut().updated_at = at;
                if let Some(expected) = expected {
                    preserve_untouched_items(&mut t, expected, now);
                    keep_item_stamps(&mut t, now, expected, clock.now)?;
                }
                save(w, &t, false)
            }
            (None, Some(mut t)) => {
                t.sync_mut().updated_at = at;
                save(w, &t, true).and_then(|()| {
                    if let RecordId::Task(task) = id
                        && !c.confirmed_reminders.is_empty()
                    {
                        w.restore_confirmed_reminders(task, &c.confirmed_reminders)?;
                        c.confirmed_reminders.clear();
                    }
                    Ok(())
                })
            }
            (Some(_), None) => {
                if let RecordId::Task(task) = id {
                    c.confirmed_reminders = w.confirmed_reminders(task)?;
                }
                w.delete(id, at)
                    .and_then(|()| w.purge(id))
                    .map_err(Error::from)
            }
            (None, None) => Ok(()),
        };
        // Проверки выше ловят всё, что знает хранилище; если оно всё же
        // отказало по связям – это тоже конфликт, а не ошибка базы.
        written.map_err(|e| match e {
            Error::Store(
                StoreError::MissingReference { .. }
                | StoreError::HeadingOutsideProject { .. }
                | StoreError::AreaWithProject { .. },
            ) => conflict(e.to_string()),
            e => e,
        })?;
        c.stamp = at;
        *c.target_mut(back) = read(w.read(), id)?;
    }
    let json = serde_json::to_string(&changes)?;
    w.mark_history(entry.seq, back, &json)?;
    let mut tasks: Vec<TaskId> = Vec::new();
    for c in &changes {
        if let Some(RecordId::Task(id)) = c.id()
            && !tasks.contains(&id)
        {
            tasks.push(id);
        }
    }
    Ok((
        Step {
            action: entry.action,
            tasks,
        },
        entry.seq,
    ))
}

#[cfg(test)]
mod tests {
    use jiff::SignedDuration;
    use tally_core::{Rank, TaskId};

    use super::*;
    use crate::exec::edited;

    fn at(s: &str) -> Timestamp {
        s.parse().unwrap_or(Timestamp::UNIX_EPOCH)
    }

    fn task() -> Task {
        Task::new(
            TaskId::from_u128(1),
            "Отчёт",
            Rank::initial(),
            at("2026-11-19T12:00:00Z"),
        )
    }

    #[test]
    fn sameness_ignores_only_the_edit_time() {
        let a = Record::Task(task());
        let mut b = task();
        b.sync.touch(at("2026-11-20T12:00:00Z"));
        assert!(a.same(&Record::Task(b.clone())));
        b.title = "Отчёт за ноябрь".into();
        assert!(!a.same(&Record::Task(b.clone())));
        let mut c = task();
        c.sync.delete(at("2026-11-20T12:00:00Z"));
        assert!(!a.same(&Record::Task(c)));
        let mut d = task();
        d.sync.created_at = at("2026-11-18T12:00:00Z");
        assert!(!a.same(&Record::Task(d)));
        // Пункты чек-листа: метка не в счёт, текст, отметка и удаление – в счёт.
        let item = tally_core::ChecklistItem::new(
            tally_core::ChecklistItemId::from_u128(2),
            "Паспорт",
            Rank::initial(),
            at("2026-11-19T12:00:00Z"),
        );
        let mut with = task();
        with.checklist.push(item);
        let a = Record::Task(with.clone());
        let mut e = with.clone();
        e.checklist[0].sync.touch(at("2036-11-19T12:00:00Z"));
        assert!(a.same(&Record::Task(e.clone())));
        e.checklist[0].title = "Билеты".into();
        assert!(!a.same(&Record::Task(e)));
        let mut f = with.clone();
        f.checklist[0].done = true;
        assert!(!a.same(&Record::Task(f)));
        let mut g = with;
        g.checklist[0].sync.delete(at("2036-11-19T12:00:00Z"));
        assert!(!a.same(&Record::Task(g)));
    }

    #[test]
    fn stamps_never_go_back() -> Result<()> {
        let ns = SignedDuration::from_nanos;
        let past = at("2026-11-18T12:00:00Z");
        let now = at("2026-11-19T12:00:00Z");
        let future = at("2036-11-19T12:00:00Z");
        assert_eq!(later(now, past)?, now);
        assert_eq!(later(now, now)?, now + ns(1));
        assert_eq!(later(now, future)?, future + ns(1));
        let mut t = task();
        assert_eq!(edited(now, &t.sync)?, now + ns(1));
        t.sync.touch(future);
        assert_eq!(edited(now, &t.sync)?, future + ns(1));
        // Расти некуда – ошибка, а не прежняя метка.
        assert!(matches!(
            later(now, Timestamp::MAX),
            Err(Error::NoLaterTime(t)) if t == Timestamp::MAX
        ));
        assert_eq!(later(Timestamp::MAX, now)?, Timestamp::MAX);
        Ok(())
    }

    #[test]
    fn item_stamps_only_move_forward() -> Result<()> {
        use tally_core::{ChecklistItem, ChecklistItemId};
        let now = at("2026-11-19T12:00:00Z");
        let fresh = at("2036-11-19T12:00:00Z");
        let item = ChecklistItem::new(
            ChecklistItemId::from_u128(2),
            "Паспорт",
            Rank::initial(),
            now,
        );
        let mut target = task();
        target.checklist.push(item.clone());
        let mut current = target.clone();
        current.checklist[0].sync.touch(fresh);
        // Тронут только метка – она сохраняется.
        let mut r = Record::Task(target.clone());
        let current_record = Record::Task(current.clone());
        keep_item_stamps(&mut r, &current_record, &current_record, now)?;
        let Record::Task(t) = &r else { return Ok(()) };
        assert_eq!(t.checklist[0].sync.updated_at, fresh);
        // Шаг меняет пункт – метка позже обеих.
        current.checklist[0].done = true;
        let mut r = Record::Task(target.clone());
        let current_record = Record::Task(current.clone());
        keep_item_stamps(&mut r, &current_record, &current_record, now)?;
        let Record::Task(t) = &r else { return Ok(()) };
        assert_eq!(
            t.checklist[0].sync.updated_at,
            fresh + SignedDuration::from_nanos(1)
        );
        // Пункт удалили с более поздней меткой, а затем очистили из корзины.
        current.checklist[0].sync.delete(fresh);
        let mut r = Record::Task(target);
        keep_item_stamps(&mut r, &Record::Task(task()), &Record::Task(current), now)?;
        let Record::Task(t) = &r else { return Ok(()) };
        assert_eq!(t.checklist[0].id, item.id);
        assert_eq!(t.checklist[0].sync.created_at, now);
        assert_eq!(
            t.checklist[0].sync.updated_at,
            fresh + SignedDuration::from_nanos(1)
        );
        Ok(())
    }

    #[test]
    fn changes_survive_json_exactly() -> Result<()> {
        let mut t = task();
        t.sync.touch(at("2026-11-19T12:00:00.000000001Z"));
        let mut changes = vec![Change {
            before: None,
            after: Some(Record::Task(t.clone())),
            stamp: t.sync.updated_at,
            confirmed_reminders: Vec::new(),
            chain_before: Vec::new(),
            chain_after: Vec::new(),
            legacy_before: Vec::new(),
            legacy_after: Vec::new(),
        }];
        let json = serde_json::to_string(&changes)?;
        // Строка журнала без цепочки – такая же, как у прежних версий.
        assert!(!json.contains("chain"), "{json}");
        let back: Vec<Change> = serde_json::from_str(&json)?;
        assert_eq!(back, changes);
        changes[0].chain_after = vec![Around::Heading {
            id: HeadingId::from_u128(1),
            project: ProjectId::from_u128(2),
            deleted: false,
            archived: true,
        }];
        let json = serde_json::to_string(&changes)?;
        // Ни заметок, ни названий контейнеров – только id и доступность.
        assert!(json.contains(r#"{"kind":"heading","id":"#), "{json}");
        // Строка прежнего формата: полные записи контейнеров в `around_*` при чтении
        // становятся флагами доступности (названия и заметки отпадают), а
        // записывается строка уже в новом виде.
        let mut p = Project::new(
            ProjectId::from_u128(2),
            "Ремонт",
            Rank::initial(),
            t.sync.created_at,
        );
        p.notes = "Заметка".into();
        p.status = tally_core::Status::Completed {
            at: t.sync.created_at,
        };
        let legacy = serde_json::json!([{
            "before": changes[0].before,
            "after": changes[0].after,
            "stamp": changes[0].stamp,
            "around_before": [Record::Project(p.clone())],
            "around_after": [Record::Project(p)],
        }])
        .to_string();
        let mut back: Vec<Change> = serde_json::from_str(&legacy)?;
        back.iter_mut().for_each(Change::upgrade);
        let flags = vec![Around::Project {
            id: ProjectId::from_u128(2),
            area: None,
            deleted: false,
            open: false,
        }];
        assert_eq!(back[0].chain_before, flags);
        assert_eq!(back[0].chain_after, flags);
        let again = serde_json::to_string(&back)?;
        assert!(
            !again.contains("around") && !again.contains("Заметка"),
            "{again}"
        );
        // Строка без цепочки вовсе – цепочки нет, проверяется всё.
        let mut bare = changes.clone();
        bare[0].chain_after.clear();
        let mut none: Vec<Change> = serde_json::from_str(&serde_json::to_string(&bare)?)?;
        none.iter_mut().for_each(Change::upgrade);
        assert!(none[0].chain_before.is_empty() && none[0].chain_after.is_empty());
        let json = serde_json::to_string(&changes)?;
        let back: Vec<Change> = serde_json::from_str(&json)?;
        assert_eq!(back, changes);
        assert_eq!(back[0].id(), Some(RecordId::Task(t.id)));
        assert_eq!(back[0].sides(true), (&changes[0].after, &None));
        Ok(())
    }
}
