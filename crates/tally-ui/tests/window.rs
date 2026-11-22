//! Окно целиком на тестовой платформе Slint (без экрана): щелчок по
//! отметке выполняет задачу, Ctrl+Z отменяет, щелчок в боковой панели
//! открывает список. Платформу можно включить один раз на процесс, поэтому
//! тест в файле один.

mod common;

use common::{Result, TODAY, id_of, seed};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalSize, SharedString};
use tally_app::App;
use tally_core::Status;
use tally_store::Store;
use tally_ui::backend::{Calendar, Inline};
use tally_ui::bind::Bound;
use tally_ui::ui::AppWindow;

fn checkbox(window: &AppWindow, title: &str) -> Result<ElementHandle> {
    Ok(ElementHandle::find_by_accessible_label(window, title)
        .find(|e| e.accessible_role() == Some(AccessibleRole::Checkbox))
        .ok_or_else(|| format!("нет отметки «{title}»"))?)
}

fn press(window: &AppWindow, text: SharedString) {
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn ctrl(window: &AppWindow, letter: &str, shift: bool) {
    let control: SharedString = Key::Control.into();
    let shift_key: SharedString = Key::Shift.into();
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: control.clone(),
    });
    if shift {
        window.window().dispatch_event(WindowEvent::KeyPressed {
            text: shift_key.clone(),
        });
    }
    press(window, letter.into());
    if shift {
        window
            .window()
            .dispatch_event(WindowEvent::KeyReleased { text: shift_key });
    }
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text: control });
}

fn status(path: &std::path::Path, title: &str) -> Result<Status> {
    let store = Store::open(path)?;
    let s = store.read().snapshot()?;
    let id = id_of(&s, title)?;
    Ok(store.read().task(id)?.ok_or("нет задачи")?.status)
}

#[test]
fn click_a_checkbox_then_ctrl_z_and_switch_lists() -> Result {
    i_slint_backend_testing::init_integration_test_with_mock_time();
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    seed(&mut Store::open(&path)?)?;

    let window = AppWindow::new()?;
    window.window().set_size(LogicalSize::new(1000.0, 680.0));
    let calendar = Calendar {
        fixed_today: Some(TODAY),
    };
    let backend = Inline::new(App::new(Store::open(&path)?), calendar);
    let _bound = Bound::attach(&window, Box::new(backend), calendar);
    window.show()?;

    assert_eq!(window.get_window_title(), "Tally");
    assert_eq!(window.get_page_title(), "Сегодня");

    // Щелчок мышью по отметке – задача выполнена, строка на месте с отметкой.
    let call = checkbox(&window, "Позвонить электрику")?;
    assert_eq!(call.accessible_checked(), Some(false));
    call.mock_single_click(PointerEventButton::Left);
    assert!(matches!(
        status(&path, "Позвонить электрику")?,
        Status::Completed { .. }
    ));
    assert_eq!(
        checkbox(&window, "Позвонить электрику")?.accessible_checked(),
        Some(true)
    );

    // Ctrl+Z – выполнение отменено, внизу сообщение.
    ctrl(&window, "z", false);
    assert_eq!(status(&path, "Позвонить электрику")?, Status::Open);
    assert_eq!(
        checkbox(&window, "Позвонить электрику")?.accessible_checked(),
        Some(false)
    );
    assert_eq!(
        window.get_message(),
        "Отменено: выполнение задачи «Позвонить электрику»"
    );
    // Ctrl+Shift+Z – снова выполнено.
    ctrl(&window, "Z", true);
    assert!(matches!(
        status(&path, "Позвонить электрику")?,
        Status::Completed { .. }
    ));
    // Сообщение исчезает само.
    i_slint_backend_testing::mock_elapsed_time(tally_ui::bind::MESSAGE_TIME * 2);
    assert_eq!(window.get_message(), "");

    // Отметка через доступность (как экранный диктор) – то же самое.
    checkbox(&window, "Оплатить интернет")?.invoke_accessible_default_action();
    assert!(matches!(
        status(&path, "Оплатить интернет")?,
        Status::Completed { .. }
    ));
    // Снять отметку с только что выполненной – отменить выполнение.
    checkbox(&window, "Оплатить интернет")?.invoke_accessible_default_action();
    assert_eq!(status(&path, "Оплатить интернет")?, Status::Open);

    // Боковая панель: «Входящие».
    let inbox = ElementHandle::find_by_accessible_label(&window, "Входящие")
        .find(|e| e.accessible_role() == Some(AccessibleRole::ListItem))
        .ok_or("нет «Входящих»")?;
    inbox.invoke_accessible_default_action();
    assert_eq!(window.get_page_title(), "Входящие");
    checkbox(&window, "Записаться к стоматологу")?;
    assert!(checkbox(&window, "Позвонить электрику").is_err());
    Ok(())
}
