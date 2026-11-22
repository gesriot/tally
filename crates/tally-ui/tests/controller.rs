//! Состояние окна: отметка, отмена и повтор, сообщения, общая с консолью
//! база.

mod common;

use common::{Result, TODAY, clock, id_of, run, seed, task};
use tally_app::{App, Clock, Close, Command, Effect, NewTask, Service, execute};
use tally_core::Status;
use tally_store::{RecordId, Store};
use tally_ui::backend::{Backend, Calendar, Inline, Purpose, Request};
use tally_ui::controller::{Controller, Selection};
use tally_ui::keys::Press;
use tally_ui::view::Mark;

/// Окно без Slint: контроллер и мгновенный поток базы.
struct Window {
    c: Controller,
    backend: Inline<App>,
}

impl Window {
    fn open(store: Store) -> Self {
        let calendar = Calendar {
            fixed_today: Some(TODAY),
        };
        let mut w = Self {
            c: Controller::new(),
            backend: Inline::new(App::new(store), calendar),
        };
        w.send(Some(Request::Load));
        w
    }

    fn send(&mut self, request: Option<Request>) -> bool {
        let Some(request) = request else {
            return false;
        };
        self.backend.submit(request);
        for reply in self.backend.replies() {
            self.c.apply(reply);
        }
        true
    }

    fn toggle(&mut self, title: &str) -> Result<bool> {
        let id = self.row(title)?.key.clone();
        let request = self.c.toggle(&id);
        Ok(self.send(request))
    }

    fn press(&mut self, text: &str, control: bool, shift: bool) -> bool {
        let request = self.c.key(Press {
            text,
            control,
            shift,
            ..Press::default()
        });
        self.send(request)
    }

    fn row(&mut self, title: &str) -> Result<tally_ui::view::Row> {
        Ok(self
            .c
            .page(TODAY)
            .rows
            .into_iter()
            .find(|r| r.title == title)
            .ok_or_else(|| format!("нет строки «{title}»"))?)
    }

    fn titles(&mut self) -> Vec<String> {
        self.c
            .page(TODAY)
            .rows
            .into_iter()
            .map(|r| r.title)
            .collect()
    }

    fn message(&self) -> Option<String> {
        self.c.message().map(|m| m.text.clone())
    }
}

fn seeded_window() -> Result<Window> {
    let mut store = Store::open_in_memory()?;
    seed(&mut store)?;
    Ok(Window::open(store))
}

#[test]
fn checking_closes_and_unchecking_undoes_that_close() -> Result {
    let mut w = seeded_window()?;
    assert_eq!(w.c.selection(), Selection::Today);
    assert!(w.toggle("Позвонить электрику")?);
    let row = w.row("Позвонить электрику")?;
    assert_eq!(row.mark, Mark::Completed);
    assert!(row.checkable);
    assert_eq!(w.message(), None);
    // Снять отметку – отменить закрытие, без сообщения.
    assert!(w.toggle("Позвонить электрику")?);
    assert_eq!(w.row("Позвонить электрику")?.mark, Mark::Open);
    assert_eq!(w.message(), None);
    // Снятие отметки отменило только закрытие: следующая отмена – уже
    // последнее действие до него.
    assert!(w.press("z", true, false));
    assert_eq!(
        w.message().as_deref(),
        Some("Отменено: добавление задачи «Посмотреть фильм»")
    );
    Ok(())
}

#[test]
fn only_the_last_close_of_this_window_can_be_unchecked() -> Result {
    let mut w = seeded_window()?;
    w.toggle("Позвонить электрику")?;
    w.toggle("Купить плитку")?;
    // Последнее действие – закрытие «Купить плитку»: снять отметку с
    // «Позвонить электрику» нельзя (это была бы отмена чужого действия).
    assert!(!w.toggle("Позвонить электрику")?);
    assert_eq!(w.row("Позвонить электрику")?.mark, Mark::Completed);
    assert!(w.toggle("Купить плитку")?);
    assert_eq!(w.row("Купить плитку")?.mark, Mark::Open);
    Ok(())
}

#[test]
fn ctrl_z_and_ctrl_shift_z_undo_and_redo_with_a_message() -> Result {
    let mut w = seeded_window()?;
    w.toggle("Оплатить интернет")?;
    assert!(w.press("z", true, false));
    assert_eq!(w.row("Оплатить интернет")?.mark, Mark::Open);
    assert_eq!(
        w.message().as_deref(),
        Some("Отменено: выполнение задачи «Оплатить интернет»")
    );
    assert!(w.press("Z", true, true));
    // Задача снова закрыта и, пока открыт тот же список, видна с отметкой.
    assert_eq!(w.row("Оплатить интернет")?.mark, Mark::Completed);
    assert_eq!(
        w.message().as_deref(),
        Some("Повторено: выполнение задачи «Оплатить интернет»")
    );
    assert!(w.press("Z", true, true));
    assert_eq!(w.message().as_deref(), Some("Нечего повторять"));
    // Другие клавиши окно не забирает.
    assert!(!w.press("z", false, false));
    Ok(())
}

#[test]
fn closing_in_the_console_after_unchecking_moves_the_task_to_logbook() -> Result {
    let title = "Отчёт";
    let f5 = slint::SharedString::from(slint::platform::Key::F5);
    for cancel in [true, false] {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("tally.db");
        let mut console = Store::open(&path)?;
        run(&mut console, Command::AddTask(task("Отчёт сегодня")))?;
        let id = id_of(&console.read().snapshot()?, title)?;
        let mut w = Window::open(Store::open(&path)?);

        assert!(w.toggle(title)?);
        let at = console
            .read()
            .task(id)?
            .and_then(|t| t.status.closed_at())
            .ok_or("задача не закрыта")?;
        assert!(w.toggle(title)?);
        assert_eq!(w.row(title)?.mark, Mark::Open);

        // У нового закрытия может быть та же метка времени, что у прежнего.
        execute(
            &mut console,
            &Command::Close(Close {
                tasks: vec![title.into()],
                cancel,
            }),
            Clock { now: at, ..clock() },
        )?;
        assert!(w.press(&f5, false, false));

        assert!(!w.titles().iter().any(|t| t == title));
        assert!(w.c.select("logbook"));
        let mark = if cancel {
            Mark::Canceled
        } else {
            Mark::Completed
        };
        assert_eq!(w.row(title)?.mark, mark);
    }
    Ok(())
}

#[test]
fn redo_after_unchecking_restores_the_mark_on_the_same_page() -> Result {
    let mut w = seeded_window()?;
    let title = "Оплатить интернет";
    assert!(w.toggle(title)?);
    assert!(w.toggle(title)?);
    assert_eq!(w.row(title)?.mark, Mark::Open);

    let f5 = slint::SharedString::from(slint::platform::Key::F5);
    assert!(w.press(&f5, false, false));
    assert!(w.press("Z", true, true));

    assert_eq!(w.row(title)?.mark, Mark::Completed);
    assert_eq!(
        w.message().as_deref(),
        Some("Повторено: выполнение задачи «Оплатить интернет»")
    );
    Ok(())
}

#[test]
fn closed_tasks_leave_when_the_list_changes() -> Result {
    let mut w = seeded_window()?;
    w.toggle("Позвонить электрику")?;
    assert!(w.c.select("inbox"));
    assert!(!w.c.select("inbox"));
    assert!(w.c.select("today"));
    assert!(!w.titles().contains(&"Позвонить электрику".to_owned()));
    // Снять отметку тоже уже нельзя – задачи на месте нет.
    Ok(())
}

#[test]
fn messages_hide_by_serial() -> Result {
    let mut w = Window::open(Store::open_in_memory()?);
    w.press("z", true, false);
    assert_eq!(w.message().as_deref(), Some("Нечего отменять"));
    let first = w.c.message().cloned().ok_or("нет сообщения")?;
    w.press("Z", true, true);
    let second = w.c.message().cloned().ok_or("нет сообщения")?;
    assert!(second.serial > first.serial);
    // Таймер первого сообщения не прячет второе.
    w.c.hide_message(first.serial);
    assert_eq!(w.message().as_deref(), Some("Нечего повторять"));
    w.c.hide_message(second.serial);
    assert_eq!(w.message(), None);
    Ok(())
}

/// База общая с консолью: изменения другого соединения видны после F5, а
/// отмена, которой мешает чужое изменение, сообщает конфликт.
#[test]
fn the_database_is_shared_with_the_console() -> Result {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let mut console = Store::open(&path)?;
    seed(&mut console)?;
    let mut w = Window::open(Store::open(&path)?);
    w.toggle("Купить плитку")?;

    // Консоль удаляет задачу в обход журнала – отменять её закрытие
    // нельзя, окно сообщает причину от `tally-app`.
    let tile = id_of(&console.read().snapshot()?, "Купить плитку")?;
    console.write(|wr| wr.delete(RecordId::Task(tile), clock().now))?;
    assert!(w.press("z", true, false));
    let message = w.message().ok_or("нет сообщения")?;
    assert!(
        message.starts_with("Нельзя отменить выполнение задачи «Купить плитку»: "),
        "{message}"
    );
    let status = console.read().task(tile)?.ok_or("нет")?.status;
    assert!(matches!(status, Status::Completed { .. }));

    // Новая задача из консоли видна после F5.
    run(
        &mut console,
        Command::AddTask(NewTask {
            text: "Из консоли".into(),
            ..NewTask::default()
        }),
    )?;
    w.c.select("inbox");
    assert!(!w.titles().contains(&"Из консоли".to_owned()));
    let f5 = slint::SharedString::from(slint::platform::Key::F5);
    assert!(w.press(&f5, false, false));
    assert!(w.titles().contains(&"Из консоли".to_owned()));
    Ok(())
}

/// Ответ на снятие отметки, если журнал успел измениться (например,
/// консоль выполнила своё действие между нажатиями): отменилось чужое
/// действие – окно об этом сообщает.
#[test]
fn an_unexpected_undo_is_reported() -> Result {
    let mut store = Store::open_in_memory()?;
    seed(&mut store)?;
    let mut app = App::new(store);
    let done = app.execute(&Command::Undo, clock())?;
    let Effect::Undone(step) = done.effect.clone() else {
        return Err("ожидалась отмена".into());
    };
    let mut c = Controller::new();
    let id = done.snapshot.tasks[0].id;
    c.apply(tally_ui::backend::Reply::Ran(Purpose::Reopen(id), Ok(done)));
    assert_eq!(
        c.message().map(|m| m.text.clone()),
        Some(format!("Отменено: {}", step.action))
    );
    Ok(())
}
