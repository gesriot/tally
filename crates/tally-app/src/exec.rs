//! Выполнение команд: снимок, поиск записей, проверки, запись и журнал
//! действий – в одной транзакции записи.

use std::collections::HashSet;

use jiff::civil::Date;
use jiff::{SignedDuration, Timestamp};
use tally_core::lists::Deleted;
use tally_core::{
    Area, AreaId, ChecklistItem, ChecklistItemId, Heading, HeadingId, Project, ProjectId, Rank,
    Start, Status, SyncMeta, Tag, TagId, Task, TaskId,
};
use tally_store::{Hit, RecordId, Snapshot, Store, StoreError};
use tally_when::{When, split_title};
use uuid::Uuid;

use crate::command::{
    Clock, Command, Done, EditChecklistItem, EditTag, EditTask, Effect, MoveTask, NewArea,
    NewHeading, NewProject, NewTask,
};
use crate::error::{Error, Result, reject, rejected};
use crate::history::{self, Record};

mod drop_task;
mod repeat;
mod trash;

use crate::index::{
    Index, area_named, by_name, find_area, find_heading, find_project, find_tag, heading_named,
    heading_named_if, headings_named, project_named,
};
pub(crate) use repeat::has_due_repeats;
pub use trash::trash_preview;

/// Выполнить команду. Снимок, поиск записей, все проверки, сама запись и
/// строка журнала действий – в одной транзакции записи (`BEGIN IMMEDIATE`):
/// другой клиент не вклинится между чтением и записью, а любая ошибка
/// откатывает всё.
pub fn execute(store: &mut Store, command: &Command, clock: Clock) -> Result<Done> {
    match command {
        Command::AddTask(c) => add_task(store, c, clock),
        Command::Close(c) => close(
            store,
            c.tasks.iter().map(|arg| CloseTarget::Query(arg)),
            c.cancel,
            clock,
        ),
        Command::CloseTask(id) => close(store, std::iter::once(CloseTarget::Id(*id)), false, clock),
        Command::EditTask(c) => edit_task(store, c, clock, false, None, None),
        Command::EditTaskRepeat { edit, repeat } => {
            edit_task(store, edit, clock, false, Some(*repeat), None)
        }
        Command::ReopenTask(c) => edit_task(store, c, clock, true, None, None),
        Command::EditTaskReminder {
            edit,
            repeat,
            reminder_at,
            reopen,
        } => edit_task(store, edit, clock, *reopen, *repeat, Some(*reminder_at)),
        Command::SetProjectRepeat {
            project,
            expected_updated_at,
            repeat,
        } => repeat::set_project(store, *project, *expected_updated_at, *repeat, clock),
        Command::MoveTask(c) => move_task(store, c, clock),
        Command::DropTask(c) => drop_task::execute(store, c, clock),
        Command::AddProject(c) => add_project(store, c, clock),
        Command::AddArea(c) => add_area(store, c, clock),
        Command::AddHeading(c) => add_heading(store, c, clock),
        Command::AddProjectInArea { title, area } => change(store, clock, |s, now| {
            let title = name_of(title)?;
            check_project_title(s, &title, None)?;
            let area = live_area(s, *area)?;
            Ok(new_project(s, title, Some(area.id), now))
        }),
        Command::AddProjectSection { title, project } => change(store, clock, |s, now| {
            plan_heading(s, live_project(s, *project)?, name_of(title)?, now)
        }),
        Command::RenameProject {
            project,
            expected_updated_at,
            title,
        } => rename_project(store, *project, *expected_updated_at, title, clock),
        Command::RenameArea {
            area,
            expected_updated_at,
            title,
        } => rename_area(store, *area, *expected_updated_at, title, clock),
        Command::CloseProject {
            project,
            expected_updated_at,
            cancel,
        } => close_project(store, *project, *expected_updated_at, *cancel, clock),
        Command::SetTrash {
            record,
            expected_updated_at,
            trashed,
        } => trash::set(store, *record, *expected_updated_at, *trashed, clock),
        Command::EmptyTrash { preview } => trash::empty(store, preview),
        Command::AdvanceRepeats => repeat::advance_due(store, clock),
        Command::Undo => step(store, clock, true, None),
        Command::Redo => step(store, clock, false, None),
        Command::UndoStep(seq) => step(store, clock, true, Some(*seq)),
    }
}

/// Вся база одним согласованным снимком.
pub fn snapshot(store: &Store) -> Result<Snapshot> {
    Ok(store.read().snapshot()?)
}

/// Снимок для конкретного дня с учётом наступивших календарных повторов.
pub fn snapshot_for_day(store: &mut Store, clock: Clock) -> Result<Snapshot> {
    let s = snapshot(store)?;
    if has_due_repeats(&s, clock.today) {
        Ok(execute(store, &Command::AdvanceRepeats, clock)?.snapshot)
    } else {
        Ok(s)
    }
}

/// Совпадения поиска и снимок для их вывода – из одного состояния базы.
pub fn search(store: &Store, query: &str) -> Result<(Vec<Hit>, Snapshot)> {
    Ok(store
        .read()
        .consistent(|r| Ok((r.search(query)?, r.snapshot()?)))?)
}

/// Что команда хочет сделать: результат, название действия для журнала и
/// будущие состояния записей (новые – с временем создания; у изменяемых
/// время правки ещё прежнее).
struct Changed {
    effect: Effect,
    action: String,
    records: Vec<Record>,
}

/// Команда, меняющая базу. `f` получает снимок и время команды и ничего не
/// пишет – только говорит, какими станут записи. Дальше, в одной транзакции
/// записи и строго по порядку:
///
/// 1. записи, которые совпадают с прежними (кроме времени правки), отпадают;
///    не осталось ни одной – команда без изменений: ничего не пишется;
/// 2. новые и изменённые ссылки проверяются на базе, какой она станет
///    (`Rejected`), – до записи;
/// 3. изменяемым записям ставится время правки ([`edited`], здесь же
///    [`Error::NoLaterTime`] – только для настоящей и допустимой правки);
/// 4. записи пишутся, их состояния до и после уходят в журнал действий.
fn change(
    store: &mut Store,
    clock: Clock,
    f: impl FnOnce(&Snapshot, Timestamp) -> Result<Changed>,
) -> Result<Done> {
    let outcome = store.write_with(|w| {
        let before = w.read().snapshot()?;
        let changed = f(&before, clock.now).map_err(Outcome::Failed)?;
        let mut records = Vec::new();
        for mut record in changed.records {
            let previous = history::find(&before, record.id());
            if let (Record::Task(task), Some(Record::Task(old))) = (&mut record, &previous) {
                repeat::schedule_changed(task, old).map_err(Outcome::Failed)?;
            }
            if previous.is_none_or(|was| !was.same(&record)) {
                records.push(record);
            }
        }
        if records.is_empty() {
            // Ничего не изменилось – журнал не трогать: ветка повтора
            // остаётся.
            return Err(Outcome::Unchanged(Box::new(Done {
                effect: changed.effect,
                snapshot: before,
                seq: None,
            })));
        }
        history::check_planned(&before, &records).map_err(Outcome::Failed)?;
        for r in &mut records {
            if let Some(was) = history::find(&before, r.id()) {
                r.sync_mut().updated_at = edited(clock.now, was.sync()).map_err(Outcome::Failed)?;
            }
        }
        for r in &records {
            let new = history::find(&before, r.id()).is_none();
            history::save(w, r, new).map_err(Outcome::Failed)?;
        }
        let touched: Vec<RecordId> = records.iter().map(Record::id).collect();
        let seq = history::record(w, &before, clock.now, &changed.action, &touched)
            .map_err(Outcome::Failed)?;
        Ok(Done {
            effect: changed.effect,
            snapshot: w.read().snapshot()?,
            seq,
        })
    });
    match outcome {
        Ok(done) => Ok(done),
        Err(Outcome::Unchanged(done)) => Ok(*done),
        Err(Outcome::Failed(e)) => Err(e),
    }
}

/// Итог транзакции [`change`], который её откатывает: ошибка или команда
/// без изменений.
enum Outcome {
    Failed(Error),
    Unchanged(Box<Done>),
}

impl From<StoreError> for Outcome {
    fn from(e: StoreError) -> Self {
        Self::Failed(e.into())
    }
}

/// Undo (`back`) или redo; `expected` – undo только действия с этим
/// номером журнала ([`Command::UndoStep`]).
fn step(store: &mut Store, clock: Clock, back: bool, expected: Option<i64>) -> Result<Done> {
    store.write_with(|w| {
        let (step, seq) = history::step(w, clock, back, expected)?;
        let effect = if back {
            Effect::Undone(step)
        } else {
            Effect::Redone(step)
        };
        Ok(Done {
            effect,
            snapshot: w.read().snapshot()?,
            seq: Some(seq),
        })
    })
}

fn add_task(store: &mut Store, c: &NewTask, clock: Clock) -> Result<Done> {
    filled(c.when.as_deref(), "пустое значение «когда»")?;
    filled(c.deadline.as_deref(), "пустой срок")?;
    filled(c.project.as_deref(), "пустое название проекта")?;
    filled(c.heading.as_deref(), "пустое название раздела")?;
    filled(c.area.as_deref(), "пустое название области")?;
    if c.tags.iter().any(|t| t.trim().is_empty()) {
        reject!("пустое название тега");
    }
    if c.heading.is_some() && c.project.is_none() {
        reject!("раздел указывается вместе с проектом");
    }
    if c.project.is_some() && c.area.is_some() {
        reject!("задача не может быть сразу в проекте и в области");
    }
    let today = clock.today;
    let (title, when) = match &c.when {
        Some(phrase) => (c.text.trim().to_owned(), Some(when(phrase, today)?)),
        None => {
            let entry = split_title(&c.text, today);
            (entry.title.to_owned(), entry.when)
        }
    };
    if title.is_empty() {
        reject!("пустое название задачи");
    }
    let deadline = c
        .deadline
        .as_deref()
        .map(|phrase| deadline(phrase, today))
        .transpose()?;
    change(store, clock, |s, now| {
        let mut task = Task::new(
            TaskId::from_uuid(Uuid::new_v4()),
            title,
            after(s.tasks.iter().map(|t| &t.index)),
            now,
        );
        task.today_index = after(s.tasks.iter().map(|t| &t.today_index));
        if let Some(project) = &c.project {
            let p = find_project(s, project)?;
            task.project = Some(p.id);
            if let Some(heading) = &c.heading {
                task.heading = Some(find_heading(s, p, heading)?.id);
            }
            task.start = Start::Anytime;
        }
        if let Some(area) = &c.area {
            task.area = Some(find_area(s, area)?.id);
            task.start = Start::Anytime;
        }
        if let Some(when) = when {
            when.apply_to_task(&mut task, now);
        }
        task.deadline = deadline;
        if let Some(notes) = &c.notes {
            task.notes = notes.clone();
        }
        let mut new_tags: Vec<Tag> = Vec::new();
        for name in &c.tags {
            let id = resolve_tag(s, &mut new_tags, name, now)?;
            if !task.tags.contains(&id) {
                task.tags.push(id);
            }
        }
        // Порядок записи – порядок журнала: теги раньше задачи, которая на
        // них ссылается.
        let action = format!("добавление задачи «{}»", task.title);
        let effect = Effect::Added(task.id);
        let mut records: Vec<Record> = new_tags.into_iter().map(Record::Tag).collect();
        records.push(Record::Task(task));
        Ok(Changed {
            effect,
            action,
            records,
        })
    })
}

fn resolve_tag(s: &Snapshot, created: &mut Vec<Tag>, name: &str, now: Timestamp) -> Result<TagId> {
    let name = name.trim();
    if name.is_empty() {
        reject!("пустое название тега");
    }
    let fresh = by_name(created.iter(), |tag| &tag.title, name);
    if let Some(tag) = find_tag(s, name)?.or(fresh.first().copied()) {
        return Ok(tag.id);
    }
    let index = after(s.tags.iter().chain(created.iter()).map(|tag| &tag.index));
    let id = TagId::from_uuid(Uuid::new_v4());
    created.push(Tag::new(id, name, index, now));
    Ok(id)
}

fn edit_task(
    store: &mut Store,
    edit: &EditTask,
    clock: Clock,
    reopen: bool,
    repeat: Option<Option<crate::Repeat>>,
    reminder_at: Option<Option<Timestamp>>,
) -> Result<Done> {
    change(store, clock, |s, now| {
        let index = Index::new(s, clock.today);
        let current = s
            .tasks
            .iter()
            .find(|task| task.id == edit.task && !index.trashed(task))
            .ok_or(Error::TaskUnavailable)?;
        if current.sync.updated_at != edit.expected_updated_at {
            return Err(Error::Conflict(
                "задача изменилась. Закройте карточку и откройте её снова".into(),
            ));
        }

        let mut task = current.clone();
        let (effect, action) = if reopen {
            if !index.can_reopen(current) {
                return Err(Error::Conflict("задачу нельзя открыть заново".into()));
            }
            task.status = Status::Open;
            (
                Effect::Reopened(edit.task),
                format!("возврат задачи «{}» в работу", current.title),
            )
        } else {
            (
                Effect::Edited(edit.task),
                format!("редактирование задачи «{}»", current.title),
            )
        };
        if edit.title != task.title {
            let title = edit.title.trim();
            if title.is_empty() {
                reject!("пустое название задачи");
            }
            task.title = title.to_owned();
        }
        task.notes.clone_from(&edit.notes);
        if let Some(when) = edit.when {
            when.apply_to_task(&mut task, current.sync.updated_at);
        }
        if let Some(repeat) = repeat {
            repeat::set_task(&mut task, repeat, clock.today)?;
        }
        task.deadline = edit.deadline;
        if let Some(reminder_at) = reminder_at {
            task.reminder_at = reminder_at;
        }
        edit_checklist(&mut task, &edit.checklist, now)?;

        let mut new_tags = Vec::new();
        task.tags.clear();
        for tag in &edit.tags {
            let id = match tag {
                EditTag::Existing(id) => {
                    if !current.tags.contains(id)
                        && !s.tags.iter().any(|t| t.id == *id && !t.sync.is_deleted())
                    {
                        reject!("тег больше недоступен");
                    }
                    *id
                }
                EditTag::Named(name) => resolve_tag(s, &mut new_tags, name, now)?,
            };
            if !task.tags.contains(&id) {
                task.tags.push(id);
            }
        }
        let mut records: Vec<Record> = new_tags.into_iter().map(Record::Tag).collect();
        records.push(Record::Task(task));
        Ok(Changed {
            effect,
            action,
            records,
        })
    })
}

fn edit_checklist(task: &mut Task, entries: &[EditChecklistItem], now: Timestamp) -> Result<()> {
    let mut retained = HashSet::with_capacity(entries.len());
    for entry in entries {
        let title = entry.title.trim();
        if let Some(id) = entry.id {
            if !retained.insert(id) {
                reject!("пункт чек-листа указан дважды");
            }
            let item = task
                .checklist
                .iter_mut()
                .find(|item| item.id == id && !item.sync.is_deleted())
                .ok_or_else(|| rejected!("пункт чек-листа больше недоступен"))?;
            let title = if entry.title == item.title {
                item.title.as_str()
            } else {
                title
            };
            if title != item.title && title.is_empty() {
                reject!("пустой пункт чек-листа");
            }
            if title != item.title || item.done != entry.done {
                item.sync.touch(edited(now, &item.sync)?);
                item.title = title.to_owned();
                item.done = entry.done;
            }
        } else {
            if title.is_empty() {
                reject!("пустой пункт чек-листа");
            }
            let id = ChecklistItemId::from_uuid(Uuid::new_v4());
            let mut item = ChecklistItem::new(
                id,
                title,
                after(task.checklist.iter().map(|i| &i.index)),
                now,
            );
            item.done = entry.done;
            retained.insert(id);
            task.checklist.push(item);
        }
    }
    for item in &mut task.checklist {
        if !item.sync.is_deleted() && !retained.contains(&item.id) {
            item.sync.delete(edited(now, &item.sync)?);
        }
    }
    Ok(())
}

enum CloseTarget<'a> {
    Query(&'a str),
    Id(TaskId),
}

fn close<'a>(
    store: &mut Store,
    targets: impl Iterator<Item = CloseTarget<'a>>,
    cancel: bool,
    clock: Clock,
) -> Result<Done> {
    let mut targets = targets.peekable();
    if targets.peek().is_none() {
        reject!("не указаны задачи");
    }
    let today = clock.today;
    change(store, clock, |s, now| {
        let v = Index::new(s, today);
        let mut changed: Vec<Task> = Vec::new();
        for target in targets {
            let task = match target {
                CloseTarget::Query(arg) => v.resolve(arg, true)?,
                CloseTarget::Id(id) => s
                    .tasks
                    .iter()
                    .find(|t| t.id == id && !v.trashed(t))
                    .ok_or_else(|| rejected!("задача «{id}» не найдена"))?,
            };
            if !task.status.is_open() {
                reject!("задача {} уже закрыта", v.label(task));
            }
            if changed.iter().any(|t| t.id == task.id) {
                continue;
            }
            let mut task = task.clone();
            task.status = if cancel {
                Status::Canceled { at: now }
            } else {
                Status::Completed { at: now }
            };
            changed.push(task);
        }
        let verb = if cancel {
            "отмена"
        } else {
            "выполнение"
        };
        let titles: Vec<String> = changed.iter().map(|t| format!("«{}»", t.title)).collect();
        let noun = if changed.len() == 1 {
            "задачи"
        } else {
            "задач"
        };
        let tasks = changed.iter().map(|t| t.id).collect();
        let mut records = Vec::new();
        for mut task in changed {
            if (!cancel || task.repeat_from == tally_core::RepeatFrom::Date)
                && let Some(next) = repeat::advance_task(&mut task, s, clock)
            {
                records.push(Record::Task(next));
            }
            records.push(Record::Task(task));
        }
        Ok(Changed {
            effect: Effect::Closed { tasks, cancel },
            action: format!("{verb} {noun} {}", titles.join(", ")),
            records,
        })
    })
}

fn move_task(store: &mut Store, c: &MoveTask, clock: Clock) -> Result<Done> {
    let nothing = c.when.is_none()
        && !c.inbox
        && c.project.is_none()
        && c.heading.is_none()
        && c.area.is_none()
        && !c.no_project
        && c.deadline.is_none()
        && !c.no_deadline;
    if nothing {
        reject!("не указано, куда перенести задачу");
    }
    filled(c.when.as_deref(), "пустое значение «когда»")?;
    filled(c.deadline.as_deref(), "пустой срок")?;
    filled(c.project.as_deref(), "пустое название проекта")?;
    filled(c.heading.as_deref(), "пустое название раздела")?;
    filled(c.area.as_deref(), "пустое название области")?;
    // Те же сочетания отсекает и командная строка (код 2).
    let parts = [
        ("во «Входящие»", c.inbox),
        ("дата", c.when.is_some()),
        ("проект", c.project.is_some()),
        ("раздел", c.heading.is_some()),
        ("область", c.area.is_some()),
        ("без проекта", c.no_project),
        ("срок", c.deadline.is_some()),
        ("без срока", c.no_deadline),
    ];
    const CLASHES: [(usize, usize); 10] = [
        (0, 1),
        (0, 2),
        (0, 3),
        (0, 4),
        (0, 5),
        (2, 4),
        (2, 5),
        (3, 4),
        (3, 5),
        (4, 5),
    ];
    for (a, b) in CLASHES {
        if parts[a].1 && parts[b].1 {
            reject!("несовместимые изменения: {} и {}", parts[a].0, parts[b].0);
        }
    }
    if c.deadline.is_some() && c.no_deadline {
        reject!("несовместимые изменения: срок и без срока");
    }
    let today = clock.today;
    let when = c.when.as_deref().map(|p| when(p, today)).transpose()?;
    let deadline = c
        .deadline
        .as_deref()
        .map(|p| deadline(p, today))
        .transpose()?;
    change(store, clock, |s, _now| {
        let v = Index::new(s, today);
        let mut task = v.resolve(&c.task, true)?.clone();
        if !task.status.is_open() {
            reject!("задача {} уже закрыта", v.label(&task));
        }
        // Метку правки ставит `change` – после того, как ясно, что задача
        // меняется и новые ссылки допустимы.
        let kept = task.sync.updated_at;
        let current = task.clone();
        if c.inbox {
            task.start = Start::Inbox;
            task.date = None;
            task.evening = false;
            task.project = None;
            task.heading = None;
            task.area = None;
        }
        if let Some(when) = when {
            when.apply_to_task(&mut task, kept);
        }
        if c.no_project {
            task.project = None;
            task.heading = None;
            task.area = None;
        }
        if let Some(name) = &c.project {
            let p = match project_named(s, name)? {
                Some(p) => p,
                // Прежний проект по названию – даже закрытый: ссылка не
                // меняется.
                None => current_project(s, &current, name)
                    .ok_or_else(|| rejected!("проекта «{name}» нет"))?,
            };
            if task.project != Some(p.id) {
                task.heading = None;
            }
            task.project = Some(p.id);
            task.area = None;
        }
        if let Some(name) = &c.heading {
            let project = task
                .project
                .and_then(|id| s.projects.iter().find(|p| p.id == id))
                .ok_or_else(|| rejected!("задача не в проекте – укажите проект: --project"))?;
            // Сначала отбираются живые одноимённые разделы, в которые можно
            // перенести задачу (вся цепочка доступна), и только среди них
            // ищется неоднозначность: несколько доступных – отказ со списком,
            // один – он. Доступных нет – прежний раздел задачи с этим
            // названием (даже архивный): ссылка не меняется. Нет и его –
            // любой живой одноимённый раздел, и проверка ссылок назовёт
            // причину отказа.
            let available = |h: &Heading| history::heading_available(s, h);
            let h = match heading_named_if(s, project, name, available)? {
                Some(h) => h,
                None => match current_heading(s, &current, project, name) {
                    Some(mine) => mine,
                    None => *headings_named(s, project, name).first().ok_or_else(|| {
                        rejected!("раздела «{name}» в проекте «{}» нет", project.title)
                    })?,
                },
            };
            task.heading = Some(h.id);
        }
        if let Some(name) = &c.area {
            task.area = Some(find_area(s, name)?.id);
            task.project = None;
            task.heading = None;
        }
        if task.start == Start::Inbox && (task.project.is_some() || task.area.is_some()) {
            task.start = Start::Anytime;
        }
        if deadline.is_some() {
            task.deadline = deadline;
        }
        if c.no_deadline {
            task.deadline = None;
        }
        Ok(Changed {
            effect: Effect::Moved(task.id),
            action: format!("перенос задачи «{}»", task.title),
            records: vec![Record::Task(task)],
        })
    })
}

/// Проект задачи `task`, если он называется `name`.
fn current_project<'a>(s: &'a Snapshot, task: &Task, name: &str) -> Option<&'a Project> {
    let p = s.projects.iter().find(|p| Some(p.id) == task.project)?;
    (!by_name([p].into_iter(), |p: &Project| &p.title, name).is_empty()).then_some(p)
}

/// Раздел задачи `task` в проекте `project`, если он называется `name`.
fn current_heading<'a>(
    s: &'a Snapshot,
    task: &Task,
    project: &Project,
    name: &str,
) -> Option<&'a Heading> {
    let h = s
        .headings
        .iter()
        .find(|h| Some(h.id) == task.heading && h.project == project.id)?;
    (!by_name([h].into_iter(), |h: &Heading| &h.title, name).is_empty()).then_some(h)
}

fn add_project(store: &mut Store, c: &NewProject, clock: Clock) -> Result<Done> {
    let title = name_of(&c.title)?;
    filled(c.area.as_deref(), "пустое название области")?;
    change(store, clock, |s, now| {
        check_project_title(s, &title, None)?;
        let area = c.area.as_ref().map(|name| find_area(s, name)).transpose()?;
        Ok(new_project(s, title, area.map(|a| a.id), now))
    })
}

fn check_project_title(s: &Snapshot, title: &str, except: Option<ProjectId>) -> Result<()> {
    if project_named(s, title)?.is_some_and(|p| Some(p.id) != except) {
        reject!("проект «{title}» уже есть");
    }
    Ok(())
}

fn new_project(s: &Snapshot, title: String, area: Option<AreaId>, now: Timestamp) -> Changed {
    let mut p = Project::new(
        ProjectId::from_uuid(Uuid::new_v4()),
        title.clone(),
        after(s.projects.iter().map(|p| &p.index)),
        now,
    );
    p.today_index = after(s.projects.iter().map(|p| &p.today_index));
    p.area = area;
    Changed {
        action: format!("создание проекта «{title}»"),
        effect: Effect::ProjectAdded { title },
        records: vec![Record::Project(p)],
    }
}

fn add_area(store: &mut Store, c: &NewArea, clock: Clock) -> Result<Done> {
    let title = name_of(&c.title)?;
    change(store, clock, |s, now| {
        if area_named(s, &title)?.is_some() {
            reject!("область «{title}» уже есть");
        }
        let a = Area::new(
            AreaId::from_uuid(Uuid::new_v4()),
            title.clone(),
            after(s.areas.iter().map(|a| &a.index)),
            now,
        );
        Ok(Changed {
            action: format!("создание области «{title}»"),
            effect: Effect::AreaAdded { title },
            records: vec![Record::Area(a)],
        })
    })
}

fn add_heading(store: &mut Store, c: &NewHeading, clock: Clock) -> Result<Done> {
    let title = name_of(&c.title)?;
    filled(Some(&c.project), "пустое название проекта")?;
    change(store, clock, |s, now| {
        let p = find_project(s, &c.project)?;
        plan_heading(s, p, title, now)
    })
}

fn plan_heading(s: &Snapshot, p: &Project, title: String, now: Timestamp) -> Result<Changed> {
    if heading_named(s, p, &title)?.is_some() {
        reject!("раздел «{title}» в проекте «{}» уже есть", p.title);
    }
    let h = Heading::new(
        HeadingId::from_uuid(Uuid::new_v4()),
        p.id,
        title.clone(),
        after(
            s.headings
                .iter()
                .filter(|h| h.project == p.id)
                .map(|h| &h.index),
        ),
        now,
    );
    Ok(Changed {
        action: format!("создание раздела «{title}» в проекте «{}»", p.title),
        effect: Effect::HeadingAdded {
            title,
            project: p.title.clone(),
        },
        records: vec![Record::Heading(h)],
    })
}

fn live_area(s: &Snapshot, id: AreaId) -> Result<&Area> {
    s.areas
        .iter()
        .find(|a| a.id == id && !a.sync.is_deleted())
        .ok_or_else(|| Error::Conflict("область больше недоступна".into()))
}

fn live_project(s: &Snapshot, id: ProjectId) -> Result<&Project> {
    s.projects
        .iter()
        .find(|p| p.id == id)
        .filter(|p| {
            let area = p.area.and_then(|id| s.areas.iter().find(|a| a.id == id));
            p.status.is_open() && !Deleted::of_project(p, area).is_trashed()
        })
        .ok_or_else(|| Error::Conflict("проект больше недоступен".into()))
}

fn rename_project(
    store: &mut Store,
    id: ProjectId,
    expected: Timestamp,
    title: &str,
    clock: Clock,
) -> Result<Done> {
    change(store, clock, |s, _| {
        let mut p = live_project(s, id)?.clone();
        if p.sync.updated_at != expected {
            return Err(Error::Conflict(
                "проект изменился. Закройте форму и откройте её снова".into(),
            ));
        }
        let title = if title == p.title {
            p.title.clone()
        } else {
            name_of(title)?
        };
        if title != p.title {
            check_project_title(s, &title, Some(id))?;
        }
        let action = format!("переименование проекта «{}»", p.title);
        p.title.clone_from(&title);
        Ok(Changed {
            action,
            effect: Effect::ProjectRenamed { title },
            records: vec![Record::Project(p)],
        })
    })
}

fn rename_area(
    store: &mut Store,
    id: AreaId,
    expected: Timestamp,
    title: &str,
    clock: Clock,
) -> Result<Done> {
    change(store, clock, |s, _| {
        let mut a = live_area(s, id)?.clone();
        if a.sync.updated_at != expected {
            return Err(Error::Conflict(
                "область изменилась. Закройте форму и откройте её снова".into(),
            ));
        }
        let title = if title == a.title {
            a.title.clone()
        } else {
            name_of(title)?
        };
        if title != a.title && area_named(s, &title)?.is_some_and(|other| other.id != id) {
            reject!("область «{title}» уже есть");
        }
        let action = format!("переименование области «{}»", a.title);
        a.title.clone_from(&title);
        Ok(Changed {
            action,
            effect: Effect::AreaRenamed { title },
            records: vec![Record::Area(a)],
        })
    })
}

fn close_project(
    store: &mut Store,
    id: ProjectId,
    expected: Timestamp,
    cancel: bool,
    clock: Clock,
) -> Result<Done> {
    change(store, clock, |s, now| {
        let mut project = live_project(s, id)?.clone();
        if project.sync.updated_at != expected {
            return Err(Error::Conflict(
                "проект изменился. Подтвердите закрытие ещё раз".into(),
            ));
        }
        project.status = if cancel {
            Status::Canceled { at: now }
        } else {
            Status::Completed { at: now }
        };
        let verb = if cancel {
            "отмена"
        } else {
            "завершение"
        };
        let records = if !cancel || project.repeat_from == tally_core::RepeatFrom::Date {
            repeat::advance_project(&mut project, s, clock)
        } else {
            Vec::new()
        };
        Ok(Changed {
            action: format!("{verb} проекта «{}»", project.title),
            effect: Effect::ProjectClosed {
                title: project.title.clone(),
                cancel,
            },
            records: std::iter::once(Record::Project(project))
                .chain(records)
                .collect(),
        })
    })
}

/// Время правки записи `meta`: время команды, но не раньше прежних меток
/// самой записи. Если её уже правили «из будущего» (часы другого клиента
/// ушли вперёд), новая правка – на наносекунду позже: время правки записи
/// не идёт назад. Чужие записи на это не влияют.
pub(crate) fn edited(now: Timestamp, meta: &SyncMeta) -> Result<Timestamp> {
    later(now, meta.updated_at.max(meta.created_at))
}

/// `now`, если оно позже `latest`, иначе `latest` + 1 нс. Метка строго
/// растёт: на `Timestamp::MAX` расти некуда – [`Error::NoLaterTime`].
pub(crate) fn later(now: Timestamp, latest: Timestamp) -> Result<Timestamp> {
    if now > latest {
        Ok(now)
    } else {
        latest
            .checked_add(SignedDuration::from_nanos(1))
            .map_err(|_| Error::NoLaterTime(latest))
    }
}

/// Ключ порядка после всех `ranks` – новая запись встаёт в конец.
fn after<'a>(ranks: impl Iterator<Item = &'a Rank>) -> Rank {
    ranks.max().map_or_else(Rank::initial, Rank::after)
}

/// Необязательное значение задано, но пустое (или из одних пробелов) –
/// ошибка `message`.
fn filled(value: Option<&str>, message: &str) -> Result<()> {
    match value {
        Some(v) if v.trim().is_empty() => reject!("{message}"),
        _ => Ok(()),
    }
}

fn name_of(title: &str) -> Result<String> {
    let title = title.trim().to_owned();
    if title.is_empty() {
        reject!("пустое название");
    }
    Ok(title)
}

/// «Когда» из выражения `tally-when`.
fn when(phrase: &str, today: Date) -> Result<When> {
    tally_when::parse(phrase, today).map_err(|e| rejected!("когда: {e}"))
}

/// Срок – день, без «вечера».
fn deadline(phrase: &str, today: Date) -> Result<Date> {
    match tally_when::parse(phrase, today).map_err(|e| rejected!("срок: {e}"))? {
        When::On {
            date,
            evening: false,
        } => Ok(date),
        When::On { evening: true, .. } => reject!("срок: «вечером» у срока не бывает"),
        When::Anytime | When::Someday => reject!("срок – это день: «{phrase}»"),
    }
}
