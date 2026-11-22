//! Быстрый ввод, карточка с напоминанием и общий журнал действий.

mod common;

use std::cell::Cell;
use std::rc::Rc;

use common::{Result, TODAY};
use i_slint_backend_testing::init_integration_test_with_mock_time;
use jiff::Timestamp;
use slint::{ComponentHandle, Model, platform::Key};
use tally_app::App;
use tally_store::Store;
use tally_ui::backend::{Calendar, Inline};
use tally_ui::bind::Bound;
use tally_ui::ui::{AppWindow, QuickEntry, ReminderNotice};

#[test]
fn quick_entry_and_reminder_use_the_normal_history() -> Result {
    init_integration_test_with_mock_time();
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let mut store = Store::open(&path)?;
    let calendar = Calendar {
        fixed_today: Some(TODAY),
    };
    let window = AppWindow::new()?;
    let entry = QuickEntry::new()?;
    let bound = Bound::attach(
        &window,
        Box::new(Inline::new(App::new(Store::open(&path)?), calendar)),
        calendar,
    );
    bound.attach_quick_entry(&entry);
    window.show()?;

    window.invoke_quick_entry();
    entry.set_notes("Сверить адрес\nУточнить время".into());
    entry.set_draft("  ".into());
    entry.invoke_submit();
    assert!(entry.get_title_invalid());
    assert!(entry.get_error().is_empty());
    assert_eq!(entry.get_notes().as_str(), "Сверить адрес\nУточнить время");
    assert!(store.read().tasks()?.is_empty());

    entry.set_draft("Позвонить завтра".into());
    entry.invoke_dismiss();
    window.invoke_quick_entry();
    assert_eq!(entry.get_draft().as_str(), "Позвонить завтра");
    assert_eq!(entry.get_notes().as_str(), "Сверить адрес\nУточнить время");
    entry.invoke_submit();
    assert!(entry.get_draft().is_empty());
    assert!(entry.get_notes().is_empty());
    assert!(!entry.get_title_invalid());
    assert!(!entry.get_busy());
    assert!(entry.get_error().is_empty());

    let task = store
        .read()
        .tasks()?
        .pop()
        .ok_or("нет добавленной задачи")?;
    assert_eq!(task.title, "Позвонить");
    assert_eq!(task.notes, "Сверить адрес\nУточнить время");
    assert_eq!(task.date, Some(TODAY.tomorrow()?));
    window.invoke_select("upcoming".into());
    let row = window
        .get_rows()
        .iter()
        .find(|row| row.editable)
        .ok_or("нет строки задачи")?;
    window.invoke_edit_task(row.row_key);
    assert_eq!(window.get_edit_task_id().as_str(), task.id.to_string());
    window.set_edit_reminder("31.02.2030 09:15".into());
    window.invoke_key(Key::Return.into(), true, false, false, false);
    assert!(!window.get_edit_error().is_empty());
    assert_eq!(
        store
            .read()
            .task(task.id)?
            .and_then(|task| task.reminder_at),
        None
    );

    window.set_edit_notes("Спросить о сроках".into());
    window.set_edit_reminder("10.01.2030 09:15".into());
    window.invoke_key(Key::Return.into(), true, false, false, false);
    assert!(window.get_edit_task_id().is_empty());
    let saved = store.read().task(task.id)?.ok_or("задача пропала")?;
    let due = saved.reminder_at.ok_or("напоминание не сохранено")?;
    assert_eq!(saved.notes, "Спросить о сроках");
    window.invoke_key("z".into(), true, false, false, false);
    let undone = store
        .read()
        .task(task.id)?
        .ok_or("задача пропала после отмены")?;
    assert_eq!(undone.notes, task.notes);
    assert!(undone.reminder_at.is_none());
    window.invoke_key("z".into(), true, true, false, false);
    assert_eq!(
        store
            .read()
            .task(task.id)?
            .and_then(|task| task.reminder_at),
        Some(due)
    );

    let before = Timestamp::from_second(due.as_second() - 1)?;
    assert!(store.claim_reminders(before)?.is_empty());
    let notifications = store.claim_reminders(due)?;
    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0].task, task.id);
    store.confirm_reminders(&notifications)?;
    assert!(Store::open(&path)?.claim_reminders(due)?.is_empty());

    let notice = ReminderNotice::new()?;
    notice.invoke_append_body("Первое".into());
    notice.invoke_append_body("Второе".into());
    assert_eq!(notice.get_body().as_str(), "Первое\nВторое");
    notice.invoke_clear_body();
    notice.invoke_append_body("Третье".into());
    assert_eq!(notice.get_body().as_str(), "Третье");

    let mut task = store.read().task(task.id)?.ok_or("задача пропала")?;
    task.reminder_at = Some(Timestamp::UNIX_EPOCH);
    store.write(|writer| writer.update_task(&task))?;
    let presented = Rc::new(Cell::new(0));
    let count = presented.clone();
    bound.enable_reminders(move |batch| count.set(count.get() + batch.len()));
    assert_eq!(presented.get(), 1);
    bound.complete_reminders(true);
    assert!(
        Store::open(path)?
            .claim_reminders(Timestamp::now())?
            .is_empty()
    );
    Ok(())
}
