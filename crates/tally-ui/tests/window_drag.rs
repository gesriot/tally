//! Перетаскивание указателем с линией вставки и автопрокруткой.

mod common;

use std::time::Duration;

use common::{Result, TODAY, find, run, seed, task};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, Model, SharedString};
use tally_app::{App, Command, NewHeading};
use tally_core::lists::Item;
use tally_store::Store;
use tally_ui::backend::{Calendar, Inline};
use tally_ui::bind::Bound;
use tally_ui::ui::{AppWindow, RowKind};

fn point(window: &AppWindow, title: &str, fraction: f32) -> Result<LogicalPosition> {
    let item = ElementHandle::find_by_accessible_label(window, title)
        .find(|e| e.accessible_role() == Some(AccessibleRole::ListItem))
        .ok_or_else(|| format!("нет строки «{title}»"))?;
    let pos = item.absolute_position();
    let size = item.size();
    assert!(
        size.width > 100.0 && size.height > 20.0,
        "{title}: {pos:?} {size:?}"
    );
    Ok(LogicalPosition::new(
        pos.x + size.width * 0.45,
        pos.y + size.height * fraction,
    ))
}

fn down(window: &AppWindow, position: LogicalPosition) {
    window.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
}

fn motion(window: &AppWindow, position: LogicalPosition) {
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
}

fn up(window: &AppWindow, position: LogicalPosition) {
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
}

fn today(store: &Store) -> Result<Vec<String>> {
    let snapshot = store.read().snapshot()?;
    Ok(snapshot
        .lists(TODAY)
        .today
        .day
        .iter()
        .map(|item| item.title().to_owned())
        .collect())
}

#[test]
fn drag_reorders_moves_rolls_back_and_scrolls_past_the_source() -> Result {
    i_slint_backend_testing::init_integration_test_with_mock_time();
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let mut console = Store::open(&path)?;
    seed(&mut console)?;
    let window = AppWindow::new()?;
    window.window().set_size(LogicalSize::new(1000.0, 560.0));
    let calendar = Calendar {
        fixed_today: Some(TODAY),
    };
    let _bound = Bound::attach(
        &window,
        Box::new(Inline::new(App::new(Store::open(&path)?), calendar)),
        calendar,
    );
    window.show()?;

    let before = console.read().snapshot()?;
    let history = console.read().history()?;
    let source = point(&window, "Позвонить электрику", 0.4)?;
    let target = point(&window, "Купить плитку", 0.25)?;
    down(&window, source);
    motion(&window, target);
    assert!(window.get_drag_active());
    assert!(window.get_drag_line() >= 0.0);
    assert_eq!(console.read().history()?, history);
    up(&window, target);
    assert!(!window.get_drag_active());
    assert_eq!(
        &today(&console)?[..2],
        ["Позвонить электрику", "Купить плитку"]
    );
    assert_eq!(console.read().history()?.len(), history.len() + 1);
    let after = console.read().snapshot()?;
    assert_eq!(
        find(&after, "Позвонить электрику")?.index,
        find(&before, "Позвонить электрику")?.index
    );
    assert!(window.invoke_key("z".into(), true, false, false, false));
    assert_eq!(
        &today(&console)?[..2],
        ["Купить плитку", "Позвонить электрику"]
    );
    assert!(window.invoke_key("z".into(), true, true, false, false));
    assert_eq!(
        &today(&console)?[..2],
        ["Позвонить электрику", "Купить плитку"]
    );

    let source = point(&window, "Купить плитку", 0.4)?;
    let target = point(&window, "Отпуск", 0.5)?;
    let history = console.read().history()?;
    down(&window, source);
    motion(&window, target);
    assert!(window.get_drag_active());
    assert!(window.get_drag_highlight().starts_with("project:"));
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: SharedString::from(Key::Escape),
    });
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: SharedString::from(Key::Escape),
    });
    up(&window, target);
    assert!(!window.get_drag_active());
    assert_eq!(console.read().history()?, history);

    down(&window, source);
    motion(&window, target);
    up(&window, target);
    let moved = console.read().snapshot()?;
    let vacation = moved
        .projects
        .iter()
        .find(|p| p.title == "Отпуск")
        .ok_or("нет проекта")?;
    let tile = find(&moved, "Купить плитку")?;
    assert_eq!(
        (tile.project, tile.heading, tile.date),
        (Some(vacation.id), None, Some(TODAY))
    );
    assert_eq!(tile.deadline, find(&before, "Купить плитку")?.deadline);

    let repair = moved
        .projects
        .iter()
        .find(|p| p.title == "Ремонт")
        .ok_or("нет проекта")?;
    run(
        &mut console,
        Command::AddHeading(NewHeading {
            title: "Балкон".into(),
            project: "Ремонт".into(),
        }),
    )?;
    window.invoke_key(SharedString::from(Key::F5), false, false, false, false);
    window.invoke_select(format!("project:{}", repair.id).into());
    assert!(
        window
            .get_rows()
            .iter()
            .any(|row| row.kind == RowKind::Section && row.title == "Балкон")
    );
    let paint = point(&window, "Выбрать краску", 0.4)?;
    let section = ElementHandle::find_by_accessible_label(&window, "Балкон")
        .find(|e| e.accessible_role() == Some(AccessibleRole::Text))
        .ok_or("нет заголовка раздела")?;
    let section_pos = section.absolute_position();
    let balcony = LogicalPosition::new(section_pos.x + 70.0, section_pos.y + 8.0);
    down(&window, paint);
    motion(&window, balcony);
    assert!(window.get_drag_line() > 0.0);
    up(&window, balcony);
    let snapshot = console.read().snapshot()?;
    let balcony_id = snapshot
        .headings
        .iter()
        .find(|h| h.title == "Балкон")
        .ok_or("нет раздела")?
        .id;
    assert_eq!(find(&snapshot, "Выбрать краску")?.heading, Some(balcony_id));

    for n in 1..=30 {
        let mut task = task(&format!("Строка {n:02}"));
        task.when = Some("сегодня".into());
        run(&mut console, Command::AddTask(task))?;
    }
    let lengths = console.read().history()?.len();
    window.invoke_key(SharedString::from(Key::F5), false, false, false, false);
    window.invoke_select("today".into());
    let source = point(&window, "Позвонить электрику", 0.45)?;
    let edge = LogicalPosition::new(source.x, 552.0);
    down(&window, source);
    motion(&window, edge);
    assert!(window.get_drag_active());
    let initial_line = window.get_drag_line();
    assert!(initial_line > 0.0);
    for _ in 0..38 {
        i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(16));
    }
    assert!(window.get_drag_line() > initial_line + 300.0);
    up(&window, edge);
    assert!(!window.get_drag_active());
    let order = today(&console)?;
    let position = order
        .iter()
        .position(|title| title == "Позвонить электрику")
        .ok_or("нет задачи")?;
    assert!(position >= 20);
    assert_eq!(console.read().history()?.len(), lengths + 1);
    assert!(
        console
            .read()
            .snapshot()?
            .lists(TODAY)
            .today
            .evening
            .iter()
            .all(|item| !matches!(item, Item::Task(t) if t.title == "Позвонить электрику"))
    );
    Ok(())
}
