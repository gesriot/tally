//! Сортировка в «Сегодня» сохраняет начало, а перенос на «Сегодня» в боковой
//! панели назначает его.

mod common;
#[path = "common/projects.rs"]
mod projects;

use common::{Result, TODAY, find, run, task};
use projects::{open_window, project};
use slint::{ComponentHandle, LogicalSize, Model};
use tally_app::{Command, MoveTask};
use tally_store::Store;
use tally_ui::ui::{DropSurface, Theme};

#[test]
fn sorting_in_today_preserves_start_but_dropping_on_its_sidebar_entry_schedules_today() -> Result {
    i_slint_backend_testing::init_integration_test_with_mock_time();
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let mut store = Store::open(&path)?;
    project(&mut store, "Работа")?;
    let mut report = task("Отчёт");
    report.project = Some("Работа".into());
    report.when = Some("завтра".into());
    report.deadline = Some("сегодня".into());
    run(&mut store, Command::AddTask(report))?;
    let mut other = task("Контроль");
    other.when = Some("сегодня".into());
    run(&mut store, Command::AddTask(other))?;

    let before = store.read().snapshot()?;
    let original = find(&before, "Отчёт")?;
    assert!(original.date.is_some_and(|date| date > TODAY));
    assert_eq!(original.deadline, Some(TODAY));
    let saved =
        || -> Result<_> { Ok(store.read().task(original.id)?.ok_or("нет задачи")?) };

    let (window, _bound) = open_window(&path, std::convert::identity)?;
    window.window().set_size(LogicalSize::new(1000.0, 680.0));
    window.invoke_select("today".into());
    let report_row = || -> Result<i32> {
        let rows = window.get_rows();
        assert_eq!(rows.row_count(), 2);
        let index = rows
            .iter()
            .position(|row| row.title == "Отчёт")
            .ok_or("нет строки")?;
        Ok(i32::try_from(index)?)
    };

    // Та же задача в том же дневном блоке. Переставляем её через соседнюю строку.
    let source = report_row()?;
    let line_y = if source == 0 {
        window.get_task_extent() - 1.0
    } else {
        1.0
    };
    assert!(window.invoke_drag_begin(source));
    window.invoke_drag_hover(DropSurface::List, line_y);
    window.invoke_drag_end(DropSurface::List, line_y);
    assert_eq!(report_row()?, 1 - source);
    assert_eq!(saved()?.date, original.date);

    // Теперь явно назначаем «Сегодня» через боковую панель.
    let history = store.read().history()?.len();
    let side_row = window
        .get_visible_sidebar()
        .iter()
        .find(|row| row.item.key == "today")
        .ok_or("нет «Сегодня» в панели")?;
    let side_y = side_row.top + window.global::<Theme<'_>>().get_side_row_height() * 0.5;
    assert!(window.invoke_drag_begin(report_row()?));
    window.invoke_drag_hover(DropSurface::Sidebar, side_y);
    assert_eq!(window.get_drag_highlight().as_str(), "today");
    window.invoke_drag_end(DropSurface::Sidebar, side_y);
    assert_eq!(saved()?.date, Some(TODAY));
    assert_eq!(saved()?.deadline, original.deadline);
    assert_eq!(store.read().history()?.len(), history + 1);

    assert!(window.invoke_key("z".into(), true, false, false, false));
    assert_eq!(saved()?.date, original.date);
    assert!(window.invoke_key("z".into(), true, true, false, false));
    assert_eq!(saved()?.date, Some(TODAY));

    let postpone = MoveTask {
        task: original.id.to_string(),
        deadline: Some("через неделю".into()),
        ..Default::default()
    };
    run(&mut store, Command::MoveTask(postpone))?;
    let snapshot = store.read().snapshot()?;
    let day = snapshot.lists(TODAY).today.day;
    assert!(day.iter().any(|item| item.title() == "Отчёт"));
    Ok(())
}
