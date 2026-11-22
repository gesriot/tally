//! Таблицы: SQL и преобразование строк в модели. Каждая сущность – свой модуль.

pub(crate) mod area;
pub(crate) mod checklist;
pub(crate) mod heading;
pub(crate) mod history;
pub(crate) mod project;
pub(crate) mod reminder;
pub(crate) mod tag;
pub(crate) mod task;
pub(crate) mod trash;

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, Row, ToSql, params};
use tally_core::{SyncMeta, TagId};
use uuid::Uuid;

use crate::codec::{self, Decoder};
use crate::error::{Kind, Result, StoreError, constraint};

/// Таблица записи данного вида.
pub(crate) fn table_of(kind: Kind) -> &'static str {
    match kind {
        Kind::Area => "areas",
        Kind::Tag => "tags",
        Kind::Project => "projects",
        Kind::Heading => "headings",
        Kind::Task => "tasks",
        Kind::ChecklistItem => "checklist_items",
    }
}

/// Значения колонок для одной записи: `(колонка, значение)`.
pub(crate) type Values<'a> = Vec<(&'static str, Box<dyn ToSql + 'a>)>;

pub(crate) fn sync_values<'a>(sync: &SyncMeta) -> Values<'a> {
    vec![
        ("created_at", Box::new(codec::ts(sync.created_at))),
        ("updated_at", Box::new(codec::ts(sync.updated_at))),
        ("deleted_at", Box::new(codec::opt_ts(sync.deleted_at))),
    ]
}

pub(crate) fn read_sync(d: Decoder, row: &RawSync) -> Result<SyncMeta> {
    Ok(SyncMeta {
        created_at: d.parse("created_at", &row.created_at)?,
        updated_at: d.parse("updated_at", &row.updated_at)?,
        deleted_at: d.parse_opt("deleted_at", row.deleted_at.as_deref())?,
    })
}

/// Сырые колонки синхронизации.
pub(crate) struct RawSync {
    created_at: String,
    updated_at: String,
    deleted_at: Option<String>,
}

impl RawSync {
    pub(crate) fn get(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
            deleted_at: row.get("deleted_at")?,
        })
    }
}

/// Вставить (`insert = true`) или обновить запись `id` в `table`.
pub(crate) fn write(
    conn: &Connection,
    kind: Kind,
    id: Uuid,
    values: &Values<'_>,
    insert: bool,
) -> Result<()> {
    let table = table_of(kind);
    let sql = if insert {
        let cols: Vec<&str> = values.iter().map(|(c, _)| *c).collect();
        let marks: Vec<String> = (1..=cols.len()).map(|i| format!("?{i}")).collect();
        format!(
            "INSERT INTO {table} (id, {}) VALUES (?{}, {})",
            cols.join(", "),
            cols.len() + 1,
            marks.join(", ")
        )
    } else {
        let sets: Vec<String> = values
            .iter()
            .enumerate()
            .map(|(i, (c, _))| format!("{c} = ?{}", i + 1))
            .collect();
        format!(
            "UPDATE {table} SET {} WHERE id = ?{}",
            sets.join(", "),
            values.len() + 1
        )
    };
    let id_text = id.to_string();
    let mut params: Vec<&dyn ToSql> = values.iter().map(|(_, v)| v.as_ref()).collect();
    params.push(&id_text);
    let changed = conn
        .prepare_cached(&sql)?
        .execute(params.as_slice())
        .map_err(|e| constraint(e, kind, id))?;
    if changed == 0 {
        return Err(StoreError::NotFound { kind, id });
    }
    Ok(())
}

/// Мягкое удаление (`deleted_at = Some`) или возврат из корзины (`None`).
pub(crate) fn set_deleted(
    conn: &Connection,
    kind: Kind,
    id: Uuid,
    deleted_at: Option<jiff::Timestamp>,
    at: jiff::Timestamp,
) -> Result<()> {
    let sql = format!(
        "UPDATE {} SET deleted_at = ?1, updated_at = ?2 WHERE id = ?3",
        table_of(kind)
    );
    let changed = conn.prepare_cached(&sql)?.execute(params![
        codec::opt_ts(deleted_at),
        codec::ts(at),
        id.to_string()
    ])?;
    if changed == 0 {
        return Err(StoreError::NotFound { kind, id });
    }
    Ok(())
}

/// Физическое удаление записи из «Корзины» (вместе с зависимыми – по `ON DELETE`).
/// Запись вне корзины – [`StoreError::NotInTrash`].
pub(crate) fn purge(conn: &Connection, kind: Kind, id: Uuid) -> Result<()> {
    let chain = trash::chain(kind);
    let deleted = conn
        .prepare_cached(&format!(
            "SELECT {} {} WHERE a.id = ?1",
            chain.flags, chain.from
        ))?
        .query_row([id.to_string()], |r| trash::deleted(r, 0))
        .optional()?;
    match deleted {
        None => return Err(StoreError::NotFound { kind, id }),
        Some(d) if !d.is_trashed() => return Err(StoreError::NotInTrash { kind, id }),
        Some(_) => {}
    }
    let sql = format!("DELETE FROM {} WHERE id = ?1", table_of(kind));
    conn.prepare_cached(&sql)?.execute([id.to_string()])?;
    Ok(())
}

/// Одна запись по id: `select` – запрос с `WHERE id = ?1`.
pub(crate) fn get_one<T>(
    conn: &Connection,
    select: &str,
    id: Uuid,
    map: impl FnOnce(&Row<'_>) -> rusqlite::Result<T>,
) -> Result<Option<T>> {
    Ok(conn
        .prepare_cached(select)?
        .query_row([id.to_string()], map)
        .optional()?)
}

/// Первая строка запроса без параметров.
pub(crate) fn get_one_where<T>(
    conn: &Connection,
    select: &str,
    map: impl FnOnce(&Row<'_>) -> rusqlite::Result<T>,
) -> Result<Option<T>> {
    Ok(conn.prepare_cached(select)?.query_row([], map).optional()?)
}

/// Все строки запроса без параметров.
pub(crate) fn get_all<T>(
    conn: &Connection,
    select: &str,
    map: impl FnMut(&Row<'_>) -> rusqlite::Result<T>,
) -> Result<Vec<T>> {
    let mut stmt = conn.prepare_cached(select)?;
    let rows = stmt.query_map([], map)?;
    Ok(rows.collect::<rusqlite::Result<Vec<T>>>()?)
}

/// Таблица связей «запись – тег».
#[derive(Clone, Copy)]
pub(crate) enum TagLinks {
    Task,
    Project,
    Area,
}

impl TagLinks {
    fn table_and_owner(self) -> (&'static str, &'static str) {
        match self {
            Self::Task => ("task_tags", "task_id"),
            Self::Project => ("project_tags", "project_id"),
            Self::Area => ("area_tags", "area_id"),
        }
    }

    /// Заменить теги записи `owner` (порядок сохраняется).
    pub(crate) fn replace(
        self,
        conn: &Connection,
        kind: Kind,
        owner: Uuid,
        tags: &[TagId],
    ) -> Result<()> {
        let (table, col) = self.table_and_owner();
        let owner_text = owner.to_string();
        conn.prepare_cached(&format!("DELETE FROM {table} WHERE {col} = ?1"))?
            .execute([&owner_text])?;
        let mut insert = conn.prepare_cached(&format!(
            "INSERT INTO {table} ({col}, tag_id, position) VALUES (?1, ?2, ?3)"
        ))?;
        for (pos, tag) in tags.iter().enumerate() {
            let pos = i64::try_from(pos).unwrap_or(i64::MAX);
            insert
                .execute(params![owner_text, tag.0.to_string(), pos])
                .map_err(|e| match constraint(e, kind, owner) {
                    StoreError::AlreadyExists { .. } => StoreError::DuplicateTag { tag: tag.0 },
                    other => other,
                })?;
        }
        Ok(())
    }

    /// Теги записей: `owner` – одна запись, `None` – все сразу.
    pub(crate) fn load(
        self,
        conn: &Connection,
        owner: Option<Uuid>,
    ) -> Result<HashMap<String, Vec<TagId>>> {
        let (table, col) = self.table_and_owner();
        let d = Decoder { table };
        let base = format!("SELECT {col}, tag_id FROM {table}");
        let order = format!("ORDER BY {col}, position");
        let rows: Vec<(String, String)> = match owner {
            Some(id) => {
                let mut stmt = conn.prepare_cached(&format!("{base} WHERE {col} = ?1 {order}"))?;
                stmt.query_map([id.to_string()], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<rusqlite::Result<_>>()?
            }
            None => get_all(conn, &format!("{base} {order}"), |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?,
        };
        let mut out: HashMap<String, Vec<TagId>> = HashMap::new();
        for (owner, tag) in rows {
            let tag = TagId::from_uuid(d.parse("tag_id", &tag)?);
            out.entry(owner).or_default().push(tag);
        }
        Ok(out)
    }
}
