//! Таблица `history` – журнал действий для отмены и повтора.

use jiff::Timestamp;
use rusqlite::{Connection, Row, params};

use super::{get_all, get_one_where};
use crate::codec::{self, Decoder};
use crate::error::Result;

const D: Decoder = Decoder { table: "history" };

/// Одно действие в журнале.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    /// Порядковый номер: чем больше, тем новее.
    pub seq: i64,
    /// Время действия.
    pub at: Timestamp,
    /// Название действия для вывода: «перенос задачи «Отчёт»».
    pub action: String,
    /// Состояния записей до и после – формат вызывающего кода.
    pub changes: String,
    /// Отменено и ждёт повтора.
    pub undone: bool,
}

const SELECT: &str = "SELECT seq, at, action, changes, undone FROM history";

struct Raw {
    seq: i64,
    at: String,
    action: String,
    changes: String,
    undone: bool,
}

fn raw(row: &Row<'_>) -> rusqlite::Result<Raw> {
    Ok(Raw {
        seq: row.get("seq")?,
        at: row.get("at")?,
        action: row.get("action")?,
        changes: row.get("changes")?,
        undone: row.get("undone")?,
    })
}

fn decode(r: Raw) -> Result<HistoryEntry> {
    Ok(HistoryEntry {
        seq: r.seq,
        at: D.parse("at", &r.at)?,
        action: r.action,
        changes: r.changes,
        undone: r.undone,
    })
}

/// Последнее неотменённое действие – его отменит undo.
pub(crate) fn last_done(conn: &Connection) -> Result<Option<HistoryEntry>> {
    let sql = format!("{SELECT} WHERE undone = 0 ORDER BY seq DESC LIMIT 1");
    get_one_where(conn, &sql, raw)?.map(decode).transpose()
}

/// Первое отменённое действие – его повторит redo.
pub(crate) fn first_undone(conn: &Connection) -> Result<Option<HistoryEntry>> {
    let sql = format!("{SELECT} WHERE undone = 1 ORDER BY seq LIMIT 1");
    get_one_where(conn, &sql, raw)?.map(decode).transpose()
}

pub(crate) fn all(conn: &Connection) -> Result<Vec<HistoryEntry>> {
    get_all(conn, &format!("{SELECT} ORDER BY seq"), raw)?
        .into_iter()
        .map(decode)
        .collect()
}

/// Новое действие: отменённые стираются (повторять их уже нечего), из
/// старых остаются последние `keep - 1`.
pub(crate) fn push(
    conn: &Connection,
    at: Timestamp,
    action: &str,
    changes: &str,
    keep: usize,
) -> Result<i64> {
    conn.prepare_cached("DELETE FROM history WHERE undone = 1")?
        .execute([])?;
    conn.prepare_cached("INSERT INTO history (at, action, changes) VALUES (?1, ?2, ?3)")?
        .execute(params![codec::ts(at), action, changes])?;
    let seq = conn.last_insert_rowid();
    let keep = i64::try_from(keep.max(1)).unwrap_or(i64::MAX);
    conn.prepare_cached(
        "DELETE FROM history WHERE seq NOT IN (SELECT seq FROM history ORDER BY seq DESC LIMIT ?1)",
    )?
    .execute([keep])?;
    Ok(seq)
}

/// Отметить действие отменённым (`undone`) или повторённым и сохранить
/// новые состояния записей.
pub(crate) fn mark(conn: &Connection, seq: i64, undone: bool, changes: &str) -> Result<bool> {
    let n = conn
        .prepare_cached("UPDATE history SET undone = ?1, changes = ?2 WHERE seq = ?3")?
        .execute(params![undone, changes, seq])?;
    Ok(n == 1)
}
