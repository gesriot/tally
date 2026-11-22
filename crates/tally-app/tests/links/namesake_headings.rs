//! Одноимённые разделы: выбор среди доступных; первый повтор переписывает
//! уже отменённую строку журнала в старом формате.

use jiff::Timestamp;
use jiff::civil::date;
use serde_json::json;
use tally_app::{Clock, Command, Effect, Error, MoveTask, execute};
use tally_core::{Heading, HeadingId, Project, ProjectId, Rank, Start, Status, Task, TaskId};
use tally_store::Store;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const PROJECT: ProjectId = ProjectId::from_u128(1);
const TASK: TaskId = TaskId::from_u128(2);
const CREATED: Timestamp = Timestamp::UNIX_EPOCH;

fn clock() -> Clock {
    Clock {
        today: date(1970, 1, 1),
        now: CREATED,
    }
}

#[test]
fn only_available_namesakes_compete_with_the_current_heading() -> Result {
    for status in [
        Status::Open,
        Status::Completed { at: CREATED },
        Status::Canceled { at: CREATED },
    ] {
        let mut store = Store::open_in_memory()?;
        let mut project = Project::new(PROJECT, "Ремонт", Rank::initial(), CREATED);
        project.status = status;
        let mut task = Task::new(TASK, "Плитка", Rank::initial(), CREATED);
        task.start = Start::Anytime;
        task.project = Some(PROJECT);
        task.heading = Some(HeadingId::from_u128(3));
        store.write(|w| {
            w.insert_project(&project)?;
            for n in 3..=5 {
                let id = HeadingId::from_u128(n);
                let mut heading = Heading::new(id, PROJECT, "Кухня", Rank::initial(), CREATED);
                heading.archived = Some(id) == task.heading;
                w.insert_heading(&heading)?;
            }
            w.insert_task(&task)
        })?;
        let before = store.read().snapshot()?;
        let history = store.read().history()?;
        let command = Command::MoveTask(MoveTask {
            task: TASK.to_string(),
            heading: Some("Кухня".into()),
            ..MoveTask::default()
        });
        let result = execute(&mut store, &command, clock()).map(|d| d.effect);
        if status.is_open() {
            assert!(
                matches!(&result, Err(Error::Rejected(s)) if s.contains("несколько")),
                "{result:?}"
            );
        } else {
            assert_eq!(result?, Effect::Moved(TASK));
        }
        assert_eq!(store.read().snapshot()?, before);
        assert_eq!(store.read().history()?, history);
    }
    Ok(())
}

#[test]
fn first_redo_upgrades_an_already_undone_legacy_row() -> Result {
    let moved: Timestamp = "2026-11-19T10:00:00Z".parse()?;
    let undone: Timestamp = "2026-11-19T11:00:00Z".parse()?;
    let mut project = Project::new(PROJECT, "Ремонт", Rank::initial(), CREATED);
    project.status = Status::Completed { at: CREATED };
    project.notes = "Старая заметка проекта".into();
    let mut before = Task::new(TASK, "Плитка", Rank::initial(), CREATED);
    before.start = Start::Anytime;
    before.project = Some(PROJECT);
    let mut after = before.clone();
    after.project = None;
    after.sync.touch(moved);
    before.sync.touch(undone);
    let legacy = json!([{
        "before": {"Task": &before},
        "after": {"Task": &after},
        "stamp": undone,
        "around_before": [{"Project": &project}],
    }])
    .to_string();
    let mut store = Store::open_in_memory()?;
    let seq = store.write(|w| {
        w.insert_project(&project)?;
        w.insert_task(&before)?;
        let seq = w.push_history(moved, "перенос задачи «Плитка»", &legacy, 100)?;
        w.mark_history(seq, true, &legacy)?;
        Ok(seq)
    })?;
    let old = store.read().history()?;
    assert_eq!(old[0].changes, legacy);
    assert!(old[0].undone);
    execute(&mut store, &Command::Redo, clock())?;
    let rows = store.read().history()?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].seq, seq);
    assert!(!rows[0].undone);
    let changes: serde_json::Value = serde_json::from_str(&rows[0].changes)?;
    assert!(changes[0].get("around_before").is_none());
    assert_eq!(changes[0]["chain_before"][0]["open"], json!(false));
    assert!(!rows[0].changes.contains("Старая заметка проекта"));
    assert_eq!(store.read().task(TASK)?.ok_or("нет задачи")?.project, None);
    execute(&mut store, &Command::Undo, clock())?;
    assert_eq!(
        store.read().task(TASK)?.ok_or("нет задачи")?.project,
        Some(PROJECT)
    );
    Ok(())
}
