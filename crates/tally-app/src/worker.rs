//! Поток – владелец базы: другие потоки (окно, напоминания, быстрый ввод)
//! шлют ему запросы через канал и ждут ответа. Интерфейс тот же, что у
//! синхронного [`crate::App`] – [`crate::Service`].

use std::sync::mpsc::{Sender, channel};
use std::thread::JoinHandle;

use jiff::Timestamp;
use tally_store::{Hit, Reminder, Snapshot, Store};

use crate::command::{Clock, Command, Done};
use crate::error::{Error, Result};
use crate::{App, DataReply, DataRequest, Service};

enum Request {
    Data(DataRequest, Clock, Sender<Result<DataReply>>),
    Execute(Command, Clock, Sender<Result<Done>>),
    Snapshot(Sender<Result<Snapshot>>),
    Reminders(Timestamp, Sender<Result<Vec<Reminder>>>),
    ConfirmReminders(Vec<Reminder>, Sender<Result<()>>),
    Search(String, Sender<Result<(Vec<Hit>, Snapshot)>>),
}

/// Хранилище в своём потоке. Запросы выполняются по одному, в порядке
/// поступления. Поток заканчивается вместе с последним `Worker`.
pub struct Worker {
    tx: Option<Sender<Request>>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    /// Отдать `store` новому потоку.
    pub fn spawn(store: Store) -> Self {
        let (tx, rx) = channel::<Request>();
        let thread = std::thread::spawn(move || {
            let mut app = App::new(store);
            // Получатель ответа мог уже уйти – тогда ответ просто некому отдать.
            for request in rx {
                match request {
                    Request::Data(request, clock, reply) => {
                        let _ = reply.send(app.data(request, clock));
                    }
                    Request::Execute(command, clock, reply) => {
                        let _ = reply.send(app.execute(&command, clock));
                    }
                    Request::Snapshot(reply) => {
                        let _ = reply.send(app.snapshot());
                    }
                    Request::Reminders(now, reply) => {
                        let _ = reply.send(app.claim_reminders(now));
                    }
                    Request::ConfirmReminders(shown, reply) => {
                        let _ = reply.send(app.confirm_reminders(&shown));
                    }
                    Request::Search(query, reply) => {
                        let _ = reply.send(app.search(&query));
                    }
                }
            }
        });
        Self {
            tx: Some(tx),
            thread: Some(thread),
        }
    }

    fn ask<T>(&self, request: impl FnOnce(Sender<Result<T>>) -> Request) -> Result<T> {
        let (reply, answer) = channel();
        self.tx
            .as_ref()
            .ok_or(Error::Stopped)?
            .send(request(reply))
            .map_err(|_| Error::Stopped)?;
        answer.recv().map_err(|_| Error::Stopped)?
    }
}

impl Service for Worker {
    fn data(&mut self, request: DataRequest, clock: Clock) -> Result<DataReply> {
        self.ask(|reply| Request::Data(request, clock, reply))
    }

    fn execute(&mut self, command: &Command, clock: Clock) -> Result<Done> {
        let command = command.clone();
        self.ask(|reply| Request::Execute(command, clock, reply))
    }

    fn snapshot(&mut self) -> Result<Snapshot> {
        self.ask(Request::Snapshot)
    }

    fn claim_reminders(&mut self, now: Timestamp) -> Result<Vec<Reminder>> {
        self.ask(|reply| Request::Reminders(now, reply))
    }

    fn confirm_reminders(&mut self, shown: &[Reminder]) -> Result<()> {
        self.ask(|reply| Request::ConfirmReminders(shown.to_vec(), reply))
    }

    fn search(&mut self, query: &str) -> Result<(Vec<Hit>, Snapshot)> {
        let query = query.to_owned();
        self.ask(|reply| Request::Search(query, reply))
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Канал закрыт – поток доделывает начатое и выходит.
        self.tx = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
