//! Отмена заметок не возвращает стёртый пункт чек-листа; восстановление после
//! конфликта и переход к задаче в «Планах».

mod common;

use common::{Result, TODAY, clock, find, run, task};
use slint::platform::Key;
use slint::{ComponentHandle, LogicalSize, SharedString};
use tally_app::{App, Command, EditChecklistItem, EditTag, EditTask, Error, MoveTask, execute};
use tally_core::{Task, TaskId};
use tally_store::Store;
use tally_ui::backend::{Calendar, Inline};
use tally_ui::bind::Bound;
use tally_ui::ui::AppWindow;

fn draft(task: &Task) -> EditTask {
    EditTask {
        task: task.id,
        expected_updated_at: task.sync.updated_at,
        title: task.title.clone(),
        notes: task.notes.clone(),
        when: None,
        deadline: task.deadline,
        tags: task.tags.iter().copied().map(EditTag::Existing).collect(),
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
    }
}

fn edit(store: &mut Store, id: TaskId, change: impl FnOnce(&mut EditTask)) -> Result {
    let task = store.read().task(id)?.ok_or("нет задачи")?;
    let mut next = draft(&task);
    change(&mut next);
    run(store, Command::EditTask(next))
}

#[test]
fn undoing_notes_does_not_reinsert_a_purged_checklist_item() -> Result {
    let mut store = Store::open_in_memory()?;
    run(&mut store, Command::AddTask(task("Отчёт")))?;
    let id = find(&store.read().snapshot()?, "Отчёт")?.id;
    edit(&mut store, id, |e| {
        e.checklist.push(EditChecklistItem {
            id: None,
            title: "Устаревший пункт".into(),
            done: false,
        })
    })?;
    edit(&mut store, id, |e| e.checklist.clear())?;
    edit(&mut store, id, |e| e.notes = "Новая заметка".into())?;
    assert_eq!(store.write(|w| w.empty_trash())?.checklist_items, 1);
    let before = store.read().snapshot()?;
    let history = store.read().history()?;
    match execute(&mut store, &Command::Undo, clock()) {
        Ok(_) => {
            let saved = store.read().task(id)?.ok_or("нет задачи")?;
            assert!(saved.notes.is_empty());
            assert!(
                saved.checklist.is_empty(),
                "очищенный пункт снова записан в БД"
            );
        }
        Err(Error::Conflict(_)) => {
            assert_eq!(store.read().snapshot()?, before);
            assert_eq!(store.read().history()?, history);
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

#[test]
fn conflict_recovery_and_navigation_to_the_task_after_two_upcoming_rows() -> Result {
    i_slint_backend_testing::init_integration_test_with_mock_time();
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let mut console = Store::open(&path)?;
    run(&mut console, Command::AddTask(task("Отчёт сегодня")))?;
    let id = find(&console.read().snapshot()?, "Отчёт")?.id;
    let mut first = task("Подготовка");
    first.when = Some("завтра".into());
    first.deadline = Some("послезавтра".into());
    run(&mut console, Command::AddTask(first))?;
    let mut last = task("Отправить");
    last.when = Some("через 3 дня".into());
    run(&mut console, Command::AddTask(last))?;

    let window = AppWindow::new()?;
    window.window().set_size(LogicalSize::new(1000.0, 760.0));
    let calendar = Calendar {
        fixed_today: Some(TODAY),
    };
    let backend = Inline::new(App::new(Store::open(&path)?), calendar);
    let _bound = Bound::attach(&window, Box::new(backend), calendar);
    window.show()?;
    window.invoke_edit_task(id.to_string().into());
    window.set_edit_notes("Мой черновик".into());
    run(
        &mut console,
        Command::MoveTask(MoveTask {
            task: id.to_string(),
            deadline: Some("сегодня".into()),
            ..MoveTask::default()
        }),
    )?;
    let history = console.read().history()?;
    // Намеренно без F5: пользователь узнаёт о втором клиенте при сохранении.
    window.invoke_edit_finish(true);
    assert!(window.get_edit_error().contains("изменилась"));
    assert_eq!(window.get_edit_notes(), "Мой черновик");
    assert_eq!(console.read().history()?, history);
    window.invoke_edit_finish(false);
    window.invoke_edit_task(id.to_string().into());
    window.set_edit_notes("Согласованная заметка".into());
    window.invoke_edit_finish(true);
    let saved = console.read().task(id)?.ok_or("нет задачи")?;
    let recovered = saved.notes == "Согласованная заметка" && saved.deadline == Some(TODAY);
    if !window.get_edit_key().is_empty() {
        window.invoke_edit_finish(false);
    }

    window.invoke_select("upcoming".into());
    for _ in 0..6 {
        window.invoke_key(
            SharedString::from(Key::DownArrow),
            false,
            false,
            false,
            false,
        );
    }
    window.invoke_key(SharedString::from(Key::Return), false, false, false, false);
    let reached_last = window.get_edit_title() == "Отправить";
    assert_eq!(
        (recovered, reached_last),
        (true, true),
        "сохранение после повторного открытия / переход за вторую строку «Подготовки»"
    );
    Ok(())
}
