//! Таблица `checklist_items`: чек-лист сохраняется вместе с задачей целиком;
//! убранный пункт остаётся надгробием (`deleted_at`) до очистки корзины.

use std::collections::HashMap;

use rusqlite::{Connection, Row, params};
use tally_core::{ChecklistItem, ChecklistItemId};
use uuid::Uuid;

use super::{RawSync, read_sync};
use crate::codec::{self, Decoder};
use crate::error::{Kind, Result, StoreError, constraint};

const D: Decoder = Decoder {
    table: "checklist_items",
};

struct Raw {
    id: String,
    task_id: String,
    title: String,
    done: bool,
    sort_index: String,
    sync: RawSync,
}

fn raw(row: &Row<'_>) -> rusqlite::Result<Raw> {
    Ok(Raw {
        id: row.get("id")?,
        task_id: row.get("task_id")?,
        title: row.get("title")?,
        done: row.get("done")?,
        sort_index: row.get("sort_index")?,
        sync: RawSync::get(row)?,
    })
}

/// Сохранить чек-лист задачи `task`: пункты массива записываются как есть
/// (вместе с надгробиями), а пропавшие из массива получают `deleted_at = at`
/// и остаются надгробием – для будущей синхронизации. Физически надгробия
/// удаляет только очистка корзины.
pub(crate) fn save(
    conn: &Connection,
    task: Uuid,
    items: &[ChecklistItem],
    at: jiff::Timestamp,
) -> Result<()> {
    let task_text = task.to_string();
    let mut upsert = conn.prepare_cached(
        "INSERT INTO checklist_items \
         (id, task_id, title, done, sort_index, created_at, updated_at, deleted_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
         ON CONFLICT (id) DO UPDATE SET title = excluded.title, done = excluded.done, \
         sort_index = excluded.sort_index, created_at = excluded.created_at, \
         updated_at = excluded.updated_at, deleted_at = excluded.deleted_at \
         WHERE checklist_items.task_id = excluded.task_id",
    )?;
    let mut seen = std::collections::HashSet::with_capacity(items.len());
    if let Some(dup) = items.iter().find(|i| !seen.insert(i.id)) {
        return Err(StoreError::DuplicateChecklistItem {
            task,
            item: dup.id.0,
        });
    }
    let mut ids = Vec::with_capacity(items.len());
    for item in items {
        let id = item.id.0.to_string();
        let changed = upsert
            .execute(params![
                id,
                task_text,
                item.title,
                item.done,
                item.index.as_str(),
                codec::ts(item.sync.created_at),
                codec::ts(item.sync.updated_at),
                codec::opt_ts(item.sync.deleted_at),
            ])
            .map_err(|e| constraint(e, Kind::Task, task))?;
        // Пункт с тем же id у другой задачи: условие `WHERE` не дало его перенести.
        if changed == 0 {
            return Err(StoreError::AlreadyExists {
                kind: Kind::ChecklistItem,
                id: item.id.0,
            });
        }
        ids.push(id);
    }
    // Пропавшие из массива – в надгробия (уже удалённые не трогаем).
    let mut live = conn.prepare_cached(
        "SELECT id FROM checklist_items WHERE task_id = ?1 AND deleted_at IS NULL",
    )?;
    let gone: Vec<String> = live
        .query_map([&task_text], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?
        .into_iter()
        .filter(|id| !ids.contains(id))
        .collect();
    let mut bury = conn.prepare_cached(
        "UPDATE checklist_items SET deleted_at = ?1, updated_at = ?1 WHERE id = ?2",
    )?;
    for id in gone {
        bury.execute(params![codec::ts(at), id])?;
    }
    Ok(())
}

/// Чек-листы: `task` – одной задачи, `None` – всех; ключ – id задачи.
pub(crate) fn load(
    conn: &Connection,
    task: Option<Uuid>,
) -> Result<HashMap<String, Vec<ChecklistItem>>> {
    let rows: Vec<Raw> = match task {
        Some(id) => conn
            .prepare_cached("SELECT * FROM checklist_items WHERE task_id = ?1 ORDER BY sort_index")?
            .query_map([id.to_string()], raw)?
            .collect::<rusqlite::Result<_>>()?,
        None => super::get_all(
            conn,
            "SELECT * FROM checklist_items ORDER BY task_id, sort_index",
            raw,
        )?,
    };
    let mut out: HashMap<String, Vec<ChecklistItem>> = HashMap::new();
    for r in rows {
        let item = ChecklistItem {
            id: ChecklistItemId::from_uuid(D.parse("id", &r.id)?),
            title: r.title,
            done: r.done,
            index: D.parse("sort_index", &r.sort_index)?,
            sync: read_sync(D, &r.sync)?,
        };
        out.entry(r.task_id).or_default().push(item);
    }
    Ok(out)
}
