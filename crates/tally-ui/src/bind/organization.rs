//! Формы области и проекта. Команды получают UUID из выбранной страницы.

use std::cell::RefCell;
use std::rc::Rc;

use jiff::Timestamp;
use slint::SharedString;
use tally_app::{Command, NewArea, NewProject, Repeat, TrashPreview, trash_preview};
use tally_core::{AreaId, ProjectId};
use tally_store::{RecordId, Snapshot};

use super::{Inner, repeat};
use crate::backend::{Purpose, Reply, Request};
use crate::controller::Selection;
use crate::strings;
use crate::ui::{
    self, OrganizationAction as Action, OrganizationMode as Mode, OrganizationSubmit as Submit,
};
use crate::view;

enum Change {
    NewArea,
    NewProject(Option<AreaId>),
    NewSection(ProjectId),
    RenameArea(AreaId, Timestamp),
    RenameProject(ProjectId, Timestamp),
    CloseProject(ProjectId, Timestamp),
    RepeatProject(ProjectId, Timestamp),
    EmptyTrash(TrashPreview),
}

pub(super) struct OrganizationForm {
    change: Change,
    pending: bool,
}

pub(super) fn connect(window: &ui::AppWindow, inner: &Rc<RefCell<Inner>>) {
    window.set_organization_labels(strings::organization_labels());
    let weak = Rc::downgrade(inner);
    window.on_organization_begin(move |action| {
        if let Some(inner) = weak.upgrade() {
            let mut me = inner.borrow_mut();
            if me.organization.is_none()
                && me.editor.is_none()
                && me.controller.organization_needs_refresh()
                && !matches!(action, Action::AddArea | Action::AddProject)
            {
                me.controller
                    .say(strings::ORGANIZATION_REFRESH_FAILED.to_owned());
                drop(me);
                Inner::render(&inner);
            } else {
                me.begin_organization(action);
            }
        }
    });
    let weak = Rc::downgrade(inner);
    window.on_trash_row(move |index| {
        if let Some(inner) = weak.upgrade() {
            Inner::trash_record(&inner, Some(index));
        }
    });
    let weak = Rc::downgrade(inner);
    window.on_trash_container(move || {
        if let Some(inner) = weak.upgrade() {
            Inner::trash_record(&inner, None);
        }
    });
    let weak = Rc::downgrade(inner);
    window.on_organization_finish(move |submit| {
        if let Some(inner) = weak.upgrade() {
            Inner::finish_organization(&inner, submit);
        }
    });
}

impl Inner {
    fn begin_organization(&mut self, action: Action) {
        if self.organization.is_some() || self.editor.is_some() || self.search.active {
            return;
        }
        let (Some(window), Some(snapshot)) = (self.window.upgrade(), self.controller.snapshot())
        else {
            return;
        };
        let labels = window.get_organization_labels();
        let empty = SharedString::new();
        let (change, prompt, name) = match (action, self.controller.selection()) {
            (Action::AddArea, _) => (Change::NewArea, labels.add_area, empty),
            (Action::AddProject, _) => (Change::NewProject(None), labels.add_project, empty),
            (Action::AddAreaProject, Selection::Area(id)) => {
                (Change::NewProject(Some(id)), labels.add_area_project, empty)
            }
            (Action::AddSection, Selection::Project(id)) => {
                (Change::NewSection(id), labels.add_section, empty)
            }
            (Action::Rename, Selection::Area(id)) => {
                let Some(area) = snapshot.areas.iter().find(|a| a.id == id) else {
                    return;
                };
                (
                    Change::RenameArea(id, area.sync.updated_at),
                    labels.rename_area,
                    area.title.as_str().into(),
                )
            }
            (Action::Rename, Selection::Project(id)) => {
                let Some(project) = snapshot.projects.iter().find(|p| p.id == id) else {
                    return;
                };
                (
                    Change::RenameProject(id, project.sync.updated_at),
                    labels.rename_project,
                    project.title.as_str().into(),
                )
            }
            (Action::RepeatProject, Selection::Project(id)) => {
                let Some(p) = snapshot.projects.iter().find(|p| p.id == id) else {
                    return;
                };
                let current = p.repeat_rule.map(|rule| Repeat {
                    rule,
                    from: p.repeat_from,
                });
                window.set_project_repeat_draft(repeat::draft(current));
                (
                    Change::RepeatProject(id, p.sync.updated_at),
                    format!("{} · «{}»", strings::REPEAT, p.title).into(),
                    empty,
                )
            }
            (Action::EmptyTrash, Selection::Trash) => {
                let preview = trash_preview(snapshot);
                if preview.is_empty() {
                    return;
                }
                let prompt = strings::trash_empty_prompt(&preview);
                (Change::EmptyTrash(preview), prompt.into(), empty)
            }
            (Action::CloseProject, Selection::Project(id)) => {
                let Some(project) = snapshot.projects.iter().find(|p| p.id == id) else {
                    return;
                };
                (
                    Change::CloseProject(id, project.sync.updated_at),
                    strings::project_close_prompt(&project.title).into(),
                    empty,
                )
            }
            _ => return,
        };
        let mode = match &change {
            Change::CloseProject(..) => Mode::Close,
            Change::RepeatProject(..) => Mode::Repeat,
            Change::EmptyTrash(_) => Mode::Confirm,
            _ => Mode::Title,
        };
        self.organization = Some(OrganizationForm {
            change,
            pending: false,
        });
        window.set_organization_prompt(prompt);
        window.set_organization_name(name);
        window.set_organization_error(SharedString::new());
        window.set_organization_mode(mode);
    }

    pub(super) fn finish_organization(this: &Rc<RefCell<Self>>, submit: Submit) {
        let request = {
            let mut me = this.borrow_mut();
            let needs_refresh = me.controller.organization_needs_refresh();
            let (Some(window), Some(form)) = (me.window.upgrade(), me.organization.as_mut()) else {
                return;
            };
            if form.pending {
                return;
            }
            if submit == Submit::Dismiss {
                me.hide_organization(&window);
                return;
            }
            if needs_refresh && !matches!(form.change, Change::NewArea | Change::NewProject(None)) {
                window.set_organization_error(strings::ORGANIZATION_REFRESH_FAILED.into());
                return;
            }
            let title = window.get_organization_name().to_string();
            let command = match (&form.change, submit) {
                (Change::NewArea, Submit::Save) => Command::AddArea(NewArea { title }),
                (Change::NewProject(None), Submit::Save) => {
                    Command::AddProject(NewProject { title, area: None })
                }
                (Change::NewProject(Some(area)), Submit::Save) => {
                    Command::AddProjectInArea { title, area: *area }
                }
                (Change::NewSection(project), Submit::Save) => Command::AddProjectSection {
                    title,
                    project: *project,
                },
                (Change::RenameProject(project, expected), Submit::Save) => {
                    Command::RenameProject {
                        project: *project,
                        expected_updated_at: *expected,
                        title,
                    }
                }
                (Change::RenameArea(area, expected), Submit::Save) => Command::RenameArea {
                    area: *area,
                    expected_updated_at: *expected,
                    title,
                },
                (
                    Change::CloseProject(project, expected),
                    Submit::Complete | Submit::CancelProject,
                ) => Command::CloseProject {
                    project: *project,
                    expected_updated_at: *expected,
                    cancel: submit == Submit::CancelProject,
                },
                (Change::RepeatProject(project, expected), Submit::Save | Submit::Clear) => {
                    let repeat = if submit == Submit::Clear {
                        None
                    } else {
                        match repeat::parse(&window.get_project_repeat_draft()) {
                            Ok(repeat) => Some(repeat),
                            Err(error) => {
                                window.set_organization_error(error.into());
                                return;
                            }
                        }
                    };
                    Command::SetProjectRepeat {
                        project: *project,
                        expected_updated_at: *expected,
                        repeat,
                    }
                }
                (Change::EmptyTrash(preview), Submit::Save) => Command::EmptyTrash {
                    preview: preview.clone(),
                },
                _ => return,
            };
            form.pending = true;
            window.set_organization_pending(true);
            window.set_organization_error(SharedString::new());
            Request::Run(Purpose::Organize, Box::new(command))
        };
        Self::submit(this, request);
    }

    fn trash_record(this: &Rc<RefCell<Self>>, index: Option<i32>) {
        let request = {
            let me = this.borrow();
            if me.organization.is_some() || me.editor.is_some() || me.drag_source.is_some() {
                return;
            }
            let selection = me.controller.selection();
            let target = if let Some(index) = index {
                usize::try_from(index)
                    .ok()
                    .and_then(|index| me.trash_targets.get(index).copied().flatten())
            } else {
                me.controller.snapshot().and_then(|s| match selection {
                    Selection::Area(id) => s
                        .areas
                        .iter()
                        .find(|a| a.id == id)
                        .map(|a| (RecordId::Area(id), a.sync.updated_at)),
                    Selection::Project(id) => s
                        .projects
                        .iter()
                        .find(|p| p.id == id)
                        .map(|p| (RecordId::Project(id), p.sync.updated_at)),
                    _ => None,
                })
            };
            let Some((record, expected_updated_at)) = target else {
                return;
            };
            Request::Run(
                Purpose::Trash,
                Box::new(Command::SetTrash {
                    record,
                    expected_updated_at,
                    trashed: me.search.active || selection != Selection::Trash,
                }),
            )
        };
        Self::submit(this, request);
    }

    fn hide_organization(&mut self, window: &ui::AppWindow) {
        self.organization = None;
        window.set_organization_mode(Mode::Hidden);
        window.set_organization_pending(false);
        window.invoke_focus_list();
    }

    pub(super) fn on_organization_reply(&mut self, reply: &Reply) {
        let error = match reply {
            Reply::Ran(Purpose::Organize, result) => result.as_ref().err().map(strings::error),
            Reply::StaleOrganization { error, refreshed } => Some(if refreshed.is_ok() {
                strings::error(error)
            } else {
                strings::ORGANIZATION_REFRESH_FAILED.to_owned()
            }),
            Reply::Loaded(Ok(snapshot)) => {
                self.refresh_confirmation(snapshot);
                return;
            }
            _ => return,
        };
        let (Some(window), Some(form)) = (self.window.upgrade(), self.organization.as_mut()) else {
            return;
        };
        if let Some(error) = error {
            form.pending = false;
            window.set_organization_pending(false);
            window.set_organization_error(error.into());
            if let Reply::StaleOrganization {
                refreshed: Ok(snapshot),
                ..
            } = reply
            {
                self.refresh_confirmation(snapshot);
            }
        } else {
            self.hide_organization(&window);
        }
    }

    fn refresh_confirmation(&mut self, snapshot: &Snapshot) {
        if let (
            Some(window),
            Some(OrganizationForm {
                change: Change::EmptyTrash(expected),
                pending: false,
            }),
        ) = (self.window.upgrade(), self.organization.as_mut())
        {
            let preview = trash_preview(snapshot);
            if preview.is_empty() {
                self.hide_organization(&window);
            } else if preview != *expected {
                window.set_organization_prompt(strings::trash_empty_prompt(&preview).into());
                *expected = preview;
                window.set_organization_error(strings::TRASH_CHANGED.into());
            }
            return;
        }
        self.refresh_close_confirmation(snapshot);
    }

    fn refresh_close_confirmation(&mut self, snapshot: &Snapshot) {
        let (
            Some(window),
            Some(OrganizationForm {
                change: Change::CloseProject(id, expected),
                pending: false,
            }),
        ) = (self.window.upgrade(), self.organization.as_mut())
        else {
            return;
        };
        let project = snapshot
            .projects
            .iter()
            .find(|p| p.id == *id)
            .filter(|p| view::can_open(snapshot, Selection::Project(p.id)));
        let Some(project) = project else {
            self.hide_organization(&window);
            self.controller
                .say(strings::PROJECT_CLOSE_UNAVAILABLE.to_owned());
            return;
        };
        if project.sync.updated_at != *expected {
            *expected = project.sync.updated_at;
            window.set_organization_prompt(strings::project_close_prompt(&project.title).into());
            window.set_organization_error(strings::PROJECT_CLOSE_CHANGED.into());
        }
    }
}
