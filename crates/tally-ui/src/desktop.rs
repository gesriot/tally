use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result};
use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, Timer, TimerMode};
use tally_client::desktop::{Desktop, Show, ShowRequest};

#[cfg(target_os = "linux")]
mod wayland;
use tally_store::Reminder;
use tally_ui::bind::{Bound, show_quick_entry};
use tally_ui::taskbar;
use tally_ui::ui::{AppWindow, DesktopTray, QuickEntry, ReminderNotice, SettingsWindow};

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{Code, HotKey, Modifiers},
};

pub fn run(
    window: &AppWindow,
    entry: &QuickEntry,
    bound: &Bound,
    ipc: &Desktop,
    initial: &ShowRequest,
) -> Result<()> {
    let tray = DesktopTray::new()?;
    let w = window.as_weak();
    tray.on_open_window(move || {
        if let Some(window) = w.upgrade() {
            log_display(show_window(&window).map_err(Into::into));
        }
    });
    let q = entry.as_weak();
    tray.on_open_quick_entry(move || {
        if let Some(entry) = q.upgrade() {
            log_display(show_quick_entry(&entry).map_err(Into::into));
        }
    });
    tray.on_quit(|| {
        let _ = slint::quit_event_loop();
    });
    tray.show()?;

    let notice = ReminderNotice::new()?;
    let settings = SettingsWindow::new()?;
    bound.attach_settings(&settings, &notice, ipc.backup_directory());
    let weak = notice.as_weak();
    notice.on_dismiss(move || {
        if let Some(notice) = weak.upgrade() {
            notice.invoke_clear_body();
            let _ = notice.hide();
        }
    });
    let weak = notice.as_weak();
    notice.window().on_close_requested(move || {
        if let Some(notice) = weak.upgrade() {
            notice.invoke_clear_body();
        }
        slint::CloseRequestResponse::HideWindow
    });

    let (delivery, outcomes) = mpsc::channel();
    bound.enable_reminders(move |reminders| {
        let delivery = delivery.clone();
        std::thread::spawn(move || {
            let _ = delivery.send(notify(reminders));
        });
    });

    show_from_shell(initial.show, Some(window), Some(entry))?;
    let requests = Rc::new(RefCell::new(vec![initial.clone()]));
    let waiting = Rc::clone(&requests);
    let window_weak = window.as_weak();
    let entry_weak = entry.as_weak();
    let hotkey = Hotkey::install();
    let timer = Timer::default();
    let mut ipc_failed = false;
    let mailbox = ipc.clone();
    let bound = bound.clone();
    let notice_weak = notice.as_weak();
    let mut pending_notice = None;
    timer.start(TimerMode::Repeated, Duration::from_millis(25), move || {
        if let Ok(outcome) = outcomes.try_recv() {
            match outcome {
                None => bound.complete_reminders(true),
                Some(body) => pending_notice = Some(body),
            }
        }
        if let Some(body) = pending_notice.take() {
            match show_notice(notice_weak.upgrade(), &body) {
                Ok(true) => bound.complete_reminders(true),
                Ok(false) => pending_notice = Some(body),
                Err(error) => {
                    eprintln!("окно напоминания: {error}");
                    bound.complete_reminders(false);
                }
            }
        }
        let pending = mailbox.receive();
        if let Err(error) = &pending
            && !ipc_failed
        {
            eprintln!("обмен с tally: {error:#}");
        }
        ipc_failed = pending.is_err();
        let window = window_weak.upgrade();
        let entry = entry_weak.upgrade();
        let mut requests = waiting.borrow_mut();
        if let Ok(pending) = pending {
            requests.extend(pending);
        }
        requests.retain(|request| {
            let reply = match show_from_shell(request.show, window.as_ref(), entry.as_ref()) {
                Ok(window) if !activate(window, request.activation_token.as_deref()) => {
                    return true;
                }
                Ok(_) => Ok(()),
                Err(error) => {
                    eprintln!("не удалось показать Tally: {error:#}");
                    Err(format!("{error:#}"))
                }
            };
            log_display(mailbox.respond(request, reply));
            false
        });
        if hotkey.pressed()
            && let Some(entry) = entry.as_ref()
        {
            log_display(show_quick_entry(entry).map_err(Into::into));
        }
    });
    // Действующий системный трей сам удерживает цикл после скрытия последнего окна.
    let outcome = slint::run_event_loop().map_err(Into::into);
    timer.stop();
    let cause = shutdown_cause(&outcome);
    for request in requests.borrow_mut().drain(..) {
        log_display(ipc.respond(&request, Err(cause.clone())));
    }
    outcome
}

pub fn shutdown_cause(outcome: &Result<()>) -> String {
    match outcome {
        Ok(()) => "Tally завершает работу".to_owned(),
        Err(error) => format!("{error:#}"),
    }
}

fn show_notice(notice: Option<ReminderNotice>, body: &str) -> Result<bool> {
    let notice = notice.context("окно напоминаний недоступно")?;
    notice.show()?;
    taskbar::set_icons(&notice);
    if notice.window().with_winit_window(|_| ()).is_none() {
        return Ok(false);
    }
    notice.invoke_append_body(body.into());
    Ok(true)
}

fn notify(reminders: Vec<Reminder>) -> Option<String> {
    let body = reminders
        .into_iter()
        .map(|reminder| reminder.title)
        .collect::<Vec<_>>()
        .join("\n");
    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
    match notify_rust::Notification::new()
        .appname("Tally")
        .summary("Tally – напоминание")
        .body(&body)
        .show()
    {
        Ok(_) => return None,
        Err(error) => eprintln!("системное уведомление: {error}"),
    }
    Some(body)
}

fn show_window(window: &AppWindow) -> Result<(), slint::PlatformError> {
    window.show()?;
    taskbar::set_icons(window);
    window.window().set_minimized(false);
    window
        .window()
        .with_winit_window(|window| window.focus_window());
    Ok(())
}

fn show_from_shell<'a>(
    show: Show,
    window: Option<&'a AppWindow>,
    entry: Option<&'a QuickEntry>,
) -> Result<&'a slint::Window> {
    match show {
        Show::Window => {
            let window = window.context("главное окно недоступно")?;
            show_window(window)?;
            Ok(window.window())
        }
        Show::QuickAdd => {
            let entry = entry.context("быстрый ввод недоступен")?;
            show_quick_entry(entry)?;
            Ok(entry.window())
        }
    }
}

fn activate(window: &slint::Window, _token: Option<&str>) -> bool {
    window
        .with_winit_window(|_native| {
            #[cfg(target_os = "linux")]
            if let Some(token) = _token
                && let Err(error) = wayland::activate(_native, token)
            {
                eprintln!("активация Wayland: {error:#}");
            }
        })
        .is_some()
}

fn log_display(result: Result<()>) {
    if let Err(error) = result {
        eprintln!("не удалось показать Tally: {error:#}");
    }
}

#[derive(Default)]
struct Hotkey {
    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
    registration: Option<(GlobalHotKeyManager, HotKey)>,
}

impl Hotkey {
    fn install() -> Self {
        #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
        {
            if cfg!(target_os = "linux")
                && (std::env::var_os("WAYLAND_DISPLAY").is_some()
                    || std::env::var("XDG_SESSION_TYPE")
                        .is_ok_and(|s| s.eq_ignore_ascii_case("wayland")))
            {
                return Self::default();
            }
            let key = HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::Space);
            let registration = GlobalHotKeyManager::new().and_then(|manager| {
                manager.register(key)?;
                Ok((manager, key))
            });
            match registration {
                Ok(registration) => {
                    return Self {
                        registration: Some(registration),
                    };
                }
                Err(error) => eprintln!("глобальный быстрый ввод Ctrl+Alt+Пробел: {error}"),
            }
        }
        Self::default()
    }

    fn pressed(&self) -> bool {
        #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
        if let Some((_, key)) = &self.registration {
            let mut pressed = false;
            for event in GlobalHotKeyEvent::receiver().try_iter() {
                pressed |= event.id() == key.id() && event.state() == HotKeyState::Pressed;
            }
            return pressed;
        }
        false
    }
}
