use std::path::Path;

use slint::ComponentHandle;
use tally_app::{App, Command, NewProject, execute};
use tally_core::ProjectId;
use tally_store::Store;
use tally_ui::backend::{Backend, Calendar, Inline};
use tally_ui::bind::Bound;
use tally_ui::ui::AppWindow;

use crate::common::{Result, TODAY, clock};

pub fn project(store: &mut Store, title: &str) -> Result<ProjectId> {
    let command = Command::AddProject(NewProject {
        title: title.into(),
        area: None,
    });
    let added = execute(store, &command, clock())?;
    let created = added
        .snapshot
        .projects
        .iter()
        .find(|p| p.title == title && !p.sync.is_deleted())
        .ok_or("нет проекта")?;
    Ok(created.id)
}

pub fn open_window<B: Backend + 'static>(
    path: &Path,
    wrap: impl FnOnce(Inline<App>) -> B,
) -> Result<(AppWindow, Bound)> {
    let window = AppWindow::new()?;
    let calendar = Calendar {
        fixed_today: Some(TODAY),
    };
    let backend = Inline::new(App::new(Store::open(path)?), calendar);
    let binding = Bound::attach(&window, Box::new(wrap(backend)), calendar);
    window.show()?;
    Ok((window, binding))
}
