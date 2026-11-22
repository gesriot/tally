//! Старый формат истории и выбор одноимённого раздела в закрытом проекте.

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

/// В базе проект теперь закрыт, задача вынесена. Меняются только сведения,
/// которые старый журнал мог сохранить о проекте на момент переноса.
fn legacy_move(project_at_move: Option<Status>) -> Result<Store> {
    let moved: Timestamp = "2026-11-19T10:00:00Z".parse()?;
    let mut project = Project::new(PROJECT, "Ремонт", Rank::initial(), CREATED);
    let mut before = Task::new(TASK, "Плитка", Rank::initial(), CREATED);
    before.start = Start::Anytime;
    before.project = Some(PROJECT);
    let mut after = before.clone();
    after.project = None;
    after.sync.touch(moved);

    // В прежнем формате журнала Record – enum с внешним тегом: {"Project": <вся запись>}.
    // Пустой around_after при таком переносе та версия не писала.
    let mut change = json!({
        "before": {"Task": &before},
        "after": {"Task": &after},
        "stamp": moved,
    });
    if let Some(status) = project_at_move {
        project.status = status;
        change["around_before"] = json!([{"Project": &project}]);
    }
    let changes = json!([change]).to_string();
    project.status = match project_at_move {
        Some(Status::Open) => {
            let later = "2026-11-20T10:00:00Z".parse()?;
            project.sync.touch(later);
            Status::Completed { at: later }
        }
        _ => Status::Completed { at: CREATED },
    };
    let mut store = Store::open_in_memory()?;
    store.write(|w| {
        w.insert_project(&project)?;
        w.insert_task(&after)?;
        w.push_history(moved, "перенос задачи «Плитка»", &changes, 100)?;
        Ok(())
    })?;
    Ok(store)
}

fn step(store: &mut Store, command: Command) -> tally_app::Result<()> {
    let clock = Clock {
        today: date(2026, 11, 20),
        now: CREATED,
    };
    execute(store, &command, clock).map(|_| ())
}

#[test]
fn legacy_move_out_of_an_already_closed_project_keeps_working_after_upgrade() -> Result {
    let mut store = legacy_move(Some(Status::Completed { at: CREATED }))?;
    for (command, expected) in [
        (Command::Undo, Some(PROJECT)),
        (Command::Redo, None),
        (Command::Undo, Some(PROJECT)),
    ] {
        step(&mut store, command)?;
        let task = store.read().task(TASK)?.ok_or("нет задачи")?;
        assert_eq!(task.project, expected);
    }
    Ok(())
}

#[test]
fn a_later_closure_or_a_missing_legacy_chain_still_conflicts() -> Result {
    for status in [Some(Status::Open), None] {
        let mut store = legacy_move(status)?;
        let snapshot = store.read().snapshot()?;
        let history = store.read().history()?;
        let result = step(&mut store, Command::Undo);
        assert!(matches!(&result, Err(Error::Conflict(_))), "{result:?}");
        assert_eq!(store.read().snapshot()?, snapshot);
        assert_eq!(store.read().history()?, history);
    }
    Ok(())
}

#[test]
fn an_unavailable_namesake_does_not_shadow_the_current_heading() -> Result {
    let mut store = Store::open_in_memory()?;
    let mut project = Project::new(PROJECT, "Ремонт", Rank::initial(), CREATED);
    project.status = Status::Completed { at: CREATED };
    let mut current = Heading::new(
        HeadingId::from_u128(3),
        PROJECT,
        "Кухня",
        Rank::initial(),
        CREATED,
    );
    current.archived = true;
    let other = Heading::new(
        HeadingId::from_u128(4),
        PROJECT,
        "Кухня",
        Rank::initial(),
        CREATED,
    );
    let mut task = Task::new(TASK, "Плитка", Rank::initial(), CREATED);
    task.project = Some(PROJECT);
    task.heading = Some(current.id);
    task.start = Start::Anytime;
    store.write(|w| {
        w.insert_project(&project)?;
        w.insert_heading(&current)?;
        w.insert_heading(&other)?;
        w.insert_task(&task)
    })?;
    let snapshot = store.read().snapshot()?;
    let history = store.read().history()?;
    let command = Command::MoveTask(MoveTask {
        task: TASK.to_string(),
        heading: Some("Кухня".into()),
        ..MoveTask::default()
    });
    let clock = Clock {
        today: date(2026, 11, 20),
        now: CREATED,
    };
    assert_eq!(
        execute(&mut store, &command, clock)?.effect,
        Effect::Moved(TASK)
    );
    assert_eq!(store.read().snapshot()?, snapshot);
    assert_eq!(store.read().history()?, history);
    Ok(())
}
