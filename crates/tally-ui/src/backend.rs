//! Связь окна с базой. Окно никогда не ждёт базу: запросы уходят в поток –
//! владелец базы через канал, ответы возвращаются в поток окна
//! (`slint::invoke_from_event_loop`) и забираются оттуда. Внутри потока –
//! любой [`Service`] из `tally-app` (обычно [`tally_app::App`]).
//!
//! Для тестов есть [`Inline`]: тот же интерфейс, но запрос выполняется сразу.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use jiff::Timestamp;
use jiff::civil::Date;
use tally_app::{Clock, Command, DataReply, DataRequest, Done, Error, Result, Service};
use tally_core::TaskId;
use tally_store::{Hit, Reminder, Snapshot};

/// Сегодняшний день и момент команды. День – `TALLY_TODAY`, если задан
/// (для тестов и снимков экрана), иначе по часам системы – на каждый
/// запрос заново, так что смена суток подхватывается без перезапуска.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Calendar {
    pub fixed_today: Option<Date>,
}

impl Calendar {
    pub fn today(&self) -> Date {
        self.fixed_today
            .unwrap_or_else(|| jiff::ZonedDateTime::now().date())
    }

    pub fn clock(&self) -> Clock {
        Clock {
            today: self.today(),
            now: Timestamp::now(),
        }
    }
}

/// Зачем выполнялась команда – от этого зависит, что сделать с ответом.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// Отметка задачи на странице с номером перехода `visit`
    /// ([`crate::controller::Controller`] считает переходы): ответ после
    /// ухода со страницы строку уже не удерживает.
    Close {
        task: TaskId,
        visit: u64,
    },
    /// Отметка в выдаче относится к этому открытию поиска.
    CloseInSearch {
        task: TaskId,
        visit: u64,
    },
    /// Снятие отметки с задачи, закрытой последним действием окна, – отмена
    /// именно этого действия ([`tally_app::Command::UndoStep`]).
    Reopen(TaskId),
    Edit(TaskId),
    Organize,
    QuickAdd,
    Trash,
    Drop,
    Undo,
    Redo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Data(DataRequest),
    /// Перечитать базу.
    Load,
    Read,
    Reminders,
    ConfirmReminders(Vec<Reminder>),
    Search {
        serial: u64,
        query: String,
    },
    Run(Purpose, Box<Command>),
}

#[derive(Debug)]
pub enum Reply {
    Data {
        backup: bool,
        result: Result<DataReply>,
    },
    Loaded(Result<Snapshot>),
    Reminders(Result<Vec<Reminder>>),
    RemindersConfirmed(Result<()>),
    Searched {
        serial: u64,
        result: Result<(Vec<Hit>, Snapshot)>,
    },
    Ran(Purpose, Result<Done>),
    StaleEdit {
        task: TaskId,
        error: Error,
        refreshed: Result<Snapshot>,
    },
    StaleOrganization {
        error: Error,
        refreshed: Result<Snapshot>,
    },
    StaleDrop {
        error: Error,
        refreshed: Result<Snapshot>,
    },
}

/// Куда окно отправляет запросы и откуда забирает ответы.
pub trait Backend {
    fn submit(&mut self, request: Request);
    /// Готовые ответы в порядке запросов.
    fn replies(&mut self) -> Vec<Reply>;
}

fn serve(service: &mut dyn Service, calendar: Calendar, request: Request) -> Reply {
    match request {
        Request::Data(request) => Reply::Data {
            backup: matches!(request, DataRequest::DailyBackup(_)),
            result: service.data(request, calendar.clock()),
        },
        Request::Load => Reply::Loaded(service.snapshot_for_day(calendar.clock())),
        Request::Read => Reply::Loaded(service.snapshot()),
        Request::Reminders => Reply::Reminders(service.claim_reminders(Timestamp::now())),
        Request::ConfirmReminders(shown) => {
            Reply::RemindersConfirmed(service.confirm_reminders(&shown))
        }
        Request::Search { serial, query } => Reply::Searched {
            serial,
            result: service.search(&query),
        },
        Request::Run(purpose, command) => {
            match (purpose, service.execute(&command, calendar.clock())) {
                (
                    Purpose::Edit(task),
                    Err(error @ (Error::Conflict(_) | Error::TaskUnavailable)),
                ) => Reply::StaleEdit {
                    task,
                    error,
                    refreshed: service.snapshot(),
                },
                (Purpose::Organize, Err(error @ Error::Conflict(_))) => Reply::StaleOrganization {
                    error,
                    refreshed: service.snapshot(),
                },
                (
                    Purpose::Drop | Purpose::Trash,
                    Err(error @ (Error::Conflict(_) | Error::TaskUnavailable)),
                ) => Reply::StaleDrop {
                    error,
                    refreshed: service.snapshot(),
                },
                (purpose, result) => Reply::Ran(purpose, result),
            }
        }
    }
}

/// Выполняет запрос сразу, в потоке вызывающего кода (тесты).
pub struct Inline<S> {
    service: S,
    calendar: Calendar,
    ready: Vec<Reply>,
}

impl<S: Service> Inline<S> {
    pub fn new(service: S, calendar: Calendar) -> Self {
        Self {
            service,
            calendar,
            ready: Vec::new(),
        }
    }
}

impl<S: Service> Backend for Inline<S> {
    fn submit(&mut self, request: Request) {
        let reply = serve(&mut self.service, self.calendar, request);
        self.ready.push(reply);
    }

    fn replies(&mut self) -> Vec<Reply> {
        std::mem::take(&mut self.ready)
    }
}

/// Поток – владелец базы. Запросы выполняются по одному, в порядке
/// поступления; после каждого ответа вызывается `wake` (окно забирает
/// ответы в своём потоке).
pub struct Threaded {
    tx: Option<Sender<Request>>,
    rx: Receiver<Reply>,
    thread: Option<JoinHandle<()>>,
}

impl Threaded {
    pub fn spawn<S>(service: S, calendar: Calendar, wake: impl Fn() + Send + 'static) -> Self
    where
        S: Service + Send + 'static,
    {
        let (tx, requests) = channel::<Request>();
        let (answers, rx) = channel::<Reply>();
        let thread = std::thread::spawn(move || {
            let mut service = service;
            for request in requests {
                if answers
                    .send(serve(&mut service, calendar, request))
                    .is_err()
                {
                    break;
                }
                wake();
            }
        });
        Self {
            tx: Some(tx),
            rx,
            thread: Some(thread),
        }
    }
}

impl Backend for Threaded {
    fn submit(&mut self, request: Request) {
        // Поток мог завершиться только вместе с окном – тогда отвечать некому.
        if let Some(tx) = &self.tx {
            let _ = tx.send(request);
        }
    }

    fn replies(&mut self) -> Vec<Reply> {
        self.rx.try_iter().collect()
    }
}

impl Drop for Threaded {
    fn drop(&mut self) {
        // Канал закрыт – поток доделывает начатое и выходит.
        self.tx = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;
    use std::sync::mpsc::sync_channel;
    use std::time::Duration;
    use tally_app::{App, Effect, NewTask};
    use tally_store::Store;

    const TODAY: Date = date(2026, 11, 19);

    fn calendar() -> Calendar {
        Calendar {
            fixed_today: Some(TODAY),
        }
    }

    #[test]
    fn the_calendar_uses_the_fixed_day() {
        assert_eq!(calendar().today(), TODAY);
        assert_eq!(calendar().clock().today, TODAY);
    }

    #[test]
    fn the_thread_answers_in_order_and_wakes_the_window() -> Result<(), Box<dyn std::error::Error>>
    {
        let (woke, wakes) = sync_channel::<()>(8);
        let app = App::new(Store::open_in_memory()?);
        let mut backend = Threaded::spawn(app, calendar(), move || {
            let _ = woke.try_send(());
        });
        let add = Command::AddTask(NewTask {
            text: "Купить плитку".into(),
            ..NewTask::default()
        });
        backend.submit(Request::Run(Purpose::Undo, Box::new(add)));
        backend.submit(Request::Load);
        let mut replies = Vec::new();
        while replies.len() < 2 {
            wakes.recv_timeout(Duration::from_secs(10))?;
            replies.extend(backend.replies());
        }
        match &replies[0] {
            Reply::Ran(Purpose::Undo, Ok(done)) => {
                assert!(matches!(done.effect, Effect::Added(_)));
            }
            other => panic!("{other:?}"),
        }
        match &replies[1] {
            Reply::Loaded(Ok(s)) => assert_eq!(s.tasks.len(), 1),
            other => panic!("{other:?}"),
        }
        Ok(())
    }
}
