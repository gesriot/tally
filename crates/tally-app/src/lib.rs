//! Слой приложения Tally: команды над `tally-store`, отмена и повтор,
//! поток – владелец базы.
//!
//! * [`Command`] – изменение базы со значениями, как их ввёл пользователь;
//!   [`execute`] выполняет его в одной транзакции записи вместе со строкой
//!   журнала действий и возвращает [`Done`]: что сделано и база после этого.
//! * [`Command::Undo`] и [`Command::Redo`] – отмена и повтор по журналу
//!   действий в базе: он общий для всех клиентов и переживает перезапуск.
//! * [`Service`] – один интерфейс для синхронного [`App`] и потока
//!   [`Worker`].
//! * [`Index`] – поиск записей в снимке: короткие номера, задача по номеру
//!   или названию.
//!
//! Время всегда приходит снаружи ([`Clock`]): `tally-app` не вызывает `now()`.

mod command;
mod data;
mod error;
mod exec;
mod history;
mod index;
mod worker;

pub use command::{
    Clock, Close, Command, Done, DropAnchor, DropPosition, DropTask, EditChecklistItem, EditTag,
    EditTask, Effect, MoveTask, NewArea, NewHeading, NewProject, NewTask, Repeat, Step, TaskLane,
    TrashPreview,
};
pub use data::{DataReply, DataRequest};
pub use error::{Error, Result};
pub use exec::{execute, search, snapshot, snapshot_for_day, trash_preview};
pub use history::HISTORY_LIMIT;
pub use index::{Index, MIN_ID, candidate_id_len, hex, id_len, short};
pub use tally_store::{Appearance, Settings};
pub use worker::Worker;

use jiff::Timestamp;
use tally_store::{Hit, Reminder, Snapshot, Store};

/// Команды и чтение базы – одинаково для [`App`] и [`Worker`].
pub trait Service {
    /// Выполнить команду.
    fn execute(&mut self, command: &Command, clock: Clock) -> Result<Done>;
    /// Вся база одним согласованным снимком.
    fn snapshot(&mut self) -> Result<Snapshot>;
    /// Снимок после создания наступивших календарных экземпляров.
    fn snapshot_for_day(&mut self, clock: Clock) -> Result<Snapshot> {
        let snapshot = self.snapshot()?;
        if exec::has_due_repeats(&snapshot, clock.today) {
            Ok(self.execute(&Command::AdvanceRepeats, clock)?.snapshot)
        } else {
            Ok(snapshot)
        }
    }
    /// Поиск и снимок для вывода совпадений – из одного состояния базы.
    fn search(&mut self, query: &str) -> Result<(Vec<Hit>, Snapshot)>;
    /// Забрать наступившие напоминания, которых ещё не было.
    fn claim_reminders(&mut self, now: Timestamp) -> Result<Vec<Reminder>>;
    fn confirm_reminders(&mut self, shown: &[Reminder]) -> Result<()>;
    fn data(&mut self, request: DataRequest, clock: Clock) -> Result<DataReply>;
}

/// Синхронный вариант: команды выполняются в потоке вызывающего кода.
pub struct App {
    store: Store,
}

impl App {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Прямой доступ к хранилищу – для тестов и миграций. Записи в обход
    /// [`Service::execute`] не попадают в журнал действий.
    pub fn store_mut(&mut self) -> &mut Store {
        &mut self.store
    }

    pub fn into_store(self) -> Store {
        self.store
    }
}

impl Service for App {
    fn data(&mut self, request: DataRequest, clock: Clock) -> Result<DataReply> {
        data::execute(&mut self.store, request, clock)
    }

    fn execute(&mut self, command: &Command, clock: Clock) -> Result<Done> {
        execute(&mut self.store, command, clock)
    }

    fn snapshot(&mut self) -> Result<Snapshot> {
        snapshot(&self.store)
    }

    fn search(&mut self, query: &str) -> Result<(Vec<Hit>, Snapshot)> {
        search(&self.store, query)
    }

    fn claim_reminders(&mut self, now: Timestamp) -> Result<Vec<Reminder>> {
        Ok(self.store.claim_reminders(now)?)
    }

    fn confirm_reminders(&mut self, shown: &[Reminder]) -> Result<()> {
        Ok(self.store.confirm_reminders(shown)?)
    }
}
