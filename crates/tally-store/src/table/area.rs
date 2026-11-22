//! Таблица `areas`.

use rusqlite::{Connection, Row};
use tally_core::{Area, AreaId};

use super::{RawSync, TagLinks, Values, get_all, get_one, read_sync, sync_values, write};
use crate::codec::Decoder;
use crate::error::{Kind, Result};
use crate::normalize::search_text;

const D: Decoder = Decoder { table: "areas" };

struct Raw {
    id: String,
    title: String,
    sort_index: String,
    sync: RawSync,
}

fn raw(row: &Row<'_>) -> rusqlite::Result<Raw> {
    Ok(Raw {
        id: row.get("id")?,
        title: row.get("title")?,
        sort_index: row.get("sort_index")?,
        sync: RawSync::get(row)?,
    })
}

fn values(a: &Area) -> Values<'_> {
    let mut v: Values<'_> = vec![
        ("title", Box::new(a.title.as_str())),
        ("search_text", Box::new(search_text([a.title.as_str()]))),
        ("sort_index", Box::new(a.index.as_str())),
    ];
    v.extend(sync_values(&a.sync));
    v
}

pub(crate) fn save(conn: &Connection, a: &Area, insert: bool) -> Result<()> {
    write(conn, Kind::Area, a.id.0, &values(a), insert)?;
    TagLinks::Area.replace(conn, Kind::Area, a.id.0, &a.tags)
}

fn build(r: Raw, tags: Vec<tally_core::TagId>) -> Result<Area> {
    Ok(Area {
        id: AreaId::from_uuid(D.parse("id", &r.id)?),
        title: r.title,
        tags,
        index: D.parse("sort_index", &r.sort_index)?,
        sync: read_sync(D, &r.sync)?,
    })
}

pub(crate) fn get(conn: &Connection, id: AreaId) -> Result<Option<Area>> {
    let Some(r) = get_one(conn, "SELECT * FROM areas WHERE id = ?1", id.0, raw)? else {
        return Ok(None);
    };
    let tags = TagLinks::Area
        .load(conn, Some(id.0))?
        .remove(&r.id)
        .unwrap_or_default();
    build(r, tags).map(Some)
}

pub(crate) fn all(conn: &Connection) -> Result<Vec<Area>> {
    let mut tags = TagLinks::Area.load(conn, None)?;
    get_all(conn, "SELECT * FROM areas ORDER BY sort_index", raw)?
        .into_iter()
        .map(|r| {
            let t = tags.remove(&r.id).unwrap_or_default();
            build(r, t)
        })
        .collect()
}
