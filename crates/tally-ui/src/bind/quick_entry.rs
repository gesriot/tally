use std::rc::Rc;

use slint::{ComponentHandle, SharedString, winit_030::WinitWindowAccessor};
use tally_app::{Command, NewTask};

use super::{Bound, Inner};
use crate::backend::{Purpose, Reply, Request};
use crate::{strings, ui};

impl Bound {
    pub fn attach_quick_entry(&self, entry: &ui::QuickEntry) {
        let mut me = self.inner.borrow_mut();
        me.quick_entry = Some(entry.as_weak());
        if let Some(window) = me.window.upgrade() {
            window.set_quick_entry_enabled(true);
            let entry = entry.as_weak();
            window.on_quick_entry(move || {
                if let Some(entry) = entry.upgrade()
                    && let Err(error) = show_quick_entry(&entry)
                {
                    eprintln!("быстрый ввод: {error}");
                }
            });
        }

        let weak = Rc::downgrade(&self.inner);
        let handle = entry.as_weak();
        entry.on_submit(move || {
            let Some((inner, entry)) = weak.upgrade().zip(handle.upgrade()) else {
                return;
            };
            if entry.get_busy() {
                return;
            }
            let text = entry.get_draft();
            if text.trim().is_empty() {
                entry.set_title_invalid(true);
                entry.set_error(SharedString::new());
                entry.invoke_focus_entry();
                return;
            }
            let notes = entry.get_notes();
            entry.set_title_invalid(false);
            entry.set_error(SharedString::new());
            entry.set_busy(true);
            Inner::submit(
                &inner,
                Request::Run(
                    Purpose::QuickAdd,
                    Box::new(Command::AddTask(NewTask {
                        text: text.to_string(),
                        notes: (!notes.is_empty()).then(|| notes.to_string()),
                        ..NewTask::default()
                    })),
                ),
            );
        });
        let handle = entry.as_weak();
        entry.on_dismiss(move || {
            if let Some(entry) = handle.upgrade() {
                let _ = entry.hide();
            }
        });
    }
}

impl Inner {
    pub(super) fn on_quick_entry_reply(&self, reply: &Reply) {
        let Reply::Ran(Purpose::QuickAdd, result) = reply else {
            return;
        };
        let Some(entry) = self.quick_entry.as_ref().and_then(slint::Weak::upgrade) else {
            return;
        };
        entry.set_busy(false);
        match result {
            Ok(_) => {
                entry.set_draft(SharedString::new());
                entry.set_notes(SharedString::new());
                let _ = entry.hide();
            }
            Err(error) => {
                entry.set_error(strings::error(error).into());
                entry.invoke_focus_entry();
            }
        }
    }
}

pub fn show_quick_entry(entry: &ui::QuickEntry) -> Result<(), slint::PlatformError> {
    entry.show()?;
    crate::taskbar::set_icons(entry);
    entry.window().set_minimized(false);
    entry.invoke_focus_entry();
    entry
        .window()
        .with_winit_window(|window| window.focus_window());
    Ok(())
}
