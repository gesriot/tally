//! Выбор текущего архивного раздела сохраняет задачу и журнал; неверный новый
//! раздел отклоняется даже на предельной метке.

use jiff::Timestamp;
use jiff::civil::date;
use tally_app::{Clock, Command, Effect, Error, MoveTask, execute};
use tally_core::{Heading, HeadingId, Project, ProjectId, Rank, Start, Status, Task, TaskId};
use tally_store::{HistoryEntry, Snapshot, Store};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

const NOW: Timestamp = Timestamp::UNIX_EPOCH;
const PROJECT: ProjectId = ProjectId::from_u128(1);
const KITCHEN: HeadingId = HeadingId::from_u128(2);
const BATH: HeadingId = HeadingId::from_u128(3);
const TILE: TaskId = TaskId::from_u128(4);

fn setup(status: Status, kitchen_archived: bool, task_stamp: Timestamp) -> Result<Store> {
    let mut store = Store::open_in_memory()?;
    let mut project = Project::new(PROJECT, "Ремонт", Rank::initial(), NOW);
    project.status = status;
    let mut kitchen = Heading::new(KITCHEN, PROJECT, "Кухня", Rank::initial(), NOW);
    kitchen.archived = kitchen_archived;
    let mut tile = Task::new(TILE, "Плитка", Rank::initial(), NOW);
    tile.start = Start::Anytime;
    tile.project = Some(PROJECT);
    tile.heading = Some(KITCHEN);
    tile.sync.touch(task_stamp);
    store.write(|w| {
        w.insert_project(&project)?;
        w.insert_heading(&kitchen)?;
        w.insert_heading(&Heading::new(BATH, PROJECT, "Ванная", Rank::initial(), NOW))?;
        w.insert_task(&tile)
    })?;
    Ok(store)
}

fn state(store: &Store) -> Result<(Snapshot, Vec<HistoryEntry>)> {
    Ok((store.read().snapshot()?, store.read().history()?))
}

fn move_to(store: &mut Store, heading: &str) -> tally_app::Result<Effect> {
    let command = Command::MoveTask(MoveTask {
        task: TILE.to_string(),
        heading: Some(heading.into()),
        ..MoveTask::default()
    });
    let clock = Clock {
        today: date(1970, 1, 1),
        now: NOW,
    };
    execute(store, &command, clock).map(|done| done.effect)
}

#[test]
fn selecting_the_current_archived_heading_keeps_the_task_and_history() -> Result {
    let mut store = setup(Status::Open, true, NOW)?;
    let before = state(&store)?;
    assert_eq!(move_to(&mut store, "Кухня")?, Effect::Moved(TILE));
    assert_eq!(state(&store)?, before);
    Ok(())
}

#[test]
fn invalid_new_heading_is_rejected_even_when_the_task_stamp_is_max() -> Result {
    let mut store = setup(Status::Completed { at: NOW }, false, Timestamp::MAX)?;
    let before = state(&store)?;
    let result = move_to(&mut store, "Ванная");
    assert!(
        matches!(&result, Err(Error::Rejected(reason)) if reason.contains("закрыт")),
        "{result:?}"
    );
    assert_eq!(state(&store)?, before);
    Ok(())
}
