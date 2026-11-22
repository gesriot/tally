//! Настоящая клавиша F5, общая история при снятии отметки, ответ после
//! перехода в «Журнал», две отметки подряд.

mod common;

use common::{Result, TODAY, id_of, run, seed, task};
use slint::platform::Key;
use tally_app::{App, Command};
use tally_store::Store;
use tally_ui::backend::{Backend, Calendar, Inline, Request};
use tally_ui::controller::Controller;
use tally_ui::keys::{Action, Press, action};
use tally_ui::view::Mark;

struct Window {
    controller: Controller,
    backend: Inline<App>,
}

impl Window {
    fn open(store: Store) -> Self {
        let mut w = Self {
            controller: Controller::new(),
            backend: Inline::new(
                App::new(store),
                Calendar {
                    fixed_today: Some(TODAY),
                },
            ),
        };
        w.backend.submit(Request::Load);
        w.drain();
        w
    }

    fn drain(&mut self) {
        for reply in self.backend.replies() {
            self.controller.apply(reply);
        }
    }

    fn click(&mut self, key: &str) {
        if let Some(request) = self.controller.toggle(key) {
            self.backend.submit(request);
        }
        self.drain();
    }
}

#[test]
fn refresh_uses_the_slint_f5_key() {
    let decode = |key: Key| {
        let text: slint::SharedString = key.into();
        action(Press {
            text: text.as_str(),
            ..Press::default()
        })
    };
    assert_eq!(
        (decode(Key::F1), decode(Key::F5)),
        (None, Some(Action::Refresh))
    );
}

#[test]
fn unchecking_leaves_a_newer_console_action_and_its_history_intact() -> Result {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let mut console = Store::open(&path)?;
    seed(&mut console)?;
    let id = id_of(&console.read().snapshot()?, "Купить плитку")?;
    let mut w = Window::open(Store::open(&path)?);
    w.click(&id.to_string());
    let status = console.read().task(id)?.ok_or("нет задачи")?.status;
    assert!(!status.is_open());

    run(&mut console, Command::AddTask(task("Добавлена из консоли")))?;
    let expected = console.read().snapshot()?;
    let history = console.read().history()?;
    w.click(&id.to_string());
    assert_eq!(console.read().snapshot()?, expected);
    assert_eq!(console.read().history()?, history);

    // Явная команда отмены по-прежнему работает с общей историей.
    let undo = w
        .controller
        .key(Press {
            text: "z",
            control: true,
            ..Press::default()
        })
        .ok_or("Ctrl+Z не принят")?;
    w.backend.submit(undo);
    w.drain();
    let after = console.read().snapshot()?;
    assert!(
        !after
            .tasks
            .iter()
            .any(|t| t.title == "Добавлена из консоли")
    );
    let status = console.read().task(id)?.ok_or("нет задачи")?.status;
    assert!(!status.is_open());
    Ok(())
}

#[test]
fn leaving_for_logbook_before_close_reply_shows_the_completed_task() -> Result {
    let mut store = Store::open_in_memory()?;
    seed(&mut store)?;
    let id = id_of(&store.read().snapshot()?, "Купить плитку")?;
    let mut w = Window::open(store);
    let request = w
        .controller
        .toggle(&id.to_string())
        .ok_or("нет команды закрытия")?;
    w.backend.submit(request);
    assert!(w.controller.select("logbook"));
    w.drain();
    let row = w
        .controller
        .page(TODAY)
        .rows
        .into_iter()
        .find(|r| r.key == id.to_string())
        .ok_or("в Журнале нет выполненной задачи")?;
    assert_eq!(row.mark, Mark::Completed);
    assert!(!row.checkable);
    Ok(())
}

#[test]
fn completing_two_tasks_leaves_only_the_last_checkbox_enabled() -> Result {
    let mut store = Store::open_in_memory()?;
    seed(&mut store)?;
    let snapshot = store.read().snapshot()?;
    let first = id_of(&snapshot, "Купить плитку")?.to_string();
    let second = id_of(&snapshot, "Позвонить электрику")?.to_string();
    let mut w = Window::open(store);
    w.click(&first);
    w.click(&second);
    let page = w.controller.page(TODAY);
    for (key, enabled) in [(first, false), (second, true)] {
        let row = page
            .rows
            .iter()
            .find(|r| r.key == key)
            .ok_or("нет строки")?;
        assert_eq!(row.mark, Mark::Completed);
        assert_eq!(row.checkable, enabled, "{}", row.title);
    }
    Ok(())
}
