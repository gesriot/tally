//! Прежний раздел или проект по названию (даже архивный или закрытый) не
//! меняет ссылку; цепочка в журнале – только id, связи и доступность.

use jiff::Timestamp;
use jiff::civil::date;
use tally_app::{Clock, Command, Effect, Error, MoveTask, execute};
use tally_core::{Heading, HeadingId, Project, ProjectId, Rank, Start, Status, Task, TaskId};
use tally_store::{HistoryEntry, RecordId, Snapshot, Store};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

const PROJECT: ProjectId = ProjectId::from_u128(1);
const KITCHEN: HeadingId = HeadingId::from_u128(2);
const OTHER_KITCHEN: HeadingId = HeadingId::from_u128(3);
const TILE: TaskId = TaskId::from_u128(4);
const NOTE: &str = "Заметка проекта, которой нечего делать в журнале";

fn now() -> Timestamp {
    "2026-11-19T12:00:00Z"
        .parse()
        .unwrap_or(Timestamp::UNIX_EPOCH)
}

fn run(s: &mut Store, c: &Command) -> tally_app::Result<Effect> {
    let clock = Clock {
        today: date(2026, 11, 19),
        now: now(),
    };
    execute(s, c, clock).map(|d| d.effect)
}

fn state(s: &Store) -> Result<(Snapshot, Vec<HistoryEntry>)> {
    Ok((s.read().snapshot()?, s.read().history()?))
}

fn moving(m: MoveTask) -> Command {
    Command::MoveTask(MoveTask {
        task: TILE.to_string(),
        ..m
    })
}

fn some(s: &str) -> Option<String> {
    Some(s.into())
}

/// «Плитка» в разделе «Кухня» проекта «Ремонт» с заметкой.
fn setup(closed: bool, archived: bool) -> Result<Store> {
    let at = Timestamp::UNIX_EPOCH;
    let mut s = Store::open_in_memory()?;
    let mut p = Project::new(PROJECT, "Ремонт", Rank::initial(), at);
    p.notes = NOTE.into();
    if closed {
        p.status = Status::Completed { at };
    }
    let mut h = Heading::new(KITCHEN, PROJECT, "Кухня", Rank::initial(), at);
    h.archived = archived;
    let mut t = Task::new(TILE, "Плитка", Rank::initial(), at);
    t.start = Start::Anytime;
    t.project = Some(PROJECT);
    t.heading = Some(KITCHEN);
    s.write(|w| {
        w.insert_project(&p)?;
        w.insert_heading(&h)?;
        w.insert_task(&t)
    })?;
    Ok(s)
}

fn place(s: &Store) -> Result<(Option<ProjectId>, Option<HeadingId>)> {
    let t = s.read().task(TILE)?.ok_or("нет задачи")?;
    Ok((t.project, t.heading))
}

/// После выноса из закрытого проекта у проекта поменяли только заметку и
/// название, а у архивного раздела – название: доступность та же, отмена
/// и повтор проходят. В журнале нет ни заметки, ни названий контейнеров.
#[test]
fn renaming_or_noting_a_container_does_not_block_undo() -> Result {
    let mut s = setup(true, true)?;
    run(
        &mut s,
        &moving(MoveTask {
            no_project: true,
            ..MoveTask::default()
        }),
    )?;
    let changes = &s.read().history()?[0].changes;
    assert!(!changes.contains(NOTE), "{changes}");
    assert!(changes.contains(r#""kind":"project""#), "{changes}");

    let mut p = s.read().project(PROJECT)?.ok_or("нет проекта")?;
    p.notes = "Другая заметка".into();
    p.title = "Ремонт кухни".into();
    let mut h = s.read().heading(KITCHEN)?.ok_or("нет раздела")?;
    h.title = "Старая кухня".into();
    s.write(|w| {
        w.update_project(&p)?;
        w.update_heading(&h)
    })?;
    run(&mut s, &Command::Undo)?;
    assert_eq!(place(&s)?, (Some(PROJECT), Some(KITCHEN)));
    run(&mut s, &Command::Redo)?;
    assert_eq!(place(&s)?, (None, None));
    Ok(())
}

/// Изменилась доступность контейнера – проект убрали в корзину или
/// переоткрыли и снова закрыли с другой областью – проверяется всё.
#[test]
fn a_changed_availability_is_checked_again() -> Result {
    let mut s = setup(true, false)?;
    run(
        &mut s,
        &moving(MoveTask {
            no_project: true,
            ..MoveTask::default()
        }),
    )?;
    s.write(|w| w.delete(RecordId::Project(PROJECT), now()))?;
    let before = state(&s)?;
    let r = run(&mut s, &Command::Undo);
    assert_eq!(
        r.map_err(|e| e.to_string()),
        Err("нельзя отменить перенос задачи «Плитка»: проект «Ремонт» в корзине".to_owned())
    );
    assert_eq!(state(&s)?, before);
    // Из корзины вернули – он снова такой же, как в момент действия.
    s.write(|w| w.restore(RecordId::Project(PROJECT), now()))?;
    run(&mut s, &Command::Undo)?;
    assert_eq!(place(&s)?, (Some(PROJECT), Some(KITCHEN)));
    Ok(())
}

/// Отменённый вместо выполненного – тоже закрыт: доступность та же.
#[test]
fn completed_to_canceled_keeps_the_project_closed() -> Result {
    let mut s = setup(true, false)?;
    run(
        &mut s,
        &moving(MoveTask {
            no_project: true,
            ..MoveTask::default()
        }),
    )?;
    let mut p = s.read().project(PROJECT)?.ok_or("нет проекта")?;
    p.status = Status::Canceled { at: now() };
    s.write(|w| w.update_project(&p))?;
    run(&mut s, &Command::Undo)?;
    assert_eq!(place(&s)?, (Some(PROJECT), Some(KITCHEN)));
    Ok(())
}

/// Прежний проект и раздел по названию – и в закрытом проекте, и в
/// архивном разделе – успех без изменений; неизвестный раздел – отказ.
#[test]
fn naming_the_current_containers_is_a_no_op() -> Result {
    for (closed, archived) in [(true, false), (false, true), (true, true)] {
        let mut s = setup(closed, archived)?;
        let before = state(&s)?;
        for m in [
            MoveTask {
                heading: some("кухня"),
                ..MoveTask::default()
            },
            MoveTask {
                project: some("Ремонт"),
                ..MoveTask::default()
            },
            MoveTask {
                project: some("ремонт"),
                heading: some("Кухня"),
                ..MoveTask::default()
            },
        ] {
            let r = run(&mut s, &moving(m.clone()));
            assert!(matches!(r, Ok(Effect::Moved(TILE))), "{m:?}: {r:?}");
            assert_eq!(state(&s)?, before, "{m:?}");
        }
        let r = run(
            &mut s,
            &moving(MoveTask {
                heading: some("Ванная"),
                ..MoveTask::default()
            }),
        );
        assert_eq!(
            r.map_err(|e| e.to_string()),
            Err("раздела «Ванная» в проекте «Ремонт» нет".to_owned())
        );
        assert_eq!(state(&s)?, before);
    }
    Ok(())
}

/// Есть и живой одноимённый раздел – выбирается он: живой раздел по
/// названию важнее прежнего архивного.
#[test]
fn a_live_heading_with_the_same_name_wins() -> Result {
    let mut s = setup(false, true)?;
    let other = Heading::new(OTHER_KITCHEN, PROJECT, "Кухня", Rank::initial(), now());
    s.write(|w| w.insert_heading(&other))?;
    run(
        &mut s,
        &moving(MoveTask {
            heading: some("Кухня"),
            ..MoveTask::default()
        }),
    )?;
    assert_eq!(place(&s)?, (Some(PROJECT), Some(OTHER_KITCHEN)));
    run(&mut s, &Command::Undo)?;
    assert_eq!(place(&s)?, (Some(PROJECT), Some(KITCHEN)));
    Ok(())
}

/// На пределе меток: недопустимая ссылка – `Rejected` (причина важнее
/// времени), допустимая настоящая правка – `NoLaterTime`, без изменений –
/// успех. Ничего не пишется.
#[test]
fn at_max_the_reason_comes_first() -> Result {
    let mut s = setup(true, false)?;
    let mut t = s.read().task(TILE)?.ok_or("нет задачи")?;
    t.sync.touch(Timestamp::MAX);
    let h = Heading::new(OTHER_KITCHEN, PROJECT, "Ванная", Rank::initial(), now());
    s.write(|w| {
        w.update_task(&t)?;
        w.insert_heading(&h)
    })?;
    let before = state(&s)?;
    let bath = run(
        &mut s,
        &moving(MoveTask {
            heading: some("Ванная"),
            ..MoveTask::default()
        }),
    );
    assert_eq!(
        bath.map_err(|e| e.to_string()),
        Err("раздел «Ванная»: проект «Ремонт» закрыт".to_owned())
    );
    let out = run(
        &mut s,
        &moving(MoveTask {
            no_project: true,
            ..MoveTask::default()
        }),
    );
    assert!(
        matches!(out, Err(Error::NoLaterTime(t)) if t == Timestamp::MAX),
        "{out:?}"
    );
    let same = run(
        &mut s,
        &moving(MoveTask {
            heading: some("Кухня"),
            ..MoveTask::default()
        }),
    );
    assert!(matches!(same, Ok(Effect::Moved(TILE))), "{same:?}");
    assert_eq!(state(&s)?, before);
    Ok(())
}
