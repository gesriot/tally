//! Перенос на «В любое время» в боковой панели очищает расписание задачи.

mod common;
#[path = "common/projects.rs"]
mod projects;

use common::{Result, TODAY, find, run, task};
use projects::{open_window, project};
use slint::{ComponentHandle, LogicalSize, Model};
use tally_app::Command;
use tally_store::Store;
use tally_ui::ui::{DropSurface, Theme};

#[test]
fn anytime_sidebar_clears_a_starred_tasks_schedule_from_the_anytime_page() -> Result {
    i_slint_backend_testing::init_integration_test_with_mock_time();
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let mut store = Store::open(&path)?;
    let project_id = project(&mut store, "Работа")?;
    let mut report = task("Отчёт");
    report.project = Some("Работа".into());
    report.when = Some("сегодня вечером".into());
    run(&mut store, Command::AddTask(report))?;
    let before = store.read().snapshot()?;
    let original = find(&before, "Отчёт")?;
    assert_eq!((original.date, original.evening), (Some(TODAY), true));
    let history = store.read().history()?;

    let (window, _bound) = open_window(&path, std::convert::identity)?;
    window.window().set_size(LogicalSize::new(1000.0, 680.0));
    window.invoke_select("anytime".into());
    let row = || window.get_rows().row_data(0).ok_or("нет строки задачи");
    assert_eq!(window.get_rows().row_count(), 1);
    assert!(row()?.star);

    // Линия после единственной строки: порядок и расписание уже правильные.
    let line_y = window.get_task_extent() + 1.0;
    assert!(window.invoke_drag_begin(0));
    window.invoke_drag_hover(DropSurface::List, line_y);
    window.invoke_drag_end(DropSurface::List, line_y);
    assert_eq!(store.read().task(original.id)?.as_ref(), Some(original));
    assert_eq!(store.read().history()?, history);

    // Пункт в панели: позиция та же, но теперь пользователь назначает режим.
    let side_row = window
        .get_visible_sidebar()
        .iter()
        .find(|row| row.item.key == "anytime")
        .ok_or("нет пункта «В любое время»")?;
    let side_y = side_row.top + window.global::<Theme<'_>>().get_side_row_height() * 0.5;
    assert!(window.invoke_drag_begin(0));
    window.invoke_drag_hover(DropSurface::Sidebar, side_y);
    assert_eq!(window.get_drag_highlight().as_str(), "anytime");
    window.invoke_drag_end(DropSurface::Sidebar, side_y);

    let moved = store.read().task(original.id)?.ok_or("нет задачи")?;
    assert_eq!((moved.date, moved.evening), (None, false));
    assert_eq!(moved.project, Some(project_id));
    assert_eq!(store.read().history()?.len(), history.len() + 1);
    assert!(!row()?.star);
    let snapshot = store.read().snapshot()?;
    let today = snapshot.lists(TODAY).today;
    assert!(today.day.is_empty() && today.evening.is_empty());

    assert!(window.invoke_key("z".into(), true, false, false, false));
    let undone = store.read().task(original.id)?.ok_or("нет задачи")?;
    assert_eq!((undone.date, undone.evening), (Some(TODAY), true));
    assert!(row()?.star);
    assert!(window.invoke_key("z".into(), true, true, false, false));
    let redone = store.read().task(original.id)?.ok_or("нет задачи")?;
    assert_eq!((redone.date, redone.evening), (None, false));
    assert!(!row()?.star);

    // То же назначение со страницы «Сегодня» даёт то же расписание.
    assert!(window.invoke_key("z".into(), true, false, false, false));
    window.invoke_select("today".into());
    let report_row = window
        .get_rows()
        .iter()
        .position(|row| row.title == "Отчёт")
        .ok_or("нет задачи в «Сегодня»")?;
    assert!(window.invoke_drag_begin(i32::try_from(report_row)?));
    window.invoke_drag_end(DropSurface::Sidebar, side_y);
    let moved_from_today = store.read().task(original.id)?.ok_or("нет задачи")?;
    assert_eq!(
        (
            moved_from_today.start,
            moved_from_today.date,
            moved_from_today.evening
        ),
        (moved.start, moved.date, moved.evening)
    );
    assert!(window.get_rows().iter().all(|row| row.title != "Отчёт"));
    Ok(())
}
