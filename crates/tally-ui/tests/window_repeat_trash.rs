//! Повтор, сроки, журнал и корзина через окно Slint.

mod common;

use common::{Result, TODAY, find, run, task};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use jiff::civil::date;
use slint::{ComponentHandle, LogicalSize, Model, platform::Key};
use tally_app::{App, Command, EditChecklistItem, EditTask};
use tally_core::{RepeatFrom, RepeatUnit};
use tally_store::Store;
use tally_ui::backend::{Calendar, Inline};
use tally_ui::bind::Bound;
use tally_ui::ui::{AppWindow, OrganizationMode, RowKind};

fn activate(window: &AppWindow, label: &str, role: AccessibleRole) -> Result {
    let item = ElementHandle::find_by_accessible_label(window, label)
        .find(|item| item.accessible_role() == Some(role))
        .ok_or_else(|| format!("нет элемента «{label}»"))?;
    assert_ne!(item.accessible_enabled(), Some(false));
    item.invoke_accessible_default_action();
    Ok(())
}

fn button(window: &AppWindow, label: &str) -> Result {
    activate(window, label, AccessibleRole::Button)
}

#[test]
fn configure_repeat_complete_and_restore_a_task_from_trash() -> Result {
    i_slint_backend_testing::init_integration_test_with_mock_time();
    let dir = tempfile::tempdir()?;
    let db = dir.path().join("repeat.db");
    let mut console = Store::open(&db)?;
    let mut recurring = task("Проверить отчёт");
    recurring.when = Some("сегодня".into());
    recurring.deadline = Some("19.11.2026".into());
    run(&mut console, Command::AddTask(recurring))?;

    let window = AppWindow::new()?;
    window.window().set_size(LogicalSize::new(1000.0, 760.0));
    let calendar = Calendar {
        fixed_today: Some(TODAY),
    };
    let _bound = Bound::attach(
        &window,
        Box::new(Inline::new(App::new(Store::open(&db)?), calendar)),
        calendar,
    );
    window.show()?;
    let task_name = "Проверить отчёт";
    assert!(
        window
            .get_rows()
            .iter()
            .any(|row| row.title == task_name && row.due)
    );
    activate(&window, task_name, AccessibleRole::ListItem)?;
    let invalid = tally_ui::ui::RepeatDraft {
        interval: "0".into(),
        unit: 1,
        mode: 1,
    };
    assert!(!window.invoke_edit_repeat(invalid, false).is_empty());
    let draft = tally_ui::ui::RepeatDraft {
        interval: "2".into(),
        unit: 1,
        mode: 1,
    };
    assert!(window.invoke_edit_repeat(draft, false).is_empty());
    assert_eq!(window.get_edit_repeat_caption(), "Повтор · 2 нед.");
    button(&window, "Сохранить")?;
    let original = find(&console.read().snapshot()?, task_name)?.clone();
    assert_eq!(
        original.repeat_rule.ok_or("нет настройки")?.unit,
        RepeatUnit::Week
    );
    assert_eq!(original.repeat_from, RepeatFrom::Completion);
    activate(&window, task_name, AccessibleRole::Checkbox)?;

    activate(&window, "Планы", AccessibleRole::ListItem)?;
    let upcoming: Vec<_> = window.get_rows().iter().collect();
    let day = upcoming
        .iter()
        .position(|r| r.kind == RowKind::Section && r.title == "пт, 4 декабря")
        .ok_or("нет заголовка точного дня")?;
    assert_eq!(upcoming[day + 1].title, task_name);
    let next = console
        .read()
        .tasks()?
        .into_iter()
        .find(|t| t.status.is_open())
        .ok_or("нет повтора")?;
    assert_eq!(next.date, Some(date(2026, 12, 4)));
    assert_eq!(next.deadline, Some(date(2026, 12, 3)));

    activate(&window, "Журнал", AccessibleRole::ListItem)?;
    let closed = window
        .get_rows()
        .iter()
        .find(|r| r.key == original.id.to_string())
        .ok_or("нет строки журнала")?;
    assert!(!closed.when.is_empty());
    window.invoke_choose_task(closed.row_key);
    button(&window, "В корзину")?;
    assert!(window.get_rows().iter().all(|r| r.title != task_name));
    activate(&window, "Корзина", AccessibleRole::ListItem)?;
    button(&window, "Очистить корзину")?;
    assert_eq!(window.get_organization_mode(), OrganizationMode::Confirm);
    assert!(console.read().task(original.id)?.is_some());
    button(&window, "Назад")?;
    button(&window, "Восстановить")?;
    assert_eq!(window.get_rows().row_count(), 0);
    activate(&window, "Журнал", AccessibleRole::ListItem)?;
    assert!(
        window
            .get_rows()
            .iter()
            .any(|r| r.key == original.id.to_string())
    );

    let card = |task: &tally_core::Task, checklist| {
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
    };
    let entries = ["Сверить сумму", "Проверить файл"].map(|title| EditChecklistItem {
        id: None,
        title: title.into(),
        done: false,
    });
    run(&mut console, card(&next, entries.to_vec()))?;
    let current = console.read().task(next.id)?.ok_or("нет открытой задачи")?;
    let last = current.checklist.last().ok_or("нет пункта")?;
    let keep = EditChecklistItem {
        id: Some(last.id),
        title: last.title.clone(),
        done: false,
    };
    run(&mut console, card(&current, vec![keep]))?;
    window.invoke_key(Key::F5.into(), false, false, false, false);
    activate(&window, "Корзина", AccessibleRole::ListItem)?;
    assert_eq!(window.get_rows().row_count(), 0);
    button(&window, "Очистить корзину")?;
    assert!(window.get_organization_prompt().contains("чек-листа – 1"));

    let current = console.read().task(next.id)?.ok_or("нет открытой задачи")?;
    run(&mut console, card(&current, Vec::new()))?;
    button(&window, "Удалить навсегда")?;
    assert_eq!(window.get_organization_mode(), OrganizationMode::Confirm);
    assert!(window.get_organization_error().contains("изменилось"));
    assert!(window.get_organization_prompt().contains("чек-листа – 2"));
    let current = console.read().task(next.id)?.ok_or("нет открытой задачи")?;
    assert_eq!(current.checklist.len(), 2);

    button(&window, "Удалить навсегда")?;
    assert_eq!(window.get_organization_mode(), OrganizationMode::Hidden);
    let current = console.read().task(next.id)?.ok_or("нет открытой задачи")?;
    assert!(current.checklist.is_empty());
    Ok(())
}
