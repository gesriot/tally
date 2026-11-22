//! Неоднозначность названия раздела ищется только среди разделов, в которые
//! можно перенести задачу.

use jiff::Timestamp;
use jiff::civil::date;
use tally_app::{Clock, Command, Effect, Error, MoveTask, execute};
use tally_core::{Heading, HeadingId, Project, ProjectId, Rank, Start, Status, Task, TaskId};
use tally_store::Store;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const PROJECT: ProjectId = ProjectId::from_u128(1);
const TASK: TaskId = TaskId::from_u128(2);
const BATH: HeadingId = HeadingId::from_u128(3);
const CREATED: Timestamp = Timestamp::UNIX_EPOCH;

fn clock() -> Clock {
    Clock {
        today: date(1970, 1, 1),
        now: "2026-11-19T12:00:00Z".parse().unwrap_or(CREATED),
    }
}

/// «Ремонт» с задачей в «Ванной» и двумя живыми «Кухнями».
fn store(status: Status) -> Result<Store> {
    let mut store = Store::open_in_memory()?;
    let mut project = Project::new(PROJECT, "Ремонт", Rank::initial(), CREATED);
    project.status = status;
    let mut task = Task::new(TASK, "Плитка", Rank::initial(), CREATED);
    task.start = Start::Anytime;
    task.project = Some(PROJECT);
    task.heading = Some(BATH);
    store.write(|w| {
        w.insert_project(&project)?;
        w.insert_heading(&Heading::new(
            BATH,
            PROJECT,
            "Ванная",
            Rank::initial(),
            CREATED,
        ))?;
        for n in 4..=5 {
            let id = HeadingId::from_u128(n);
            w.insert_heading(&Heading::new(
                id,
                PROJECT,
                "Кухня",
                Rank::initial(),
                CREATED,
            ))?;
        }
        w.insert_task(&task)
    })?;
    Ok(store)
}

fn kitchen() -> Command {
    Command::MoveTask(MoveTask {
        task: TASK.to_string(),
        heading: Some("Кухня".into()),
        ..MoveTask::default()
    })
}

/// Доступных «Кухонь» нет, а текущий раздел называется иначе: отказ
/// называет настоящую причину (закрытый проект), а не неоднозначность.
#[test]
fn no_available_namesake_and_another_current_heading_gives_the_real_reason() -> Result {
    for status in [
        Status::Completed { at: CREATED },
        Status::Canceled { at: CREATED },
    ] {
        let mut store = store(status)?;
        let before = store.read().snapshot()?;
        let history = store.read().history()?;
        let result = execute(&mut store, &kitchen(), clock()).map(|d| d.effect);
        assert!(
            matches!(&result, Err(Error::Rejected(s)) if s == "раздел «Кухня»: проект «Ремонт» закрыт"),
            "{result:?}"
        );
        assert_eq!(store.read().snapshot()?, before);
        assert_eq!(store.read().history()?, history);
    }
    Ok(())
}

/// В открытом проекте обе «Кухни» доступны: отказ со списком обеих, а
/// после того как одну убрали в архив, переносится в оставшуюся.
#[test]
fn in_an_open_project_two_namesakes_are_ambiguous_and_one_wins() -> Result {
    let mut store = store(Status::Open)?;
    let result = execute(&mut store, &kitchen(), clock()).map(|d| d.effect);
    assert!(
        matches!(&result, Err(Error::Rejected(s))
            if s.contains("несколько") && s.contains("4] Кухня") && s.contains("5] Кухня")),
        "{result:?}"
    );
    let mut first = store
        .read()
        .snapshot()?
        .headings
        .into_iter()
        .find(|h| h.id == HeadingId::from_u128(4))
        .ok_or("нет раздела")?;
    first.archived = true;
    store.write(|w| w.update_heading(&first))?;
    assert_eq!(
        execute(&mut store, &kitchen(), clock())?.effect,
        Effect::Moved(TASK)
    );
    assert_eq!(
        store.read().task(TASK)?.ok_or("нет задачи")?.heading,
        Some(HeadingId::from_u128(5))
    );
    execute(&mut store, &Command::Undo, clock())?;
    assert_eq!(
        store.read().task(TASK)?.ok_or("нет задачи")?.heading,
        Some(BATH)
    );
    Ok(())
}
