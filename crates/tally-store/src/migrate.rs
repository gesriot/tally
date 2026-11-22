//! Миграции схемы: номер версии – `PRAGMA user_version`.
//!
//! Миграции только добавляются в конец списка и никогда не меняются задним
//! числом. Каждая применяется в своей транзакции `BEGIN IMMEDIATE` вместе с новым
//! номером версии; номер читается уже под блокировкой записи.

use rusqlite::{Connection, TransactionBehavior};

use crate::error::{Result, StoreError};

const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_init.sql"),
    include_str!("../migrations/0002_history.sql"),
    include_str!("../migrations/0003_repeat_anchor.sql"),
    include_str!("../migrations/0004_remove_undated_repeats.sql"),
    include_str!("../migrations/0005_task_reminders.sql"),
    include_str!("../migrations/0006_reminder_confirmation.sql"),
    include_str!("../migrations/0007_settings.sql"),
];

/// Версия схемы, которую знает эта сборка.
pub const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

pub(crate) fn version(conn: &Connection) -> Result<i64> {
    Ok(conn.pragma_query_value(None, "user_version", |r| r.get(0))?)
}

/// Довести схему до [`SCHEMA_VERSION`]. Возвращает число применённых миграций.
///
/// Версия перечитывается под блокировкой записи (`BEGIN IMMEDIATE`): два
/// процесса, открывшие новую базу одновременно, не применят одну миграцию
/// дважды – второй дождётся первого и увидит уже новую версию.
pub(crate) fn migrate(conn: &mut Connection) -> Result<usize> {
    let mut applied = 0;
    loop {
        // Быстрый путь без блокировки: база уже свежая.
        if applied == 0 && version(conn)? == SCHEMA_VERSION {
            return Ok(0);
        }
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let found = version(&tx)?;
        if found > SCHEMA_VERSION {
            return Err(StoreError::NewerSchema {
                found,
                known: SCHEMA_VERSION,
            });
        }
        if found == SCHEMA_VERSION {
            tx.commit()?;
            return Ok(applied);
        }
        let done = usize::try_from(found).map_err(|_| StoreError::Corrupt {
            table: "pragma",
            column: "user_version",
            value: found.to_string(),
        })?;
        tx.execute_batch(MIGRATIONS[done])?;
        tx.pragma_update(None, "user_version", found + 1)?;
        tx.commit()?;
        applied += 1;
    }
}
