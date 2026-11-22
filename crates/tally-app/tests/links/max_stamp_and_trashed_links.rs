//! Предельная метка времени и ссылки на корзину при отмене и повторе
//! переноса раздела.

use jiff::Timestamp;
use jiff::civil::date;
use tally_app::{Clock, Command, Effect, Error, MoveTask, NewTask, execute};
use tally_core::{Area, AreaId, Heading, HeadingId, Project, ProjectId, Rank, Start, Task, TaskId};
use tally_store::{HistoryEntry, RecordId, Snapshot, Store};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

const AREA: AreaId = AreaId::from_u128(1);
const PROJECT: ProjectId = ProjectId::from_u128(2);
const KITCHEN: HeadingId = HeadingId::from_u128(3);
const BATH: HeadingId = HeadingId::from_u128(4);
const TASK: TaskId = TaskId::from_u128(5);
const NOW: Timestamp = Timestamp::UNIX_EPOCH;

fn run(store: &mut Store, command: Command) -> tally_app::Result<Effect> {
    let clock = Clock {
        today: date(1970, 1, 1),
        now: NOW,
    };
    execute(store, &command, clock).map(|done| done.effect)
}

fn state(store: &Store) -> Result<(Snapshot, Vec<HistoryEntry>)> {
    Ok((store.read().snapshot()?, store.read().history()?))
}

#[test]
fn no_change_at_max_time_succeeds_and_keeps_an_unrelated_redo() -> Result {
    let mut store = Store::open_in_memory()?;
    let mut task = Task::new(TASK, "Отчёт", Rank::initial(), NOW);
    task.sync.touch(Timestamp::MAX);
    store.write(|w| w.insert_task(&task))?;
    run(
        &mut store,
        Command::AddTask(NewTask {
            text: "Письмо".into(),
            ..NewTask::default()
        }),
    )?;
    run(&mut store, Command::Undo)?;

    let before = state(&store)?;
    let effect = run(
        &mut store,
        Command::MoveTask(MoveTask {
            task: "Отчёт".into(),
            no_deadline: true,
            ..MoveTask::default()
        }),
    )?;
    assert_eq!(effect, Effect::Moved(TASK));
    assert_eq!(state(&store)?, before);
    run(&mut store, Command::Redo)?;
    assert!(store.read().tasks()?.iter().any(|t| t.title == "Письмо"));
    Ok(())
}

fn moved_between_headings() -> Result<Store> {
    let mut store = Store::open_in_memory()?;
    let mut project = Project::new(PROJECT, "Ремонт", Rank::initial(), NOW);
    project.area = Some(AREA);
    let mut task = Task::new(TASK, "Плитка", Rank::initial(), NOW);
    task.project = Some(PROJECT);
    task.heading = Some(KITCHEN);
    task.start = Start::Anytime;
    store.write(|w| {
        w.insert_area(&Area::new(AREA, "Дом", Rank::initial(), NOW))?;
        w.insert_project(&project)?;
        for (id, title) in [(KITCHEN, "Кухня"), (BATH, "Ванная")] {
            w.insert_heading(&Heading::new(id, PROJECT, title, Rank::initial(), NOW))?;
        }
        w.insert_task(&task)
    })?;
    run(
        &mut store,
        Command::MoveTask(MoveTask {
            task: "Плитка".into(),
            heading: Some("Ванная".into()),
            ..MoveTask::default()
        }),
    )?;
    Ok(store)
}

fn conflict_until_restored(store: &mut Store, parent: RecordId, step: Command) -> Result {
    store.write(|w| w.delete(parent, NOW))?;
    let before = state(store)?;
    let result = run(store, step.clone());
    assert!(matches!(&result, Err(Error::Conflict(_))), "{result:?}");
    assert_eq!(state(store)?, before);
    store.write(|w| w.restore(parent, NOW))?;
    run(store, step)?;
    Ok(())
}

#[test]
fn undo_heading_move_checks_the_trashed_project() -> Result {
    let mut store = moved_between_headings()?;
    conflict_until_restored(&mut store, RecordId::Project(PROJECT), Command::Undo)?;
    let task = store.read().task(TASK)?.ok_or("нет задачи")?;
    assert_eq!(task.heading, Some(KITCHEN));
    Ok(())
}

#[test]
fn redo_heading_move_checks_the_trashed_area() -> Result {
    let mut store = moved_between_headings()?;
    run(&mut store, Command::Undo)?;
    conflict_until_restored(&mut store, RecordId::Area(AREA), Command::Redo)?;
    let task = store.read().task(TASK)?.ok_or("нет задачи")?;
    assert_eq!(task.heading, Some(BATH));
    Ok(())
}
