//! Строка журнала в старом формате при первой отмене переписывается в новом
//! виде.

use jiff::Timestamp;
use jiff::civil::date;
use serde_json::json;
use tally_app::{Clock, Command, execute};
use tally_core::{Project, ProjectId, Rank, Start, Status, Task, TaskId};
use tally_store::Store;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

const PROJECT: ProjectId = ProjectId::from_u128(1);
const TASK: TaskId = TaskId::from_u128(2);

#[test]
fn a_legacy_row_is_rewritten_without_names_and_notes() -> Result {
    let at = Timestamp::UNIX_EPOCH;
    let moved: Timestamp = "2026-11-19T10:00:00Z".parse()?;
    let mut project = Project::new(PROJECT, "Ремонт", Rank::initial(), at);
    project.status = Status::Completed { at };
    project.notes = "Купить затирку".into();
    let mut before = Task::new(TASK, "Плитка", Rank::initial(), at);
    before.start = Start::Anytime;
    before.project = Some(PROJECT);
    let mut after = before.clone();
    after.project = None;
    after.sync.touch(moved);
    let changes = json!([{
        "before": {"Task": &before},
        "after": {"Task": &after},
        "stamp": moved,
        "around_before": [{"Project": &project}],
    }])
    .to_string();
    let mut s = Store::open_in_memory()?;
    s.write(|w| {
        w.insert_project(&project)?;
        w.insert_task(&after)?;
        w.push_history(moved, "перенос задачи «Плитка»", &changes, 100)?;
        Ok(())
    })?;
    let clock = Clock {
        today: date(2026, 11, 20),
        now: "2026-11-20T12:00:00Z".parse()?,
    };
    execute(&mut s, &Command::Undo, clock)?;
    let row = &s.read().history()?[0].changes;
    assert!(
        row.contains(r#""chain_before":[{"kind":"project""#),
        "{row}"
    );
    assert!(
        !row.contains("around") && !row.contains("Купить затирку"),
        "{row}"
    );
    // Переписанная строка работает так же: повтор и снова отмена.
    execute(&mut s, &Command::Redo, clock)?;
    execute(&mut s, &Command::Undo, clock)?;
    assert_eq!(s.read().task(TASK)?.and_then(|t| t.project), Some(PROJECT));
    Ok(())
}
