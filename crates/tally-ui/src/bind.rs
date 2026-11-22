//! Привязка [`Controller`] к окну Slint: модели строк, обратные вызовы,
//! сообщение внизу окна. Решений здесь нет – только перенос данных.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, VecModel};

use crate::backend::{Backend, Calendar, Reply, Request};
use crate::controller::Controller;
use crate::keys::Press;
use crate::strings;
use crate::ui;
use crate::view::{Icon, Mark, Row, RowKind, SideKind, SideRow};

mod drag;
mod editor;
mod organization;
mod quick_entry;
mod reminders;
mod repeat;
mod search;
mod settings;
mod viewport;
use drag::DragLayout;
use editor::Editor;
use organization::OrganizationForm;
pub use quick_entry::show_quick_entry;
use viewport::Viewports;

/// Сколько видно сообщение внизу окна.
pub const MESSAGE_TIME: Duration = Duration::from_secs(5);

/// Окно с подключённой базой.
#[derive(Clone)]
pub struct Bound {
    inner: Rc<RefCell<Inner>>,
}

struct Inner {
    controller: Controller,
    backend: Box<dyn Backend>,
    calendar: Calendar,
    window: slint::Weak<ui::AppWindow>,
    sidebar: Rc<VecModel<ui::SideItem>>,
    rows: Rc<VecModel<ui::RowItem>>,
    message_timer: Timer,
    day_timer: Timer,
    reminders: reminders::Delivery,
    settings: Option<settings::Binding>,
    observed_day: jiff::civil::Date,
    shown_message: u64,
    selected_task: SharedString,
    editor: Option<Editor>,
    organization: Option<OrganizationForm>,
    quick_entry: Option<slint::Weak<ui::QuickEntry>>,
    drag_layout: DragLayout,
    drag_source: Option<drag::Source>,
    viewports: Viewports,
    search: search::QuickFind,
    trash_targets: Vec<Option<(tally_store::RecordId, jiff::Timestamp)>>,
}

impl Bound {
    /// Подключить окно: обратные вызовы, первая загрузка базы.
    pub fn attach(window: &ui::AppWindow, backend: Box<dyn Backend>, calendar: Calendar) -> Self {
        let sidebar = Rc::new(VecModel::default());
        let rows = Rc::new(VecModel::default());
        window.set_window_title(strings::WINDOW_TITLE.into());
        window.set_sidebar(ModelRc::from(sidebar.clone()));
        window.set_rows(ModelRc::from(rows.clone()));
        window.set_editor_labels(strings::editor_labels());
        window.set_repeat_labels(strings::repeat_labels());
        window.set_trash_move_label(strings::TRASH_MOVE.into());
        window.set_trash_restore_label(strings::TRASH_RESTORE.into());
        window.set_trash_empty_label(strings::TRASH_EMPTY.into());
        window.set_trash_confirm_label(strings::TRASH_CONFIRM.into());
        let inner = Rc::new(RefCell::new(Inner {
            controller: Controller::new(),
            backend,
            calendar,
            window: window.as_weak(),
            sidebar,
            rows,
            message_timer: Timer::default(),
            day_timer: Timer::default(),
            reminders: reminders::Delivery::default(),
            settings: None,
            observed_day: calendar.today(),
            shown_message: 0,
            selected_task: SharedString::new(),
            editor: None,
            organization: None,
            quick_entry: None,
            drag_layout: DragLayout::new(window),
            drag_source: None,
            viewports: Viewports::new(window),
            search: search::QuickFind::default(),
            trash_targets: Vec::new(),
        }));

        let weak = Rc::downgrade(&inner);
        window.on_select(move |key| {
            if let Some(inner) = weak.upgrade() {
                let changed = {
                    let mut me = inner.borrow_mut();
                    if me.organization.is_some() {
                        return;
                    }
                    if me.editor.is_some() {
                        if key.as_str() != me.controller.selection().key()
                            && let Some(window) = me.window.upgrade()
                        {
                            window.set_edit_error(strings::EDITOR_FINISH_FIRST.into());
                        }
                        return;
                    }
                    let changed = me.controller.select(&key);
                    let changed = if changed || key.as_str() == me.controller.selection().key() {
                        me.dismiss_search() || changed
                    } else {
                        false
                    };
                    if changed {
                        me.cancel_drag();
                        me.selected_task = SharedString::new();
                    }
                    changed
                };
                if changed {
                    Inner::render(&inner);
                }
            }
        });
        let weak = Rc::downgrade(&inner);
        window.on_toggle(move |key| {
            if let Some(inner) = weak.upgrade() {
                let request = {
                    let mut me = inner.borrow_mut();
                    if me.organization.is_some()
                        || me.editor.as_ref().is_some_and(|e| e.row.key == key)
                    {
                        return;
                    }
                    let in_search = me.search.active;
                    me.controller.toggle_in(&key, in_search)
                };
                if let Some(request) = request
                    && !Inner::submit(&inner, request)
                {
                    // Обновить состояние отметки, пока ответ в пути.
                    Inner::render(&inner);
                }
            }
        });
        let weak = Rc::downgrade(&inner);
        window.on_key(move |text, control, shift, alt, meta| {
            let Some(inner) = weak.upgrade() else {
                return false;
            };
            let press = Press {
                text: &text,
                control,
                shift,
                alt,
                meta,
            };
            if inner.borrow().drag_source.is_some() {
                inner.borrow_mut().cancel_drag();
                if crate::keys::is(&text, slint::platform::Key::Escape) {
                    return true;
                }
            }
            if inner.borrow().organization.is_some() {
                if crate::keys::is(&text, slint::platform::Key::Escape) {
                    Inner::finish_organization(&inner, ui::OrganizationSubmit::Dismiss);
                    return true;
                }
                if crate::keys::action(press) != Some(crate::keys::Action::Refresh) {
                    return false;
                }
            }
            if Inner::search_key(&inner, press) || Inner::editor_key(&inner, press) {
                return true;
            }
            if inner.borrow().editor.is_some()
                && crate::keys::action(press) != Some(crate::keys::Action::Refresh)
            {
                return false;
            }
            let request = inner.borrow_mut().controller.key(press);
            match request {
                Some(request) => {
                    Inner::submit(&inner, request);
                    true
                }
                None => false,
            }
        });
        let weak = Rc::downgrade(&inner);
        window.on_poll(move || {
            if let Some(inner) = weak.upgrade() {
                Inner::drain(&inner);
            }
        });

        let weak = Rc::downgrade(&inner);
        window.on_viewport_changed(move || {
            if let Some(inner) = weak.upgrade()
                && let Ok(mut me) = inner.try_borrow_mut()
                && let Some(window) = me.window.upgrade()
            {
                me.refresh_viewports(&window);
            }
        });
        drag::connect(window, &inner);
        editor::connect(window, &inner);
        organization::connect(window, &inner);
        search::connect(window, &inner);
        let weak = Rc::downgrade(&inner);
        inner.borrow().day_timer.start(
            slint::TimerMode::Repeated,
            Duration::from_secs(30),
            move || {
                if let Some(inner) = weak.upgrade() {
                    let reload = {
                        let mut me = inner.borrow_mut();
                        let today = me.calendar.today();
                        let changed = today != me.observed_day;
                        me.observed_day = today;
                        changed
                    };
                    if reload {
                        Inner::submit(&inner, Request::Load);
                    }
                }
            },
        );
        Inner::submit(&inner, Request::Load);
        Self { inner }
    }

    /// Забрать готовые ответы потока базы (то же делает `poll` окна).
    pub fn poll(&self) {
        Inner::drain(&self.inner);
    }
}

impl Inner {
    fn submit(this: &Rc<RefCell<Self>>, request: Request) -> bool {
        this.borrow_mut().backend.submit(request);
        Self::drain(this)
    }

    fn drain(this: &Rc<RefCell<Self>>) -> bool {
        let mut render = false;
        let mut notify = None;
        {
            let mut me = this.borrow_mut();
            loop {
                let replies = me.backend.replies();
                if replies.is_empty() {
                    break;
                }
                if replies.iter().any(|reply| {
                    !matches!(
                        reply,
                        Reply::Data { .. } | Reply::Reminders(_) | Reply::RemindersConfirmed(_)
                    )
                }) {
                    render = true;
                    me.cancel_drag();
                }
                let mut refresh_search = false;
                for reply in replies {
                    match reply {
                        Reply::Data { backup, result } => {
                            me.on_data_reply(backup, result);
                            continue;
                        }
                        Reply::Reminders(result) => {
                            notify = me.reminders.claimed(result);
                            continue;
                        }
                        Reply::RemindersConfirmed(result) => {
                            me.reminders.confirmed(result);
                            continue;
                        }
                        _ => {}
                    }
                    if let Reply::Searched { serial, result } = reply {
                        me.accept_search(serial, result);
                        continue;
                    }
                    refresh_search |= matches!(
                        &reply,
                        Reply::Loaded(Ok(_))
                            | Reply::Ran(_, Ok(_))
                            | Reply::StaleEdit {
                                refreshed: Ok(_),
                                ..
                            }
                            | Reply::StaleOrganization {
                                refreshed: Ok(_),
                                ..
                            }
                            | Reply::StaleDrop {
                                refreshed: Ok(_),
                                ..
                            }
                    );
                    me.on_editor_reply(&reply);
                    me.on_quick_entry_reply(&reply);
                    me.on_organization_reply(&reply);
                    me.controller.apply(reply);
                }
                if refresh_search && let Some(request) = me.search.refresh() {
                    me.backend.submit(request);
                }
            }
        }
        if render {
            Self::render(this);
        }
        if let Some((notify, batch)) = notify {
            notify(batch);
        }
        render
    }

    fn render(this: &Rc<RefCell<Self>>) {
        let mut borrowed = this.borrow_mut();
        let me = &mut *borrowed;
        let Some(window) = me.window.upgrade() else {
            return;
        };
        let scroll_top = -window.get_task_scroll_y();
        let old_visible = me.search_selection_bounds().filter(|&(top, bottom)| {
            bottom > scroll_top && top < scroll_top + window.get_task_view_height()
        });
        let today = me.calendar.today();
        let reopenable = me.refresh_editor_reopenable(today);
        let sidebar = me.controller.sidebar(today);
        let page = if me.search.active {
            me.controller.search_page(today, &me.search.hits)
        } else {
            me.controller.page(today)
        };
        let selection = me.controller.selection();
        let switched_page = me.drag_layout.update(selection, &page.rows, &sidebar);
        let side: Vec<ui::SideItem> = sidebar.iter().map(side_item).collect();
        me.trash_targets = page.rows.iter().map(|row| row.trash_record).collect();
        let drag_allowed = !me.search.active
            && me.controller.tag_filter().is_none()
            && !matches!(selection, crate::view::Selection::Tag(_));
        let mut rows: Vec<ui::RowItem> = page.rows.iter().map(row_item).collect();
        if !drag_allowed {
            for row in &mut rows {
                row.draggable = false;
            }
        }
        if let Some(editor) = &me.editor {
            // F5 может перенести или удалить строку. Карточку с черновиком
            // всё равно оставляем доступной.
            if !rows.iter().any(|row| row.row_key == editor.row.row_key) {
                rows.insert(editor.position.min(rows.len()), editor.row.clone());
            }
        } else if !rows
            .iter()
            .any(|row| row.editable && row.row_key == me.selected_task)
        {
            me.selected_task = SharedString::new();
        }
        if !me.sidebar.iter().eq(side.iter().cloned()) {
            me.sidebar.set_vec(side);
            me.viewports.layout_side(&window, &me.sidebar);
        }
        let editor_index = me.editor.as_ref().and_then(|editor| {
            rows.iter()
                .position(|row| row.row_key == editor.row.row_key)
        });
        window.set_edit_row_index(
            editor_index
                .and_then(|index| i32::try_from(index).ok())
                .unwrap_or(-1),
        );
        if !me.rows.iter().eq(rows.iter().cloned()) {
            me.rows.set_vec(rows);
            me.viewports.layout_main(&window, &me.rows);
        }
        let message = me.controller.message().cloned();
        window.set_empty_evening(drag_allowed && me.drag_layout.has_empty_evening());
        window.set_evening_title(strings::EVENING.into());
        if switched_page {
            window.invoke_reveal_row(0.0, 0.0);
        }
        me.sync_search_selection(&page.rows);
        window.invoke_constrain_scroll();
        if let Some(old) = old_visible
            && let Some((top, bottom)) = me.search_selection_bounds()
            && old != (top, bottom)
        {
            window.invoke_reveal_row(top, bottom);
        }
        me.refresh_viewports(&window);
        window.set_selected_task(me.selected_task.clone());
        window.set_edit_reopenable(reopenable);
        let trash_page = !me.search.active && selection == crate::view::Selection::Trash;
        window.set_trash_page(trash_page);
        window.set_trash_nonempty(
            trash_page
                && me
                    .controller
                    .snapshot()
                    .is_some_and(|snapshot| !tally_app::trash_preview(snapshot).is_empty()),
        );
        window.set_page_title(page.title.into());
        me.render_search(&window, &page.rows);
        window.set_page_icon(icon(selection.icon()));
        window.set_organization_page(match selection {
            _ if me.search.active => ui::OrganizationPage::List,
            crate::view::Selection::Area(_) => ui::OrganizationPage::Area,
            crate::view::Selection::Project(_) => ui::OrganizationPage::Project,
            _ => ui::OrganizationPage::List,
        });
        window.set_organization_ready(me.controller.snapshot().is_some());
        window.set_page_progress(page.progress.map_or(0.0, |p| p.fraction() as f32));
        window.set_page_progress_text(page.progress.map_or_else(SharedString::new, |p| {
            strings::progress(p.done, p.total).into()
        }));
        match message {
            Some(m) => {
                window.set_message(m.text.into());
                if m.serial != me.shown_message {
                    me.shown_message = m.serial;
                    let weak = Rc::downgrade(this);
                    me.message_timer
                        .start(slint::TimerMode::SingleShot, MESSAGE_TIME, move || {
                            if let Some(inner) = weak.upgrade() {
                                inner.borrow_mut().controller.hide_message(m.serial);
                                Self::render(&inner);
                            }
                        });
                }
            }
            None => window.set_message(SharedString::new()),
        }
    }
}

fn icon(i: Icon) -> ui::Icon {
    match i {
        Icon::Inbox => ui::Icon::Inbox,
        Icon::Today => ui::Icon::Today,
        Icon::Upcoming => ui::Icon::Upcoming,
        Icon::Anytime => ui::Icon::Anytime,
        Icon::Someday => ui::Icon::Someday,
        Icon::Logbook => ui::Icon::Logbook,
        Icon::Trash => ui::Icon::Trash,
        Icon::Area => ui::Icon::Area,
        Icon::Project => ui::Icon::Project,
    }
}

fn side_item(r: &SideRow) -> ui::SideItem {
    ui::SideItem {
        kind: match r.kind {
            SideKind::List => ui::SideKind::List,
            SideKind::Gap => ui::SideKind::Gap,
            SideKind::Area => ui::SideKind::Area,
            SideKind::Project => ui::SideKind::Project,
        },
        nested: r.nested,
        key: r.key.as_str().into(),
        title: r.title.as_str().into(),
        count: r.count.as_str().into(),
        due: r.due,
        selected: r.selected,
        icon: icon(r.icon),
        progress: r.progress,
    }
}

fn row_item(r: &Row) -> ui::RowItem {
    ui::RowItem {
        kind: match r.kind {
            RowKind::Task => ui::RowKind::Task,
            RowKind::Project => ui::RowKind::Project,
            RowKind::Link => ui::RowKind::Link,
            RowKind::Section => ui::RowKind::Section,
            RowKind::Other => ui::RowKind::Other,
        },
        key: r.key.as_str().into(),
        title: r.title.as_str().into(),
        when: r.when.as_str().into(),
        meta: r.meta.as_str().into(),
        deadline: r.deadline.as_str().into(),
        due: r.due,
        mark: match r.mark {
            Mark::Open => ui::Mark::Open,
            Mark::Completed => ui::Mark::Completed,
            Mark::Canceled => ui::Mark::Canceled,
        },
        checkable: r.checkable,
        opens: r.opens,
        row_key: r.row_key.as_str().into(),
        draggable: r.drag_task.is_some(),
        editable: r.editable,
        reopenable: r.reopenable,
        trashable: r.trash_record.is_some(),
        star: r.star,
        progress: r.progress,
    }
}
