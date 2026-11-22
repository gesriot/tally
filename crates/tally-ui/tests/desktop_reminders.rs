//! Занятая оконная запись не трогает чужие файлы; перезапуск возвращает
//! напоминание, не дошедшее до уведомителя.

use std::cell::Cell;
use std::fs;
use std::rc::Rc;

use jiff::Timestamp;
use tally_app::App;
use tally_client::desktop::{Desktop, Show};
use tally_core::{Rank, Task, TaskId};
use tally_store::Store;
use tally_ui::backend::{Backend, Calendar, Inline, Reply, Request};
use tally_ui::bind::Bound;
use tally_ui::ui::AppWindow;

type Result = std::result::Result<(), Box<dyn std::error::Error>>;

#[test]
fn claiming_a_desktop_preserves_an_unrelated_sibling_file() -> Result {
    let dir = tempfile::tempdir()?;
    let db = dir.path().join("work.db");
    let existing = dir.path().join("work.db.tally-ui.request");
    let original = b"Existing user file, not an IPC mailbox.";
    fs::write(&existing, original)?;

    // Допустимы и отдельное место для IPC, и отказ до изменения файла.
    if let Ok(desktop) = Desktop::for_db(&db) {
        let _claim = desktop.claim(Show::Window);
    }
    assert_eq!(fs::read(&existing)?.as_slice(), original);
    Ok(())
}

// База выполнила запрос. Ответ остаётся в остановленном процессе,
// поэтому код, показывающий уведомления, его заведомо не получил.
struct StopBeforeNotification {
    inner: Inline<App>,
    intercepted: Rc<Cell<bool>>,
}

impl Backend for StopBeforeNotification {
    fn submit(&mut self, request: Request) {
        self.inner.submit(request);
    }

    fn replies(&mut self) -> Vec<Reply> {
        self.inner
            .replies()
            .into_iter()
            .filter(|reply| match reply {
                Reply::Reminders(Ok(batch)) if !batch.is_empty() => {
                    self.intercepted.set(true);
                    false
                }
                _ => true,
            })
            .collect()
    }
}

#[test]
fn a_restart_recovers_a_reminder_that_never_reached_the_notifier() -> Result {
    i_slint_backend_testing::init_integration_test_with_mock_time();
    let dir = tempfile::tempdir()?;
    let db = dir.path().join("reminders.db");
    let task_id = TaskId::from_u128(7);
    let epoch = Timestamp::UNIX_EPOCH;
    {
        let mut store = Store::open(&db)?;
        let mut task = Task::new(task_id, "Позвонить", Rank::initial(), epoch);
        task.reminder_at = Some(epoch);
        store.write(|w| w.insert_task(&task))?;
    }

    let calendar = Calendar { fixed_today: None };
    {
        let intercepted = Rc::new(Cell::new(false));
        let window = AppWindow::new()?;
        let backend = StopBeforeNotification {
            inner: Inline::new(App::new(Store::open(&db)?), calendar),
            intercepted: Rc::clone(&intercepted),
        };
        let bound = Bound::attach(&window, Box::new(backend), calendar);
        bound.enable_reminders(|_| panic!("ответ не должен был дойти до уведомителя"));
        assert!(
            intercepted.get(),
            "не получен ответ базы о наступившем напоминании"
        );
    }

    let window = AppWindow::new()?;
    let backend = Inline::new(App::new(Store::open(&db)?), calendar);
    let bound = Bound::attach(&window, Box::new(backend), calendar);
    let received = Rc::new(Cell::new(0));
    let count = Rc::clone(&received);
    bound.enable_reminders(move |batch| {
        count.set(count.get() + batch.iter().filter(|r| r.task == task_id).count());
    });
    assert_eq!(received.get(), 1, "напоминание потерялось при перезапуске");
    Ok(())
}
