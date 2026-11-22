//! Черновик карточки и его привязка к окну. До сохранения база не меняется.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use jiff::Timestamp;
use jiff::civil::{Date, DateTime};
use jiff::tz::TimeZone;
use slint::{Model, ModelRc, SharedString, VecModel};
use tally_app::{Command, EditChecklistItem, EditTag, EditTask, Index, Repeat};
use tally_core::{Start, Task, TaskId};
use tally_store::{Snapshot, normalize};
use tally_when::When;
use uuid::Uuid;

use super::{Inner, repeat};
use crate::backend::{Purpose, Reply, Request};
use crate::keys::{self, EditorAction, Press};
use crate::{strings, ui};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Finish {
    Discard,
    Save,
    Reopen,
}

pub(super) struct Editor {
    edit: EditTask,
    initial_when: Option<When>,
    initial_repeat: Option<Repeat>,
    initial_reminder: Option<Timestamp>,
    repeat: Option<Repeat>,
    repeat_editable: bool,
    tag_labels: Vec<Option<String>>,
    checks: Rc<VecModel<ui::EditorCheck>>,
    tags: Rc<VecModel<ui::EditorTag>>,
    pub(super) row: ui::RowItem,
    pub(super) position: usize,
    saving: bool,
}

impl Editor {
    fn new(task: &Task, snapshot: &Snapshot, row: ui::RowItem, position: usize) -> Self {
        let initial_when = if let Some(date) = task.date {
            Some(When::On {
                date,
                evening: task.evening,
            })
        } else {
            match task.start {
                Start::Inbox => None,
                Start::Anytime => Some(When::Anytime),
                Start::Someday => Some(When::Someday),
            }
        };
        let edit = EditTask {
            task: task.id,
            expected_updated_at: task.sync.updated_at,
            title: task.title.clone(),
            notes: task.notes.clone(),
            when: None,
            deadline: task.deadline,
            checklist: task
                .checklist
                .iter()
                .filter(|i| !i.sync.is_deleted())
                .map(|i| EditChecklistItem {
                    id: Some(i.id),
                    title: i.title.clone(),
                    done: i.done,
                })
                .collect(),
            tags: task.tags.iter().copied().map(EditTag::Existing).collect(),
        };
        let tag_labels = task
            .tags
            .iter()
            .map(|id| {
                snapshot
                    .tags
                    .iter()
                    .find(|tag| tag.id == *id && !tag.sync.is_deleted())
                    .map(|tag| tag.title.clone())
            })
            .collect();
        let repeat = task.repeat_rule.map(|rule| Repeat {
            rule,
            from: task.repeat_from,
        });
        Self {
            initial_repeat: repeat,
            initial_reminder: task.reminder_at,
            repeat,
            repeat_editable: task.status.is_open(),
            checks: Rc::new(VecModel::from(
                edit.checklist.iter().map(check_row).collect::<Vec<_>>(),
            )),
            tags: Rc::new(VecModel::default()),
            edit,
            initial_when,
            tag_labels,
            row,
            position,
            saving: false,
        }
    }

    fn show(&self, window: &ui::AppWindow, today: Date) {
        window.set_edit_title(self.edit.title.as_str().into());
        window.set_edit_notes(self.edit.notes.as_str().into());
        window.set_edit_reminder(reminder_text(self.initial_reminder).into());
        window.set_new_check(SharedString::new());
        window.set_new_tag(SharedString::new());
        window.set_edit_error(SharedString::new());
        window.set_edit_busy(false);
        self.refresh_tags();
        window.set_edit_checklist(ModelRc::from(self.checks.clone()));
        window.set_edit_tags(ModelRc::from(self.tags.clone()));
        self.show_dates(window, today);
        self.show_repeat(window);
        window.set_edit_repeat_editable(self.repeat_editable);
        window.set_editor_focus(ui::EditorFocus::Title);
        window.set_edit_task_id(self.row.key.clone());
        window.set_edit_reopenable(self.row.reopenable);
        window.set_edit_key(self.row.row_key.clone());
    }

    fn show_repeat(&self, window: &ui::AppWindow) {
        window.set_edit_repeat_draft(repeat::draft(self.repeat));
        window.set_edit_repeat_caption(repeat::caption(self.repeat).into());
    }

    fn show_dates(&self, window: &ui::AppWindow, today: Date) {
        let when = self.edit.when.or(self.initial_when);
        let value = when.map_or_else(String::new, date_value);
        let label = match when {
            None => strings::INBOX.to_owned(),
            Some(When::Anytime) => strings::ANYTIME.to_owned(),
            Some(When::Someday) => strings::SOMEDAY.to_owned(),
            Some(When::On { date, evening }) => {
                let day = strings::date(date, today);
                if evening {
                    strings::with_evening(&day)
                } else {
                    day
                }
            }
        };
        window.set_edit_when(format!("{} · {label}", strings::EDITOR_WHEN).into());
        window.set_when_value(value.into());

        let deadline = self.edit.deadline;
        let label = deadline.map_or_else(
            || strings::EDITOR_DEADLINE.to_owned(),
            |day| {
                format!(
                    "{} · {}",
                    strings::EDITOR_DEADLINE,
                    strings::date(day, today)
                )
            },
        );
        window.set_edit_deadline(label.into());
        window.set_deadline_value(
            deadline
                .map_or_else(String::new, |date| {
                    date_value(When::On {
                        date,
                        evening: false,
                    })
                })
                .into(),
        );
    }

    fn set_date(
        &mut self,
        window: &ui::AppWindow,
        today: Date,
        deadline: bool,
        value: &str,
    ) -> Result<(), String> {
        let value = value.trim();
        let parsed = (!value.is_empty())
            .then(|| tally_when::parse(value, today))
            .transpose()
            .map_err(|error| error.to_string())?;
        if deadline {
            self.edit.deadline = match parsed {
                Some(When::On {
                    date,
                    evening: false,
                }) => Some(date),
                None => None,
                _ => return Err(strings::EDITOR_DEADLINE_ERROR.to_owned()),
            };
        } else {
            self.edit.when = parsed.or(Some(When::Anytime));
        }
        window.set_edit_error(SharedString::new());
        self.show_dates(window, today);
        Ok(())
    }

    fn refresh_tags(&self) {
        self.tags.set_vec(
            self.tag_labels
                .iter()
                .enumerate()
                .filter_map(|(index, label)| {
                    Some(ui::EditorTag {
                        title: label.as_deref()?.into(),
                        slot: i32::try_from(index).ok()?,
                    })
                })
                .collect::<Vec<_>>(),
        );
    }

    fn add_tag(&mut self, window: &ui::AppWindow) {
        let value = window.get_new_tag();
        let name = value.trim();
        if !name.is_empty() {
            let normalized = normalize(name);
            if !self
                .tag_labels
                .iter()
                .flatten()
                .any(|existing| normalize(existing) == normalized)
            {
                self.edit.tags.push(EditTag::Named(name.to_owned()));
                self.tag_labels.push(Some(name.to_owned()));
                self.refresh_tags();
            }
        }
        window.set_new_tag(SharedString::new());
    }

    fn add_check(&mut self, window: &ui::AppWindow) {
        let value = window.get_new_check();
        let title = value.trim();
        if !title.is_empty() {
            let item = EditChecklistItem {
                id: None,
                title: title.to_owned(),
                done: false,
            };
            self.checks.push(check_row(&item));
            self.edit.checklist.push(item);
        }
        window.set_new_check(SharedString::new());
    }

    fn prepare(&mut self, window: &ui::AppWindow, reopen: bool) -> Result<Request, &'static str> {
        let title = window.get_edit_title();
        if title.trim().is_empty() {
            return Err(strings::EDITOR_TITLE_ERROR);
        }
        if self
            .edit
            .checklist
            .iter()
            .any(|item| item.title.trim().is_empty())
        {
            return Err(strings::EDITOR_CHECK_ERROR);
        }
        let entered = window.get_edit_reminder();
        let reminder = entered.trim();
        let reminder_at = if reminder == reminder_text(self.initial_reminder) {
            self.initial_reminder
        } else if reminder.is_empty() {
            None
        } else {
            let datetime = DateTime::strptime("%d.%m.%Y %H:%M", reminder)
                .map_err(|_| strings::EDITOR_REMINDER_ERROR)?;
            Some(
                TimeZone::system()
                    .to_ambiguous_timestamp(datetime)
                    .unambiguous()
                    .map_err(|_| strings::EDITOR_REMINDER_TIME_ERROR)?,
            )
        };
        self.add_check(window);
        self.add_tag(window);
        self.edit.title = title.to_string();
        self.edit.notes = window.get_edit_notes().to_string();
        self.saving = true;
        window.set_edit_error(SharedString::new());
        window.set_edit_busy(true);
        let command = if reminder_at != self.initial_reminder {
            Command::EditTaskReminder {
                edit: self.edit.clone(),
                repeat: (!reopen && self.repeat != self.initial_repeat).then_some(self.repeat),
                reminder_at,
                reopen,
            }
        } else if reopen {
            Command::ReopenTask(self.edit.clone())
        } else if self.repeat != self.initial_repeat {
            Command::EditTaskRepeat {
                edit: self.edit.clone(),
                repeat: self.repeat,
            }
        } else {
            Command::EditTask(self.edit.clone())
        };
        Ok(Request::Run(
            Purpose::Edit(self.edit.task),
            Box::new(command),
        ))
    }
}

fn reminder_text(at: Option<Timestamp>) -> String {
    at.map_or_else(String::new, |at| {
        at.to_zoned_datetime(TimeZone::system())
            .strftime("%d.%m.%Y %H:%M")
            .to_string()
    })
}

fn check_row(item: &EditChecklistItem) -> ui::EditorCheck {
    ui::EditorCheck {
        title: item.title.as_str().into(),
        done: item.done,
    }
}

fn date_value(when: When) -> String {
    match when {
        When::Anytime => strings::ANYTIME.to_owned(),
        When::Someday => strings::SOMEDAY.to_owned(),
        When::On { date, evening } => {
            // Исходное значение поля не должно сдвинуться, если наступила полночь.
            let suffix = if evening { " вечером" } else { "" };
            format!(
                "{:02}.{:02}.{:04}{suffix}",
                date.day(),
                date.month(),
                date.year()
            )
        }
    }
}

pub(super) fn connect(window: &ui::AppWindow, inner: &Rc<RefCell<Inner>>) {
    let weak = Rc::downgrade(inner);
    window.on_choose_task(move |key| {
        if let Some(inner) = weak.upgrade() {
            inner.borrow_mut().choose_task(key);
        }
    });
    let weak = Rc::downgrade(inner);
    window.on_edit_task(move |key| {
        if let Some(inner) = weak.upgrade() {
            inner.borrow_mut().open_editor(&key);
        }
    });
    let weak = Rc::downgrade(inner);
    window.on_edit_finish(move |save| {
        if let Some(inner) = weak.upgrade() {
            Inner::finish_edit(&inner, if save { Finish::Save } else { Finish::Discard });
        }
    });
    let weak = Rc::downgrade(inner);
    window.on_edit_reopen(move || {
        if let Some(inner) = weak.upgrade() {
            Inner::finish_edit(&inner, Finish::Reopen);
        }
    });
    let weak = Rc::downgrade(inner);
    window.on_edit_add_tag(move || {
        change_draft(&weak, |editor, window| editor.add_tag(window));
    });
    let weak = Rc::downgrade(inner);
    window.on_edit_remove_tag(move |index| {
        change_draft(&weak, |editor, _| {
            if let Ok(index) = usize::try_from(index)
                && editor.tag_labels.get(index).is_some_and(Option::is_some)
            {
                editor.tag_labels.remove(index);
                editor.edit.tags.remove(index);
                editor.refresh_tags();
            }
        });
    });
    let weak = Rc::downgrade(inner);
    window.on_edit_add_check(move || {
        change_draft(&weak, |editor, window| editor.add_check(window));
    });
    let weak = Rc::downgrade(inner);
    window.on_edit_remove_check(move |index| {
        change_draft(&weak, |editor, _| {
            if let Ok(index) = usize::try_from(index)
                && index < editor.edit.checklist.len()
            {
                editor.edit.checklist.remove(index);
                editor.checks.remove(index);
            }
        });
    });
    let weak = Rc::downgrade(inner);
    window.on_edit_check_title(move |index, title| {
        change_check(&weak, index, |item| item.title = title.to_string());
    });
    let weak = Rc::downgrade(inner);
    window.on_edit_check_done(move |index, done| {
        change_check(&weak, index, |item| item.done = done);
    });
    let weak = Rc::downgrade(inner);
    window.on_edit_repeat(move |draft, clear| {
        let repeat = if clear {
            None
        } else {
            match repeat::parse(&draft) {
                Ok(repeat) => Some(repeat),
                Err(error) => return error.into(),
            }
        };
        change_draft(&weak, |editor, window| {
            if editor.repeat_editable {
                editor.repeat = repeat;
                editor.show_repeat(window);
            }
        });
        SharedString::new()
    });
    let weak = Rc::downgrade(inner);
    window.on_edit_date(move |deadline, text| {
        let Some(inner) = weak.upgrade() else {
            return SharedString::new();
        };
        let mut me = inner.borrow_mut();
        let today = me.calendar.today();
        let Some(window) = me.window.upgrade() else {
            return SharedString::new();
        };
        me.editor
            .as_mut()
            .filter(|e| !e.saving)
            .and_then(|editor| editor.set_date(&window, today, deadline, &text).err())
            .map_or_else(SharedString::new, |s| s.into())
    });
}

fn change_draft(weak: &Weak<RefCell<Inner>>, change: impl FnOnce(&mut Editor, &ui::AppWindow)) {
    if let Some(inner) = weak.upgrade() {
        let mut me = inner.borrow_mut();
        if let Some(window) = me.window.upgrade()
            && let Some(editor) = me.editor.as_mut().filter(|editor| !editor.saving)
        {
            change(editor, &window);
            window.set_edit_error(SharedString::new());
        }
    }
}

fn change_check(
    weak: &Weak<RefCell<Inner>>,
    index: i32,
    change: impl FnOnce(&mut EditChecklistItem),
) {
    change_draft(weak, |editor, _| {
        if let Ok(index) = usize::try_from(index)
            && let Some(item) = editor.edit.checklist.get_mut(index)
        {
            change(item);
            editor.checks.set_row_data(index, check_row(item));
        }
    });
}

impl Inner {
    pub(super) fn refresh_editor_reopenable(&mut self, today: Date) -> bool {
        let Some(editor) = self.editor.as_mut() else {
            return false;
        };
        editor.row.reopenable = self
            .controller
            .editable_task(editor.edit.task, today)
            .is_some_and(|(task, snapshot)| Index::new(snapshot, today).can_reopen(task));
        editor.row.reopenable
    }

    fn choose_task(&mut self, key: SharedString) {
        if self.editor.is_none()
            && self.organization.is_none()
            && self
                .rows
                .iter()
                .any(|row| row.editable && row.row_key == key)
            && let Some(window) = self.window.upgrade()
        {
            self.selected_task = key;
            window.set_selected_task(self.selected_task.clone());
            window.invoke_focus_list();
        }
    }

    pub(super) fn open_editor(&mut self, key: &str) -> bool {
        if self.editor.is_some() || self.organization.is_some() {
            return false;
        }
        let Some(window) = self.window.upgrade() else {
            return false;
        };
        let Some((position, row)) = self
            .rows
            .iter()
            .enumerate()
            .find(|(_, row)| row.editable && row.row_key == key)
        else {
            return false;
        };
        let Ok(id) = Uuid::parse_str(&row.key) else {
            return false;
        };
        let today = self.calendar.today();
        let Some((task, snapshot)) = self.controller.editable_task(TaskId(id), today) else {
            return false;
        };
        self.selected_task = row.row_key.clone();
        window.set_selected_task(self.selected_task.clone());
        let editor = Editor::new(task, snapshot, row, position);
        window.set_edit_row_index(i32::try_from(position).unwrap_or(-1));
        self.refresh_viewports(&window);
        editor.show(&window, today);
        self.editor = Some(editor);
        true
    }

    fn move_task_cursor(&mut self, backwards: bool) -> bool {
        if self.editor.is_some() {
            return false;
        }
        let items: Vec<_> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.editable)
            .collect();
        if items.is_empty() {
            return false;
        }
        let last = items.len() - 1;
        let position = items
            .iter()
            .position(|(_, row)| row.row_key == self.selected_task)
            .map_or(if backwards { last } else { 0 }, |p| {
                if backwards {
                    p.saturating_sub(1)
                } else {
                    (p + 1).min(last)
                }
            });
        let (index, row) = &items[position];
        self.choose_task(row.row_key.clone());
        if let Some(window) = self.window.upgrade()
            && let Some((top, bottom)) = self.drag_layout.bounds(*index)
        {
            window.invoke_reveal_row(top, bottom);
        }
        true
    }

    pub(super) fn editor_key(this: &Rc<RefCell<Self>>, press: Press<'_>) -> bool {
        let today = this.borrow().calendar.today();
        let Some(action) = keys::editor_action(press, today) else {
            return false;
        };
        match action {
            EditorAction::Save | EditorAction::Dismiss => Self::finish_edit(
                this,
                if action == EditorAction::Save {
                    Finish::Save
                } else {
                    Finish::Discard
                },
            ),
            EditorAction::Previous | EditorAction::Next => this
                .borrow_mut()
                .move_task_cursor(action == EditorAction::Previous),
            _ => this.borrow_mut().edit_shortcut(action, today),
        }
    }

    fn edit_shortcut(&mut self, action: EditorAction, today: Date) -> bool {
        if self.editor.is_none() {
            let key = self.selected_task.clone();
            if !self.open_editor(&key) {
                return false;
            }
        } else if action == EditorAction::Open {
            return false;
        }
        let Some(editor) = self.editor.as_mut() else {
            return false;
        };
        if editor.saving {
            return true;
        }
        let Some(window) = self.window.upgrade() else {
            return false;
        };
        let target = match action {
            EditorAction::When => ui::EditorFocus::When,
            EditorAction::Deadline => ui::EditorFocus::Deadline,
            EditorAction::Checklist => ui::EditorFocus::Checklist,
            EditorAction::Tags => ui::EditorFocus::Tags,
            EditorAction::Schedule(when) => {
                editor.edit.when = Some(when);
                editor.show_dates(&window, today);
                return true;
            }
            _ => return true,
        };
        window.set_editor_focus(target);
        window.set_editor_focus_serial(window.get_editor_focus_serial().wrapping_add(1));
        true
    }

    fn finish_edit(this: &Rc<RefCell<Self>>, finish: Finish) -> bool {
        let request = {
            let mut me = this.borrow_mut();
            let Some(window) = me.window.upgrade() else {
                return false;
            };
            let Some(editor) = me.editor.as_mut() else {
                return false;
            };
            if editor.saving {
                return true;
            }
            if finish == Finish::Discard {
                me.close_editor(&window);
                drop(me);
                Self::render(this);
                return true;
            }
            if finish == Finish::Reopen && !editor.row.reopenable {
                return false;
            }
            match editor.prepare(&window, finish == Finish::Reopen) {
                Ok(request) => request,
                Err(error) => {
                    window.set_edit_error(error.into());
                    return true;
                }
            }
        };
        Self::submit(this, request);
        true
    }

    fn close_editor(&mut self, window: &ui::AppWindow) {
        self.editor = None;
        window.set_edit_row_index(-1);
        window.set_edit_extra_height(0.0);
        window.set_edit_key(SharedString::new());
        window.set_edit_task_id(SharedString::new());
        window.set_edit_reopenable(false);
        window.set_edit_busy(false);
        if self.search.active {
            window.invoke_focus_search();
        } else {
            window.invoke_focus_list();
        }
    }

    pub(super) fn on_editor_reply(&mut self, reply: &Reply) {
        let (id, error) = match reply {
            Reply::Ran(Purpose::Edit(id), result) => {
                (*id, result.as_ref().err().map(strings::error))
            }
            Reply::StaleEdit {
                task,
                error,
                refreshed,
            } => {
                let message = if refreshed.is_ok() {
                    strings::error(error)
                } else {
                    strings::EDITOR_REFRESH_FAILED.to_owned()
                };
                (*task, Some(message))
            }
            _ => return,
        };
        let Some(editor) = self.editor.as_mut().filter(|e| e.edit.task == id) else {
            return;
        };
        let Some(window) = self.window.upgrade() else {
            return;
        };
        if let Some(error) = error {
            editor.saving = false;
            window.set_edit_busy(false);
            window.set_edit_error(error.into());
        } else {
            self.close_editor(&window);
        }
    }
}
