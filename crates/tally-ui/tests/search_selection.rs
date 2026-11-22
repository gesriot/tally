//! Enter после исчезновения совпадения открывает только видимый выбор.

mod common;

use std::time::Duration;

use common::{Result, TODAY, clock};
use i_slint_backend_testing::{init_integration_test_with_mock_time, mock_elapsed_time};
use slint::{ComponentHandle, LogicalSize, Model, platform::Key};
use tally_app::App;
use tally_core::{Rank, Task, TaskId};
use tally_store::Store;
use tally_ui::backend::{Calendar, Inline};
use tally_ui::bind::Bound;
use tally_ui::ui::{AppWindow, Theme};

fn press(window: &AppWindow, key: Key) {
    assert!(window.invoke_key(key.into(), false, false, false, false));
}

fn selected_is_visible(window: &AppWindow) -> bool {
    let selected = window.get_selected_result();
    let top = -window.get_task_scroll_y();
    let bottom = top + window.get_task_view_height();
    let height = window.global::<Theme<'_>>().get_row_height();
    window
        .get_visible_rows()
        .iter()
        .any(|row| row.item.row_key == selected && row.top + height > top && row.top < bottom)
}

#[test]
fn enter_after_a_match_disappears_only_opens_a_visible_selection() -> Result {
    init_integration_test_with_mock_time();
    let dir = tempfile::tempdir()?;
    let db = dir.path().join("tally.db");
    let mut store = Store::open(&db)?;
    store.write(|w| {
        for (i, rank) in Rank::spread(96).into_iter().enumerate() {
            let task = Task::new(
                TaskId::from_u128(i as u128 + 1),
                format!("Письмо {i:03}"),
                rank,
                clock().now,
            );
            w.insert_task(&task)?;
        }
        Ok(())
    })?;

    let window = AppWindow::new()?;
    window.window().set_size(LogicalSize::new(900.0, 600.0));
    let calendar = Calendar {
        fixed_today: Some(TODAY),
    };
    let backend = Inline::new(App::new(Store::open(&db)?), calendar);
    let _bound = Bound::attach(&window, Box::new(backend), calendar);
    window.show()?;
    window.invoke_search_begin();
    window.invoke_search_query("письмо".into());
    mock_elapsed_time(Duration::from_millis(150));
    assert_eq!(window.get_rows().row_count(), 96);
    for _ in 1..96 {
        press(&window, Key::DownArrow);
    }
    mock_elapsed_time(Duration::from_millis(32));
    let last = TaskId::from_u128(96);
    assert_eq!(window.get_selected_result().as_str(), last.to_string());
    assert!(selected_is_visible(&window));

    let mut renamed = store.read().task(last)?.ok_or("нет последней задачи")?;
    renamed.title = "Отправить отчёт".into();
    store.write(|w| w.update_task(&renamed))?;
    press(&window, Key::F5);
    mock_elapsed_time(Duration::from_millis(32));
    assert_eq!(window.get_rows().row_count(), 95);
    let selected = window.get_selected_result();
    assert_ne!(selected.as_str(), last.to_string());
    if !selected.is_empty() {
        assert!(selected_is_visible(&window), "новое выделение вне экрана");
    }
    window.invoke_key(Key::Return.into(), false, false, false, false);
    assert_eq!(window.get_edit_task_id(), selected);
    Ok(())
}
