//! База, открытая через символическую ссылку, сохраняет единственного владельца окна.

#![cfg(unix)]

use std::os::unix::fs::symlink;

use anyhow::{Context, Result};
use tally_client::desktop::{Desktop, Show, ShowRequest};
use tally_store::Store;
use uuid::Uuid;

#[test]
fn a_new_database_opened_through_a_symlink_keeps_its_desktop_owner() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let target = directory.path().join("real.db");
    let alias = directory.path().join("work.db");
    symlink("real.db", &alias)?;
    assert!(!target.exists());

    // Порядок совпадает с запуском tally-ui: сначала занять экземпляр, затем открыть БД.
    let desktop = Desktop::for_db(&alias)?;
    let _owner = desktop.claim(Show::Window)?.context("первый экземпляр")?;
    let _store = Store::open(&alias)?;
    assert!(target.is_file());

    // Повторить тот же --db и затем обратиться к созданному файлу напрямую.
    for path in [&alias, &target] {
        let client = Desktop::for_db(path)?;
        let request = ShowRequest::new(Show::QuickAdd, Some(Uuid::new_v4()));
        assert!(
            client.claim_request(&request)?.is_none(),
            "второй владелец той же базы при обращении через {}",
            path.display()
        );
        let pending = desktop.receive()?;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].reply_to, request.reply_to);
        assert_eq!(pending[0].show, Show::QuickAdd);
    }
    Ok(())
}
