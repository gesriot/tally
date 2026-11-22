//! Состояние окна и решения по действиям пользователя – без Slint, чтобы
//! их проверяли модульные тесты. Окно передаёт сюда нажатия, отправляет
//! полученные [`Request`] в [`crate::backend::Backend`] и возвращает
//! ответы в [`Controller::apply`].

use jiff::civil::Date;
use tally_app::{Command, Effect, Error, Index};
use tally_core::{Status, TagId, Task, TaskId};
use tally_store::{Hit, Snapshot};
use uuid::Uuid;

use crate::backend::{Purpose, Reply, Request};
use crate::keys::{self, Action, Press};
use crate::strings;
use crate::view::{self, Page, SideRow, Toggle};

pub use crate::view::Selection;

/// Сообщение внизу окна и его номер: окно прячет сообщение через
/// несколько секунд, если за это время не пришло новое.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub serial: u64,
    pub text: String,
}

#[derive(Debug)]
struct KeptClose {
    task: TaskId,
    seq: i64,
    /// Состояние, при котором строка остаётся на странице. После отмены
    /// остаётся номер шага, чтобы вернуть отметку при повторе того же закрытия.
    status: Option<Status>,
}

fn closed_status(snapshot: &Snapshot, id: TaskId) -> Option<Status> {
    snapshot
        .tasks
        .iter()
        .find(|task| task.id == id && !task.status.is_open())
        .map(|task| task.status)
}

#[derive(Debug)]
pub struct Controller {
    snapshot: Option<Snapshot>,
    /// Если обновление после конфликта не удалось, прежняя версия
    /// задачи не годится для новой карточки.
    unavailable_for_edit: Option<TaskId>,
    organization_needs_refresh: bool,
    selection: Selection,
    tag_filter: Option<TagId>,
    /// Номер посещения страницы: растёт при каждом переходе. Ответ на
    /// отметку, сделанную при другом посещении, строку не удерживает – даже
    /// если пользователь успел уйти и вернуться.
    visit: u64,
    /// Закрытия из этого окна при текущем посещении страницы.
    kept: Vec<KeptClose>,
    search_visit: u64,
    search_close: Option<KeptClose>,
    /// Задача, закрытая последним действием этого окна, и номер этого
    /// действия в журнале: снять с неё отметку – отменить именно его
    /// ([`Command::UndoStep`]).
    last_close: Option<(TaskId, i64)>,
    /// Отметки, ответа на которые ещё нет: повторный щелчок не уходит.
    pending: Vec<TaskId>,
    message: Option<Message>,
    serial: u64,
}

impl Default for Controller {
    fn default() -> Self {
        Self::new()
    }
}

impl Controller {
    pub fn new() -> Self {
        Self {
            snapshot: None,
            unavailable_for_edit: None,
            organization_needs_refresh: false,
            selection: Selection::Today,
            tag_filter: None,
            visit: 0,
            kept: Vec::new(),
            search_visit: 0,
            search_close: None,
            last_close: None,
            pending: Vec::new(),
            message: None,
            serial: 0,
        }
    }

    pub fn selection(&self) -> Selection {
        self.selection
    }

    pub(crate) fn tag_filter(&self) -> Option<TagId> {
        self.tag_filter
    }

    pub(crate) fn set_tag_filter(&mut self, tag: Option<TagId>) {
        self.tag_filter = tag;
    }

    pub(crate) fn search_page(&self, today: Date, hits: &[Hit]) -> Page {
        self.snapshot.as_ref().map_or_else(
            || Page {
                title: strings::SEARCH.to_owned(),
                progress: None,
                rows: Vec::new(),
            },
            |s| view::search_page(s, today, hits, &|id, open| self.allow(id, open, true)),
        )
    }

    pub(crate) fn end_search(&mut self) {
        self.search_visit = self.search_visit.wrapping_add(1);
        self.search_close = None;
    }

    pub(crate) fn snapshot(&self) -> Option<&Snapshot> {
        self.snapshot.as_ref()
    }

    pub(crate) fn organization_needs_refresh(&self) -> bool {
        self.organization_needs_refresh
    }

    pub(crate) fn editable_task(&self, id: TaskId, today: Date) -> Option<(&Task, &Snapshot)> {
        if self.unavailable_for_edit == Some(id) {
            return None;
        }
        let snapshot = self.snapshot.as_ref()?;
        let task = snapshot.tasks.iter().find(|t| t.id == id)?;
        (!Index::new(snapshot, today).trashed(task)).then_some((task, snapshot))
    }

    pub fn message(&self) -> Option<&Message> {
        self.message.as_ref()
    }

    /// Убрать сообщение `serial` (если его ещё не сменило новое).
    pub fn hide_message(&mut self, serial: u64) {
        if self.message.as_ref().is_some_and(|m| m.serial == serial) {
            self.message = None;
        }
    }

    pub(crate) fn say(&mut self, text: String) {
        self.serial += 1;
        self.message = Some(Message {
            serial: self.serial,
            text,
        });
    }

    /// Открыть список, проект или область по ключу из боковой панели или
    /// строки проекта. `true` – выбор изменился.
    /// Закрытый проект в «Журнале» и удалённый в «Корзине» не открываются
    /// ([`view::can_open`]).
    pub fn select(&mut self, key: &str) -> bool {
        let opens = |s: Selection| {
            self.snapshot
                .as_ref()
                .is_none_or(|snap| view::can_open(snap, s))
        };
        Selection::parse(key)
            .filter(|&s| opens(s))
            .is_some_and(|s| self.set_selection(s))
    }

    fn set_selection(&mut self, selection: Selection) -> bool {
        if selection == self.selection {
            return false;
        }
        self.selection = selection;
        self.tag_filter = None;
        self.visit += 1;
        self.kept.clear();
        self.last_close = None;
        true
    }

    fn validate_selection(&mut self, today: Date) {
        if let Some(s) = &self.snapshot {
            self.set_selection(view::valid(s, today, self.selection));
        }
    }

    /// Можно ли нажать отметку задачи `id` (`open` – задача открыта):
    /// закрытие не ждёт ответа, а снять отметку можно только у задачи,
    /// закрытой последним действием окна на этой странице. Это правило
    /// [`view::toggle`] применяет и к виду строки, и к щелчку.
    fn allow(&self, id: TaskId, open: bool, in_search: bool) -> bool {
        if self.pending.contains(&id) {
            return false;
        }
        open || self.last_close.is_some_and(|(task, seq)| {
            let closes = if in_search {
                self.search_close.as_slice()
            } else {
                &self.kept
            };
            task == id
                && closes
                    .iter()
                    .any(|close| close.seq == seq && close.status.is_some())
        })
    }

    /// Нажатие на отметку задачи `key` (UUID). Открытая задача
    /// выполняется. Отметку только что закрытой задачи можно снять, пока
    /// её закрытие – последнее действие окна: это отмена именно этого
    /// действия, со сверкой номера журнала в той же транзакции. Иначе
    /// отметка не нажимается (и выглядит ненажимаемой).
    pub fn toggle(&mut self, key: &str) -> Option<Request> {
        self.toggle_in(key, false)
    }

    pub(crate) fn toggle_in(&mut self, key: &str, in_search: bool) -> Option<Request> {
        let id = TaskId(Uuid::parse_str(key).ok()?);
        let s = self.snapshot.as_ref()?;
        match view::toggle(s, id, &|id, open| self.allow(id, open, in_search))? {
            Toggle::Close => {
                self.pending.push(id);
                let purpose = if in_search {
                    Purpose::CloseInSearch {
                        task: id,
                        visit: self.search_visit,
                    }
                } else {
                    Purpose::Close {
                        task: id,
                        visit: self.visit,
                    }
                };
                Some(Request::Run(purpose, Box::new(Command::CloseTask(id))))
            }
            Toggle::Reopen => {
                let (_, seq) = self.last_close?;
                self.pending.push(id);
                Some(Request::Run(
                    Purpose::Reopen(id),
                    Box::new(Command::UndoStep(seq)),
                ))
            }
        }
    }

    /// Сочетание клавиш.
    pub fn key(&mut self, press: Press<'_>) -> Option<Request> {
        Some(match keys::action(press)? {
            Action::Undo => Request::Run(Purpose::Undo, Box::new(Command::Undo)),
            Action::Redo => Request::Run(Purpose::Redo, Box::new(Command::Redo)),
            Action::Refresh => Request::Load,
        })
    }

    /// Ответ потока базы. После каждого ответа окно перерисовывается
    /// по новому снимку.
    pub fn apply(&mut self, reply: Reply) {
        match reply {
            Reply::Data { .. } | Reply::Reminders(_) | Reply::RemindersConfirmed(_) => {}
            Reply::Loaded(Ok(s)) => self.accept_snapshot(s),
            Reply::Searched { .. } => {}
            Reply::Loaded(Err(e)) => self.say(strings::load_failed(&e)),
            Reply::StaleEdit {
                task,
                error,
                refreshed,
            } => match refreshed {
                Ok(snapshot) => {
                    self.accept_snapshot(snapshot);
                    self.say(strings::error(&error));
                }
                Err(error) => {
                    self.unavailable_for_edit = Some(task);
                    self.say(strings::load_failed(&error));
                }
            },
            Reply::StaleOrganization { refreshed, .. } => match refreshed {
                Ok(snapshot) => self.accept_snapshot(snapshot),
                Err(error) => {
                    self.organization_needs_refresh = true;
                    self.say(strings::load_failed(&error));
                }
            },
            Reply::StaleDrop { error, refreshed } => match refreshed {
                Ok(snapshot) => {
                    self.accept_snapshot(snapshot);
                    self.say(strings::error(&error));
                }
                Err(error) => self.say(strings::load_failed(&error)),
            },
            Reply::Ran(Purpose::Organize, Err(_)) => {}
            Reply::Ran(purpose, Err(e)) => {
                self.settle(&purpose);
                let text = match (&purpose, &e) {
                    (Purpose::Reopen(id), Error::NotLast) => {
                        if self.last_close.is_some_and(|(task, _)| task == *id) {
                            self.last_close = None;
                        }
                        strings::REOPEN_NOT_LAST.to_owned()
                    }
                    _ => strings::error(&e),
                };
                self.say(text);
            }
            Reply::Ran(purpose, Ok(done)) => {
                self.settle(&purpose);
                // Снимок принимается всегда; удержание строки – только если
                // со щелчка не было перехода.
                self.accept_snapshot(done.snapshot);
                match (purpose, done.effect) {
                    (Purpose::Close { task, visit }, _) if visit == self.visit => {
                        self.last_close = done.seq.map(|seq| (task, seq));
                        if let Some(seq) = done.seq {
                            let status =
                                self.snapshot.as_ref().and_then(|s| closed_status(s, task));
                            self.kept.push(KeptClose { task, seq, status });
                        }
                    }
                    (Purpose::CloseInSearch { task, visit }, _) if visit == self.search_visit => {
                        self.last_close = done.seq.map(|seq| (task, seq));
                        self.search_close = done.seq.map(|seq| KeptClose {
                            task,
                            seq,
                            status: self.snapshot.as_ref().and_then(|s| closed_status(s, task)),
                        });
                    }
                    (Purpose::Close { .. } | Purpose::CloseInSearch { .. }, _) => {
                        self.last_close = None;
                    }
                    // Отменилось ровно закрытие этой задачи – молча, как
                    // обычное снятие отметки.
                    (Purpose::Reopen(id), Effect::Undone(step)) if step.tasks == [id] => {
                        self.last_close = None;
                    }
                    (_, Effect::Undone(step)) => {
                        self.last_close = None;
                        self.say(strings::undone(&step.action));
                    }
                    (_, Effect::Redone(step)) => {
                        if let Some(close) = self.kept.iter_mut().find(|c| Some(c.seq) == done.seq)
                        {
                            close.status = self
                                .snapshot
                                .as_ref()
                                .and_then(|s| closed_status(s, close.task));
                        }
                        self.last_close = None;
                        self.say(strings::redone(&step.action));
                    }
                    (Purpose::QuickAdd, Effect::Added(_)) => {
                        self.last_close = None;
                        self.say("Задача добавлена".to_owned());
                    }
                    (Purpose::Organize | Purpose::Trash, effect) => {
                        if done.seq.is_some() || effect == Effect::TrashEmptied {
                            self.last_close = None;
                            self.organized(effect);
                        }
                    }
                    (Purpose::Edit(_) | Purpose::Drop, _) if done.seq.is_none() => {}
                    _ => self.last_close = None,
                }
            }
        }
    }

    fn organized(&mut self, effect: Effect) {
        let Some(snapshot) = self.snapshot.as_ref() else {
            return;
        };
        let selection = match effect {
            Effect::AreaAdded { title } => snapshot
                .areas
                .iter()
                .filter(|a| a.title == title)
                .map(|a| Selection::Area(a.id))
                .find(|&s| view::can_open(snapshot, s)),
            Effect::ProjectAdded { title } => snapshot
                .projects
                .iter()
                .filter(|p| p.title == title)
                .map(|p| Selection::Project(p.id))
                .find(|&s| view::can_open(snapshot, s)),
            Effect::ProjectRepeatSet => {
                self.say(strings::PROJECT_REPEAT_SET.to_owned());
                None
            }
            Effect::TrashChanged { title, trashed } => {
                let verb = if trashed {
                    strings::TRASH_MOVE
                } else {
                    strings::TRASH_RESTORE
                };
                self.say(format!("{verb} · «{title}»"));
                None
            }
            Effect::TrashEmptied => {
                self.say(strings::TRASH_EMPTIED.to_owned());
                None
            }
            Effect::ProjectClosed { title, cancel } => {
                self.say(strings::project_closed(&title, cancel));
                None
            }
            _ => None,
        };
        if let Some(selection) = selection {
            self.set_selection(selection);
        }
    }

    pub(crate) fn accept_snapshot(&mut self, snapshot: Snapshot) {
        self.tag_filter = self
            .tag_filter
            .filter(|&id| view::can_open(&snapshot, Selection::Tag(id)));
        for close in self.kept.iter_mut().chain(&mut self.search_close) {
            close.status = close
                .status
                .filter(|&status| closed_status(&snapshot, close.task) == Some(status));
        }
        self.unavailable_for_edit = None;
        self.organization_needs_refresh = false;
        self.snapshot = Some(snapshot);
    }

    /// Ответ на отметку пришёл: она больше не «в пути».
    fn settle(&mut self, purpose: &Purpose) {
        if let Purpose::Close { task: id, .. }
        | Purpose::CloseInSearch { task: id, .. }
        | Purpose::Reopen(id) = purpose
        {
            self.pending.retain(|t| t != id);
        }
    }

    /// Боковая панель на день `today`.
    pub fn sidebar(&mut self, today: Date) -> Vec<SideRow> {
        self.validate_selection(today);
        match &self.snapshot {
            Some(s) => view::sidebar(s, today, self.selection),
            None => Vec::new(),
        }
    }

    /// Страница выбранного списка на день `today`.
    pub fn page(&mut self, today: Date) -> Page {
        self.validate_selection(today);
        match &self.snapshot {
            Some(s) => {
                let kept: Vec<TaskId> = self
                    .kept
                    .iter()
                    .filter(|c| c.status.is_some())
                    .map(|c| c.task)
                    .collect();
                view::page_filtered(
                    s,
                    today,
                    self.selection,
                    &kept,
                    &|id, open| self.allow(id, open, false),
                    self.tag_filter,
                )
            }
            None => Page {
                title: String::new(),
                progress: None,
                rows: Vec::new(),
            },
        }
    }
}
