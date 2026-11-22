use std::collections::HashMap;
use std::fmt::Write;
use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use crate::error::{Result, StoreError};
use crate::{migrate, settings};

pub(crate) fn verify(source: &Connection, path: &Path) -> Result<()> {
    // Копия самодостаточна. immutable позволяет читать и старые копии с
    // заголовком WAL, не открывая и не создавая рядом -wal и -shm.
    let mut uri = String::from("file:");
    for byte in path.as_os_str().as_encoded_bytes() {
        let _ = write!(uri, "%{byte:02x}");
    }
    uri.push_str("?immutable=1");
    let copy = Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    if migrate::version(&copy)? != migrate::version(source)? {
        return Err(StoreError::InvalidBackup(
            "версия схемы отличается от исходной базы".into(),
        ));
    }
    let expected = tables(source)?;
    let actual = tables(&copy)?;
    if expected
        .iter()
        .any(|(name, sql)| actual.get(name) != Some(sql))
    {
        return Err(StoreError::InvalidBackup(
            "отсутствует или изменена таблица Tally".into(),
        ));
    }
    settings::load(&copy)?;
    let integrity: String = copy.query_row("PRAGMA integrity_check(1)", [], |row| row.get(0))?;
    if integrity != "ok" {
        return Err(StoreError::InvalidBackup(integrity));
    }
    Ok(())
}

fn tables(conn: &Connection) -> Result<HashMap<String, String>> {
    let mut query = conn.prepare(
        "SELECT name, sql FROM sqlite_schema WHERE type = 'table' AND name NOT GLOB 'sqlite_*'",
    )?;
    Ok(query
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?)
}
