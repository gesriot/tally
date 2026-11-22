//! Ошибки хранилища.

use std::fmt;

use uuid::Uuid;

/// Вид записи – для сообщений об ошибках и для [`crate::RecordId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Area,
    Tag,
    Project,
    Heading,
    Task,
    ChecklistItem,
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Area => "область",
            Self::Tag => "тег",
            Self::Project => "проект",
            Self::Heading => "раздел",
            Self::Task => "задача",
            Self::ChecklistItem => "пункт чек-листа",
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("ошибка SQLite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("{kind} {id}: нет в базе")]
    NotFound { kind: Kind, id: Uuid },
    #[error("{kind} {id}: уже есть в базе")]
    AlreadyExists { kind: Kind, id: Uuid },
    #[error("{kind} {id}: ссылка на запись, которой нет в базе")]
    MissingReference { kind: Kind, id: Uuid },
    #[error("задача {task}: раздел {heading} – из другого проекта")]
    HeadingOutsideProject { task: Uuid, heading: Uuid },
    #[error("задача {task}: у задачи в проекте область берётся из проекта")]
    AreaWithProject { task: Uuid },
    #[error("задача {task}: пункт чек-листа {item} указан дважды")]
    DuplicateChecklistItem { task: Uuid, item: Uuid },
    #[error(
        "{kind} {id}: последнее изменение в {latest} позже метки операции – повторите с текущим временем"
    )]
    StaleTimestamp {
        kind: Kind,
        id: Uuid,
        latest: jiff::Timestamp,
    },
    #[error("{kind} {id}: не в корзине")]
    NotInTrash { kind: Kind, id: Uuid },
    #[error("тег {tag} указан у записи дважды")]
    DuplicateTag { tag: Uuid },
    #[error("повреждённое значение {value:?} в {table}.{column}")]
    Corrupt {
        table: &'static str,
        column: &'static str,
        value: String,
    },
    #[error("копия SQLite непригодна: {0}")]
    InvalidBackup(String),
    #[error("схема базы v{found} новее, чем знает программа (v{known})")]
    NewerSchema { found: i64, known: i64 },
}

pub type Result<T, E = StoreError> = std::result::Result<T, E>;

/// Перевести ошибку ограничения SQLite в понятную ошибку для записи `kind`/`id`.
pub(crate) fn constraint(e: rusqlite::Error, kind: Kind, id: Uuid) -> StoreError {
    use rusqlite::ffi;
    if let rusqlite::Error::SqliteFailure(code, _) = &e {
        match code.extended_code {
            ffi::SQLITE_CONSTRAINT_PRIMARYKEY => return StoreError::AlreadyExists { kind, id },
            ffi::SQLITE_CONSTRAINT_FOREIGNKEY => {
                return StoreError::MissingReference { kind, id };
            }
            _ => {}
        }
    }
    StoreError::Sqlite(e)
}
