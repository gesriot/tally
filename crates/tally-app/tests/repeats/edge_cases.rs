//! Граничные случаи: подтверждение очистки корзины, предел календаря,
//! повторная загрузка в тот же день, правило повтора без даты.

use jiff::Timestamp;
use jiff::civil::{Date, date};
use tally_app::{
    Clock, Command, Done, EditChecklistItem, EditTask, Effect, Error, MoveTask, NewProject,
    NewTask, Repeat, execute, snapshot_for_day, trash_preview,
};
use tally_core::{RepeatFrom, RepeatRule, RepeatUnit, Task};
use tally_store::{RecordId, Store};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn clock(day: i8) -> Clock {
    Clock {
        today: date(2027, 1, day),
        now: Timestamp::UNIX_EPOCH,
    }
}

fn run(db: &mut Store, command: Command) -> Result<Done> {
    Ok(execute(db, &command, clock(1))?)
}

fn add(db: &mut Store, title: &str, project: Option<&str>) -> Result<Task> {
    let done = run(
        db,
        Command::AddTask(NewTask {
            text: title.into(),
            project: project.map(str::to_owned),
            when: Some("сегодня".into()),
            ..NewTask::default()
        }),
    )?;
    let Effect::Added(id) = done.effect else {
        return Err("ожидалось создание задачи".into());
    };
    Ok(db.read().task(id)?.ok_or("нет задачи")?)
}

fn edit(t: &Task) -> EditTask {
    EditTask {
        task: t.id,
        expected_updated_at: t.sync.updated_at,
        title: t.title.clone(),
        notes: t.notes.clone(),
        when: None,
        deadline: t.deadline,
        checklist: t
            .checklist
            .iter()
            .filter(|item| !item.sync.is_deleted())
            .map(|item| EditChecklistItem {
                id: Some(item.id),
                title: item.title.clone(),
                done: item.done,
            })
            .collect(),
        tags: Vec::new(),
    }
}

fn calendar(unit: RepeatUnit) -> Repeat {
    Repeat {
        rule: RepeatRule { interval: 1, unit },
        from: RepeatFrom::Date,
    }
}

fn configure_daily(db: &mut Store, edit: EditTask) -> Result<Done> {
    run(
        db,
        Command::EditTaskRepeat {
            edit,
            repeat: Some(calendar(RepeatUnit::Day)),
        },
    )
}

fn prepare_trash_and_live_task(db: &mut Store) -> Result<Task> {
    let discarded = add(db, "Черновик", None)?;
    run(
        db,
        Command::SetTrash {
            record: RecordId::Task(discarded.id),
            expected_updated_at: discarded.sync.updated_at,
            trashed: true,
        },
    )?;
    let live = add(db, "Отчёт", None)?;
    let mut card = edit(&live);
    card.checklist.push(EditChecklistItem {
        id: None,
        title: "Сверить сумму".into(),
        done: false,
    });
    run(db, Command::EditTask(card))?;
    Ok(db.read().task(live.id)?.ok_or("нет живой задачи")?)
}

fn remove_item(db: &mut Store, live: &Task) -> Result<Task> {
    let mut card = edit(live);
    card.checklist.clear();
    run(db, Command::EditTask(card))?;
    Ok(db.read().task(live.id)?.ok_or("нет живой задачи")?)
}

#[test]
fn stale_confirmation_cannot_purge_a_new_checklist_tombstone() -> Result {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let mut window = Store::open(&path)?;
    let live = prepare_trash_and_live_task(&mut window)?;
    let preview = trash_preview(&window.read().snapshot()?);
    let mut other = Store::open(&path)?;
    remove_item(&mut other, &live)?;
    let before = other.read().snapshot()?;
    let history = other.read().history()?;

    let result = execute(&mut window, &Command::EmptyTrash { preview }, clock(1));
    assert!(matches!(result, Err(Error::Conflict(_))));
    assert_eq!(other.read().snapshot()?, before);
    assert_eq!(other.read().history()?, history);
    Ok(())
}

#[test]
fn emptying_through_the_app_keeps_the_notes_step_on_a_surviving_task() -> Result {
    let mut db = Store::open_in_memory()?;
    let live = prepare_trash_and_live_task(&mut db)?;
    let updated = remove_item(&mut db, &live)?;
    let mut card = edit(&updated);
    card.notes = "Согласовано".into();
    let note_step = run(&mut db, Command::EditTask(card))?
        .seq
        .ok_or("нет шага")?;
    let preview = trash_preview(&db.read().snapshot()?);
    run(&mut db, Command::EmptyTrash { preview })?;

    for (command, expected) in [(Command::Undo, ""), (Command::Redo, "Согласовано")] {
        assert_eq!(run(&mut db, command)?.seq, Some(note_step));
        let task = db.read().task(live.id)?.ok_or("исчезла живая задача")?;
        assert_eq!(task.notes, expected);
        assert!(task.checklist.is_empty());
    }
    Ok(())
}

#[test]
fn a_deadline_at_the_calendar_limit_cannot_block_opening_the_database() -> Result {
    let mut db = Store::open_in_memory()?;
    let healthy = add(&mut db, "Обычный повтор", None)?;
    configure_daily(&mut db, edit(&healthy))?;
    let problem = add(&mut db, "Дальний срок", None)?;
    let mut card = edit(&problem);
    card.deadline = Some(Date::MAX);
    // Допустим явный отказ при настройке. Уже принятая запись должна
    // оставаться доступной после смены дня вместе с остальными задачами.
    match configure_daily(&mut db, card) {
        Ok(_) => {}
        Err(error) if matches!(error.downcast_ref::<Error>(), Some(Error::Rejected(_))) => {}
        Err(error) => return Err(error),
    }
    let snapshot = snapshot_for_day(&mut db, clock(2))?;
    assert!(snapshot.tasks.iter().any(|t| t.id == problem.id));
    assert!(snapshot.tasks.iter().any(|t| {
        t.title == healthy.title && t.id != healthy.id && t.date == Some(clock(2).today)
    }));
    Ok(())
}

#[test]
fn a_second_load_on_the_same_day_does_not_advance_a_new_projects_child() -> Result {
    let mut db = Store::open_in_memory()?;
    let added = run(
        &mut db,
        Command::AddProject(NewProject {
            title: "Неделя".into(),
            area: None,
        }),
    )?;
    let project = added.snapshot.projects.first().ok_or("нет проекта")?;
    run(
        &mut db,
        Command::SetProjectRepeat {
            project: project.id,
            expected_updated_at: project.sync.updated_at,
            repeat: Some(calendar(RepeatUnit::Week)),
        },
    )?;
    let child = add(&mut db, "Сверка", Some("Неделя"))?;
    configure_daily(&mut db, edit(&child))?;
    let first = snapshot_for_day(&mut db, clock(10))?;
    assert!(
        first
            .projects
            .iter()
            .any(|p| p.date == Some(clock(8).today))
    );
    let history = db.read().history()?;
    let again = snapshot_for_day(&mut db, clock(10))?;
    assert_eq!(first, again);
    assert_eq!(db.read().history()?, history);
    Ok(())
}

#[test]
fn clearing_the_start_cannot_leave_a_calendar_rule_that_never_fires() -> Result {
    let mut db = Store::open_in_memory()?;
    let task = add(&mut db, "Ежедневная", None)?;
    configure_daily(&mut db, edit(&task))?;
    let result = execute(
        &mut db,
        &Command::MoveTask(MoveTask {
            task: task.id.to_string(),
            when: Some("в любое время".into()),
            ..Default::default()
        }),
        clock(1),
    );
    let moved = match result {
        Ok(done) => done,
        Err(Error::Rejected(_)) => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let current = moved.snapshot.tasks.first().ok_or("нет задачи")?;
    assert!(current.date.is_none());
    if current.repeat_rule.is_some() && current.repeat_from == RepeatFrom::Date {
        let next_day = snapshot_for_day(&mut db, clock(2))?;
        assert!(next_day.tasks.iter().any(|t| {
            t.id != task.id && t.title == task.title && t.date == Some(clock(2).today)
        }));
    }
    Ok(())
}
