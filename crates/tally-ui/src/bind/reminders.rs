use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use tally_app::Result;
use tally_store::Reminder;

use super::{Bound, Inner};
use crate::backend::Request;

type Notify = Rc<dyn Fn(Vec<Reminder>)>;

#[derive(Default)]
enum State {
    #[default]
    Idle,
    Claiming,
    Showing(Vec<Reminder>),
    Confirming {
        shown: Vec<Reminder>,
        submitted: bool,
    },
}

#[derive(Default)]
pub(super) struct Delivery {
    timer: slint::Timer,
    notify: Option<Notify>,
    state: State,
}

impl Bound {
    /// Начать опрос. Уведомитель подтверждает показ через `complete_reminders`.
    pub fn enable_reminders(&self, notify: impl Fn(Vec<Reminder>) + 'static) {
        let weak = Rc::downgrade(&self.inner);
        let check = move || {
            if let Some(inner) = weak.upgrade() {
                poll(&inner);
            }
        };
        {
            let mut inner = self.inner.borrow_mut();
            inner.reminders.notify = Some(Rc::new(notify));
            inner.reminders.timer.start(
                slint::TimerMode::Repeated,
                Duration::from_secs(5),
                check.clone(),
            );
        }
        check();
    }

    pub fn complete_reminders(&self, shown: bool) {
        let mut inner = self.inner.borrow_mut();
        let state = &mut inner.reminders.state;
        if matches!(state, State::Showing(_))
            && let State::Showing(batch) = std::mem::take(state)
            && shown
        {
            *state = State::Confirming {
                shown: batch,
                submitted: false,
            };
            drop(inner);
            poll(&self.inner);
        }
    }
}

fn poll(inner: &Rc<RefCell<Inner>>) {
    let request = {
        let mut inner = inner.borrow_mut();
        let state = &mut inner.reminders.state;
        match state {
            State::Idle => {
                *state = State::Claiming;
                Request::Reminders
            }
            State::Confirming { shown, submitted } if !*submitted => {
                *submitted = true;
                Request::ConfirmReminders(shown.clone())
            }
            _ => return,
        }
    };
    Inner::submit(inner, request);
}

impl Delivery {
    pub(super) fn claimed(
        &mut self,
        result: Result<Vec<Reminder>>,
    ) -> Option<(Notify, Vec<Reminder>)> {
        self.state = State::Idle;
        match result {
            Ok(batch) if !batch.is_empty() => {
                let notify = self.notify.clone()?;
                self.state = State::Showing(batch.clone());
                Some((notify, batch))
            }
            Ok(_) => None,
            Err(error) => {
                eprintln!("напоминания: {error}");
                None
            }
        }
    }

    pub(super) fn confirmed(&mut self, result: Result<()>) {
        match result {
            Ok(()) => self.state = State::Idle,
            Err(error) => {
                eprintln!("подтверждение напоминаний: {error}");
                if let State::Confirming { submitted, .. } = &mut self.state {
                    *submitted = false;
                }
            }
        }
    }
}
