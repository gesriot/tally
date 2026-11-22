//! Хранилище Tally: SQLite, миграции, CRUD и поиск по кириллице.
//!
//! * [`Store::open`] / [`Store::open_in_memory`] – соединение и миграции;
//! * [`Store::read`] → [`Reader`]: записи по id, все записи, [`Snapshot`], поиск;
//! * [`Store::write`] → [`Writer`] внутри одной транзакции: вставка, обновление,
//!   мягкое удаление и возврат, физическое удаление, очистка корзины;
//! * журнал действий для отмены и повтора ([`HistoryEntry`]) – пишется в той
//!   же транзакции, что и само действие.
//!
//! Схема – `migrations/0001_init.sql`, журнал действий – `0002_history.sql`. Даты – текст `YYYY-MM-DD`, моменты – текст
//! UTC фиксированной ширины, ключи порядка – строки `tally-rank`.
//! Поиск – по колонке `search_text`, которую заполняет [`normalize()`].

mod backup;
mod codec;
mod error;
mod migrate;
mod settings;
pub use settings::{Appearance, Settings};
mod normalize;
mod search;
mod store;
mod table;

pub use error::{Kind, Result, StoreError};
pub use migrate::SCHEMA_VERSION;
pub use normalize::{normalize, query_words};
pub use search::{Found, Hit};
pub use store::{Purged, Reader, RecordId, Snapshot, Store, Writer};
pub use table::history::HistoryEntry;
pub use table::reminder::Reminder;
