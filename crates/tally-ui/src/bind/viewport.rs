//! Видимые строки в двух прокручиваемых панелях. Модель окна хранит полный
//! порядок, но Slint создаёт компоненты только около видимой области.

use std::ops::Range;
use std::rc::Rc;

use slint::{ComponentHandle, Model, ModelRc, VecModel};

use super::Inner;
use crate::ui::{self, AppWindow};

struct Pane<T> {
    model: Rc<VecModel<T>>,
    indices: Vec<usize>,
    ends: Vec<f32>,
}

impl<T: Clone + PartialEq + 'static> Pane<T> {
    fn new() -> Self {
        Self {
            model: Rc::new(VecModel::default()),
            indices: Vec::new(),
            ends: Vec::new(),
        }
    }

    fn layout(&mut self, heights: impl Iterator<Item = f32>) -> f32 {
        let mut end = 0.0;
        self.ends = heights
            .map(|height| {
                end += height;
                end
            })
            .collect();
        end
    }

    fn sync(
        &mut self,
        visible: Range<usize>,
        pin: Option<usize>,
        row_at: impl Fn(usize, f32) -> Option<T>,
        expanded: Option<Expansion>,
    ) {
        let mut wanted: Vec<_> = visible.collect();
        if let Some(index) = pin.filter(|&index| index < self.ends.len())
            && let Err(position) = wanted.binary_search(&index)
        {
            wanted.insert(position, index);
        }

        // Существующий компонент остаётся на месте, пока его индекс нужен.
        // В частности, не пересоздаём строку, которая захватила указатель.
        let mut position = 0;
        for index in wanted {
            let top = index
                .checked_sub(1)
                .map_or(0.0, |i| end_at(&self.ends, i, expanded));
            let Some(row) = row_at(index, top) else {
                continue;
            };
            while self.indices.get(position).is_some_and(|&old| old < index) {
                self.indices.remove(position);
                self.model.remove(position);
            }
            if self.indices.get(position) == Some(&index) {
                if self.model.row_data(position).as_ref() != Some(&row) {
                    self.model.set_row_data(position, row);
                }
            } else {
                self.indices.insert(position, index);
                self.model.insert(position, row);
            }
            position += 1;
        }
        while self.indices.len() > position {
            self.indices.remove(position);
            self.model.remove(position);
        }
    }

    fn visible(&self, scroll_y: f32, height: f32, extra: Option<Expansion>) -> Range<usize> {
        let overscan = height.max(0.0) * 0.5 + 64.0;
        let offset = (-scroll_y).max(0.0);
        let start = row_after(&self.ends, (offset - overscan).max(0.0), extra);
        let end = row_after(&self.ends, offset + height.max(0.0) + overscan, extra);
        start..(end + 1).min(self.ends.len())
    }
}

#[derive(Clone, Copy)]
struct Expansion {
    index: usize,
    extra: f32,
}

fn end_at(ends: &[f32], index: usize, expanded: Option<Expansion>) -> f32 {
    ends[index]
        + expanded
            .filter(|entry| index >= entry.index)
            .map_or(0.0, |entry| entry.extra)
}

fn row_after(ends: &[f32], y: f32, expanded: Option<Expansion>) -> usize {
    let mut range = 0..ends.len();
    while range.start < range.end {
        let middle = range.start + (range.end - range.start) / 2;
        if end_at(ends, middle, expanded) <= y {
            range.start = middle + 1;
        } else {
            range.end = middle;
        }
    }
    range.start
}

pub(super) struct Viewports {
    main: Pane<ui::PositionedTask>,
    side: Pane<ui::PositionedSide>,
}

impl Viewports {
    pub(super) fn new(window: &AppWindow) -> Self {
        let main = Pane::new();
        let side = Pane::new();
        window.set_visible_rows(ModelRc::from(main.model.clone()));
        window.set_visible_sidebar(ModelRc::from(side.model.clone()));
        Self { main, side }
    }

    pub(super) fn layout_main(&mut self, window: &AppWindow, rows: &VecModel<ui::RowItem>) {
        let theme = window.global::<ui::Theme<'_>>();
        let end = self.main.layout(rows.iter().map(|row| match row.kind {
            ui::RowKind::Section => theme.get_section_height(),
            _ if !row.meta.is_empty() => theme.get_row_height_meta(),
            _ => theme.get_row_height(),
        }));
        window.set_task_extent(end);
    }

    pub(super) fn layout_side(&mut self, window: &AppWindow, rows: &VecModel<ui::SideItem>) {
        let theme = window.global::<ui::Theme<'_>>();
        let end = self.side.layout(rows.iter().map(|row| {
            if row.kind == ui::SideKind::Gap {
                theme.get_side_gap_height()
            } else {
                theme.get_side_row_height()
            }
        }));
        window.set_side_extent(end);
    }
}

impl Inner {
    pub(super) fn refresh_viewports(&mut self, window: &AppWindow) {
        let editor = usize::try_from(window.get_edit_row_index()).ok();
        let expanded = editor.map(|index| Expansion {
            index,
            extra: window.get_edit_extra_height(),
        });
        let bounds = self.viewports.main.visible(
            window.get_task_scroll_y(),
            window.get_task_view_height(),
            expanded,
        );
        let pin = self.drag_source.map(|source| source.index).or(editor);
        self.viewports.main.sync(
            bounds,
            pin,
            |index, top| {
                Some(ui::PositionedTask {
                    index: i32::try_from(index).ok()?,
                    top,
                    item: self.rows.row_data(index)?,
                })
            },
            expanded,
        );
        let bounds = self.viewports.side.visible(
            window.get_side_scroll_y(),
            window.get_side_view_height(),
            None,
        );
        self.viewports.side.sync(
            bounds,
            None,
            |index, top| {
                Some(ui::PositionedSide {
                    top,
                    item: self.sidebar.row_data(index)?,
                })
            },
            None,
        );
    }
}
