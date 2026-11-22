//! Окно целиком (тестовая платформа Slint): настоящая клавиша F5, сообщение
//! при снятии отметки после действия консоли, доступность отметок и строк
//! проекта. Платформа – одна на процесс, поэтому тест в файле один.

mod common;

use common::{Result, TODAY, clock, run, seed, task};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalSize, SharedString};
use tally_app::{App, Command};
use tally_core::Status;
use tally_store::Store;
use tally_ui::backend::{Calendar, Inline};
use tally_ui::bind::Bound;
use tally_ui::ui::AppWindow;

fn find(window: &AppWindow, label: &str, role: AccessibleRole) -> Result<ElementHandle> {
    Ok(ElementHandle::find_by_accessible_label(window, label)
        .find(|e| e.accessible_role() == Some(role))
        .ok_or_else(|| format!("нет «{label}» ({role:?})"))?)
}

fn checkbox(window: &AppWindow, title: &str) -> Result<ElementHandle> {
    find(window, title, AccessibleRole::Checkbox)
}

fn sidebar(window: &AppWindow, title: &str) -> Result<()> {
    find(window, title, AccessibleRole::ListItem)?.invoke_accessible_default_action();
    Ok(())
}

#[test]
fn f5_console_conflict_and_disabled_rows() -> Result {
    i_slint_backend_testing::init_integration_test_with_mock_time();
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let mut console = Store::open(&path)?;
    seed(&mut console)?;
    // Закрытый проект – для «Журнала».
    let trip = console
        .read()
        .snapshot()?
        .projects
        .into_iter()
        .find(|p| p.title == "Отпуск")
        .ok_or("нет проекта")?;
    let mut closed = trip.clone();
    closed.status = Status::Completed { at: clock().now };
    console.write(|w| w.update_project(&closed))?;

    let window = AppWindow::new()?;
    window.window().set_size(LogicalSize::new(1000.0, 680.0));
    let calendar = Calendar {
        fixed_today: Some(TODAY),
    };
    let backend = Inline::new(App::new(Store::open(&path)?), calendar);
    let _bound = Bound::attach(&window, Box::new(backend), calendar);
    window.show()?;

    // Две отметки подряд: нажимаема только последняя.
    checkbox(&window, "Купить плитку")?.mock_single_click(PointerEventButton::Left);
    checkbox(&window, "Позвонить электрику")?.mock_single_click(PointerEventButton::Left);
    assert_eq!(
        checkbox(&window, "Купить плитку")?.accessible_enabled(),
        Some(false)
    );
    assert_eq!(
        checkbox(&window, "Позвонить электрику")?.accessible_enabled(),
        Some(true)
    );

    // Консоль добавила задачу – снятие отметки ничего не отменяет.
    run(&mut console, Command::AddTask(task("Заказать доставку")))?;
    let history = console.read().history()?;
    checkbox(&window, "Позвонить электрику")?.mock_single_click(PointerEventButton::Left);
    assert_eq!(
        window.get_message(),
        "После выполнения задачи было другое действие"
    );
    assert_eq!(console.read().history()?, history);
    assert_eq!(
        checkbox(&window, "Позвонить электрику")?.accessible_enabled(),
        Some(false)
    );

    // F5 – настоящая клавиша Slint – показывает задачу из консоли.
    sidebar(&window, "Входящие")?;
    assert!(checkbox(&window, "Заказать доставку").is_err());
    let f5: SharedString = Key::F5.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: f5.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text: f5 });
    assert_eq!(
        checkbox(&window, "Заказать доставку")?.accessible_enabled(),
        Some(true)
    );
    // F1 ничего не делает.
    run(&mut console, Command::AddTask(task("Ещё из консоли")))?;
    let f1: SharedString = Key::F1.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: f1.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text: f1 });
    assert!(checkbox(&window, "Ещё из консоли").is_err());

    // Закрытый проект в «Журнале» – строка без перехода.
    sidebar(&window, "Журнал")?;
    let row = find(&window, "Отпуск", AccessibleRole::ListItem)?;
    assert_eq!(row.accessible_enabled(), Some(false));
    row.invoke_accessible_default_action();
    row.mock_single_click(PointerEventButton::Left);
    assert_eq!(window.get_page_title(), "Журнал");
    // Выполненные задачи в «Журнале» не нажимаются.
    assert_eq!(
        checkbox(&window, "Купить плитку")?.accessible_enabled(),
        Some(false)
    );

    // Проект в работе на странице области открывается.
    sidebar(&window, "Дом")?;
    let repair = ElementHandle::find_by_accessible_label(&window, "Ремонт")
        .filter(|e| e.accessible_role() == Some(AccessibleRole::ListItem))
        .find(|e| {
            e.accessible_enabled() == Some(true) && e.accessible_item_selectable() != Some(true)
        })
        .ok_or("нет строки «Ремонт» на странице области")?;
    repair.mock_single_click(PointerEventButton::Left);
    assert_eq!(window.get_page_title(), "Ремонт");
    Ok(())
}
