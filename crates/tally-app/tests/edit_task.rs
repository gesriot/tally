use jiff::Timestamp;
use jiff::civil::date;
use tally_app::{
    Clock, Command, EditChecklistItem, EditTag, EditTask, Effect, Error, NewTask, execute,
};
use tally_core::{Start, Task};
use tally_store::Store;
use tally_when::When;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn clock() -> Clock {
    Clock {
        today: date(2026, 11, 20),
        now: Timestamp::UNIX_EPOCH,
    }
}

fn original(store: &mut Store) -> Result<Task> {
    let added = execute(
        store,
        &Command::AddTask(NewTask {
            text: "Отчёт".into(),
            tags: vec!["Работа".into()],
            ..NewTask::default()
        }),
        clock(),
    )?;
    Ok(added
        .snapshot
        .tasks
        .into_iter()
        .next()
        .ok_or("нет задачи")?)
}

fn draft(task: &Task) -> EditTask {
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
            .filter(|item| !item.sync.is_deleted())
            .map(|item| EditChecklistItem {
                id: Some(item.id),
                title: item.title.clone(),
                done: item.done,
            })
            .collect(),
        tags: task.tags.iter().copied().map(EditTag::Existing).collect(),
    }
}

#[test]
fn card_is_one_history_step_and_checklist_tombstones_do_not_block_earlier_undo() -> Result {
    let mut store = Store::open_in_memory()?;
    let old = original(&mut store)?;
    let mut edit = draft(&old);
    edit.title = "Отчёт за неделю".into();
    edit.notes = "Черновик\nПроверить цифры".into();
    edit.when = Some(When::On {
        date: clock().today,
        evening: true,
    });
    edit.deadline = Some(date(2026, 11, 23));
    edit.tags.push(EditTag::Named("финансы".into()));
    edit.tags.push(EditTag::Named("Финансы".into()));
    edit.checklist.push(EditChecklistItem {
        id: None,
        title: "Собрать цифры".into(),
        done: true,
    });
    let result = execute(&mut store, &Command::EditTask(edit), clock())?;
    assert_eq!(result.effect, Effect::Edited(old.id));
    assert_eq!(store.read().history()?.len(), 2);
    let Some(saved) = result.snapshot.tasks.first() else {
        return Err("нет сохранённой задачи".into());
    };
    assert_eq!(saved.title, "Отчёт за неделю");
    assert_eq!(saved.notes, "Черновик\nПроверить цифры");
    assert_eq!(saved.date, Some(clock().today));
    assert!(saved.evening);
    assert_eq!(saved.deadline, Some(date(2026, 11, 23)));
    assert_eq!(saved.tags.len(), 2);
    assert_eq!(saved.checklist[0].title, "Собрать цифры");
    assert!(saved.checklist[0].done);
    let check_id = saved.checklist[0].id;

    let undone = execute(&mut store, &Command::Undo, clock())?;
    let back = &undone.snapshot.tasks[0];
    assert_eq!(back.title, old.title);
    assert_eq!(back.start, Start::Inbox);
    assert!(back.checklist.iter().all(|item| item.sync.is_deleted()));
    assert_eq!(undone.snapshot.tags.len(), 1);
    assert!(
        execute(&mut store, &Command::Undo, clock())?
            .snapshot
            .tasks
            .is_empty()
    );
    execute(&mut store, &Command::Redo, clock())?;
    let redone = execute(&mut store, &Command::Redo, clock())?;
    let task = &redone.snapshot.tasks[0];
    assert_eq!(task.checklist[0].id, check_id);
    assert!(task.checklist[0].done);
    assert!(!task.checklist[0].sync.is_deleted());
    assert_eq!(task.notes, saved.notes);

    let mut cleared = draft(task);
    cleared.checklist.clear();
    cleared.tags.clear();
    cleared.when = Some(When::Anytime);
    cleared.deadline = None;
    let cleared = execute(&mut store, &Command::EditTask(cleared), clock())?;
    let task = &cleared.snapshot.tasks[0];
    assert!(task.checklist[0].sync.is_deleted());
    assert!(task.tags.is_empty());
    assert_eq!(cleared.snapshot.tags.len(), 2);
    assert_eq!(
        (task.start, task.date, task.evening, task.deadline),
        (Start::Anytime, None, false, None)
    );
    let restored = execute(&mut store, &Command::Undo, clock())?;
    let task = &restored.snapshot.tasks[0];
    assert_eq!(task.checklist[0].id, check_id);
    assert!(!task.checklist[0].sync.is_deleted());
    assert_eq!(task.tags.len(), 2);
    assert!(
        execute(&mut store, &Command::Redo, clock())?.snapshot.tasks[0].checklist[0]
            .sync
            .is_deleted()
    );
    Ok(())
}

#[test]
fn undo_and_redo_of_notes_keep_a_purged_checklist_item_absent() -> Result {
    let mut store = Store::open_in_memory()?;
    let original = original(&mut store)?;
    let mut add_item = draft(&original);
    add_item.checklist.push(EditChecklistItem {
        id: None,
        title: "Архивный пункт".into(),
        done: false,
    });
    let added = execute(&mut store, &Command::EditTask(add_item), clock())?;
    let mut remove_item = draft(&added.snapshot.tasks[0]);
    remove_item.checklist.clear();
    let removed = execute(&mut store, &Command::EditTask(remove_item), clock())?;
    let mut change_notes = draft(&removed.snapshot.tasks[0]);
    change_notes.notes = "Новая заметка".into();
    execute(&mut store, &Command::EditTask(change_notes), clock())?;
    assert_eq!(
        store.write(|writer| writer.empty_trash())?.checklist_items,
        1
    );

    for (command, expected_notes) in [
        (Command::Undo, ""),
        (Command::Redo, "Новая заметка"),
        (Command::Undo, ""),
    ] {
        let result = execute(&mut store, &command, clock())?;
        let task = &result.snapshot.tasks[0];
        assert!(task.checklist.is_empty());
        assert_eq!(task.notes, expected_notes);
    }
    Ok(())
}

#[test]
fn rejected_or_stale_cards_leave_the_database_and_history_unchanged() -> Result {
    let mut store = Store::open_in_memory()?;
    let task = original(&mut store)?;
    let mut invalid = draft(&task);
    invalid.notes = "Черновик".into();
    invalid.checklist.push(EditChecklistItem {
        id: None,
        title: " ".into(),
        done: false,
    });
    let snapshot = store.read().snapshot()?;
    let history = store.read().history()?;
    assert!(matches!(
        execute(&mut store, &Command::EditTask(invalid), clock()),
        Err(Error::Rejected(_))
    ));
    assert_eq!(store.read().snapshot()?, snapshot);
    assert_eq!(store.read().history()?, history);

    let unchanged = execute(&mut store, &Command::EditTask(draft(&task)), clock())?;
    assert_eq!(unchanged.seq, None);
    assert_eq!(unchanged.snapshot, snapshot);

    let mut change = draft(&task);
    change.notes = "С консоли".into();
    execute(&mut store, &Command::EditTask(change), clock())?;
    let latest = store.read().snapshot()?;
    let history = store.read().history()?;
    assert!(matches!(
        execute(&mut store, &Command::EditTask(draft(&task)), clock()),
        Err(Error::Conflict(_))
    ));
    assert_eq!(store.read().snapshot()?, latest);
    assert_eq!(store.read().history()?, history);
    Ok(())
}
