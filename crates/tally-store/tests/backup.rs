use std::{fs, path::Path};

use rusqlite::Connection;
use tally_store::{SCHEMA_VERSION, Store, StoreError};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn verifies_a_standalone_wal_copy_without_creating_sidecars() -> Result {
    let directory = tempfile::tempdir()?;
    let source = Store::open(directory.path().join("active.db"))?;
    let copy = directory.path().join("копия # 100%.sqlite3");
    source.backup_into(&copy)?;
    let bytes = fs::read(&copy)?;
    assert_eq!(&bytes[18..20], &[2, 2], "копия с заголовком WAL");
    source.verify_backup(&copy)?;
    assert_eq!(fs::read(&copy)?, bytes);
    assert!(!copy.with_extension("sqlite3-wal").exists());
    assert!(!copy.with_extension("sqlite3-shm").exists());
    Ok(())
}

#[test]
fn empty_and_unrelated_sqlite_files_are_not_usable_backups() -> Result {
    let directory = tempfile::tempdir()?;
    let source = Store::open_in_memory()?;
    let empty = directory.path().join("empty.sqlite3");
    fs::write(&empty, [])?;
    let unrelated = directory.path().join("unrelated.sqlite3");
    let connection = Connection::open(&unrelated)?;
    connection.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    connection.execute("CREATE TABLE notes (body TEXT)", [])?;
    drop(connection);

    for path in [empty.as_path(), unrelated.as_path()] {
        assert_unusable(&source, path)?;
    }
    Ok(())
}

fn assert_unusable(source: &Store, path: &Path) -> Result {
    let original = fs::read(path)?;
    assert!(matches!(
        source.verify_backup(path),
        Err(StoreError::InvalidBackup(_))
    ));
    assert_eq!(fs::read(path)?, original);
    Ok(())
}
