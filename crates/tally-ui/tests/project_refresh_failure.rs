//! Восстановление после ошибки чтения снимка.

mod common;
#[path = "common/projects.rs"]
mod projects;

use common::{Result, run};
use projects::{open_window, project};
use slint::SharedString;
use slint::platform::Key;
use tally_app::{App, Command, Error};
use tally_store::Store;
use tally_ui::backend::{Backend, Inline, Reply, Request};
use tally_ui::ui::{
    OrganizationAction as Action, OrganizationMode as Mode, OrganizationSubmit as Submit,
};

struct FailFirstConflictRefresh {
    inner: Inline<App>,
    failed: bool,
}

impl Backend for FailFirstConflictRefresh {
    fn submit(&mut self, request: Request) {
        self.inner.submit(request);
    }

    fn replies(&mut self) -> Vec<Reply> {
        let mut replies = self.inner.replies();
        for reply in &mut replies {
            if !self.failed
                && let Reply::StaleOrganization { refreshed, .. } = reply
            {
                *refreshed = Err(Error::Stopped);
                self.failed = true;
            }
        }
        replies
    }
}

#[test]
fn a_failed_refresh_does_not_offer_the_known_stale_name_again() -> Result {
    i_slint_backend_testing::init_integration_test_with_mock_time();
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let mut store = Store::open(&path)?;
    let id = project(&mut store, "Черновик")?;
    let before = store.read().project(id)?.ok_or("нет проекта")?;
    let (window, _bound) = open_window(&path, |inner| FailFirstConflictRefresh {
        inner,
        failed: false,
    })?;
    window.invoke_select(format!("project:{id}").into());
    window.invoke_organization_begin(Action::Rename);
    window.set_organization_name("Мой вариант".into());
    run(
        &mut store,
        Command::RenameProject {
            project: id,
            expected_updated_at: before.sync.updated_at,
            title: "Общее название".into(),
        },
    )?;
    window.invoke_organization_finish(Submit::Save);
    assert_eq!(window.get_organization_name(), "Мой вариант");
    assert!(window.get_organization_error().contains("F5"));

    window.invoke_organization_finish(Submit::Dismiss);
    window.invoke_organization_begin(Action::Rename);
    assert_eq!(window.get_organization_mode(), Mode::Hidden);
    assert!(window.get_message().contains("F5"));

    assert!(window.invoke_key(SharedString::from(Key::F5), false, false, false, false));
    window.invoke_organization_begin(Action::Rename);
    assert_eq!(window.get_organization_mode(), Mode::Title);
    assert_eq!(window.get_organization_name(), "Общее название");
    Ok(())
}
