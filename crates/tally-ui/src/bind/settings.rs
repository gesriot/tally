use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, Timer, TimerMode, winit_030::WinitWindowAccessor};
use tally_app::{Appearance, DataReply, DataRequest, Result, Settings};

use super::{Bound, Inner, Request, ui};

pub(super) struct Binding {
    window: slint::Weak<ui::SettingsWindow>,
    notice: slint::Weak<ui::ReminderNotice>,
    directory: Option<PathBuf>,
    current: Option<Settings>,
    backup_pending: bool,
    timer: Timer,
}

impl Bound {
    pub fn attach_settings(
        &self,
        settings: &ui::SettingsWindow,
        notice: &ui::ReminderNotice,
        directory: std::result::Result<PathBuf, impl std::fmt::Display>,
    ) {
        let directory = match directory {
            Ok(path) => {
                settings.set_backup_directory(path.to_string_lossy().as_ref().into());
                Some(path)
            }
            Err(error) => {
                settings.set_backup_error(error.to_string().into());
                None
            }
        };
        let weak = Rc::downgrade(&self.inner);
        let timer = Timer::default();
        timer.start(TimerMode::Repeated, Duration::from_secs(60), move || {
            if let Some(inner) = weak.upgrade() {
                inner.borrow_mut().request_backup();
                Inner::drain(&inner);
            }
        });
        let mut inner = self.inner.borrow_mut();
        let Some(window) = inner.window.upgrade() else {
            return;
        };
        inner.settings = Some(Binding {
            window: settings.as_weak(),
            notice: notice.as_weak(),
            directory,
            current: None,
            backup_pending: false,
            timer,
        });
        drop(inner);

        window.set_settings_available(true);
        let weak = Rc::downgrade(&self.inner);
        let handle = settings.as_weak();
        window.on_open_settings(move || {
            if let Some(settings) = handle.upgrade() {
                if let Err(error) = settings.show() {
                    eprintln!("окно настроек: {error}");
                    return;
                }
                crate::taskbar::set_icons(&settings);
                settings.window().set_minimized(false);
                settings
                    .window()
                    .with_winit_window(|window| window.focus_window());
                let Some(inner) = weak.upgrade() else { return };
                if !settings.get_ready() && !settings.get_busy() {
                    settings.set_busy(true);
                    Inner::submit(&inner, Request::Data(DataRequest::Settings));
                }
            }
        });
        let handle = settings.as_weak();
        settings.on_dismiss(move || {
            if let Some(settings) = handle.upgrade() {
                let _ = settings.hide();
            }
        });
        let weak = Rc::downgrade(&self.inner);
        settings.on_set_appearance(move |index| {
            if let Some(inner) = weak.upgrade() {
                let current = inner
                    .borrow()
                    .settings
                    .as_ref()
                    .and_then(|binding| binding.current);
                if let Some(mut value) = current {
                    value.appearance = match index {
                        1 => Appearance::Light,
                        2 => Appearance::Dark,
                        _ => Appearance::System,
                    };
                    Self::settings_request(&inner, DataRequest::SaveSettings(value));
                }
            }
        });
        let weak = Rc::downgrade(&self.inner);
        settings.on_set_automatic_backups(move |enabled| {
            if let Some(inner) = weak.upgrade() {
                let current = inner
                    .borrow()
                    .settings
                    .as_ref()
                    .and_then(|binding| binding.current);
                if let Some(mut value) = current {
                    value.automatic_backups = enabled;
                    Self::settings_request(&inner, DataRequest::SaveSettings(value));
                }
            }
        });
        for import in [false, true] {
            let weak = Rc::downgrade(&self.inner);
            let handle = settings.as_weak();
            let transfer = move || {
                if let (Some(inner), Some(settings)) = (weak.upgrade(), handle.upgrade()) {
                    let input = settings.get_json_path();
                    if input.trim().is_empty() {
                        settings.set_status("Укажите путь к файлу JSON".into());
                        settings.set_status_error(true);
                        return;
                    }
                    let path = PathBuf::from(input.as_str());
                    let request = if import {
                        DataRequest::Import(path)
                    } else {
                        DataRequest::Export(path)
                    };
                    Self::settings_request(&inner, request);
                }
            };
            if import {
                settings.on_import_json(transfer);
            } else {
                settings.on_export_json(transfer);
            }
        }
        settings.set_busy(true);
        Inner::submit(&self.inner, Request::Data(DataRequest::Settings));
    }

    fn settings_request(inner: &Rc<std::cell::RefCell<Inner>>, request: DataRequest) {
        let handle = inner
            .borrow()
            .settings
            .as_ref()
            .and_then(|binding| binding.window.upgrade());
        if let Some(window) = handle
            && window.get_ready()
            && !window.get_busy()
        {
            window.set_status("".into());
            window.set_busy(true);
            Inner::submit(inner, Request::Data(request));
        }
    }
}

impl Inner {
    fn request_backup(&mut self) {
        let Some(binding) = self.settings.as_mut() else {
            return;
        };
        if !binding.backup_pending
            && binding
                .current
                .is_some_and(|settings| settings.automatic_backups)
            && let Some(directory) = binding.directory.clone()
        {
            binding.backup_pending = true;
            self.backend
                .submit(Request::Data(DataRequest::DailyBackup(directory)));
        }
    }

    pub(super) fn on_data_reply(&mut self, backup: bool, result: Result<DataReply>) {
        let Some(binding) = self.settings.as_mut() else {
            return;
        };
        let Some(settings) = binding.window.upgrade() else {
            return;
        };
        if backup {
            binding.backup_pending = false;
            settings.set_backup_error(match result {
                Ok(_) => "".into(),
                Err(error) => format!("Резервная копия не создана. {error}").into(),
            });
            return;
        }
        settings.set_busy(false);
        settings.set_status_error(result.is_err());
        match result {
            Ok(DataReply::Settings(value)) => {
                let appearance = match value.appearance {
                    Appearance::System => 0,
                    Appearance::Light => 1,
                    Appearance::Dark => 2,
                };
                let start_backups = value.automatic_backups
                    && binding
                        .current
                        .is_none_or(|current| !current.automatic_backups);
                binding.current = Some(value);
                settings.set_appearance(appearance);
                settings.set_ready(true);
                settings.set_status("".into());
                settings.set_automatic_backups(value.automatic_backups);
                settings.invoke_restore_controls(appearance, value.automatic_backups);
                settings
                    .global::<ui::Theme<'_>>()
                    .invoke_set_appearance(appearance);
                if let Some(window) = self.window.upgrade() {
                    window
                        .global::<ui::Theme<'_>>()
                        .invoke_set_appearance(appearance);
                }
                if let Some(entry) = self.quick_entry.as_ref().and_then(slint::Weak::upgrade) {
                    entry
                        .global::<ui::Theme<'_>>()
                        .invoke_set_appearance(appearance);
                }
                if let Some(notice) = binding.notice.upgrade() {
                    notice
                        .global::<ui::Theme<'_>>()
                        .invoke_set_appearance(appearance);
                }
                if start_backups {
                    self.request_backup();
                }
            }
            Ok(DataReply::Exported(path)) => {
                settings.set_status(format!("JSON сохранён в {}", path.display()).into());
            }
            Ok(DataReply::Imported(count)) => {
                settings.set_status(format!("Добавлено записей: {count}").into());
                if count > 0 {
                    self.backend.submit(Request::Read);
                }
            }
            Ok(DataReply::DailyBackup) => {}
            Err(error) => {
                if let Some(value) = binding.current {
                    // Встроенные виджеты сразу меняют своё значение.
                    let appearance = settings.get_appearance();
                    settings.invoke_restore_controls(appearance, value.automatic_backups);
                }
                settings.set_status(error.to_string().into());
            }
        }
    }
}

impl Drop for Binding {
    fn drop(&mut self) {
        self.timer.stop();
    }
}
