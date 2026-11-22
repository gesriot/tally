//! Таблица `tags`.

use rusqlite::{Connection, Row};
use tally_core::{Tag, TagId};

use super::{RawSync, Values, get_all, get_one, read_sync, sync_values, write};
use crate::codec::Decoder;
use crate::error::{Kind, Result};
use crate::normalize::search_text;

const D: Decoder = Decoder { table: "tags" };

struct Raw {
    id: String,
    title: String,
    parent_id: Option<String>,
    sort_index: String,
    sync: RawSync,
}

fn raw(row: &Row<'_>) -> rusqlite::Result<Raw> {
    Ok(Raw {
        id: row.get("id")?,
        title: row.get("title")?,
        parent_id: row.get("parent_id")?,
        sort_index: row.get("sort_index")?,
        sync: RawSync::get(row)?,
    })
}

fn values(t: &Tag) -> Values<'_> {
    let mut v: Values<'_> = vec![
        ("title", Box::new(t.title.as_str())),
        ("search_text", Box::new(search_text([t.title.as_str()]))),
        ("parent_id", Box::new(t.parent.map(|p| p.0.to_string()))),
        ("sort_index", Box::new(t.index.as_str())),
    ];
    v.extend(sync_values(&t.sync));
    v
}

pub(crate) fn save(conn: &Connection, t: &Tag, insert: bool) -> Result<()> {
    write(conn, Kind::Tag, t.id.0, &values(t), insert)
}

fn build(r: Raw) -> Result<Tag> {
    Ok(Tag {
        id: TagId::from_uuid(D.parse("id", &r.id)?),
        title: r.title,
        parent: D
            .parse_opt("parent_id", r.parent_id.as_deref())?
            .map(TagId::from_uuid),
        index: D.parse("sort_index", &r.sort_index)?,
        sync: read_sync(D, &r.sync)?,
    })
}

pub(crate) fn get(conn: &Connection, id: TagId) -> Result<Option<Tag>> {
    get_one(conn, "SELECT * FROM tags WHERE id = ?1", id.0, raw)?
        .map(build)
        .transpose()
}

pub(crate) fn all(conn: &Connection) -> Result<Vec<Tag>> {
    get_all(conn, "SELECT * FROM tags ORDER BY sort_index", raw)?
        .into_iter()
        .map(build)
        .collect()
}
