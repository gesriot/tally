//! Страницы областей и проектов в окне Slint, общий журнал действий.

mod common;

use common::{Result, TODAY, run, seed, task};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalSize, Model, SharedString};
use tally_app::{App, Command};
use tally_core::Status;
use tally_store::Store;
use tally_ui::backend::{Calendar, Inline};
use tally_ui::bind::Bound;
use tally_ui::ui::{AppWindow, Mark, OrganizationMode, RowKind};

fn element(window: &AppWindow, label: &str, role: AccessibleRole) -> Result<ElementHandle> {
    ElementHandle::find_by_accessible_label(window, label)
        .find(|item| item.accessible_role() == Some(role))
        .ok_or_else(|| format!("нет элемента «{label}»").into())
}

fn button(window: &AppWindow, label: &str) -> Result {
    let item = element(window, label, AccessibleRole::Button)?;
    assert_ne!(item.accessible_enabled(), Some(false));
    item.invoke_accessible_default_action();
    Ok(())
}

fn select(window: &AppWindow, label: &str) -> Result {
    element(window, label, AccessibleRole::ListItem)?.invoke_accessible_default_action();
    Ok(())
}

fn title(window: &AppWindow, value: &str) -> Result {
    element(window, "Название", AccessibleRole::TextInput)?.set_accessible_value(value);
    Ok(())
}

fn press(window: &AppWindow, key: Key) {
    let text: SharedString = key.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

#[test]
fn organize_from_window_and_close_projects_without_changing_tasks() -> Result {
    i_slint_backend_testing::init_integration_test_with_mock_time();
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let mut console = Store::open(&path)?;
    seed(&mut console)?;

    let window = AppWindow::new()?;
    window.window().set_size(LogicalSize::new(640.0, 680.0));
    let calendar = Calendar {
        fixed_today: Some(TODAY),
    };
    let _bound = Bound::attach(
        &window,
        Box::new(Inline::new(App::new(Store::open(&path)?), calendar)),
        calendar,
    );
    window.show()?;

    button(&window, "Новая область")?;
    title(&window, "Учёба")?;
    press(&window, Key::Return);
    assert_eq!(window.get_page_title(), "Учёба");
    button(&window, "Проект в области")?;
    button(&window, "Сохранить")?;
    assert_eq!(window.get_organization_mode(), OrganizationMode::Title);
    assert!(!window.get_organization_error().is_empty());
    title(&window, "Курсовая")?;
    button(&window, "Сохранить")?;
    assert_eq!(window.get_page_title(), "Курсовая");
    assert_eq!(window.get_page_progress_text(), "0 из 0");
    let snapshot = console.read().snapshot()?;
    let area = snapshot
        .areas
        .iter()
        .find(|a| a.title == "Учёба")
        .ok_or("нет области")?;
    let project = snapshot
        .projects
        .iter()
        .find(|p| p.title == "Курсовая")
        .ok_or("нет проекта")?;
    assert_eq!(project.area, Some(area.id));
    let project_id = project.id;

    button(&window, "Новый раздел")?;
    title(&window, "Источники")?;
    button(&window, "Сохранить")?;
    assert!(
        window
            .get_rows()
            .iter()
            .any(|r| r.kind == RowKind::Section && r.title == "Источники")
    );
    let mut research = task("Проверить литературу");
    research.project = Some("Курсовая".into());
    research.heading = Some("Источники".into());
    research.when = Some("сегодня".into());
    run(&mut console, Command::AddTask(research))?;
    let mut bibliography = task("Собрать ссылки");
    bibliography.project = Some("Курсовая".into());
    run(&mut console, Command::AddTask(bibliography))?;
    press(&window, Key::F5);
    assert_eq!(window.get_page_progress_text(), "0 из 2");
    let rows: Vec<_> = window.get_rows().iter().map(|r| r.title).collect();
    assert_eq!(
        rows,
        ["Собрать ссылки", "Источники", "Проверить литературу"].map(SharedString::from)
    );
    element(&window, "Собрать ссылки", AccessibleRole::Checkbox)?
        .invoke_accessible_default_action();
    assert_eq!(window.get_page_progress_text(), "1 из 2");
    assert_eq!(window.get_page_progress(), 0.5);

    button(&window, "Переименовать")?;
    title(&window, "Локальное имя")?;
    let project = console.read().project(project_id)?.ok_or("нет проекта")?;
    run(
        &mut console,
        Command::RenameProject {
            project: project_id,
            expected_updated_at: project.sync.updated_at,
            title: "Курсовая работа".into(),
        },
    )?;
    button(&window, "Сохранить")?;
    assert_eq!(window.get_organization_name(), "Локальное имя");
    assert!(window.get_organization_error().contains("Проект изменился"));
    assert_eq!(window.get_page_title(), "Курсовая работа");
    press(&window, Key::Escape);
    assert_eq!(window.get_organization_mode(), OrganizationMode::Hidden);
    button(&window, "Переименовать")?;
    assert_eq!(window.get_organization_name(), "Курсовая работа");
    title(&window, "Готовая работа")?;
    button(&window, "Сохранить")?;

    let tasks_before = console.read().snapshot()?.tasks;
    button(&window, "Закрыть проект")?;
    button(&window, "Назад")?;
    assert_eq!(
        console
            .read()
            .project(project_id)?
            .ok_or("нет проекта")?
            .status,
        Status::Open
    );
    button(&window, "Закрыть проект")?;
    button(&window, "Завершить")?;
    assert_eq!(window.get_page_title(), "Сегодня");
    assert!(
        window
            .get_rows()
            .iter()
            .all(|r| r.title != "Проверить литературу")
    );
    assert!(matches!(
        console
            .read()
            .project(project_id)?
            .ok_or("нет проекта")?
            .status,
        Status::Completed { .. }
    ));
    select(&window, "Журнал")?;
    assert!(
        window
            .get_rows()
            .iter()
            .any(|r| r.title == "Готовая работа" && r.mark == Mark::Completed && !r.opens)
    );
    assert_eq!(console.read().snapshot()?.tasks, tasks_before);

    assert!(window.invoke_key("z".into(), true, false, false, false));
    assert_eq!(
        console
            .read()
            .project(project_id)?
            .ok_or("нет проекта")?
            .status,
        Status::Open
    );
    assert!(
        window
            .get_sidebar()
            .iter()
            .any(|row| row.title == "Готовая работа")
    );
    select(&window, "Готовая работа")?;
    assert_eq!(window.get_page_progress_text(), "1 из 2");
    button(&window, "Закрыть проект")?;
    button(&window, "Отменить проект")?;
    select(&window, "Журнал")?;
    assert!(
        window
            .get_rows()
            .iter()
            .any(|r| r.title == "Готовая работа" && r.mark == Mark::Canceled)
    );
    assert_eq!(console.read().snapshot()?.tasks, tasks_before);

    select(&window, "Учёба")?;
    button(&window, "Переименовать")?;
    assert_eq!(window.get_organization_name(), "Учёба");
    title(&window, "Университет")?;
    button(&window, "Сохранить")?;
    assert_eq!(window.get_page_title(), "Университет");
    assert_eq!(
        console
            .read()
            .project(project_id)?
            .ok_or("нет проекта")?
            .area,
        Some(area.id)
    );

    button(&window, "Новый проект")?;
    title(&window, "Поездка")?;
    button(&window, "Сохранить")?;
    assert_eq!(window.get_page_title(), "Поездка");
    assert!(
        console
            .read()
            .snapshot()?
            .projects
            .iter()
            .any(|p| p.title == "Поездка" && p.area.is_none())
    );
    Ok(())
}
