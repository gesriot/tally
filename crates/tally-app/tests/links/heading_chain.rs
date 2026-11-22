//! Новая ссылка на раздел проверяет всю цепочку «раздел → проект → область»,
//! метка правки ставится только настоящей правке.

use jiff::Timestamp;
use jiff::civil::{Date, date};
use tally_app::{
    Clock, Close, Command, Effect, Error, MoveTask, NewArea, NewHeading, NewProject, NewTask,
    execute,
};
use tally_core::{Status, TaskId};
use tally_store::{HistoryEntry, RecordId, Snapshot, Store};

const TODAY: Date = date(2026, 11, 19);

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn now() -> Timestamp {
    "2026-11-19T12:00:00Z"
        .parse()
        .unwrap_or(Timestamp::UNIX_EPOCH)
}

fn run(s: &mut Store, c: Command) -> tally_app::Result<Effect> {
    let clock = Clock {
        today: TODAY,
        now: now(),
    };
    execute(s, &c, clock).map(|d| d.effect)
}

fn state(s: &Store) -> Result<(Snapshot, Vec<HistoryEntry>)> {
    Ok((s.read().snapshot()?, s.read().history()?))
}

/// Конфликт с текстом `message`, база и журнал не меняются.
fn conflict(s: &mut Store, c: Command, message: &str) -> Result {
    let before = state(s)?;
    let r = run(s, c);
    assert!(matches!(&r, Err(Error::Conflict(_))), "{r:?}");
    assert_eq!(r.map_err(|e| e.to_string()), Err(message.to_owned()));
    assert_eq!(state(s)?, before);
    Ok(())
}

fn move_to(task: &str) -> MoveTask {
    MoveTask {
        task: task.into(),
        ..MoveTask::default()
    }
}

/// Область «Дом», в ней проект «Ремонт» с разделами «Кухня» и «Ванная»;
/// «Плитка» перенесена из «Кухни» в «Ванную».
fn moved() -> Result<Store> {
    let mut s = Store::open_in_memory()?;
    run(
        &mut s,
        Command::AddArea(NewArea {
            title: "Дом".into(),
        }),
    )?;
    run(
        &mut s,
        Command::AddProject(NewProject {
            title: "Ремонт".into(),
            area: Some("Дом".into()),
        }),
    )?;
    for h in ["Кухня", "Ванная"] {
        run(
            &mut s,
            Command::AddHeading(NewHeading {
                title: h.into(),
                project: "Ремонт".into(),
            }),
        )?;
    }
    run(
        &mut s,
        Command::AddTask(NewTask {
            text: "Плитка".into(),
            project: Some("Ремонт".into()),
            heading: Some("Кухня".into()),
            ..NewTask::default()
        }),
    )?;
    run(
        &mut s,
        Command::MoveTask(MoveTask {
            heading: Some("Ванная".into()),
            ..move_to("Плитка")
        }),
    )?;
    Ok(s)
}

fn ids(s: &Store) -> Result<(RecordId, RecordId, RecordId)> {
    let snap = s.read().snapshot()?;
    let area = snap.areas.first().ok_or("нет области")?.id;
    let project = snap.projects.first().ok_or("нет проекта")?.id;
    let kitchen = snap
        .headings
        .iter()
        .find(|h| h.title == "Кухня")
        .ok_or("нет раздела")?
        .id;
    Ok((
        RecordId::Area(area),
        RecordId::Project(project),
        RecordId::Heading(kitchen),
    ))
}

fn heading_of(s: &Store) -> Result<String> {
    let snap = s.read().snapshot()?;
    let task = snap.tasks.first().ok_or("нет задачи")?;
    let h = snap.headings.iter().find(|h| Some(h.id) == task.heading);
    Ok(h.map(|h| h.title.clone()).unwrap_or_default())
}

#[test]
fn undo_into_a_heading_names_the_trashed_link_of_its_chain() -> Result {
    let at = now();
    for (trash, message) in [
        (
            0,
            "нельзя отменить перенос задачи «Плитка»: раздел «Кухня»: область «Дом» проекта «Ремонт» в корзине",
        ),
        (
            1,
            "нельзя отменить перенос задачи «Плитка»: раздел «Кухня»: проект «Ремонт» в корзине",
        ),
        (
            2,
            "нельзя отменить перенос задачи «Плитка»: раздел «Кухня» в корзине",
        ),
    ] {
        let mut s = moved()?;
        let (area, project, kitchen) = ids(&s)?;
        let id = [area, project, kitchen][trash];
        s.write(|w| w.delete(id, at))?;
        conflict(&mut s, Command::Undo, message)?;
        s.write(|w| w.restore(id, at))?;
        run(&mut s, Command::Undo)?;
        assert_eq!(heading_of(&s)?, "Кухня", "{message}");
    }
    Ok(())
}

#[test]
fn undo_into_a_heading_of_a_closed_project_or_an_archived_heading_conflicts() -> Result {
    let mut s = moved()?;
    let mut p = s.read().snapshot()?.projects.remove(0);
    let open = p.clone();
    p.status = Status::Completed { at: now() };
    s.write(|w| w.update_project(&p))?;
    conflict(
        &mut s,
        Command::Undo,
        "нельзя отменить перенос задачи «Плитка»: раздел «Кухня»: проект «Ремонт» закрыт",
    )?;
    s.write(|w| w.update_project(&open))?;

    let snap = s.read().snapshot()?;
    let mut kitchen = snap
        .headings
        .into_iter()
        .find(|h| h.title == "Кухня")
        .ok_or("нет раздела")?;
    kitchen.archived = true;
    s.write(|w| w.update_heading(&kitchen))?;
    conflict(
        &mut s,
        Command::Undo,
        "нельзя отменить перенос задачи «Плитка»: раздел «Кухня» в архиве",
    )?;
    kitchen.archived = false;
    s.write(|w| w.update_heading(&kitchen))?;
    run(&mut s, Command::Undo)?;
    assert_eq!(heading_of(&s)?, "Кухня");
    Ok(())
}

/// Связь, которую шаг не меняет, сохраняется: отмена `done` в том же
/// разделе проходит, даже когда проект и область в корзине, а раздел в
/// архиве.
#[test]
fn undo_of_done_in_the_same_heading_still_passes() -> Result {
    let mut s = moved()?;
    run(
        &mut s,
        Command::Close(Close {
            tasks: vec!["Плитка".into()],
            cancel: false,
        }),
    )?;
    let (area, project, _) = ids(&s)?;
    let mut bath = s
        .read()
        .snapshot()?
        .headings
        .into_iter()
        .find(|h| h.title == "Ванная")
        .ok_or("нет раздела")?;
    bath.archived = true;
    s.write(|w| {
        w.update_heading(&bath)?;
        w.delete(project, now())?;
        w.delete(area, now())
    })?;
    run(&mut s, Command::Undo)?;
    let task = s.read().snapshot()?.tasks.remove(0);
    assert!(task.status.is_open());
    assert_eq!(heading_of(&s)?, "Ванная");
    Ok(())
}

/// Новая ссылка задачи на проект, чья область в корзине, тоже называет
/// область.
#[test]
fn a_new_project_link_names_the_trashed_area() -> Result {
    let mut s = moved()?;
    let (area, _, _) = ids(&s)?;
    run(
        &mut s,
        Command::AddTask(NewTask {
            text: "Шпатель".into(),
            ..NewTask::default()
        }),
    )?;
    run(
        &mut s,
        Command::MoveTask(MoveTask {
            project: Some("Ремонт".into()),
            ..move_to("Шпатель")
        }),
    )?;
    run(&mut s, Command::Undo)?;
    s.write(|w| w.delete(area, now()))?;
    conflict(
        &mut s,
        Command::Redo,
        "нельзя повторить перенос задачи «Шпатель»: область «Дом» проекта «Ремонт» в корзине",
    )?;
    Ok(())
}

/// На пределе меток: любой перенос без изменений успешен и ничего не
/// пишет, настоящий перенос и `done` – `NoLaterTime`, тоже без записи.
#[test]
fn at_max_only_real_edits_fail() -> Result {
    let mut s = moved()?;
    let mut task = s.read().snapshot()?.tasks.remove(0);
    let id: TaskId = task.id;
    task.sync.touch(Timestamp::MAX);
    s.write(|w| w.update_task(&task))?;
    let before = state(&s)?;
    for m in [
        MoveTask {
            project: Some("Ремонт".into()),
            ..move_to("Плитка")
        },
        MoveTask {
            heading: Some("Ванная".into()),
            ..move_to("Плитка")
        },
        MoveTask {
            when: Some("в любое время".into()),
            ..move_to("Плитка")
        },
        MoveTask {
            no_deadline: true,
            ..move_to("Плитка")
        },
    ] {
        let r = run(&mut s, Command::MoveTask(m.clone()));
        assert!(matches!(r, Ok(Effect::Moved(t)) if t == id), "{m:?}: {r:?}");
        assert_eq!(state(&s)?, before, "{m:?}");
    }
    for c in [
        Command::MoveTask(MoveTask {
            heading: Some("Кухня".into()),
            ..move_to("Плитка")
        }),
        Command::MoveTask(MoveTask {
            when: Some("завтра".into()),
            ..move_to("Плитка")
        }),
        Command::Close(Close {
            tasks: vec!["Плитка".into()],
            cancel: false,
        }),
    ] {
        let r = run(&mut s, c.clone());
        assert!(
            matches!(r, Err(Error::NoLaterTime(t)) if t == Timestamp::MAX),
            "{c:?}: {r:?}"
        );
        assert_eq!(state(&s)?, before, "{c:?}");
    }
    Ok(())
}
