//! Подтверждение закрытия и сохранение прежних имён после восстановления.

mod common;
#[path = "common/projects.rs"]
mod projects;

use common::{Result, clock, run};
use projects::{open_window, project};
use tally_app::{Command, NewArea, execute};
use tally_core::Status;
use tally_store::{RecordId, Store};
use tally_ui::ui::{
    OrganizationAction as Action, OrganizationMode as Mode, OrganizationSubmit as Submit,
};

#[test]
fn a_close_confirmation_cannot_outlive_the_title_it_displayed() -> Result {
    i_slint_backend_testing::init_integration_test_with_mock_time();
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let mut store = Store::open(&path)?;
    let id = project(&mut store, "Черновик")?;
    let (window, _binding) = open_window(&path, std::convert::identity)?;
    window.invoke_select(format!("project:{id}").into());
    window.invoke_organization_begin(Action::CloseProject);
    assert_eq!(window.get_organization_mode(), Mode::Close);
    assert!(window.get_organization_prompt().contains("Черновик"));

    let before = store.read().project(id)?.ok_or("нет проекта")?;
    run(
        &mut store,
        Command::RenameProject {
            project: id,
            expected_updated_at: before.sync.updated_at,
            title: "Годовой отчёт".into(),
        },
    )?;
    let history = store.read().history()?;
    window.invoke_organization_finish(Submit::Complete);
    let after = store.read().project(id)?.ok_or("нет проекта")?;
    assert_eq!(after.title, "Годовой отчёт");
    assert_eq!(after.status, Status::Open);
    assert_eq!(store.read().history()?, history);

    assert_eq!(window.get_organization_mode(), Mode::Close);
    assert!(window.get_organization_prompt().contains("Годовой отчёт"));
    assert!(window.get_organization_error().contains("Подтвердите"));
    window.invoke_organization_finish(Submit::Complete);
    let closed = store.read().project(id)?.ok_or("нет проекта")?;
    assert!(matches!(closed.status, Status::Completed { .. }));
    assert_eq!(store.read().history()?.len(), history.len() + 1);
    Ok(())
}

#[test]
fn saving_existing_names_by_id_is_a_no_op_after_trash_restore() -> Result {
    let mut store = Store::open_in_memory()?;
    let area_command = Command::AddArea(NewArea {
        title: "Дом".into(),
    });
    let area = execute(&mut store, &area_command, clock())?.snapshot.areas[0].id;
    let id = project(&mut store, "Ремонт")?;
    for record in [RecordId::Area(area), RecordId::Project(id)] {
        store.write(|w| w.delete(record, clock().now))?;
    }
    run(&mut store, area_command)?;
    project(&mut store, "Ремонт")?;
    for record in [RecordId::Area(area), RecordId::Project(id)] {
        store.write(|w| w.restore(record, clock().now))?;
    }
    let old_area = store.read().area(area)?.ok_or("нет области")?;
    let old_project = store.read().project(id)?.ok_or("нет проекта")?;
    let snapshot = store.read().snapshot()?;
    let history = store.read().history()?;
    for command in [
        Command::RenameArea {
            area,
            expected_updated_at: old_area.sync.updated_at,
            title: old_area.title,
        },
        Command::RenameProject {
            project: id,
            expected_updated_at: old_project.sync.updated_at,
            title: old_project.title,
        },
    ] {
        assert_eq!(execute(&mut store, &command, clock())?.seq, None);
        assert_eq!(store.read().snapshot()?, snapshot);
        assert_eq!(store.read().history()?, history);
    }
    Ok(())
}
