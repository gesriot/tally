//! Таблица `tasks` (вместе с тегами и чек-листом).

use rusqlite::{Connection, OptionalExtension, Row};
use tally_core::{AreaId, HeadingId, ProjectId, Task, TaskId};

use super::project::{RawSchedule, Schedule, schedule_values};
use super::{
    RawSync, TagLinks, Values, checklist, get_all, get_one, read_sync, sync_values, write,
};
use crate::codec::{self, Decoder};
use crate::error::{Kind, Result, StoreError};
use crate::normalize::search_text;

const D: Decoder = Decoder { table: "tasks" };

struct Raw {
    id: String,
    title: String,
    notes: String,
    reminder_at: Option<String>,
    schedule: RawSchedule,
    project_id: Option<String>,
    heading_id: Option<String>,
    area_id: Option<String>,
    sort_index: String,
    today_index: String,
    sync: RawSync,
}

fn raw(row: &Row<'_>) -> rusqlite::Result<Raw> {
    Ok(Raw {
        id: row.get("id")?,
        title: row.get("title")?,
        notes: row.get("notes")?,
        reminder_at: row.get("reminder_at")?,
        schedule: RawSchedule::get(row)?,
        project_id: row.get("project_id")?,
        heading_id: row.get("heading_id")?,
        area_id: row.get("area_id")?,
        sort_index: row.get("sort_index")?,
        today_index: row.get("today_index")?,
        sync: RawSync::get(row)?,
    })
}

/// Текст для поиска: название, заметки и живые пункты чек-листа
/// (повтор id пункта отклоняет [`checklist::save`], поэтому текст совпадает
/// с тем, что ляжет в таблицу).
fn task_search_text(t: &Task) -> String {
    let checklist = t
        .checklist
        .iter()
        .filter(|c| !c.sync.is_deleted())
        .map(|c| c.title.as_str());
    search_text(
        [t.title.as_str(), t.notes.as_str()]
            .into_iter()
            .chain(checklist),
    )
}

fn values(t: &Task) -> Values<'_> {
    let mut v: Values<'_> = vec![
        ("title", Box::new(t.title.as_str())),
        ("notes", Box::new(t.notes.as_str())),
        ("reminder_at", Box::new(codec::opt_ts(t.reminder_at))),
        ("search_text", Box::new(task_search_text(t))),
        ("project_id", Box::new(t.project.map(|p| p.0.to_string()))),
        ("heading_id", Box::new(t.heading.map(|h| h.0.to_string()))),
        ("area_id", Box::new(t.area.map(|a| a.0.to_string()))),
        ("sort_index", Box::new(t.index.as_str())),
        ("today_index", Box::new(t.today_index.as_str())),
    ];
    v.extend(schedule_values(Schedule {
        start: t.start,
        date: t.date,
        evening: t.evening,
        deadline: t.deadline,
        status: t.status,
        repeat_rule: t.repeat_rule,
        repeat_from: t.repeat_from,
        repeat_anchor: t.repeat_anchor,
        repeat_advanced: t.repeat_advanced,
    }));
    v.extend(sync_values(&t.sync));
    v
}

pub(crate) fn save(conn: &Connection, t: &Task, insert: bool) -> Result<()> {
    check_heading(conn, t)?;
    write(conn, Kind::Task, t.id.0, &values(t), insert)?;
    TagLinks::Task.replace(conn, Kind::Task, t.id.0, &t.tags)?;
    checklist::save(conn, t.id.0, &t.checklist, t.sync.updated_at)
}

/// Раздел задачи – раздел её проекта, а своей области у задачи в проекте
/// нет (она наследует область проекта). То же держит и схема (составная
/// ссылка `(heading_id, project_id)` и `CHECK`); здесь – понятная ошибка
/// вместо ошибки ограничения.
fn check_heading(conn: &Connection, t: &Task) -> Result<()> {
    if t.project.is_some() && t.area.is_some() {
        return Err(StoreError::AreaWithProject { task: t.id.0 });
    }
    let Some(heading) = t.heading else {
        return Ok(());
    };
    let owner: Option<String> = conn
        .prepare_cached("SELECT project_id FROM headings WHERE id = ?1")?
        .query_row([heading.0.to_string()], |r| r.get(0))
        .optional()?;
    match owner {
        None => Err(StoreError::MissingReference {
            kind: Kind::Task,
            id: t.id.0,
        }),
        Some(p) if t.project.is_some_and(|own| own.0.to_string() == p) => Ok(()),
        Some(_) => Err(StoreError::HeadingOutsideProject {
            task: t.id.0,
            heading: heading.0,
        }),
    }
}

fn build(
    r: Raw,
    tags: Vec<tally_core::TagId>,
    checklist: Vec<tally_core::ChecklistItem>,
) -> Result<Task> {
    let s = r.schedule.build(D)?;
    Ok(Task {
        id: TaskId::from_uuid(D.parse("id", &r.id)?),
        title: r.title,
        notes: r.notes,
        start: s.start,
        date: s.date,
        evening: s.evening,
        deadline: s.deadline,
        reminder_at: D.parse_opt("reminder_at", r.reminder_at.as_deref())?,
        status: s.status,
        project: D
            .parse_opt("project_id", r.project_id.as_deref())?
            .map(ProjectId::from_uuid),
        heading: D
            .parse_opt("heading_id", r.heading_id.as_deref())?
            .map(HeadingId::from_uuid),
        area: D
            .parse_opt("area_id", r.area_id.as_deref())?
            .map(AreaId::from_uuid),
        tags,
        checklist,
        index: D.parse("sort_index", &r.sort_index)?,
        today_index: D.parse("today_index", &r.today_index)?,
        repeat_rule: s.repeat_rule,
        repeat_from: s.repeat_from,
        repeat_anchor: s.repeat_anchor,
        repeat_advanced: s.repeat_advanced,
        sync: read_sync(D, &r.sync)?,
    })
}

pub(crate) fn get(conn: &Connection, id: TaskId) -> Result<Option<Task>> {
    let Some(r) = get_one(conn, "SELECT * FROM tasks WHERE id = ?1", id.0, raw)? else {
        return Ok(None);
    };
    let tags = TagLinks::Task
        .load(conn, Some(id.0))?
        .remove(&r.id)
        .unwrap_or_default();
    let items = checklist::load(conn, Some(id.0))?
        .remove(&r.id)
        .unwrap_or_default();
    build(r, tags, items).map(Some)
}

pub(crate) fn all(conn: &Connection) -> Result<Vec<Task>> {
    let mut tags = TagLinks::Task.load(conn, None)?;
    let mut items = checklist::load(conn, None)?;
    get_all(conn, "SELECT * FROM tasks ORDER BY sort_index", raw)?
        .into_iter()
        .map(|r| {
            let t = tags.remove(&r.id).unwrap_or_default();
            let c = items.remove(&r.id).unwrap_or_default();
            build(r, t, c)
        })
        .collect()
}
