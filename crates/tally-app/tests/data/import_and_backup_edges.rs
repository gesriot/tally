//! Граничные случаи импорта и копии: правило без даты, предельная метка,
//! напоминания, нечитаемая копия за день.

use std::fs;
use std::path::Path;

use jiff::{Timestamp, civil::date};
use tally_app::{App, Clock, Command, DataReply, DataRequest, Service};
use tally_core::{Area, AreaId, Project, ProjectId, Rank, RepeatRule, RepeatUnit, Task, TaskId};
use tally_store::{Snapshot, Store};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn clock() -> Clock {
    Clock {
        now: Timestamp::UNIX_EPOCH,
        today: date(1970, 1, 1),
    }
}

fn task() -> Task {
    Task::new(
        TaskId::from_u128(21),
        "Позвонить",
        Rank::initial(),
        clock().now,
    )
}

fn write_json(path: &Path, snapshot: &Snapshot) -> Result {
    let document = serde_json::json!({"format": "tally", "version": 1, "data": snapshot});
    fs::write(path, serde_json::to_vec(&document)?)?;
    Ok(())
}

#[test]
fn undated_calendar_rule_rejects_the_whole_file_for_task_and_project() -> Result {
    let directory = tempfile::tempdir()?;
    let file = directory.path().join("import.json");
    let rule = Some(RepeatRule {
        interval: 1,
        unit: RepeatUnit::Day,
    });
    for is_project in [false, true] {
        let mut incoming = Snapshot::default();
        incoming.areas.push(Area::new(
            AreaId::from_u128(10),
            "Работа",
            Rank::initial(),
            clock().now,
        ));
        if is_project {
            let mut project = Project::new(
                ProjectId::from_u128(22),
                "Дом",
                Rank::initial(),
                clock().now,
            );
            project.repeat_rule = rule;
            incoming.projects.push(project);
        } else {
            let mut task = task();
            task.repeat_rule = rule;
            incoming.tasks.push(task);
        }
        write_json(&file, &incoming)?;
        let mut app = App::new(Store::open_in_memory()?);
        assert!(
            app.data(DataRequest::Import(file.clone()), clock())
                .is_err()
        );
        assert_eq!(app.snapshot()?, Snapshot::default());
        assert!(app.store().read().history()?.is_empty());
    }
    Ok(())
}

#[test]
fn import_at_max_timestamp_does_not_leave_an_unundoable_history_entry() -> Result {
    let directory = tempfile::tempdir()?;
    let file = directory.path().join("import.json");
    let mut task = task();
    task.sync.updated_at = Timestamp::MAX;
    let snapshot = Snapshot {
        tasks: vec![task],
        ..Snapshot::default()
    };
    write_json(&file, &snapshot)?;
    let mut app = App::new(Store::open_in_memory()?);
    match app.data(DataRequest::Import(file), clock()) {
        Err(_) => {
            assert_eq!(app.snapshot()?, Snapshot::default());
            assert!(app.store().read().history()?.is_empty());
        }
        Ok(DataReply::Imported(1)) => {
            assert_eq!(app.snapshot()?, snapshot);
            assert_eq!(
                app.execute(&Command::Undo, clock())?.snapshot,
                Snapshot::default()
            );
            let restored = app.execute(&Command::Redo, clock())?.snapshot;
            assert_eq!(restored.tasks.len(), 1);
            assert_eq!(restored.tasks[0].id, snapshot.tasks[0].id);
        }
        Ok(other) => panic!("неожиданный ответ: {other:?}"),
    }
    Ok(())
}

#[test]
fn replaying_import_keeps_confirmed_and_pending_reminders_distinct() -> Result {
    let directory = tempfile::tempdir()?;
    let file = directory.path().join("import.json");
    let mut shown = task();
    shown.reminder_at = Some(clock().now);
    let mut pending = shown.clone();
    pending.id = TaskId::from_u128(22);
    pending.title = "Уточнить адрес".into();
    let pending_id = pending.id;
    let shown_id = shown.id;
    let snapshot = Snapshot {
        tasks: vec![shown, pending],
        ..Snapshot::default()
    };
    write_json(&file, &snapshot)?;
    let mut app = App::new(Store::open_in_memory()?);
    app.data(DataRequest::Import(file), clock())?;
    let claimed = app.claim_reminders(clock().now)?;
    assert_eq!(claimed.len(), 2);
    let delivered = claimed
        .iter()
        .find(|r| r.task == shown_id)
        .ok_or("нет напоминания")?;
    app.confirm_reminders(std::slice::from_ref(delivered))?;
    let due = |app: &mut App| -> Result<Vec<TaskId>> {
        Ok(app
            .claim_reminders(clock().now)?
            .into_iter()
            .map(|r| r.task)
            .collect())
    };
    assert_eq!(due(&mut app)?, vec![pending_id]);
    app.execute(&Command::Undo, clock())?;
    assert!(due(&mut app)?.is_empty());
    app.execute(&Command::Redo, clock())?;
    assert_eq!(due(&mut app)?, vec![pending_id]);
    Ok(())
}

#[test]
fn existing_unreadable_daily_backup_reports_failure_and_preserves_the_file() -> Result {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("1970-01-01.sqlite3");
    let original = b"Existing file without a SQLite database";
    fs::write(&path, original)?;
    let mut app = App::new(Store::open_in_memory()?);
    assert!(app.store().settings()?.automatic_backups);
    let result = app.data(
        DataRequest::DailyBackup(directory.path().to_owned()),
        clock(),
    );
    assert!(result.is_err(), "нечитаемый файл принят за готовую копию");
    assert_eq!(fs::read(path)?.as_slice(), original);
    Ok(())
}
