//! Конфликты аргументов, поиск задачи по названию, «Планы», пустой тег, путь
//! к базе из окружения.

use std::error::Error;
use std::path::Path;
use std::process::{Command, Output};

use tally_core::Status;
use tally_store::Store;

type TestResult = Result<(), Box<dyn Error>>;

fn cli(db: &Path, args: &[&str]) -> std::io::Result<Output> {
    Command::new(env!("CARGO_BIN_EXE_tally"))
        .arg("--db")
        .arg(db)
        .args(args)
        .env("TALLY_TODAY", "2026-11-19")
        .output()
}

fn ok(db: &Path, args: &[&str]) -> Result<String, Box<dyn Error>> {
    let out = cli(db, args)?;
    assert!(out.status.success(), "{args:?}: {out:?}");
    Ok(String::from_utf8(out.stdout)?)
}

#[test]
fn heading_and_area_conflict_before_move() -> TestResult {
    let dir = tempfile::tempdir()?;
    let db = dir.path().join("tally.db");
    ok(&db, &["area", "Дом"])?;
    ok(&db, &["project", "Ремонт"])?;
    ok(&db, &["heading", "Покупки", "-p", "Ремонт"])?;
    ok(&db, &["add", "Краска", "-p", "Ремонт"])?;
    let before = Store::open(&db)?.read().tasks()?;
    let out = cli(
        &db,
        &["move", "Краска", "--heading", "Покупки", "-a", "Дом"],
    )?;
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert_eq!(Store::open(&db)?.read().tasks()?, before);
    Ok(())
}

#[test]
fn closed_project_child_does_not_compete_for_done_by_title() -> TestResult {
    let dir = tempfile::tempdir()?;
    let db = dir.path().join("tally.db");
    ok(&db, &["project", "Старый проект"])?;
    ok(&db, &["add", "Отчёт", "-p", "Старый проект"])?;
    ok(&db, &["add", "Отчёт"])?;
    {
        let mut store = Store::open(&db)?;
        let mut project = store.read().projects()?.pop().ok_or("нет проекта")?;
        project.status = Status::Completed {
            at: project.sync.updated_at,
        };
        store.write(|w| w.update_project(&project))?;
    }
    ok(&db, &["done", "Отчёт"])?;
    let tasks = Store::open(&db)?.read().tasks()?;
    let loose = tasks
        .iter()
        .find(|t| t.project.is_none())
        .ok_or("нет отдельной задачи")?;
    let child = tasks
        .iter()
        .find(|t| t.project.is_some())
        .ok_or("нет задачи проекта")?;
    assert!(matches!(loose.status, Status::Completed { .. }));
    assert!(child.status.is_open());
    Ok(())
}

#[test]
fn upcoming_keeps_all_seven_day_groups() -> TestResult {
    let dir = tempfile::tempdir()?;
    let db = dir.path().join("tally.db");
    ok(&db, &["add", "Встреча завтра"])?;
    let output = ok(&db, &["list", "планы"])?;
    let mut lines = output.lines();
    for day in [
        "пт, 20 ноября",
        "сб, 21 ноября",
        "вс, 22 ноября",
        "пн, 23 ноября",
        "вт, 24 ноября",
        "ср, 25 ноября",
        "чт, 26 ноября",
    ] {
        assert!(lines.any(|line| line.contains(day)), "{day}: {output}");
    }
    Ok(())
}

#[test]
fn blank_tag_does_not_create_a_task_or_other_tags() -> TestResult {
    let dir = tempfile::tempdir()?;
    let db = dir.path().join("tally.db");
    ok(&db, &["list", "inbox"])?;
    let out = cli(&db, &["add", "Письмо", "-t", "Срочно", "-t", "   "])?;
    assert!(!out.status.success(), "{out:?}");
    let snapshot = Store::open(&db)?.read().snapshot()?;
    assert!(snapshot.tags.is_empty());
    assert!(snapshot.tasks.is_empty());
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn db_env_accepts_native_paths_without_falling_back() -> TestResult {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let dir = tempfile::tempdir()?;
    let db = dir.path().join(OsStr::from_bytes(b"tally-\xff.db"));
    let output = Command::new(env!("CARGO_BIN_EXE_tally"))
        .args(["add", "Письмо"])
        .env("TALLY_DB", &db)
        .env("TALLY_TODAY", "2026-11-19")
        .env("XDG_DATA_HOME", dir.path())
        .env("HOME", dir.path())
        .output()?;
    assert!(output.status.success(), "{output:?}");
    assert!(db.is_file(), "запись ушла в другую базу");
    let tasks = Store::open(&db)?.read().tasks()?;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "Письмо");
    Ok(())
}
