use jiff::Timestamp;
use rusqlite::{Connection, params};
use tally_core::TaskId;

use crate::codec::{Decoder, ts};
use crate::error::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reminder {
    pub task: TaskId,
    pub title: String,
    pub at: Timestamp,
}

pub(crate) fn due(conn: &Connection, now: Timestamp) -> Result<Vec<Reminder>> {
    let sql = "
        SELECT t.id, t.title, t.reminder_at
        FROM tasks t
        LEFT JOIN projects p ON p.id = t.project_id
        LEFT JOIN headings h ON h.id = t.heading_id
        LEFT JOIN areas a ON a.id = COALESCE(t.area_id, p.area_id)
        WHERE t.status = 'open' AND t.deleted_at IS NULL
          AND t.reminder_at IS NOT NULL AND t.reminder_at <= ?1
          AND (p.id IS NULL OR (p.status = 'open' AND p.deleted_at IS NULL))
          AND h.deleted_at IS NULL AND a.deleted_at IS NULL
          AND NOT EXISTS (
              SELECT 1 FROM delivered_task_reminders d
              WHERE d.task_id = t.id AND d.reminder_at = t.reminder_at AND d.shown = 1
          )
        ORDER BY t.reminder_at, t.id";
    let mut query = conn.prepare_cached(sql)?;
    let mut rows = query.query([ts(now)])?;
    let mut reminders = Vec::new();
    let decoder = Decoder { table: "tasks" };
    while let Some(row) = rows.next()? {
        reminders.push(Reminder {
            task: TaskId::from_uuid(decoder.parse("id", &row.get::<_, String>(0)?)?),
            title: row.get(1)?,
            at: decoder.parse("reminder_at", &row.get::<_, String>(2)?)?,
        });
    }
    Ok(reminders)
}

pub(crate) fn claim(conn: &Connection, now: Timestamp) -> Result<Vec<Reminder>> {
    let reminders = due(conn, now)?;
    if !reminders.is_empty() {
        let mut insert = conn.prepare_cached(
            "INSERT INTO delivered_task_reminders (task_id, reminder_at, shown) VALUES (?1, ?2, 0)
             ON CONFLICT (task_id, reminder_at) DO NOTHING",
        )?;
        for reminder in &reminders {
            insert.execute(params![reminder.task.0.to_string(), ts(reminder.at)])?;
        }
    }
    Ok(reminders)
}

pub(crate) fn confirmed(conn: &Connection, task: TaskId) -> Result<Vec<Timestamp>> {
    let mut query = conn.prepare_cached(
        "SELECT reminder_at FROM delivered_task_reminders WHERE task_id = ?1 AND shown = 1 ORDER BY reminder_at",
    )?;
    let decoder = Decoder {
        table: "delivered_task_reminders",
    };
    query
        .query_map([task.0.to_string()], |row| row.get::<_, String>(0))?
        .map(|value| decoder.parse("reminder_at", &value?))
        .collect()
}

pub(crate) fn restore_confirmed(
    conn: &Connection,
    task: TaskId,
    reminders: &[Timestamp],
) -> Result<()> {
    let mut insert = conn.prepare_cached(
        "INSERT INTO delivered_task_reminders (task_id, reminder_at, shown) VALUES (?1, ?2, 1)",
    )?;
    for &at in reminders {
        insert.execute(params![task.0.to_string(), ts(at)])?;
    }
    Ok(())
}

pub(crate) fn confirm(conn: &Connection, shown: &[Reminder]) -> Result<()> {
    let mut update = conn.prepare_cached(
        "UPDATE delivered_task_reminders SET shown = 1 WHERE task_id = ?1 AND reminder_at = ?2",
    )?;
    for reminder in shown {
        update.execute(params![reminder.task.0.to_string(), ts(reminder.at)])?;
    }
    Ok(())
}
