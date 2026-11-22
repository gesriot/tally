//! Календарные повторы и повторы после выполнения, повтор проекта, корзина и
//! очистка.

use jiff::{
    Timestamp,
    civil::{Date, date},
};
use tally_app::{
    Clock, Close, Command, Done, EditChecklistItem, EditTask, Effect, Error, NewArea, NewHeading,
    NewProject, NewTask, Repeat, execute, snapshot_for_day, trash_preview,
};
use tally_core::{RepeatFrom, RepeatRule, RepeatUnit, Task};
use tally_store::{RecordId, Store};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

const JAN: Date = date(2027, 1, 31);

fn clock(today: Date) -> Clock {
    Clock {
        today,
        now: Timestamp::UNIX_EPOCH,
    }
}

fn run(store: &mut Store, command: Command) -> Result<Done> {
    Ok(execute(store, &command, clock(JAN))?)
}

fn rule(unit: RepeatUnit, from: RepeatFrom) -> Repeat {
    Repeat {
        rule: RepeatRule { interval: 1, unit },
        from,
    }
}

fn add(store: &mut Store, task: NewTask) -> Result<Task> {
    let done = run(store, Command::AddTask(task))?;
    let Effect::Added(id) = done.effect else {
        return Err("ожидалась новая задача".into());
    };
    Ok(done
        .snapshot
        .tasks
        .into_iter()
        .find(|t| t.id == id)
        .ok_or("нет задачи")?)
}

fn edit(task: &Task) -> EditTask {
    EditTask {
        task: task.id,
        expected_updated_at: task.sync.updated_at,
        title: task.title.clone(),
        notes: task.notes.clone(),
        when: None,
        deadline: task.deadline,
        checklist: task
            .checklist
            .iter()
            .map(|i| EditChecklistItem {
                id: Some(i.id),
                title: i.title.clone(),
                done: i.done,
            })
            .collect(),
        tags: Vec::new(),
    }
}

#[test]
fn calendar_repeat_carries_the_month_day_through_february_and_undo() -> Result {
    let mut store = Store::open_in_memory()?;
    let first = add(
        &mut store,
        NewTask {
            text: "Ежемесячная проверка".into(),
            when: Some("сегодня вечером".into()),
            deadline: Some("завтра".into()),
            ..NewTask::default()
        },
    )?;
    let mut card = edit(&first);
    card.notes = "Сверить выписку".into();
    card.checklist.push(EditChecklistItem {
        id: None,
        title: "Счёт".into(),
        done: true,
    });
    let configured = run(
        &mut store,
        Command::EditTaskRepeat {
            edit: card,
            repeat: Some(rule(RepeatUnit::Month, RepeatFrom::Date)),
        },
    )?;
    let seq_before = configured.seq;
    let closed = run(&mut store, Command::CloseTask(first.id))?;
    assert_eq!(closed.seq, seq_before.map(|seq| seq + 1));
    let original = closed
        .snapshot
        .tasks
        .iter()
        .find(|t| t.id == first.id)
        .ok_or("нет исходной")?;
    let feb = closed
        .snapshot
        .tasks
        .iter()
        .find(|t| t.status.is_open())
        .ok_or("нет повтора")?;
    assert_eq!(feb.date, Some(date(2027, 2, 28)));
    assert_eq!(feb.deadline, Some(date(2027, 3, 1)));
    assert_eq!(feb.notes, "Сверить выписку");
    assert!(feb.evening);
    assert!(original.checklist[0].done);
    assert!(!feb.checklist[0].done);
    assert_ne!(original.checklist[0].id, feb.checklist[0].id);

    let undone = run(&mut store, Command::UndoStep(closed.seq.ok_or("нет шага")?))?;
    assert_eq!(undone.snapshot.tasks.len(), 1);
    assert!(undone.snapshot.tasks[0].status.is_open());
    let redone = execute(&mut store, &Command::Redo, clock(date(2027, 4, 1)))?;
    assert!(
        redone
            .snapshot
            .tasks
            .iter()
            .any(|t| t.id == feb.id && t.date == feb.date)
    );

    let original = redone
        .snapshot
        .tasks
        .iter()
        .find(|t| t.id == first.id)
        .ok_or("нет исходной")?;
    run(&mut store, Command::ReopenTask(edit(original)))?;
    let closed_again = run(&mut store, Command::CloseTask(first.id))?;
    assert_eq!(closed_again.snapshot.tasks.len(), 2);

    let march = execute(
        &mut store,
        &Command::CloseTask(feb.id),
        clock(date(2027, 2, 28)),
    )?;
    assert!(
        march
            .snapshot
            .tasks
            .iter()
            .any(|t| t.status.is_open() && t.date == Some(date(2027, 3, 31)))
    );
    Ok(())
}

#[test]
fn calendar_repeat_is_created_on_its_date_while_the_previous_copy_is_open() -> Result {
    let mut store = Store::open_in_memory()?;
    let first = add(
        &mut store,
        NewTask {
            text: "Сверить платёж".into(),
            when: Some("сегодня".into()),
            ..NewTask::default()
        },
    )?;
    run(
        &mut store,
        Command::EditTaskRepeat {
            edit: edit(&first),
            repeat: Some(rule(RepeatUnit::Month, RepeatFrom::Date)),
        },
    )?;
    assert_eq!(
        snapshot_for_day(&mut store, clock(date(2027, 2, 27)))?
            .tasks
            .len(),
        1
    );
    let due = snapshot_for_day(&mut store, clock(date(2027, 2, 28)))?;
    let next = due
        .tasks
        .iter()
        .find(|t| t.id != first.id)
        .ok_or("нет календарного экземпляра")?;
    assert_eq!(next.date, Some(date(2027, 2, 28)));
    assert!(due.tasks.iter().all(|t| t.status.is_open()));
    assert_eq!(snapshot_for_day(&mut store, clock(date(2027, 2, 28)))?, due);
    let later = snapshot_for_day(&mut store, clock(date(2027, 5, 7)))?;
    assert_eq!(later.tasks.len(), 3);
    assert!(
        later
            .tasks
            .iter()
            .any(|t| t.date == Some(date(2027, 4, 30)))
    );
    Ok(())
}

#[test]
fn completion_based_repeat_uses_the_actual_day_and_cancel_does_not_advance() -> Result {
    let mut store = Store::open_in_memory()?;
    let task = add(
        &mut store,
        NewTask {
            text: "Полить растения".into(),
            ..NewTask::default()
        },
    )?;
    run(
        &mut store,
        Command::EditTaskRepeat {
            edit: edit(&task),
            repeat: Some(rule(RepeatUnit::Week, RepeatFrom::Completion)),
        },
    )?;
    let canceled = run(
        &mut store,
        Command::Close(Close {
            tasks: vec![task.id.to_string()],
            cancel: true,
        }),
    )?;
    assert_eq!(canceled.snapshot.tasks.len(), 1);
    assert!(!canceled.snapshot.tasks[0].repeat_advanced);
    run(&mut store, Command::Undo)?;
    let completed = execute(
        &mut store,
        &Command::CloseTask(task.id),
        clock(date(2027, 2, 10)),
    )?;
    let next = completed
        .snapshot
        .tasks
        .iter()
        .find(|t| t.status.is_open())
        .ok_or("нет повтора")?;
    assert_eq!(next.date, Some(date(2027, 2, 17)));
    Ok(())
}

#[test]
fn repeating_project_copies_live_template_and_preserves_closed_project_in_history() -> Result {
    let mut store = Store::open_in_memory()?;
    let added = run(
        &mut store,
        Command::AddProject(NewProject {
            title: "Месячное закрытие".into(),
            area: None,
        }),
    )?;
    let old = &added.snapshot.projects[0];
    let config = run(
        &mut store,
        Command::SetProjectRepeat {
            project: old.id,
            expected_updated_at: old.sync.updated_at,
            repeat: Some(rule(RepeatUnit::Month, RepeatFrom::Date)),
        },
    )?;
    run(
        &mut store,
        Command::AddHeading(NewHeading {
            title: "Выписка".into(),
            project: old.title.clone(),
        }),
    )?;
    let task = add(
        &mut store,
        NewTask {
            text: "Проверить начисления".into(),
            project: Some(old.title.clone()),
            heading: Some("Выписка".into()),
            when: Some("завтра".into()),
            ..NewTask::default()
        },
    )?;
    run(&mut store, Command::CloseTask(task.id))?;
    let close = Command::CloseProject {
        project: old.id,
        expected_updated_at: config.snapshot.projects[0].sync.updated_at,
        cancel: false,
    };
    let done = run(&mut store, close)?;
    let new = done
        .snapshot
        .projects
        .iter()
        .find(|p| p.id != old.id)
        .ok_or("нет нового проекта")?;
    assert_eq!(new.date, Some(date(2027, 2, 28)));
    let new_task = done
        .snapshot
        .tasks
        .iter()
        .find(|t| t.project == Some(new.id))
        .ok_or("нет копии задачи")?;
    let section = done
        .snapshot
        .headings
        .iter()
        .find(|h| h.project == new.id)
        .ok_or("нет копии раздела")?;
    assert_eq!(new_task.heading, Some(section.id));
    assert_eq!(new_task.date, Some(date(2027, 3, 1)));
    assert!(new_task.status.is_open());
    assert_ne!(new_task.id, task.id);
    assert!(
        !done
            .snapshot
            .tasks
            .iter()
            .find(|t| t.id == task.id)
            .ok_or("нет старой задачи")?
            .status
            .is_open()
    );
    let undo = run(&mut store, Command::Undo)?;
    assert_eq!(undo.snapshot.projects.len(), 1);
    assert_eq!(undo.snapshot.tasks.len(), 1);
    assert_eq!(undo.snapshot.headings.len(), 1);
    assert!(undo.snapshot.projects[0].status.is_open());
    let redo = run(&mut store, Command::Redo)?;
    assert!(
        redo.snapshot
            .tasks
            .iter()
            .any(|t| t.id == new_task.id && t.project == Some(new.id))
    );
    Ok(())
}

#[test]
fn purge_keeps_unrelated_undo_available_after_a_mixed_action() -> Result {
    let mut store = Store::open_in_memory()?;
    let unrelated = add(
        &mut store,
        NewTask {
            text: "Независимая".into(),
            ..NewTask::default()
        },
    )?;
    let removed = add(
        &mut store,
        NewTask {
            text: "Удалённая".into(),
            ..NewTask::default()
        },
    )?;
    let kept = add(
        &mut store,
        NewTask {
            text: "Останется".into(),
            ..NewTask::default()
        },
    )?;
    let closed = run(
        &mut store,
        Command::Close(Close {
            tasks: vec![removed.id.to_string(), kept.id.to_string()],
            cancel: false,
        }),
    )?;
    let removed = closed
        .snapshot
        .tasks
        .iter()
        .find(|t| t.id == removed.id)
        .ok_or("нет задачи")?;
    let trashed = run(
        &mut store,
        Command::SetTrash {
            record: RecordId::Task(removed.id),
            expected_updated_at: removed.sync.updated_at,
            trashed: true,
        },
    )?;
    run(
        &mut store,
        Command::EmptyTrash {
            preview: trash_preview(&trashed.snapshot),
        },
    )?;

    let undone = run(&mut store, Command::Undo)?;
    assert!(
        undone
            .snapshot
            .tasks
            .iter()
            .all(|t| t.id != unrelated.id && t.id != removed.id)
    );
    assert!(
        undone
            .snapshot
            .tasks
            .iter()
            .any(|t| t.id == kept.id && !t.status.is_open())
    );
    Ok(())
}

#[test]
fn trash_restores_hierarchy_and_stale_purge_cannot_remove_newly_deleted_rows() -> Result {
    let mut store = Store::open_in_memory()?;
    let created = run(
        &mut store,
        Command::AddArea(NewArea {
            title: "Работа".into(),
        }),
    )?;
    let area = &created.snapshot.areas[0];
    let child = add(
        &mut store,
        NewTask {
            text: "Рабочие записи".into(),
            area: Some(area.title.clone()),
            ..NewTask::default()
        },
    )?;
    let area_id = RecordId::Area(area.id);
    let child_id = RecordId::Task(child.id);
    let moved = run(
        &mut store,
        Command::SetTrash {
            record: area_id,
            expected_updated_at: area.sync.updated_at,
            trashed: true,
        },
    )?;
    assert_eq!(
        trash_preview(&moved.snapshot).records,
        vec![
            (area_id, moved.snapshot.areas[0].sync.updated_at),
            (child_id, child.sync.updated_at),
        ]
    );
    assert_eq!(moved.snapshot.tasks[0].sync, child.sync);
    let restored = run(
        &mut store,
        Command::SetTrash {
            record: area_id,
            expected_updated_at: moved.snapshot.areas[0].sync.updated_at,
            trashed: false,
        },
    )?;
    assert!(trash_preview(&restored.snapshot).is_empty());
    assert!(
        run(&mut store, Command::Undo)?.snapshot.areas[0]
            .sync
            .is_deleted()
    );
    let stale = trash_preview(&restored.snapshot);
    assert!(matches!(
        execute(
            &mut store,
            &Command::EmptyTrash { preview: stale },
            clock(JAN)
        ),
        Err(Error::Conflict(_)),
    ));
    let unrelated = add(
        &mut store,
        NewTask {
            text: "Прочитать письмо".into(),
            ..NewTask::default()
        },
    )?;
    let preview = trash_preview(&store.read().snapshot()?);
    let emptied = run(&mut store, Command::EmptyTrash { preview })?;
    assert!(emptied.seq.is_none());
    assert!(emptied.snapshot.areas.is_empty());
    assert_eq!(emptied.snapshot.tasks, vec![unrelated]);
    assert!(
        store
            .read()
            .history()?
            .iter()
            .all(|e| !e.changes.contains(&child.id.to_string()))
    );
    let after_undo = run(&mut store, Command::Undo)?;
    assert!(after_undo.snapshot.tasks.is_empty());
    assert!(after_undo.snapshot.areas.is_empty());
    assert!(store.read().task(child.id)?.is_none());
    assert!(
        !trash_preview(&after_undo.snapshot)
            .records
            .iter()
            .any(|(id, _)| *id == child_id)
    );
    Ok(())
}
