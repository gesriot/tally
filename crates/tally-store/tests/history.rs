//! Журнал действий: порядок, отмена, стирание отменённого, предел, миграция.

mod common;

use common::*;
use rusqlite::{Connection, params};
use tally_core::{ProjectId, RepeatFrom, RepeatRule, RepeatUnit, Start, TaskId};
use tally_store::{SCHEMA_VERSION, Store};

fn store() -> Store {
    Store::open_in_memory().unwrap_or_else(|e| panic!("база в памяти: {e}"))
}

#[test]
fn undo_takes_the_newest_and_redo_the_oldest_undone() -> tally_store::Result<()> {
    let mut s = store();
    assert_eq!(s.read().last_done()?, None);
    assert_eq!(s.read().first_undone()?, None);
    let (a, b, c) = s.write(|w| {
        Ok((
            w.push_history(t0(), "а", "[1]", 10)?,
            w.push_history(t0(), "б", "[2]", 10)?,
            w.push_history(t0(), "в", "[3]", 10)?,
        ))
    })?;
    assert!(a < b && b < c);
    assert_eq!(s.read().last_done()?.map(|e| e.action), Some("в".into()));
    s.write(|w| {
        assert!(w.mark_history(c, true, "[3']")?);
        assert!(w.mark_history(b, true, "[2']")?);
        assert!(!w.mark_history(c + 100, true, "[]")?);
        Ok(())
    })?;
    assert_eq!(s.read().last_done()?.map(|e| e.action), Some("а".into()));
    let redo = s.read().first_undone()?;
    assert_eq!(
        redo.map(|e| (e.action, e.changes, e.undone, e.at)),
        Some(("б".into(), "[2']".into(), true, t0()))
    );
    // Новое действие стирает отменённые.
    s.write(|w| w.push_history(t0(), "г", "[4]", 10))?;
    let all: Vec<String> = s.read().history()?.into_iter().map(|e| e.action).collect();
    assert_eq!(all, ["а", "г"]);
    assert_eq!(s.read().first_undone()?, None);
    Ok(())
}

#[test]
fn only_the_last_entries_are_kept() -> tally_store::Result<()> {
    let mut s = store();
    for n in 0..5 {
        s.write(|w| w.push_history(t0(), &n.to_string(), "[]", 3))?;
    }
    let all: Vec<String> = s.read().history()?.into_iter().map(|e| e.action).collect();
    assert_eq!(all, ["2", "3", "4"]);
    Ok(())
}

#[test]
fn history_rolls_back_with_the_transaction() -> tally_store::Result<()> {
    let mut s = store();
    let failed: tally_store::Result<()> = s.write(|w| {
        w.insert_task(&task(1, "Отчёт"))?;
        w.push_history(t0(), "добавление", "[]", 10)?;
        w.insert_task(&task(1, "Дубль"))
    });
    assert!(failed.is_err());
    assert!(s.read().history()?.is_empty());
    assert!(s.read().tasks()?.is_empty());
    Ok(())
}

/// База первой версии получает журнал, данные остаются.
#[test]
fn version_one_database_gets_the_history_table() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("v1.db");
    {
        let conn = Connection::open(&path)?;
        conn.execute_batch(include_str!("../migrations/0001_init.sql"))?;
        conn.pragma_update(None, "user_version", 1)?;
        conn.execute(
            "INSERT INTO areas (id, title, search_text, sort_index, created_at, updated_at) \
             VALUES ('00000000-0000-0000-0000-000000000001', 'Дом', 'дом', 'a0', \
             '2026-11-19T09:00:00.000000000Z', '2026-11-19T09:00:00.000000000Z')",
            [],
        )?;
    }
    let mut s = Store::open(&path)?;
    assert_eq!(s.schema_version()?, SCHEMA_VERSION);
    assert_eq!(s.read().areas()?.len(), 1);
    s.write(|w| w.push_history(t0(), "а", "[]", 10))?;
    assert_eq!(s.read().history()?.len(), 1);
    Ok(())
}

#[test]
fn version_three_drops_only_calendar_rules_without_a_date() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("v3.db");
    {
        let conn = Connection::open(&path)?;
        for sql in [
            include_str!("../migrations/0001_init.sql"),
            include_str!("../migrations/0002_history.sql"),
            include_str!("../migrations/0003_repeat_anchor.sql"),
        ] {
            conn.execute_batch(sql)?;
        }
        conn.pragma_update(None, "user_version", 3)?;
        for (id, date, from) in [
            (1, None, "date"),
            (2, Some("2026-11-19"), "date"),
            (3, None, "completion"),
        ] {
            conn.execute(
                "INSERT INTO tasks (id, title, notes, search_text, start, date, evening, status,                      sort_index, today_index, repeat_interval, repeat_unit, repeat_from, repeat_anchor,                      created_at, updated_at) VALUES (?1, 'Старая задача', 'Заметка', 'старая задача',                      'anytime', ?2, 0, 'open', 'a0', 'a0', 1, 'day', ?3, ?2,                      '2026-11-19T09:00:00Z', '2026-11-19T09:00:00Z')",
                params![TaskId::from_u128(id).to_string(), date, from],
            )?;
        }
        conn.execute(
            "INSERT INTO projects (id, title, notes, search_text, start, evening, status,                  sort_index, today_index, repeat_interval, repeat_unit, repeat_from,                  created_at, updated_at) VALUES (?1, 'Старый проект', '', 'старый проект',                  'someday', 0, 'open', 'a0', 'a0', 1, 'week', 'date',                  '2026-11-19T09:00:00Z', '2026-11-19T09:00:00Z')",
            [ProjectId::from_u128(4).to_string()],
        )?;
    }

    let s = Store::open(&path)?;
    assert_eq!(s.schema_version()?, SCHEMA_VERSION);
    let old = s.read().task(TaskId::from_u128(1))?.ok_or("нет задачи")?;
    assert_eq!(old.repeat_rule, None);
    assert_eq!(old.date, None);
    assert_eq!(old.start, Start::Anytime);
    assert_eq!(old.sync.updated_at, t0());
    assert_eq!(old.notes, "Заметка");
    for id in [2, 3] {
        let valid = s.read().task(TaskId::from_u128(id))?.ok_or("нет задачи")?;
        assert_eq!(
            valid.repeat_rule,
            Some(RepeatRule {
                interval: 1,
                unit: RepeatUnit::Day
            })
        );
        if id == 2 {
            assert_eq!(valid.date, Some(jiff::civil::date(2026, 11, 19)));
            assert_eq!(valid.repeat_anchor, valid.date);
        } else {
            assert_eq!(valid.date, None);
            assert_eq!(valid.repeat_from, RepeatFrom::Completion);
        }
    }
    let old = s
        .read()
        .project(ProjectId::from_u128(4))?
        .ok_or("нет проекта")?;
    assert_eq!(old.repeat_rule, None);
    assert_eq!(old.start, Start::Someday);
    Ok(())
}
