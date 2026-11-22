//! Перенос по номеру не назначает другой раздел внутри закрытого проекта;
//! прежний раздел и вынос из проекта работают.

use jiff::Timestamp;
use jiff::civil::date;
use tally_app::{Clock, Command, Effect, Error, MoveTask, execute};
use tally_core::{Heading, HeadingId, Project, ProjectId, Rank, Start, Status, Task, TaskId};
use tally_store::{HistoryEntry, Snapshot, Store};

const NOW: Timestamp = Timestamp::UNIX_EPOCH;
const PROJECT: ProjectId = ProjectId::from_u128(1);
const KITCHEN: HeadingId = HeadingId::from_u128(2);
const BATH: HeadingId = HeadingId::from_u128(3);
const TASK: TaskId = TaskId::from_u128(4);

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn fixture(status: Status) -> Result<Store> {
    let mut s = Store::open_in_memory()?;
    let mut project = Project::new(PROJECT, "Ремонт", Rank::initial(), NOW);
    project.status = status;
    let mut task = Task::new(TASK, "Плитка", Rank::initial(), NOW);
    task.start = Start::Anytime;
    task.project = Some(PROJECT);
    task.heading = Some(KITCHEN);
    s.write(|w| {
        w.insert_project(&project)?;
        for (id, title) in [(KITCHEN, "Кухня"), (BATH, "Ванная")] {
            w.insert_heading(&Heading::new(id, PROJECT, title, Rank::initial(), NOW))?;
        }
        w.insert_task(&task)
    })?;
    Ok(s)
}

fn state(s: &Store) -> Result<(Snapshot, Vec<HistoryEntry>)> {
    Ok((s.read().snapshot()?, s.read().history()?))
}

fn run_move(s: &mut Store, options: MoveTask) -> tally_app::Result<Effect> {
    let command = Command::MoveTask(MoveTask {
        task: TASK.to_string(),
        ..options
    });
    let clock = Clock {
        today: date(1970, 1, 1),
        now: NOW,
    };
    execute(s, &command, clock).map(|done| done.effect)
}

#[test]
fn moving_by_id_cannot_assign_another_heading_inside_a_closed_project() -> Result {
    for status in [Status::Completed { at: NOW }, Status::Canceled { at: NOW }] {
        let mut s = fixture(status)?;
        let before = state(&s)?;
        let result = run_move(
            &mut s,
            MoveTask {
                heading: Some("Ванная".into()),
                ..MoveTask::default()
            },
        );
        assert!(matches!(&result, Err(Error::Rejected(_))), "{result:?}");
        assert_eq!(state(&s)?, before);
    }
    Ok(())
}

#[test]
fn keeping_the_heading_and_moving_out_of_a_closed_project_by_id_still_work() -> Result {
    let mut s = fixture(Status::Completed { at: NOW })?;
    let before = state(&s)?;
    let same = run_move(
        &mut s,
        MoveTask {
            heading: Some("Кухня".into()),
            ..MoveTask::default()
        },
    )?;
    assert_eq!(same, Effect::Moved(TASK));
    assert_eq!(state(&s)?, before);

    let moved = run_move(
        &mut s,
        MoveTask {
            no_project: true,
            ..MoveTask::default()
        },
    )?;
    assert_eq!(moved, Effect::Moved(TASK));
    let task = s.read().task(TASK)?.ok_or("нет задачи")?;
    assert_eq!((task.project, task.heading), (None, None));
    Ok(())
}
