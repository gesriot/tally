//! Проверки команд и конфликты отмены и повтора: повторный перенос, раздел
//! без проекта, пустой тег, закрытие без задач, корзина, метки чек-листа.

use jiff::Timestamp;
use jiff::civil::date;
use tally_app::{Clock, Close, Command, Error, MoveTask, NewHeading, NewProject, NewTask, execute};
use tally_core::{ChecklistItem, ChecklistItemId, Rank, Status, Task, TaskId};
use tally_store::{HistoryEntry, RecordId, Snapshot, Store};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn run(store: &mut Store, command: Command) -> tally_app::Result<()> {
    let clock = Clock {
        today: date(2026, 11, 19),
        now: Timestamp::UNIX_EPOCH,
    };
    execute(store, &command, clock).map(|_| ())
}

fn state(store: &Store) -> Result<(Snapshot, Vec<HistoryEntry>)> {
    Ok((store.read().snapshot()?, store.read().history()?))
}

fn fails_atomically(store: &mut Store, command: Command, expected: fn(&Error) -> bool) -> Result {
    let before = state(store)?;
    let result = run(store, command);
    assert!(
        matches!(&result, Err(error) if expected(error)),
        "{result:?}"
    );
    assert_eq!(state(store)?, before);
    Ok(())
}

fn project(title: &str) -> Command {
    Command::AddProject(NewProject {
        title: title.into(),
        area: None,
    })
}

fn close(title: &str) -> Command {
    Command::Close(Close {
        tasks: vec![title.into()],
        cancel: false,
    })
}

#[test]
fn redundant_move_preserves_the_redo_branch() -> Result {
    let mut s = Store::open_in_memory()?;
    run(
        &mut s,
        Command::AddTask(NewTask {
            text: "Отчёт".into(),
            ..NewTask::default()
        }),
    )?;
    run(&mut s, close("Отчёт"))?;
    run(&mut s, Command::Undo)?;
    let before = state(&s)?;
    run(
        &mut s,
        Command::MoveTask(MoveTask {
            task: "Отчёт".into(),
            no_deadline: true,
            ..MoveTask::default()
        }),
    )?;
    assert_eq!(state(&s)?, before);
    run(&mut s, Command::Redo)?;
    assert!(matches!(
        s.read().tasks()?[0].status,
        Status::Completed { .. }
    ));
    Ok(())
}

#[test]
fn service_rejects_heading_without_project() -> Result {
    let mut s = Store::open_in_memory()?;
    let command = Command::AddTask(NewTask {
        text: "Отчёт".into(),
        heading: Some("Кухня".into()),
        ..NewTask::default()
    });
    fails_atomically(&mut s, command, |e| matches!(e, Error::Rejected(_)))
}

#[test]
fn service_rejects_blank_tag() -> Result {
    let mut s = Store::open_in_memory()?;
    let command = Command::AddTask(NewTask {
        text: "Отчёт".into(),
        tags: vec!["   ".into()],
        ..NewTask::default()
    });
    fails_atomically(&mut s, command, |e| matches!(e, Error::Rejected(_)))
}

#[test]
fn service_rejects_close_without_tasks() -> Result {
    let mut s = Store::open_in_memory()?;
    fails_atomically(&mut s, Command::Close(Close::default()), |e| {
        matches!(e, Error::Rejected(_))
    })
}

#[test]
fn redo_into_a_trashed_project_is_an_atomic_conflict() -> Result {
    let mut s = Store::open_in_memory()?;
    run(&mut s, project("Дача"))?;
    let id = s.read().projects()?[0].id;
    run(
        &mut s,
        Command::AddTask(NewTask {
            text: "Полить".into(),
            project: Some("Дача".into()),
            ..NewTask::default()
        }),
    )?;
    run(&mut s, Command::Undo)?;
    s.write(|w| w.delete(RecordId::Project(id), Timestamp::UNIX_EPOCH))?;
    fails_atomically(&mut s, Command::Redo, |e| matches!(e, Error::Conflict(_)))
}

#[test]
fn undo_into_a_heading_moved_to_another_project_is_a_conflict() -> Result {
    let mut s = Store::open_in_memory()?;
    run(&mut s, project("Ремонт"))?;
    run(&mut s, project("Дача"))?;
    run(
        &mut s,
        Command::AddHeading(NewHeading {
            title: "Кухня".into(),
            project: "Ремонт".into(),
        }),
    )?;
    run(
        &mut s,
        Command::AddTask(NewTask {
            text: "Купить плитку".into(),
            project: Some("Ремонт".into()),
            heading: Some("Кухня".into()),
            ..NewTask::default()
        }),
    )?;
    run(
        &mut s,
        Command::MoveTask(MoveTask {
            task: "Купить плитку".into(),
            project: Some("Дача".into()),
            ..MoveTask::default()
        }),
    )?;
    let snapshot = s.read().snapshot()?;
    let mut heading = snapshot.headings[0].clone();
    heading.project = snapshot
        .projects
        .iter()
        .find(|p| p.title == "Дача")
        .ok_or("нет Дачи")?
        .id;
    heading.sync.touch("2036-11-19T12:00:00Z".parse()?);
    s.write(|w| w.update_heading(&heading))?;
    fails_atomically(&mut s, Command::Undo, |e| matches!(e, Error::Conflict(_)))
}

#[test]
fn newer_checklist_metadata_survives_undo_and_redo() -> Result {
    let mut s = Store::open_in_memory()?;
    let id = TaskId::from_u128(1);
    let mut task = Task::new(id, "Сборы", Rank::initial(), Timestamp::UNIX_EPOCH);
    task.checklist.push(ChecklistItem::new(
        ChecklistItemId::from_u128(2),
        "Паспорт",
        Rank::initial(),
        Timestamp::UNIX_EPOCH,
    ));
    s.write(|w| w.insert_task(&task))?;
    run(&mut s, close("Сборы"))?;
    let mut edited = s.read().task(id)?.ok_or("нет задачи")?;
    let future: Timestamp = "2036-11-19T12:00:00Z".parse()?;
    edited.checklist[0].sync.touch(future);
    s.write(|w| w.update_task(&edited))?;
    for (command, open) in [(Command::Undo, true), (Command::Redo, false)] {
        run(&mut s, command)?;
        let t = s.read().task(id)?.ok_or("нет задачи")?;
        assert_eq!(t.status.is_open(), open);
        assert_eq!(t.checklist[0].sync.updated_at, future);
    }
    Ok(())
}
