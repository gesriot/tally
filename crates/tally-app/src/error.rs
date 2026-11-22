//! Ошибки команд.

use tally_store::StoreError;

/// Ошибка команды. Текст – для пользователя, по-русски. Если команда
/// вернула ошибку, в базе ничего не изменилось.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Команду нельзя выполнить: нет задачи, неверная дата, одноимённые
    /// записи, задача уже закрыта.
    #[error("{0}")]
    Rejected(String),
    /// Задача карточки пропала из базы или оказалась в корзине.
    #[error("задача больше недоступна")]
    TaskUnavailable,
    /// Отменять нечего: журнал пуст или всё уже отменено.
    #[error("нечего отменять")]
    NothingToUndo,
    /// Повторять нечего: ничего не отменено или после отмены была новая команда.
    #[error("нечего повторять")]
    NothingToRedo,
    /// [`crate::Command::UndoStep`]: после этого действия было другое
    /// (или его уже отменили) – отменять его отдельно нельзя.
    #[error("после этого действия было другое действие")]
    NotLast,
    /// Текущее состояние записей не позволяет применить действие.
    #[error("{0}")]
    Conflict(String),
    /// Время правки записи уже предельное (`Timestamp::MAX`): правку нельзя
    /// отметить строго позже прежней.
    #[error("время правки записи достигло предела ({0}) – отметить правку позже нельзя")]
    NoLaterTime(jiff::Timestamp),
    /// Запись журнала не читается.
    #[error("журнал действий повреждён: {0}")]
    History(#[from] serde_json::Error),
    #[error("не удалось обработать файл «{path}»: {source}")]
    File {
        path: String,
        source: std::io::Error,
    },
    #[error("пригодной копии за сегодня нет. «{path}»: {reason}")]
    UnusableBackup { path: String, reason: String },
    #[error("не удалось прочитать данные Tally: {0}")]
    DataJson(serde_json::Error),
    /// Поток базы ([`crate::Worker`]) остановлен.
    #[error("поток базы остановлен")]
    Stopped,
    #[error(transparent)]
    Store(#[from] StoreError),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// `return Err(Error::Rejected(format!(…)))`.
macro_rules! reject {
    ($($arg:tt)*) => {
        return Err($crate::Error::Rejected(format!($($arg)*)))
    };
}
pub(crate) use reject;

/// `Error::Rejected(format!(…))`.
macro_rules! rejected {
    ($($arg:tt)*) => {
        $crate::Error::Rejected(format!($($arg)*))
    };
}
pub(crate) use rejected;
