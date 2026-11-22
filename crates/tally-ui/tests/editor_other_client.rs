//! Отмена создания в другом клиенте оставляет только черновик; метка пункта
//! чек-листа после отмены стирания.

mod common;

use common::{Result, TODAY, clock, find, run, task};
use slint::{ComponentHandle, Model};
use tally_app::{App, Clock, Command, EditChecklistItem, EditTask, execute};
use tally_core::Task;
use tally_store::Store;
use tally_ui::backend::{Calendar, Inline};
use tally_ui::bind::Bound;
use tally_ui::ui::AppWindow;

#[test]
fn undo_of_creation_in_another_client_keeps_only_the_open_draft() -> Result {
    i_slint_backend_testing::init_integration_test_with_mock_time();
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let mut console = Store::open(&path)?;
    let mut added = task("Только что создана");
    added.when = Some("сегодня".into());
    run(&mut console, Command::AddTask(added))?;
    let id = find(&console.read().snapshot()?, "Только что создана")?.id;
    let key = id.to_string();

    let window = AppWindow::new()?;
    let calendar = Calendar {
        fixed_today: Some(TODAY),
    };
    let backend = Inline::new(App::new(Store::open(&path)?), calendar);
    let _bound = Bound::attach(&window, Box::new(backend), calendar);
    window.show()?;
    window.invoke_edit_task(key.as_str().into());
    assert_eq!(window.get_edit_key().as_str(), key.as_str());
    window.set_edit_notes("Черновик из окна".into());

    run(&mut console, Command::Undo)?;
    assert!(console.read().task(id)?.is_none());
    let history = console.read().history()?;
    window.invoke_edit_finish(true);
    assert!(!window.get_edit_error().is_empty());
    assert_eq!(window.get_edit_notes(), "Черновик из окна");
    assert_eq!(window.get_edit_key().as_str(), key.as_str());
    assert_eq!(console.read().history()?, history);

    window.invoke_edit_finish(false);
    assert!(
        !window
            .get_rows()
            .iter()
            .any(|row| row.key.as_str() == key.as_str())
    );
    window.invoke_edit_task(key.as_str().into());
    assert!(window.get_edit_key().is_empty());
    Ok(())
}

fn replace_checklist(task: &Task, checklist: Vec<EditChecklistItem>) -> Command {
    Command::EditTask(EditTask {
        task: task.id,
        expected_updated_at: task.sync.updated_at,
        title: task.title.clone(),
        notes: task.notes.clone(),
        when: None,
        deadline: task.deadline,
        checklist,
        tags: Vec::new(),
    })
}

#[test]
fn undo_of_a_purged_checklist_deletion_advances_the_item_timestamp() -> Result {
    let mut store = Store::open_in_memory()?;
    run(&mut store, Command::AddTask(task("Отчёт")))?;
    let snapshot = store.read().snapshot()?;
    let original = find(&snapshot, "Отчёт")?;
    let new_item = EditChecklistItem {
        id: None,
        title: "Сверить цифры".into(),
        done: false,
    };
    let at = |hours| Clock {
        now: clock().now + jiff::SignedDuration::from_hours(hours),
        ..clock()
    };
    let added = execute(
        &mut store,
        &replace_checklist(original, vec![new_item]),
        at(1),
    )?;
    let task_with_item = find(&added.snapshot, "Отчёт")?;
    let item = &task_with_item.checklist[0];
    let removed = execute(
        &mut store,
        &replace_checklist(task_with_item, Vec::new()),
        at(2),
    )?;
    let deleted = &find(&removed.snapshot, "Отчёт")?.checklist[0];
    assert!(deleted.sync.is_deleted());
    assert_eq!(store.write(|w| w.empty_trash())?.checklist_items, 1);

    let undone = execute(&mut store, &Command::Undo, at(3))?;
    let restored = &find(&undone.snapshot, "Отчёт")?.checklist[0];
    assert_eq!(restored.id, item.id);
    assert_eq!(restored.sync.created_at, item.sync.created_at);
    assert!(!restored.sync.is_deleted());
    assert!(restored.sync.updated_at > deleted.sync.updated_at);
    assert_eq!(restored.sync.updated_at, at(3).now);
    Ok(())
}
