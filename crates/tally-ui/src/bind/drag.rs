//! Раскладка строк и перевод жеста в команду с точными идентификаторами.

use std::cell::RefCell;
use std::rc::Rc;

use jiff::Timestamp;
use slint::{ComponentHandle, Model, SharedString};
use tally_app::{Command, DropAnchor, DropPosition, DropTask, TaskLane};
use tally_core::TaskId;

use super::Inner;
use crate::backend::{Purpose, Request};
use crate::ui::{self, DropSurface};
use crate::view::{Row, RowKind, Selection, SideKind, SideRow};

#[derive(Clone, Copy)]
pub(super) struct Source {
    pub(super) index: usize,
    task: TaskId,
    version: Timestamp,
    lane: Option<TaskLane>,
}

pub(super) struct DragLayout {
    selection: Selection,
    rows: Vec<Row>,
    sidebar: Vec<SideRow>,
    ends: Vec<f32>,
    side_ends: Vec<f32>,
    row_height: f32,
    meta_height: f32,
    section_height: f32,
    side_height: f32,
    gap_height: f32,
}

struct Target {
    lane: TaskLane,
    position: DropPosition,
    line: f32,
    highlight: String,
}

impl Target {
    fn line(lane: TaskLane, position: DropPosition, y: f32) -> Self {
        Self {
            lane,
            position,
            line: y,
            highlight: String::new(),
        }
    }

    fn highlight(lane: TaskLane, position: DropPosition, key: String) -> Self {
        Self {
            lane,
            position,
            line: -1.0,
            highlight: key,
        }
    }
}

impl DragLayout {
    pub(super) fn new(window: &ui::AppWindow) -> Self {
        let theme = window.global::<ui::Theme<'_>>();
        Self {
            selection: Selection::Today,
            rows: Vec::new(),
            sidebar: Vec::new(),
            ends: Vec::new(),
            side_ends: Vec::new(),
            row_height: theme.get_row_height(),
            meta_height: theme.get_row_height_meta(),
            section_height: theme.get_section_height(),
            side_height: theme.get_side_row_height(),
            gap_height: theme.get_side_gap_height(),
        }
    }

    pub(super) fn update(
        &mut self,
        selection: Selection,
        rows: &[Row],
        sidebar: &[SideRow],
    ) -> bool {
        let switched_page = self.selection != selection;
        self.selection = selection;
        if self.rows != rows {
            self.rows = rows.to_vec();
            let mut bottom = 0.0;
            self.ends = rows
                .iter()
                .map(|row| {
                    bottom += match row.kind {
                        RowKind::Section => self.section_height,
                        _ if !row.meta.is_empty() => self.meta_height,
                        _ => self.row_height,
                    };
                    bottom
                })
                .collect();
        }
        if self.sidebar != sidebar {
            self.sidebar = sidebar.to_vec();
            let mut bottom = 0.0;
            self.side_ends = sidebar
                .iter()
                .map(|row| {
                    bottom += if row.kind == SideKind::Gap {
                        self.gap_height
                    } else {
                        self.side_height
                    };
                    bottom
                })
                .collect();
        }
        switched_page
    }

    pub(super) fn bounds(&self, index: usize) -> Option<(f32, f32)> {
        let bottom = *self.ends.get(index)?;
        let top = index.checked_sub(1).map_or(0.0, |i| self.ends[i]);
        Some((top, bottom))
    }

    pub(super) fn has_empty_evening(&self) -> bool {
        self.selection == Selection::Today
            && self
                .rows
                .last()
                .is_none_or(|row| row.drop_lane != Some(TaskLane::Today { evening: true }))
    }

    fn source(&self, index: usize) -> Option<Source> {
        let row = self.rows.get(index)?;
        let (task, version) = row.drag_task?;
        Some(Source {
            index,
            task,
            version,
            lane: row.drop_lane,
        })
    }

    fn target(&self, surface: DropSurface, y: f32) -> Option<Target> {
        if !y.is_finite() || y < 0.0 {
            return None;
        }
        match surface {
            DropSurface::None => None,
            DropSurface::Sidebar => {
                let index = self.side_ends.partition_point(|&bottom| bottom <= y);
                let row = self.sidebar.get(index)?;
                Some(Target::highlight(
                    Selection::parse(&row.key)?.drop_lane()?,
                    DropPosition::Sidebar,
                    row.key.clone(),
                ))
            }
            DropSurface::List => self.list_target(y),
        }
    }

    fn list_target(&self, y: f32) -> Option<Target> {
        let base = self.selection.drop_lane()?;
        let index = self.ends.partition_point(|&bottom| bottom <= y);
        let Some(row) = self.rows.get(index) else {
            let end = self.ends.last().copied().unwrap_or(0.0);
            if self.has_empty_evening() && y >= end + self.section_height * 0.5 {
                return Some(Target::line(
                    TaskLane::Today { evening: true },
                    DropPosition::End,
                    end + self.section_height,
                ));
            }
            let (lane, boundary) = self
                .rows
                .iter()
                .zip(&self.ends)
                .rev()
                .find_map(|(row, &end)| row.drop_lane.map(|lane| (lane, end)))
                .unwrap_or((base, 0.0));
            return Some(Target::line(lane, DropPosition::End, boundary));
        };
        let (top, bottom) = self.bounds(index)?;
        let after = y >= (top + bottom) * 0.5;
        if row.kind == RowKind::Section {
            if after {
                return Some(self.before_next(index + 1, row.drop_lane?, bottom));
            }
            let lane = self.rows[..index]
                .iter()
                .rev()
                .find_map(|r| r.drop_lane)
                .unwrap_or(base);
            return Some(Target::line(lane, DropPosition::End, top));
        }
        if let Some(lane) = row.drop_lane {
            let line = if after { bottom } else { top };
            if let Some(anchor) = row_anchor(row) {
                let position = if after {
                    DropPosition::After(anchor)
                } else {
                    DropPosition::Before(anchor)
                };
                return Some(Target::line(lane, position, line));
            }
            return Some(self.before_next(index + 1, lane, line));
        }
        if row.opens
            && let Some(Selection::Project(project)) = Selection::parse(&row.key)
        {
            return Some(Target::highlight(
                TaskLane::Project(project, None),
                DropPosition::End,
                row.key.clone(),
            ));
        }
        None
    }

    fn before_next(&self, start: usize, lane: TaskLane, y: f32) -> Target {
        let position = self.rows[start..]
            .iter()
            .take_while(|r| r.kind != RowKind::Section && r.drop_lane == Some(lane))
            .find_map(row_anchor)
            .map_or(DropPosition::End, DropPosition::Before);
        Target::line(lane, position, y)
    }
}

fn row_anchor(row: &Row) -> Option<DropAnchor> {
    match row.kind {
        RowKind::Task => row.drag_task.map(|(task, _)| DropAnchor::Task(task)),
        RowKind::Project if row.opens => match Selection::parse(&row.key) {
            Some(Selection::Project(id)) => Some(DropAnchor::Project(id)),
            _ => None,
        },
        _ => None,
    }
}

pub(super) fn connect(window: &ui::AppWindow, inner: &Rc<RefCell<Inner>>) {
    let weak = Rc::downgrade(inner);
    window.on_drag_begin(move |index| {
        let Some(inner) = weak.upgrade() else {
            return false;
        };
        let Ok(index) = usize::try_from(index) else {
            return false;
        };
        inner.borrow_mut().begin_drag(index)
    });

    let weak = Rc::downgrade(inner);
    window.on_drag_hover(move |surface, y| {
        if let Some(inner) = weak.upgrade() {
            inner.borrow().show_drop_target(surface, y);
        }
    });

    let weak = Rc::downgrade(inner);
    window.on_drag_end(move |surface, y| {
        let Some(inner) = weak.upgrade() else { return };
        let request = inner.borrow_mut().drop_task(surface, y);
        if let Some(request) = request {
            Inner::submit(&inner, request);
        }
    });
}

impl Inner {
    fn begin_drag(&mut self, index: usize) -> bool {
        if self.editor.is_some()
            || self.organization.is_some()
            || self.drag_source.is_some()
            || self.search.active
            || self.controller.tag_filter().is_some()
        {
            return false;
        }
        let (Some(window), Some(source), Some(row)) = (
            self.window.upgrade(),
            self.drag_layout.source(index),
            self.rows.row_data(index),
        ) else {
            return false;
        };
        if !row.draggable {
            return false;
        }
        self.drag_source = Some(source);
        window.set_drag_key(row.row_key);
        window.set_drag_title(row.title);
        window.set_drag_line(-1.0);
        window.set_drag_active(true);
        self.refresh_viewports(&window);
        window.invoke_focus_list();
        true
    }

    fn show_drop_target(&self, surface: DropSurface, y: f32) {
        let (Some(window), Some(source)) = (self.window.upgrade(), self.drag_source) else {
            return;
        };
        let target = self.drag_layout.target(surface, y).filter(|t| !matches!(
            t.position,
            DropPosition::Before(DropAnchor::Task(id)) | DropPosition::After(DropAnchor::Task(id))
                if id == source.task
        ));
        window.set_drag_line(target.as_ref().map_or(-1.0, |t| t.line));
        window.set_drag_highlight(target.map_or_else(SharedString::new, |t| t.highlight.into()));
    }

    fn drop_task(&mut self, surface: DropSurface, y: f32) -> Option<Request> {
        let source = self.drag_source?;
        self.cancel_drag();
        let target = self.drag_layout.target(surface, y)?;
        Some(Request::Run(
            Purpose::Drop,
            Box::new(Command::DropTask(DropTask {
                task: source.task,
                expected_updated_at: source.version,
                source: source.lane,
                destination: target.lane,
                position: target.position,
            })),
        ))
    }

    pub(super) fn cancel_drag(&mut self) {
        if self.drag_source.take().is_some()
            && let Some(window) = self.window.upgrade()
        {
            window.set_drag_active(false);
            window.set_drag_key(SharedString::new());
            window.set_drag_highlight(SharedString::new());
            window.invoke_constrain_scroll();
            self.refresh_viewports(&window);
        }
    }
}
