//! Повторное открытие задачи при изменениях в другом окне.

mod common;
#[path = "common/projects.rs"]
mod projects;

use common::{Result, id_of, run, task};
use projects::{open_window, project};
use tally_app::Command;
use tally_store::Store;
use tally_ui::ui::{OrganizationAction as Action, OrganizationSubmit as Submit};

#[test]
fn closing_the_parent_in_another_window_refreshes_the_reopen_action() -> Result {
    i_slint_backend_testing::init_integration_test_with_mock_time();
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let mut store = Store::open(&path)?;
    let id = project(&mut store, "Ремонт")?;
    let mut tile = task("Плитка");
    tile.project = Some("Ремонт".into());
    run(&mut store, Command::AddTask(tile))?;
    let task_id = id_of(&store.read().snapshot()?, "Плитка")?;
    run(&mut store, Command::CloseTask(task_id))?;
    let (journal, _journal_binding) = open_window(&path, std::convert::identity)?;
    journal.invoke_select("logbook".into());
    journal.invoke_edit_task(task_id.to_string().into());
    assert!(journal.get_edit_reopenable());
    journal.set_edit_notes("Мой черновик".into());

    let (other, _other_binding) = open_window(&path, std::convert::identity)?;
    other.invoke_select(format!("project:{id}").into());
    other.invoke_organization_begin(Action::CloseProject);
    other.invoke_organization_finish(Submit::Complete);
    let history = store.read().history()?;
    let snapshot = store.read().snapshot()?;
    assert!(!snapshot.projects[0].status.is_open());
    journal.invoke_edit_reopen();
    assert!(!journal.get_edit_error().is_empty());
    assert!(!journal.get_edit_reopenable());
    assert_eq!(journal.get_edit_notes(), "Мой черновик");
    assert_eq!(store.read().history()?, history);
    assert_eq!(store.read().snapshot()?, snapshot);

    journal.invoke_edit_finish(false);
    journal.invoke_edit_task(task_id.to_string().into());
    assert_eq!(journal.get_edit_key().as_str(), task_id.to_string());
    assert!(!journal.get_edit_reopenable());
    Ok(())
}
