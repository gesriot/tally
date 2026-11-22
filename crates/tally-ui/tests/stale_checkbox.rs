//! Устаревшая отметка, повтор снятия после конфликта, возврат в «Сегодня»,
//! отметка в пути, Ctrl+Z управляющим символом.

mod common;

use std::cell::Cell;
use std::rc::Rc;

use common::{Result, TODAY, clock, id_of, run, seed, task};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use slint::{ComponentHandle, LogicalSize};
use tally_app::{App, Command};
use tally_core::{Status, TaskId};
use tally_store::{RecordId, Store};
use tally_ui::backend::{Backend, Calendar, Inline, Reply, Request};
use tally_ui::bind::Bound;
use tally_ui::controller::{Controller, Selection};
use tally_ui::keys::{Action, Press, action};
use tally_ui::ui::AppWindow;

fn calendar() -> Calendar {
    Calendar {
        fixed_today: Some(TODAY),
    }
}

struct Window {
    controller: Controller,
    backend: Inline<App>,
}

impl Window {
    fn open(store: Store) -> Self {
        let mut window = Self {
            controller: Controller::new(),
            backend: Inline::new(App::new(store), calendar()),
        };
        window.send(Request::Load);
        window
    }

    fn send(&mut self, request: Request) {
        self.backend.submit(request);
        for reply in self.backend.replies() {
            self.controller.apply(reply);
        }
    }

    fn click(&mut self, id: TaskId) -> Result {
        let request = self
            .controller
            .toggle(&id.to_string())
            .ok_or("отметка недоступна")?;
        self.send(request);
        Ok(())
    }
}

#[test]
fn stale_checkbox_cannot_close_another_task_by_title() -> Result {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let mut console = Store::open(&path)?;
    run(&mut console, Command::AddTask(task("Отчёт сегодня")))?;
    let id = id_of(&console.read().snapshot()?, "Отчёт")?;
    let mut window = Window::open(Store::open(&path)?);
    let today = window.controller.page(TODAY);
    assert!(today.rows.iter().any(|r| r.key == id.to_string()));

    run(&mut console, Command::Undo)?;
    let reference = format!("Проверить ссылку {id} вручную");
    run(&mut console, Command::AddTask(task(&reference)))?;
    let expected = console.read().snapshot()?;
    let history = console.read().history()?;
    window.click(id)?;
    assert_eq!(console.read().snapshot()?, expected);
    assert_eq!(console.read().history()?, history);
    Ok(())
}

#[test]
fn unchecking_can_be_retried_after_the_conflict_is_resolved() -> Result {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let mut console = Store::open(&path)?;
    seed(&mut console)?;
    let id = id_of(&console.read().snapshot()?, "Купить плитку")?;
    let mut window = Window::open(Store::open(&path)?);
    window.click(id)?;
    let history = console.read().history()?;
    console.write(|w| w.delete(RecordId::Task(id), clock().now))?;
    window.click(id)?;
    assert!(window.controller.message().is_some());
    assert_eq!(console.read().history()?, history);

    console.write(|w| w.restore(RecordId::Task(id), clock().now))?;
    window.send(Request::Load);
    window.click(id)?;
    let reopened = console.read().task(id)?.ok_or("нет задачи")?;
    assert_eq!(reopened.status, Status::Open);
    Ok(())
}

#[test]
fn automatic_return_to_today_finishes_the_old_visit() -> Result {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let mut console = Store::open(&path)?;
    seed(&mut console)?;
    let s = console.read().snapshot()?;
    let id = id_of(&s, "Купить плитку")?;
    let project = s
        .projects
        .iter()
        .find(|p| p.title == "Ремонт")
        .ok_or("нет проекта")?
        .id;
    let mut window = Window::open(Store::open(&path)?);
    assert!(window.controller.select(&Selection::Project(project).key()));
    window.click(id)?;

    console.write(|w| w.delete(RecordId::Project(project), clock().now))?;
    window.send(Request::Load);
    assert_eq!(window.controller.page(TODAY).title, "Сегодня");
    console.write(|w| w.restore(RecordId::Project(project), clock().now))?;
    window.send(Request::Load);
    let today = window.controller.page(TODAY);
    assert!(!today.rows.iter().any(|r| r.key == id.to_string()));
    assert!(window.controller.select("logbook"));
    let logbook = window.controller.page(TODAY);
    assert!(logbook.rows.iter().any(|r| r.key == id.to_string()));
    Ok(())
}

struct DelayedReplies {
    inner: Inline<App>,
    hold: Rc<Cell<bool>>,
}

impl Backend for DelayedReplies {
    fn submit(&mut self, request: Request) {
        self.inner.submit(request);
    }

    fn replies(&mut self) -> Vec<Reply> {
        if self.hold.get() {
            Vec::new()
        } else {
            self.inner.replies()
        }
    }
}

fn checkbox(window: &AppWindow, title: &str) -> Result<ElementHandle> {
    Ok(ElementHandle::find_by_accessible_label(window, title)
        .find(|e| e.accessible_role() == Some(AccessibleRole::Checkbox))
        .ok_or("нет отметки")?)
}

#[test]
fn pending_close_disables_the_slint_control_before_the_reply() -> Result {
    i_slint_backend_testing::init_integration_test_with_mock_time();
    let mut store = Store::open_in_memory()?;
    seed(&mut store)?;
    let hold = Rc::new(Cell::new(false));
    let backend = DelayedReplies {
        inner: Inline::new(App::new(store), calendar()),
        hold: Rc::clone(&hold),
    };
    let window = AppWindow::new()?;
    window.window().set_size(LogicalSize::new(1000.0, 680.0));
    let bound = Bound::attach(&window, Box::new(backend), calendar());
    window.show()?;
    let title = "Купить плитку";
    assert_eq!(checkbox(&window, title)?.accessible_enabled(), Some(true));
    hold.set(true);
    checkbox(&window, title)?.invoke_accessible_default_action();
    assert_eq!(checkbox(&window, title)?.accessible_enabled(), Some(false));
    hold.set(false);
    bound.poll();
    assert_eq!(checkbox(&window, title)?.accessible_checked(), Some(true));
    assert_eq!(checkbox(&window, title)?.accessible_enabled(), Some(true));
    Ok(())
}

#[test]
fn ctrl_z_accepts_the_previously_supported_control_character() {
    for (shift, expected) in [(false, Action::Undo), (true, Action::Redo)] {
        let key = Press {
            text: "\u{001a}",
            control: true,
            shift,
            ..Press::default()
        };
        assert_eq!(action(key), Some(expected));
    }
}

#[test]
fn stale_checkbox_leaves_trashed_tasks_and_history_unchanged() -> Result {
    for delete_project in [false, true] {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("tally.db");
        let mut console = Store::open(&path)?;
        seed(&mut console)?;
        let snapshot = console.read().snapshot()?;
        let id = id_of(&snapshot, "Купить плитку")?;
        let project = snapshot
            .projects
            .iter()
            .find(|p| p.title == "Ремонт")
            .ok_or("нет проекта")?
            .id;
        let mut window = Window::open(Store::open(&path)?);
        let record = if delete_project {
            RecordId::Project(project)
        } else {
            RecordId::Task(id)
        };
        console.write(|w| w.delete(record, clock().now))?;
        let reference = format!("Проверить ссылку {id} вручную");
        run(&mut console, Command::AddTask(task(&reference)))?;
        let expected = console.read().snapshot()?;
        let history = console.read().history()?;

        window.click(id)?;

        assert_eq!(console.read().snapshot()?, expected);
        assert_eq!(console.read().history()?, history);
    }
    Ok(())
}
