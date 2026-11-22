use jiff::Timestamp;
use jiff::civil::date;
use tally_app::{Clock, Command, Error, NewArea, NewProject, NewTask, execute};
use tally_core::Status;
use tally_core::lists::Item;
use tally_store::{RecordId, Store};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn clock() -> Clock {
    Clock {
        today: date(2026, 11, 20),
        now: Timestamp::UNIX_EPOCH,
    }
}

#[test]
fn closing_project_changes_only_its_status_and_is_one_history_step() -> Result {
    let mut store = Store::open_in_memory()?;
    let clock = clock();
    let done = execute(
        &mut store,
        &Command::AddArea(NewArea {
            title: "Дом".into(),
        }),
        clock,
    )?;
    let home = done.snapshot.areas.first().ok_or("нет области")?.id;
    let done = execute(
        &mut store,
        &Command::AddProjectInArea {
            title: "Ремонт".into(),
            area: home,
        },
        clock,
    )?;
    let repair = done.snapshot.projects.first().ok_or("нет проекта")?.id;
    execute(
        &mut store,
        &Command::AddProjectSection {
            project: repair,
            title: "Кухня".into(),
        },
        clock,
    )?;
    let before = execute(
        &mut store,
        &Command::AddTask(NewTask {
            text: "Купить плитку".into(),
            project: Some("Ремонт".into()),
            heading: Some("Кухня".into()),
            when: Some("сегодня".into()),
            ..NewTask::default()
        }),
        clock,
    )?
    .snapshot;

    for (cancel, expected) in [
        (false, Status::Completed { at: clock.now }),
        (true, Status::Canceled { at: clock.now }),
    ] {
        let current = store.read().project(repair)?.ok_or("нет проекта")?;
        let command = Command::CloseProject {
            project: repair,
            expected_updated_at: current.sync.updated_at,
            cancel,
        };
        let done = execute(&mut store, &command, clock)?;
        let seq = done.seq.ok_or("нет шага закрытия")?;
        assert_eq!(done.snapshot.projects[0].status, expected);
        assert_eq!(done.snapshot.tasks, before.tasks);
        assert_eq!(done.snapshot.headings, before.headings);
        assert_eq!(done.snapshot.areas, before.areas);
        let lists = done.snapshot.lists(clock.today);
        assert!(lists.today.day.is_empty());
        assert!(matches!(lists.logbook.as_slice(), [Item::Project(p)] if p.id == repair));

        assert!(matches!(
            execute(&mut store, &command, clock),
            Err(Error::Conflict(_))
        ));
        let undone = execute(&mut store, &Command::Undo, clock)?;
        assert_eq!(undone.seq, Some(seq));
        assert_eq!(undone.snapshot.projects[0].status, Status::Open);
        assert_eq!(undone.snapshot.tasks, before.tasks);
        assert!(matches!(
            undone.snapshot.lists(clock.today).today.day.as_slice(),
            [Item::Task(_)]
        ));
        let redone = execute(&mut store, &Command::Redo, clock)?;
        assert_eq!(redone.seq, Some(seq));
        assert_eq!(redone.snapshot.projects[0].status, expected);
        assert_eq!(redone.snapshot.tasks, before.tasks);
        execute(&mut store, &Command::Undo, clock)?;
    }
    Ok(())
}

#[test]
fn stale_targets_never_resolve_to_replacements_with_the_same_title() -> Result {
    let mut store = Store::open_in_memory()?;
    let clock = clock();
    let create_area = Command::AddArea(NewArea {
        title: "Дом".into(),
    });
    let done = execute(&mut store, &create_area, clock)?;
    let home = done.snapshot.areas.first().ok_or("нет области")?.id;
    let done = execute(
        &mut store,
        &Command::AddProjectInArea {
            title: "Ремонт".into(),
            area: home,
        },
        clock,
    )?;
    let project = done.snapshot.projects.first().ok_or("нет проекта")?;
    let repair = project.id;
    store.write(|w| w.delete(RecordId::Area(home), clock.now))?;
    execute(&mut store, &create_area, clock)?;
    let before = execute(
        &mut store,
        &Command::AddProject(NewProject {
            title: "Ремонт".into(),
            area: Some("Дом".into()),
        }),
        clock,
    )?
    .snapshot;
    for command in [
        Command::AddProjectInArea {
            title: "Пристройка".into(),
            area: home,
        },
        Command::AddProjectSection {
            title: "Кухня".into(),
            project: repair,
        },
        Command::CloseProject {
            project: repair,
            expected_updated_at: project.sync.updated_at,
            cancel: true,
        },
    ] {
        assert!(matches!(
            execute(&mut store, &command, clock),
            Err(Error::Conflict(_))
        ));
        assert_eq!(store.read().snapshot()?, before);
    }
    assert_eq!(
        execute(&mut store, &Command::Undo, clock)?
            .snapshot
            .projects
            .len(),
        1
    );
    Ok(())
}
