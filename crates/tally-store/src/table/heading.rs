//! Таблица `headings`.

use rusqlite::{Connection, OptionalExtension, Row};
use tally_core::{Heading, HeadingId, ProjectId};

use super::{RawSync, Values, get_all, get_one, read_sync, sync_values, write};
use crate::codec::{self, Decoder};
use crate::error::{Kind, Result, StoreError};

const D: Decoder = Decoder { table: "headings" };

struct Raw {
    id: String,
    project_id: String,
    title: String,
    archived: bool,
    sort_index: String,
    sync: RawSync,
}

fn raw(row: &Row<'_>) -> rusqlite::Result<Raw> {
    Ok(Raw {
        id: row.get("id")?,
        project_id: row.get("project_id")?,
        title: row.get("title")?,
        archived: row.get("archived")?,
        sort_index: row.get("sort_index")?,
        sync: RawSync::get(row)?,
    })
}

fn values(h: &Heading) -> Values<'_> {
    let mut v: Values<'_> = vec![
        ("project_id", Box::new(h.project.0.to_string())),
        ("title", Box::new(h.title.as_str())),
        ("archived", Box::new(h.archived)),
        ("sort_index", Box::new(h.index.as_str())),
    ];
    v.extend(sync_values(&h.sync));
    v
}

/// Записать раздел. Перенос в другой проект переносит и его задачи
/// (`ON UPDATE CASCADE` в схеме); их `updated_at` становится временем
/// правки раздела, чтобы выборка изменений по времени их не пропустила.
///
/// Метка переноса не может быть раньше последнего изменения самого раздела
/// или любой его задачи (`created_at`/`updated_at`): иначе у задачи время
/// правки ушло бы назад. Такой перенос отклоняется с
/// [`StoreError::StaleTimestamp`] – вызывающий код повторит его с текущим временем.
pub(crate) fn save(conn: &Connection, h: &Heading, insert: bool) -> Result<()> {
    let old: Option<(String, String)> = if insert {
        None
    } else {
        conn.prepare_cached("SELECT project_id, updated_at FROM headings WHERE id = ?1")?
            .query_row([h.id.0.to_string()], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?
    };
    let moved = old
        .as_ref()
        .is_some_and(|(project, _)| *project != h.project.0.to_string());
    if moved {
        check_move_time(conn, h, old.map(|(_, at)| at).unwrap_or_default())?;
    }
    write(conn, Kind::Heading, h.id.0, &values(h), insert)?;
    if moved {
        conn.prepare_cached("UPDATE tasks SET updated_at = ?1 WHERE heading_id = ?2")?
            .execute([codec::ts(h.sync.updated_at), h.id.0.to_string()])?;
    }
    Ok(())
}

/// Метка переноса `h` – не раньше `heading_updated_at` (текущая метка раздела
/// в базе) и последнего изменения каждой задачи раздела. Метки в базе –
/// UTC фиксированной ширины, поэтому строки сравниваются как время.
fn check_move_time(conn: &Connection, h: &Heading, heading_updated_at: String) -> Result<()> {
    let at = codec::ts(h.sync.updated_at);
    if heading_updated_at > at {
        return Err(StoreError::StaleTimestamp {
            kind: Kind::Heading,
            id: h.id.0,
            latest: D.parse("updated_at", &heading_updated_at)?,
        });
    }
    let newest: Option<(String, String)> = conn
        .prepare_cached(
            "SELECT id, max(created_at, updated_at) AS changed FROM tasks \
             WHERE heading_id = ?1 ORDER BY changed DESC LIMIT 1",
        )?
        .query_row([h.id.0.to_string()], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    match newest {
        Some((task, changed)) if changed > at => {
            let d = Decoder { table: "tasks" };
            Err(StoreError::StaleTimestamp {
                kind: Kind::Task,
                id: d.parse("id", &task)?,
                latest: d.parse("updated_at", &changed)?,
            })
        }
        _ => Ok(()),
    }
}

fn build(r: Raw) -> Result<Heading> {
    Ok(Heading {
        id: HeadingId::from_uuid(D.parse("id", &r.id)?),
        project: ProjectId::from_uuid(D.parse("project_id", &r.project_id)?),
        title: r.title,
        archived: r.archived,
        index: D.parse("sort_index", &r.sort_index)?,
        sync: read_sync(D, &r.sync)?,
    })
}

pub(crate) fn get(conn: &Connection, id: HeadingId) -> Result<Option<Heading>> {
    get_one(conn, "SELECT * FROM headings WHERE id = ?1", id.0, raw)?
        .map(build)
        .transpose()
}

pub(crate) fn all(conn: &Connection) -> Result<Vec<Heading>> {
    get_all(
        conn,
        "SELECT * FROM headings ORDER BY project_id, sort_index",
        raw,
    )?
    .into_iter()
    .map(build)
    .collect()
}
