//! Быстрый поиск и фильтр текущего списка.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use slint::{Model, ModelRc, SharedString, Timer, TimerMode, VecModel, platform::Key};
use tally_core::TagId;
use tally_store::{Hit, Snapshot, normalize};

use super::Inner;
use crate::backend::Request;
use crate::keys::{self, Press};
use crate::view::{Row, Selection};
use crate::{strings, ui};

#[derive(Default)]
pub(super) struct QuickFind {
    pub(super) active: bool,
    pub(super) hits: Vec<Hit>,
    query: String,
    serial: u64,
    pending: bool,
    error: String,
    selected: SharedString,
    typing: Timer,
    tag_choices: Vec<(TagId, SharedString)>,
}

impl QuickFind {
    fn set_query(&mut self, query: String) {
        self.query = query;
        self.hits.clear();
        self.error.clear();
        self.selected = SharedString::new();
        self.serial = self.serial.wrapping_add(1);
        self.pending = !self.query.trim().is_empty();
    }

    fn request(&self) -> Option<Request> {
        (self.active && !self.query.trim().is_empty()).then(|| Request::Search {
            serial: self.serial,
            query: self.query.trim().to_owned(),
        })
    }

    pub(super) fn refresh(&mut self) -> Option<Request> {
        if !self.active || self.query.trim().is_empty() {
            return None;
        }
        self.typing.stop();
        self.serial = self.serial.wrapping_add(1);
        self.pending = true;
        self.request()
    }
}

pub(super) fn connect(window: &ui::AppWindow, inner: &Rc<RefCell<Inner>>) {
    window.set_search_label(strings::SEARCH.into());
    window.set_search_placeholder(strings::SEARCH_PLACEHOLDER.into());
    window.set_search_close_label(strings::SEARCH_CLOSE.into());
    window.set_tag_filter_label(strings::FILTER_TAG.into());
    let weak = Rc::downgrade(inner);
    window.on_search_begin(move || {
        if let Some(inner) = weak.upgrade() {
            Inner::begin_search(&inner, "");
        }
    });
    let weak = Rc::downgrade(inner);
    window.on_search_end(move || {
        if let Some(inner) = weak.upgrade() {
            let changed = inner.borrow_mut().dismiss_search();
            if changed {
                Inner::render(&inner);
            }
        }
    });
    let weak = Rc::downgrade(inner);
    window.on_search_query(move |query| {
        if let Some(inner) = weak.upgrade() {
            Inner::queue_search(&inner, query.to_string());
        }
    });
    let weak = Rc::downgrade(inner);
    window.on_search_open(move |key| {
        if let Some(inner) = weak.upgrade() {
            Inner::open_search_row(&inner, &key);
        }
    });
    let weak = Rc::downgrade(inner);
    window.on_tag_filter(move |index| {
        let Some(inner) = weak.upgrade() else { return };
        {
            let mut me = inner.borrow_mut();
            if me.search.active || me.editor.is_some() || me.organization.is_some() {
                return;
            }
            let tag = if index == 0 {
                None
            } else {
                let Some((id, _)) = usize::try_from(index - 1)
                    .ok()
                    .and_then(|i| me.search.tag_choices.get(i))
                else {
                    return;
                };
                Some(*id)
            };
            me.cancel_drag();
            me.controller.set_tag_filter(tag);
            if let Some(window) = me.window.upgrade() {
                window.invoke_focus_list();
                window.invoke_reveal_row(0.0, 0.0);
            }
        }
        Inner::render(&inner);
    });
}

impl Inner {
    fn begin_search(this: &Rc<RefCell<Self>>, seed: &str) -> bool {
        {
            let mut me = this.borrow_mut();
            if me.editor.is_some() || me.organization.is_some() {
                return false;
            }
            if me.search.active {
                if let Some(window) = me.window.upgrade() {
                    window.invoke_focus_search();
                }
                return true;
            }
            me.cancel_drag();
            me.search.active = true;
            me.selected_task = SharedString::new();
        }
        Self::queue_search(this, seed.to_owned());
        if let Some(window) = this.borrow().window.upgrade() {
            window.invoke_reveal_row(0.0, 0.0);
            window.invoke_focus_search();
        }
        true
    }

    fn queue_search(this: &Rc<RefCell<Self>>, query: String) {
        {
            let mut me = this.borrow_mut();
            if !me.search.active || me.editor.is_some() {
                return;
            }
            me.search.set_query(query);
            me.search.typing.stop();
            if me.search.pending {
                let weak = Rc::downgrade(this);
                me.search.typing.start(
                    TimerMode::SingleShot,
                    Duration::from_millis(90),
                    move || {
                        let Some(inner) = weak.upgrade() else { return };
                        let request = inner.borrow().search.request();
                        if let Some(request) = request {
                            Inner::submit(&inner, request);
                        }
                    },
                );
            }
        }
        Self::render(this);
    }

    pub(super) fn accept_search(
        &mut self,
        serial: u64,
        result: tally_app::Result<(Vec<Hit>, Snapshot)>,
    ) {
        if !self.search.active || self.search.serial != serial {
            return;
        }
        self.search.pending = false;
        match result {
            Ok((hits, snapshot)) => {
                self.search.hits = hits;
                self.search.error.clear();
                self.controller.accept_snapshot(snapshot);
            }
            Err(error) => self.search.error = strings::error(&error),
        }
    }

    pub(super) fn dismiss_search(&mut self) -> bool {
        if !self.search.active || self.editor.is_some() {
            return false;
        }
        self.search.active = false;
        self.controller.end_search();
        self.search.typing.stop();
        self.search.set_query(String::new());
        self.selected_task = SharedString::new();
        if let Some(window) = self.window.upgrade() {
            window.invoke_focus_list();
        }
        true
    }

    pub(super) fn search_key(this: &Rc<RefCell<Self>>, press: Press<'_>) -> bool {
        let active = {
            let me = this.borrow();
            if me.editor.is_some() || me.organization.is_some() {
                return false;
            }
            me.search.active
        };
        if press.control
            && !press.shift
            && !press.alt
            && !press.meta
            && matches!(press.text, "f" | "F" | "а" | "А" | "\u{0006}")
        {
            return Self::begin_search(this, "");
        }
        if !active {
            return !press.control
                && !press.alt
                && !press.meta
                && press.text.chars().next().is_some_and(char::is_alphanumeric)
                && Self::begin_search(this, press.text);
        }
        if press.control || press.alt || press.meta || press.shift {
            return false;
        }
        if keys::is(press.text, Key::Escape) {
            this.borrow_mut().dismiss_search();
            Self::render(this);
        } else if keys::is(press.text, Key::UpArrow) || keys::is(press.text, Key::DownArrow) {
            let mut me = this.borrow_mut();
            if me.rows.row_count() == 0 {
                return true;
            }
            let current = me
                .rows
                .iter()
                .position(|row| row.row_key == me.search.selected)
                .unwrap_or(0);
            let index = if keys::is(press.text, Key::UpArrow) {
                current.saturating_sub(1)
            } else {
                (current + 1).min(me.rows.row_count() - 1)
            };
            if let Some(row) = me.rows.row_data(index)
                && let Some(window) = me.window.upgrade()
            {
                me.search.selected = row.row_key;
                window.set_selected_result(me.search.selected.clone());
                if let Some((top, bottom)) = me.drag_layout.bounds(index) {
                    window.invoke_reveal_row(top, bottom);
                }
            }
        } else if keys::is(press.text, Key::Return) {
            let key = this.borrow().search.selected.clone();
            if !key.is_empty() {
                Self::open_search_row(this, &key);
            }
        } else {
            return false;
        }
        true
    }

    fn open_search_row(this: &Rc<RefCell<Self>>, key: &str) {
        let reveal = {
            let mut me = this.borrow_mut();
            if !me.search.active || me.editor.is_some() {
                return;
            }
            let Some(row) = me.rows.iter().find(|r| r.row_key == key) else {
                return;
            };
            me.search.selected = row.row_key.clone();
            if row.editable {
                if me.open_editor(key)
                    && let Some(window) = me.window.upgrade()
                {
                    window.set_selected_result(row.row_key);
                }
                return;
            }
            let journal = row.kind == ui::RowKind::Project && !row.opens;
            if !row.opens && !journal {
                return;
            }
            me.controller.set_tag_filter(None);
            me.controller
                .select(if journal { "logbook" } else { &row.key });
            me.dismiss_search();
            journal.then_some(row.key)
        };
        Self::render(this);
        if let Some(key) = reveal {
            let me = this.borrow();
            if let Some((index, _)) = me.rows.iter().enumerate().find(|(_, r)| r.key == key)
                && let Some((top, bottom)) = me.drag_layout.bounds(index)
                && let Some(window) = me.window.upgrade()
            {
                window.invoke_reveal_row(top, bottom);
            }
        }
    }

    pub(super) fn search_selection_bounds(&self) -> Option<(f32, f32)> {
        if !self.search.active || self.search.selected.is_empty() || self.editor.is_some() {
            return None;
        }
        let index = self
            .rows
            .iter()
            .position(|row| row.row_key == self.search.selected)?;
        self.drag_layout.bounds(index)
    }

    pub(super) fn sync_search_selection(&mut self, rows: &[Row]) {
        let search = &mut self.search;
        if search.active
            && !rows
                .iter()
                .any(|row| row.row_key == search.selected.as_str())
        {
            search.selected = rows
                .first()
                .map_or_else(SharedString::new, |row| row.row_key.as_str().into());
        }
    }

    pub(super) fn render_search(&mut self, window: &ui::AppWindow, rows: &[Row]) {
        let search = &mut self.search;
        window.set_search_active(search.active);
        if window.get_search_text().as_str() != search.query {
            window.set_search_text(search.query.as_str().into());
        }
        window.set_search_length(i32::try_from(search.query.len()).unwrap_or(i32::MAX));
        if search.active {
            window.set_selected_result(search.selected.clone());
        } else {
            window.set_selected_result(SharedString::new());
        }
        let mut tags: Vec<_> = self
            .controller
            .snapshot()
            .into_iter()
            .flat_map(|s| crate::view::tag_labels(&s.tags))
            .map(|(id, label)| (id, SharedString::from(label)))
            .collect();
        tags.sort_by_cached_key(|(id, title)| (normalize(title), *id));
        let reset = normalize(strings::ALL_TAGS);
        if let Some(index) = tags.iter().position(|(_, title)| normalize(title) == reset) {
            let mut label = format!("#{}", tags[index].1);
            let used: HashSet<_> = tags.iter().map(|(_, title)| normalize(title)).collect();
            while used.contains(&normalize(&label)) {
                label.insert(0, '#');
            }
            tags[index].1 = label.into();
        }
        if tags != search.tag_choices {
            let values = std::iter::once(strings::ALL_TAGS.into())
                .chain(tags.iter().map(|(_, t)| t.clone()))
                .collect::<Vec<_>>();
            window.set_tag_filter_items(ModelRc::from(Rc::new(VecModel::from(values))));
            search.tag_choices = tags;
        }
        let current = self
            .controller
            .tag_filter()
            .and_then(|tag| search.tag_choices.iter().position(|(id, _)| *id == tag))
            .and_then(|index| i32::try_from(index + 1).ok())
            .unwrap_or(0);
        window.set_tag_filter_index(current);
        let selection = self.controller.selection();
        window.set_tag_filter_visible(
            !search.active
                && !search.tag_choices.is_empty()
                && !matches!(selection, Selection::Trash | Selection::Tag(_)),
        );
        let state = if search.active {
            if !search.error.is_empty() {
                search.error.as_str()
            } else if search.pending && rows.is_empty() {
                strings::SEARCH_PENDING
            } else if !search.query.trim().is_empty() && rows.is_empty() {
                strings::SEARCH_EMPTY
            } else {
                ""
            }
        } else if rows
            .iter()
            .all(|row| row.kind == crate::view::RowKind::Section)
            && (current != 0 || matches!(selection, Selection::Tag(_)))
        {
            strings::FILTER_EMPTY
        } else {
            ""
        };
        window.set_search_status(state.into());
    }
}
