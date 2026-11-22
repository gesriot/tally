//! Карточка в настоящем дереве Slint, общий журнал и черновик при конфликте.

mod common;

use std::cell::Cell;
use std::rc::Rc;

use common::{Result, TODAY, clock, find, run, seed, task};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalSize, Model, SharedString};
use tally_app::{App, Clock, Close, Command, MoveTask, execute};
use tally_core::Status;
use tally_store::Store;
use tally_ui::backend::{Backend, Calendar, Inline, Reply, Request};
use tally_ui::bind::Bound;
use tally_ui::ui::AppWindow;

fn by_label(window: &AppWindow, label: &str, role: AccessibleRole) -> Result<ElementHandle> {
    Ok(ElementHandle::find_by_accessible_label(window, label)
        .find(|element| element.accessible_role() == Some(role))
        .ok_or_else(|| format!("нет элемента «{label}»"))?)
}

fn press(window: &AppWindow, key: impl Into<SharedString>, control: bool) {
    let key = key.into();
    let control_key: SharedString = Key::Control.into();
    if control {
        window.window().dispatch_event(WindowEvent::KeyPressed {
            text: control_key.clone(),
        });
    }
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: key.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text: key });
    if control {
        window
            .window()
            .dispatch_event(WindowEvent::KeyReleased { text: control_key });
    }
}

struct Delayed {
    inner: Inline<App>,
    hold: Rc<Cell<bool>>,
}

impl Backend for Delayed {
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

#[test]
fn edit_card_saves_all_fields_and_keeps_a_conflicting_draft() -> Result {
    i_slint_backend_testing::init_integration_test_with_mock_time();
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let mut console = Store::open(&path)?;
    seed(&mut console)?;
    let window = AppWindow::new()?;
    window.window().set_size(LogicalSize::new(1000.0, 760.0));
    let calendar = Calendar {
        fixed_today: Some(TODAY),
    };
    let hold = Rc::new(Cell::new(false));
    let backend = Delayed {
        inner: Inline::new(App::new(Store::open(&path)?), calendar),
        hold: Rc::clone(&hold),
    };
    let bound = Bound::attach(&window, Box::new(backend), calendar);
    window.show()?;

    // Стрелка выделяет задачу, Enter раскрывает карточку в списке.
    press(&window, Key::DownArrow, false);
    let selected = window.get_selected_task();
    assert!(!selected.is_empty());
    press(&window, Key::Return, false);
    assert_eq!(window.get_edit_key(), selected);
    by_label(&window, "Название задачи", AccessibleRole::TextInput)?;

    press(&window, Key::Escape, false);
    assert!(window.get_edit_key().is_empty());
    let old = find(&console.read().snapshot()?, "Купить плитку")?.clone();
    by_label(&window, &old.title, AccessibleRole::ListItem)?.invoke_accessible_default_action();
    assert_eq!(window.get_edit_title(), old.title.as_str());
    by_label(&window, "Название задачи", AccessibleRole::TextInput)?
        .set_accessible_value("Купить плитку и клей");
    by_label(&window, "Заметки", AccessibleRole::TextInput)?
        .set_accessible_value("Взять образец\nСверить размеры");
    window.set_new_check("Позвонить в магазин".into());
    window.invoke_edit_add_check();
    window.invoke_edit_check_done(0, true);
    window.set_new_tag("Ремонт".into());
    window.invoke_edit_add_tag();
    assert!(
        !window
            .invoke_edit_date(true, "сегодня вечером".into())
            .is_empty()
    );
    assert!(
        window
            .invoke_edit_date(false, "сегодня вечером".into())
            .is_empty()
    );
    assert!(window.invoke_edit_date(true, "завтра".into()).is_empty());
    let next_day = TODAY.checked_add(jiff::Span::new().days(1))?;
    let saved_when = tally_when::parse(&window.get_when_value(), next_day)?;
    assert_eq!(
        saved_when,
        tally_when::When::On {
            date: TODAY,
            evening: true
        }
    );
    let saved_deadline = tally_when::parse(&window.get_deadline_value(), next_day)?;
    assert_eq!(saved_deadline.date(), Some(next_day));
    let history_before = console.read().history()?.len();
    hold.set(true);
    press(&window, Key::Return, true);
    assert!(window.get_edit_busy(), "{}", window.get_edit_error());
    window.invoke_edit_finish(true);
    assert_eq!(console.read().history()?.len(), history_before + 1);
    hold.set(false);
    bound.poll();
    assert!(
        window.get_edit_key().is_empty(),
        "{}",
        window.get_edit_error()
    );

    let snapshot = console.read().snapshot()?;
    let updated = find(&snapshot, "Купить плитку и клей")?;
    assert_eq!(updated.notes, "Взять образец\nСверить размеры");
    assert!(updated.evening);
    assert!(updated.checklist[0].done);
    assert_eq!(updated.tags.len(), 1);
    assert_eq!(
        updated.deadline,
        Some(TODAY.checked_add(jiff::Span::new().days(1))?)
    );

    window.invoke_edit_task(updated.id.to_string().into());
    window.set_edit_notes("Мой черновик".into());
    run(
        &mut console,
        Command::MoveTask(MoveTask {
            task: updated.id.to_string(),
            when: Some("завтра".into()),
            ..MoveTask::default()
        }),
    )?;
    press(&window, Key::F5, false);
    window.invoke_edit_finish(true);
    assert_eq!(window.get_edit_notes(), "Мой черновик");
    assert!(!window.get_edit_key().is_empty());
    assert!(window.get_edit_error().contains("Задача изменилась"));
    window.invoke_edit_finish(false);
    assert!(window.get_edit_key().is_empty());
    assert!(by_label(&window, &updated.title, AccessibleRole::ListItem).is_err());

    // Один TaskId в двух разных строках. Карточка раскрывается только у срока.
    let mut preparation = task("Подготовка");
    preparation.when = Some("завтра".into());
    preparation.deadline = Some("послезавтра".into());
    run(&mut console, Command::AddTask(preparation))?;
    window.invoke_select("upcoming".into());
    press(&window, Key::F5, false);
    let rows = window.get_rows();
    let occurrences: Vec<_> = rows
        .iter()
        .filter(|row| row.title == "Подготовка")
        .collect();
    let [start, deadline] = occurrences.as_slice() else {
        return Err("в «Планах» нужны две строки задачи".into());
    };
    assert_eq!(start.key, deadline.key);
    assert_ne!(start.row_key, deadline.row_key);
    window.invoke_edit_task(deadline.row_key.clone());
    assert_eq!(window.get_edit_key(), deadline.row_key);
    assert_eq!(
        ElementHandle::find_by_accessible_label(&window, "Название задачи")
            .filter(|element| element.accessible_role() == Some(AccessibleRole::TextInput))
            .count(),
        1
    );
    window.invoke_edit_finish(false);

    // Вчерашнее завершение и отмена возвращаются в работу прямо из «Журнала».
    let yesterday = clock().now - jiff::SignedDuration::from_hours(24);
    for (title, cancel) in [("Выполнено вчера", false), ("Отменено вчера", true)]
    {
        run(&mut console, Command::AddTask(task(title)))?;
        execute(
            &mut console,
            &Command::Close(Close {
                tasks: vec![title.into()],
                cancel,
            }),
            Clock {
                now: yesterday,
                ..clock()
            },
        )?;
        let before = console.read().snapshot()?;
        let old = find(&before, title)?;
        let history_before = console.read().history()?.len();
        window.invoke_select("logbook".into());
        press(&window, Key::F5, false);
        window.invoke_edit_task(old.id.to_string().into());
        window.set_edit_notes("Новый комментарий".into());
        by_label(&window, "Открыть заново", AccessibleRole::Button)?
            .invoke_accessible_default_action();
        assert!(window.get_edit_key().is_empty());
        assert!(by_label(&window, title, AccessibleRole::ListItem).is_err());
        let reopened = console.read().task(old.id)?.ok_or("нет задачи")?;
        assert_eq!(reopened.status, Status::Open);
        assert_eq!(reopened.notes, "Новый комментарий");
        assert_eq!(console.read().history()?.len(), history_before + 1);
        press(&window, "z", true);
        let undone = console.read().task(old.id)?.ok_or("нет задачи")?;
        assert_eq!(undone.status, old.status);
        assert_eq!(undone.notes, old.notes);
    }
    Ok(())
}
