//! Таблица `projects` и общие с задачами колонки расписания.

use jiff::civil::Date;
use rusqlite::{Connection, Row};
use tally_core::{AreaId, Project, ProjectId, RepeatFrom, RepeatRule, Start, Status};

use super::{RawSync, TagLinks, Values, get_all, get_one, read_sync, sync_values, write};
use crate::codec::{self, Decoder};
use crate::error::{Kind, Result};
use crate::normalize::search_text;

const D: Decoder = Decoder { table: "projects" };

/// «Когда», срок, состояние и повтор – одинаковы у задач и проектов.
pub(crate) struct Schedule {
    pub start: Start,
    pub date: Option<Date>,
    pub evening: bool,
    pub deadline: Option<Date>,
    pub status: Status,
    pub repeat_rule: Option<RepeatRule>,
    pub repeat_from: RepeatFrom,
    pub repeat_anchor: Option<Date>,
    pub repeat_advanced: bool,
}

pub(crate) struct RawSchedule {
    start: String,
    date: Option<String>,
    evening: bool,
    deadline: Option<String>,
    status: String,
    closed_at: Option<String>,
    repeat_interval: Option<i64>,
    repeat_unit: Option<String>,
    repeat_from: String,
    repeat_anchor: Option<String>,
    repeat_advanced: bool,
}

impl RawSchedule {
    pub(crate) fn get(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            start: row.get("start")?,
            date: row.get("date")?,
            evening: row.get("evening")?,
            deadline: row.get("deadline")?,
            status: row.get("status")?,
            closed_at: row.get("closed_at")?,
            repeat_interval: row.get("repeat_interval")?,
            repeat_unit: row.get("repeat_unit")?,
            repeat_from: row.get("repeat_from")?,
            repeat_anchor: row.get("repeat_anchor")?,
            repeat_advanced: row.get("repeat_advanced")?,
        })
    }

    pub(crate) fn build(self, d: Decoder) -> Result<Schedule> {
        Ok(Schedule {
            start: d.start(&self.start)?,
            date: d.parse_opt("date", self.date.as_deref())?,
            evening: self.evening,
            deadline: d.parse_opt("deadline", self.deadline.as_deref())?,
            status: d.status(&self.status, self.closed_at.as_deref())?,
            repeat_rule: d.repeat_rule(self.repeat_interval, self.repeat_unit.as_deref())?,
            repeat_from: d.repeat_from(&self.repeat_from)?,
            repeat_anchor: d.parse_opt("repeat_anchor", self.repeat_anchor.as_deref())?,
            repeat_advanced: self.repeat_advanced,
        })
    }
}

pub(crate) fn schedule_values<'a>(s: Schedule) -> Values<'a> {
    let (status, closed_at) = codec::status(s.status);
    let (interval, unit) = codec::repeat_rule(s.repeat_rule);
    vec![
        ("start", Box::new(codec::start(s.start))),
        ("date", Box::new(codec::opt_date(s.date))),
        ("evening", Box::new(s.evening)),
        ("deadline", Box::new(codec::opt_date(s.deadline))),
        ("status", Box::new(status)),
        ("closed_at", Box::new(closed_at)),
        ("repeat_interval", Box::new(interval)),
        ("repeat_unit", Box::new(unit)),
        ("repeat_from", Box::new(codec::repeat_from(s.repeat_from))),
        ("repeat_anchor", Box::new(codec::opt_date(s.repeat_anchor))),
        ("repeat_advanced", Box::new(s.repeat_advanced)),
    ]
}

struct Raw {
    id: String,
    title: String,
    notes: String,
    area_id: Option<String>,
    schedule: RawSchedule,
    sort_index: String,
    today_index: String,
    sync: RawSync,
}

fn raw(row: &Row<'_>) -> rusqlite::Result<Raw> {
    Ok(Raw {
        id: row.get("id")?,
        title: row.get("title")?,
        notes: row.get("notes")?,
        area_id: row.get("area_id")?,
        schedule: RawSchedule::get(row)?,
        sort_index: row.get("sort_index")?,
        today_index: row.get("today_index")?,
        sync: RawSync::get(row)?,
    })
}

fn values(p: &Project) -> Values<'_> {
    let mut v: Values<'_> = vec![
        ("title", Box::new(p.title.as_str())),
        ("notes", Box::new(p.notes.as_str())),
        (
            "search_text",
            Box::new(search_text([p.title.as_str(), p.notes.as_str()])),
        ),
        ("area_id", Box::new(p.area.map(|a| a.0.to_string()))),
        ("sort_index", Box::new(p.index.as_str())),
        ("today_index", Box::new(p.today_index.as_str())),
    ];
    v.extend(schedule_values(Schedule {
        start: p.start,
        date: p.date,
        evening: p.evening,
        deadline: p.deadline,
        status: p.status,
        repeat_rule: p.repeat_rule,
        repeat_from: p.repeat_from,
        repeat_anchor: p.repeat_anchor,
        repeat_advanced: p.repeat_advanced,
    }));
    v.extend(sync_values(&p.sync));
    v
}

pub(crate) fn save(conn: &Connection, p: &Project, insert: bool) -> Result<()> {
    write(conn, Kind::Project, p.id.0, &values(p), insert)?;
    TagLinks::Project.replace(conn, Kind::Project, p.id.0, &p.tags)
}

fn build(r: Raw, tags: Vec<tally_core::TagId>) -> Result<Project> {
    let s = r.schedule.build(D)?;
    Ok(Project {
        id: ProjectId::from_uuid(D.parse("id", &r.id)?),
        title: r.title,
        notes: r.notes,
        area: D
            .parse_opt("area_id", r.area_id.as_deref())?
            .map(AreaId::from_uuid),
        start: s.start,
        date: s.date,
        evening: s.evening,
        deadline: s.deadline,
        status: s.status,
        tags,
        index: D.parse("sort_index", &r.sort_index)?,
        today_index: D.parse("today_index", &r.today_index)?,
        repeat_rule: s.repeat_rule,
        repeat_from: s.repeat_from,
        repeat_anchor: s.repeat_anchor,
        repeat_advanced: s.repeat_advanced,
        sync: read_sync(D, &r.sync)?,
    })
}

pub(crate) fn get(conn: &Connection, id: ProjectId) -> Result<Option<Project>> {
    let Some(r) = get_one(conn, "SELECT * FROM projects WHERE id = ?1", id.0, raw)? else {
        return Ok(None);
    };
    let tags = TagLinks::Project
        .load(conn, Some(id.0))?
        .remove(&r.id)
        .unwrap_or_default();
    build(r, tags).map(Some)
}

pub(crate) fn all(conn: &Connection) -> Result<Vec<Project>> {
    let mut tags = TagLinks::Project.load(conn, None)?;
    get_all(conn, "SELECT * FROM projects ORDER BY sort_index", raw)?
        .into_iter()
        .map(|r| {
            let t = tags.remove(&r.id).unwrap_or_default();
            build(r, t)
        })
        .collect()
}
