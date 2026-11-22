//! Быстрый поиск, выдача и фильтр тегов через окно Slint.

mod common;

use std::{cell::Cell, rc::Rc, time::Duration};

use common::{Result, TODAY, clock, run, task};
use i_slint_backend_testing::{AccessibleRole, ElementHandle, mock_elapsed_time};
use slint::{
    ComponentHandle, LogicalSize, Model, SharedString,
    platform::{Key, WindowEvent},
};
use tally_app::{App, Command, Effect, NewArea, NewProject, execute};
use tally_core::{ChecklistItem, ChecklistItemId, Rank, Tag, TagId};
use tally_store::{RecordId, Store};
use tally_ui::{
    backend::{Backend, Calendar, Inline, Reply, Request},
    bind::Bound,
    ui::{AppWindow, RowKind},
};

struct OutOfOrder {
    inline: Inline<App>,
    hold: Rc<Cell<bool>>,
    searches: Vec<Reply>,
}

impl Backend for OutOfOrder {
    fn submit(&mut self, request: Request) {
        self.inline.submit(request);
    }

    fn replies(&mut self) -> Vec<Reply> {
        let mut ready = Vec::new();
        for reply in self.inline.replies() {
            if matches!(reply, Reply::Searched { .. }) {
                self.searches.push(reply);
            } else {
                ready.push(reply);
            }
        }
        if !self.hold.get() {
            ready.extend(self.searches.drain(..).rev());
        }
        ready
    }
}

fn element(window: &AppWindow, label: &str, role: AccessibleRole) -> Result<ElementHandle> {
    Ok(ElementHandle::find_by_accessible_label(window, label)
        .find(|e| e.accessible_role() == Some(role))
        .ok_or_else(|| format!("нет элемента «{label}»"))?)
}

fn press(window: &AppWindow, key: impl Into<SharedString>) {
    let text = key.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn query(window: &AppWindow, value: &str) -> Result {
    element(window, "Поиск", AccessibleRole::TextInput)?.set_accessible_value(value);
    mock_elapsed_time(Duration::from_millis(150));
    Ok(())
}

fn titles(window: &AppWindow) -> Vec<String> {
    window
        .get_rows()
        .iter()
        .filter(|r| r.kind != RowKind::Section)
        .map(|r| r.title.to_string())
        .collect()
}

#[test]
fn quick_find_opens_a_result_ignores_old_replies_and_filters_by_tag() -> Result {
    i_slint_backend_testing::init_integration_test_with_mock_time();
    let dir = tempfile::tempdir()?;
    let db = dir.path().join("tally.db");
    let mut console = Store::open(&db)?;
    run(
        &mut console,
        Command::AddArea(NewArea {
            title: "Семья".into(),
        }),
    )?;
    run(
        &mut console,
        Command::AddProject(NewProject {
            title: "Поездка".into(),
            area: Some("Семья".into()),
        }),
    )?;
    let mut tagged = task("Ёлочный план");
    tagged.when = Some("сегодня".into());
    tagged.notes = Some("Видеосвязь".into());
    tagged.tags = vec!["Праздник".into()];
    let added = execute(&mut console, &Command::AddTask(tagged), clock())?;
    let Effect::Added(id) = added.effect else {
        return Err("нет задачи".into());
    };
    let mut target = console.read().task(id)?.ok_or("нет задачи")?;
    target.checklist.push(ChecklistItem::new(
        ChecklistItemId::from_u128(6),
        "Маршрут",
        Rank::initial(),
        clock().now,
    ));
    console.write(|w| w.update_task(&target))?;
    let mut inherited = task("Оформить билеты");
    inherited.when = Some("сегодня".into());
    inherited.project = Some("Поездка".into());
    run(&mut console, Command::AddTask(inherited))?;
    let mut plain = task("Проверить почту");
    plain.when = Some("сегодня".into());
    run(&mut console, Command::AddTask(plain))?;
    let mut vanished = task("Ёлочный архив");
    vanished.when = Some("сегодня".into());
    let hidden = execute(&mut console, &Command::AddTask(vanished), clock())?;
    let Effect::Added(hidden) = hidden.effect else {
        return Err("нет задачи".into());
    };
    let old = console.read().task(hidden)?.ok_or("нет задачи")?;
    run(
        &mut console,
        Command::SetTrash {
            record: RecordId::Task(hidden),
            expected_updated_at: old.sync.updated_at,
            trashed: true,
        },
    )?;
    let parent = console
        .read()
        .tags()?
        .into_iter()
        .find(|t| t.title == "Праздник")
        .ok_or("нет тега")?;
    let mut child = Tag::new(TagId::from_u128(7), "Зима", Rank::initial(), clock().now);
    child.parent = Some(parent.id);
    let mut area = console
        .read()
        .areas()?
        .into_iter()
        .next()
        .ok_or("нет области")?;
    area.tags.push(child.id);
    console.write(|w| {
        w.insert_tag(&child)?;
        w.update_area(&area)
    })?;

    let window = AppWindow::new()?;
    window.window().set_size(LogicalSize::new(1000.0, 760.0));
    let calendar = Calendar {
        fixed_today: Some(TODAY),
    };
    let hold = Rc::new(Cell::new(false));
    let backend = OutOfOrder {
        inline: Inline::new(App::new(Store::open(&db)?), calendar),
        hold: Rc::clone(&hold),
        searches: Vec::new(),
    };
    let bound = Bound::attach(&window, Box::new(backend), calendar);
    window.show()?;
    window.invoke_focus_list();
    press(&window, "е");
    assert!(window.get_search_active());
    assert_eq!(window.get_search_text(), "е");
    press(&window, "л");
    assert_eq!(window.get_search_text(), "ел");
    mock_elapsed_time(Duration::from_millis(150));
    assert_eq!(titles(&window), ["Ёлочный план"]);
    press(&window, Key::Return);
    assert_eq!(window.get_edit_task_id(), target.id.to_string());
    press(&window, Key::Escape);
    assert!(window.get_search_active());
    assert!(window.get_edit_key().is_empty());

    hold.set(true);
    query(&window, "ПОЕЗДКА")?;
    query(&window, "видеосвязь")?;
    hold.set(false);
    bound.poll();
    assert_eq!(titles(&window), ["Ёлочный план"]);
    query(&window, "МАРШРУТ")?;
    assert_eq!(titles(&window), ["Ёлочный план"]);
    run(&mut console, Command::AddTask(task("Маршрут поезда")))?;
    press(&window, Key::F5);
    assert_eq!(titles(&window), ["Маршрут поезда", "Ёлочный план"]);
    press(&window, Key::DownArrow);
    press(&window, Key::Return);
    assert_eq!(window.get_edit_task_id(), target.id.to_string());
    press(&window, Key::Escape);
    press(&window, Key::Escape);
    assert!(!window.get_search_active());
    assert_eq!(window.get_page_title(), "Сегодня");

    let option = window
        .get_tag_filter_items()
        .iter()
        .position(|v| v == "Праздник")
        .ok_or("нет варианта фильтра")?;
    window.invoke_tag_filter(i32::try_from(option)?);
    assert_eq!(titles(&window), ["Ёлочный план", "Оформить билеты"]);
    assert!(window.get_rows().iter().all(|r| !r.draggable));
    window.invoke_search_begin();
    query(&window, "праздник")?;
    press(&window, Key::Return);
    assert!(!window.get_search_active());
    assert_eq!(window.get_page_title(), "Праздник");
    let results = titles(&window);
    assert!(results.iter().any(|title| title == "Поездка"));
    assert!(results.iter().any(|title| title == "Ёлочный план"));
    assert!(results.iter().any(|title| title == "Оформить билеты"));
    assert!(!results.iter().any(|title| title == "Проверить почту"));
    Ok(())
}
