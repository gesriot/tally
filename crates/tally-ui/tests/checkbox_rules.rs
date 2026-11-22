//! Снятие отметки отменяет только своё закрытие, удержание строки – только при
//! том же посещении страницы, отметка выглядит нажимаемой ровно тогда, когда
//! щелчок что-то сделает, закрытые и удалённые проекты не открываются.

mod common;

use common::{Result, TODAY, clock, id_of, run, seed, task};
use tally_app::{App, Command};
use tally_core::{ProjectId, Status};
use tally_store::{RecordId, Store};
use tally_ui::backend::{Backend, Calendar, Inline, Request};
use tally_ui::controller::{Controller, Selection};
use tally_ui::strings;
use tally_ui::view::{Mark, Row, RowKind};

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
        w.send(Request::Load);
        w
    }

    fn send(&mut self, request: Request) {
        self.backend.submit(request);
        self.drain();
    }

    fn drain(&mut self) {
        for reply in self.backend.replies() {
            self.c.apply(reply);
        }
    }

    fn row(&mut self, title: &str) -> Result<Row> {
        Ok(self
            .c
            .page(TODAY)
            .rows
            .into_iter()
            .find(|r| r.title == title)
            .ok_or_else(|| format!("нет строки «{title}»"))?)
    }

    fn click(&mut self, title: &str) -> Result<bool> {
        let key = self.row(title)?.key;
        Ok(match self.c.toggle(&key) {
            Some(r) => {
                self.send(r);
                true
            }
            None => false,
        })
    }

    fn message(&self) -> Option<String> {
        self.c.message().map(|m| m.text.clone())
    }
}

fn shared() -> Result<(tempfile::TempDir, Store, Window)> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let mut console = Store::open(&path)?;
    seed(&mut console)?;
    let w = Window::open(Store::open(&path)?);
    Ok((dir, console, w))
}

/// Консоль добавила задачу после закрытия в окне: снятие отметки не
/// проходит, окно говорит почему, отметка больше не выглядит нажимаемой.
#[test]
fn unchecking_after_a_console_action_says_why_and_disables_the_mark() -> Result {
    let (_dir, mut console, mut w) = shared()?;
    assert!(w.click("Купить плитку")?);
    run(&mut console, Command::AddTask(task("Заказать доставку")))?;
    let history = console.read().history()?;
    assert!(w.row("Купить плитку")?.checkable);
    assert!(w.click("Купить плитку")?);
    assert_eq!(w.message().as_deref(), Some(strings::REOPEN_NOT_LAST));
    assert_eq!(
        strings::REOPEN_NOT_LAST,
        "После выполнения задачи было другое действие"
    );
    assert_eq!(console.read().history()?, history);
    let row = w.row("Купить плитку")?;
    assert_eq!(row.mark, Mark::Completed);
    assert!(!row.checkable);
    assert!(!w.click("Купить плитку")?);
    Ok(())
}

/// Уйти и быстро вернуться до ответа базы – тоже переход: строку не
/// удерживать, задача уже в «Журнале».
#[test]
fn leaving_and_returning_before_the_reply_counts_as_leaving() -> Result {
    let mut store = Store::open_in_memory()?;
    seed(&mut store)?;
    let id = id_of(&store.read().snapshot()?, "Купить плитку")?;
    let mut w = Window::open(store);
    let request = w.c.toggle(&id.to_string()).ok_or("нет закрытия")?;
    w.backend.submit(request);
    assert!(w.c.select("inbox"));
    assert!(w.c.select("today"));
    w.drain();
    assert!(w.row("Купить плитку").is_err());
    // Снимок принят: в «Журнале» задача есть и выполнена.
    assert!(w.c.select("logbook"));
    let row = w.row("Купить плитку")?;
    assert_eq!(row.mark, Mark::Completed);
    assert!(!row.checkable);
    Ok(())
}

/// Без перехода запоздалый ответ удерживает строку, как обычно.
#[test]
fn a_late_reply_on_the_same_visit_keeps_the_row() -> Result {
    let mut store = Store::open_in_memory()?;
    seed(&mut store)?;
    let id = id_of(&store.read().snapshot()?, "Купить плитку")?;
    let mut w = Window::open(store);
    let request = w.c.toggle(&id.to_string()).ok_or("нет закрытия")?;
    w.backend.submit(request);
    // Выбор того же списка – не переход.
    assert!(!w.c.select("today"));
    w.drain();
    let row = w.row("Купить плитку")?;
    assert_eq!(row.mark, Mark::Completed);
    assert!(row.checkable);
    Ok(())
}

/// Пока ответа на отметку нет, второй щелчок не уходит, и отметка не
/// выглядит нажимаемой.
#[test]
fn a_pending_mark_is_not_clickable() -> Result {
    let mut store = Store::open_in_memory()?;
    seed(&mut store)?;
    let id = id_of(&store.read().snapshot()?, "Купить плитку")?.to_string();
    let mut w = Window::open(store);
    let first = w.c.toggle(&id).ok_or("нет закрытия")?;
    assert!(!w.row("Купить плитку")?.checkable);
    assert!(w.c.toggle(&id).is_none());
    w.send(first);
    assert!(w.row("Купить плитку")?.checkable);
    assert_eq!(w.message(), None);
    Ok(())
}

/// Отметка задачи в корзине – самой или через проект – не нажимается.
#[test]
fn trashed_tasks_are_not_clickable() -> Result {
    let mut store = Store::open_in_memory()?;
    seed(&mut store)?;
    let s = store.read().snapshot()?;
    let repair = project_id(&store, "Ремонт")?;
    let tile = id_of(&s, "Купить плитку")?;
    let mom = id_of(&s, "Позвонить маме")?;
    store.write(|w| {
        w.delete(RecordId::Project(repair), clock().now)?;
        w.delete(RecordId::Task(mom), clock().now)
    })?;
    let mut w = Window::open(store);
    assert!(w.c.toggle(&tile.to_string()).is_none());
    assert!(w.c.select("trash"));
    let row = w.row("Позвонить маме")?;
    assert_eq!(row.mark, Mark::Open);
    assert!(!row.checkable);
    assert!(w.c.toggle(&mom.to_string()).is_none());
    Ok(())
}

fn project_id(store: &Store, title: &str) -> Result<ProjectId> {
    Ok(store
        .read()
        .snapshot()?
        .projects
        .iter()
        .find(|p| p.title == title)
        .ok_or("нет проекта")?
        .id)
}

/// Закрытый проект в «Журнале» и удалённый в «Корзине» – строки без
/// перехода; переход по их ключу тоже ничего не делает.
#[test]
fn closed_and_trashed_projects_do_not_open() -> Result {
    let mut store = Store::open_in_memory()?;
    seed(&mut store)?;
    let repair = project_id(&store, "Ремонт")?;
    let trip = project_id(&store, "Отпуск")?;
    let mut closed = store.read().project(repair)?.ok_or("нет проекта")?;
    closed.status = Status::Completed { at: clock().now };
    store.write(|w| {
        w.update_project(&closed)?;
        w.delete(RecordId::Project(trip), clock().now)
    })?;
    let mut w = Window::open(store);
    for (list, title, key) in [("logbook", "Ремонт", repair), ("trash", "Отпуск", trip)]
    {
        assert!(w.c.select(list));
        let row = w.row(title)?;
        assert_eq!(row.kind, RowKind::Project);
        assert!(!row.opens, "{title}");
        assert!(!w.c.select(&row.key));
        assert_eq!(row.key, Selection::Project(key).key());
        assert_eq!(w.c.page(TODAY).title, w.c.selection().title());
    }
    Ok(())
}

/// Проект в работе (на странице области и в «Когда-нибудь») открывается.
#[test]
fn live_projects_open() -> Result {
    let mut store = Store::open_in_memory()?;
    seed(&mut store)?;
    let s = store.read().snapshot()?;
    let home = s
        .areas
        .iter()
        .find(|a| a.title == "Дом")
        .ok_or("нет области")?
        .id;
    let mut w = Window::open(store);
    assert!(w.c.select(&Selection::Area(home).key()));
    let row = w.row("Ремонт")?;
    assert!(row.opens);
    assert!(w.c.select(&row.key));
    assert_eq!(w.c.page(TODAY).title, "Ремонт");
    Ok(())
}
