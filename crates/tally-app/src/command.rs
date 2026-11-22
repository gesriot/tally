//! Команды и их результаты.

use jiff::Timestamp;
use jiff::civil::Date;
use tally_core::{
    AreaId, ChecklistItemId, HeadingId, ProjectId, RepeatFrom, RepeatRule, TagId, TaskId,
};
use tally_store::{RecordId, Snapshot};
use tally_when::When;

/// Изменение базы. Значения – как их ввёл пользователь: названия, начала
/// номеров задач и выражения дат («завтра вечером», «15 дек») разбирает
/// `tally-app`. Правки записей сохраняются в журнале действий. Окончательная
/// очистка корзины удаляет данные и затронутые шаги истории.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    AddTask(NewTask),
    Close(Close),
    /// Выполнить выбранную задачу по её точному идентификатору.
    CloseTask(TaskId),
    EditTask(EditTask),
    EditTaskRepeat {
        edit: EditTask,
        repeat: Option<Repeat>,
    },
    /// Сохранить карточку и новое время напоминания одним действием.
    EditTaskReminder {
        edit: EditTask,
        repeat: Option<Option<Repeat>>,
        reminder_at: Option<Timestamp>,
        reopen: bool,
    },
    SetProjectRepeat {
        project: ProjectId,
        expected_updated_at: Timestamp,
        repeat: Option<Repeat>,
    },
    /// Сохранить карточку и вернуть закрытую задачу в работу.
    ReopenTask(EditTask),
    MoveTask(MoveTask),
    DropTask(DropTask),
    AddProject(NewProject),
    AddArea(NewArea),
    AddHeading(NewHeading),
    AddProjectInArea {
        title: String,
        area: AreaId,
    },
    AddProjectSection {
        title: String,
        project: ProjectId,
    },
    RenameProject {
        project: ProjectId,
        expected_updated_at: Timestamp,
        title: String,
    },
    RenameArea {
        area: AreaId,
        expected_updated_at: Timestamp,
        title: String,
    },
    /// Закрыть проект той версии, которую пользователь подтвердил.
    CloseProject {
        project: ProjectId,
        expected_updated_at: Timestamp,
        cancel: bool,
    },
    SetTrash {
        record: RecordId,
        expected_updated_at: Timestamp,
        trashed: bool,
    },
    /// Очистить корзину, если набор и версии удаляемых записей не изменились.
    EmptyTrash {
        preview: TrashPreview,
    },
    AdvanceRepeats,
    /// Отменить последнее действие.
    Undo,
    /// Повторить последнее отменённое действие.
    Redo,
    /// Отменить действие с номером журнала `seq` ([`Done::seq`]) – только
    /// если оно и есть последнее неотменённое. Сверка и отмена – в одной
    /// транзакции; после него было другое действие (или его уже отменили) –
    /// [`crate::Error::NotLast`], база и журнал не меняются.
    UndoStep(i64),
}

/// Записи, которые физически исчезнут при очистке, включая содержимое контейнеров.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrashPreview {
    pub records: Vec<(RecordId, Timestamp)>,
    pub checklist_items: Vec<(ChecklistItemId, Timestamp)>,
}

impl TrashPreview {
    pub fn is_empty(&self) -> bool {
        self.records.is_empty() && self.checklist_items.is_empty()
    }
}

/// Новая задача.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewTask {
    /// Название; если `when` нет, дата в конце разбирается, как в быстром вводе.
    pub text: String,
    /// Когда начать – текст тогда не разбирается.
    pub when: Option<String>,
    pub deadline: Option<String>,
    pub project: Option<String>,
    /// Раздел проекта `project`.
    pub heading: Option<String>,
    pub area: Option<String>,
    pub notes: Option<String>,
    /// Теги по названию; нового тега ещё нет – он создаётся.
    pub tags: Vec<String>,
}

/// Выполнить или отменить задачи.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Close {
    /// Начала номеров или части названий.
    pub tasks: Vec<String>,
    /// Отменить, а не выполнить.
    pub cancel: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Repeat {
    pub rule: RepeatRule,
    pub from: RepeatFrom,
}

/// Сохранить карточку одним действием. Проверка версии и запись выполняются
/// под одной блокировкой: другой клиент может изменить задачу во время ввода.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditTask {
    pub task: TaskId,
    pub expected_updated_at: Timestamp,
    pub title: String,
    pub notes: String,
    /// None оставляет «Когда» как есть, в том числе во «Входящих».
    pub when: Option<When>,
    pub deadline: Option<Date>,
    pub checklist: Vec<EditChecklistItem>,
    pub tags: Vec<EditTag>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditChecklistItem {
    /// None добавляет новый пункт в конец.
    pub id: Option<ChecklistItemId>,
    pub title: String,
    pub done: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditTag {
    Existing(TagId),
    Named(String),
}

/// Перенести задачу.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MoveTask {
    pub task: String,
    pub when: Option<String>,
    /// Во «Входящие»: без даты, проекта, раздела и области.
    pub inbox: bool,
    pub project: Option<String>,
    /// Раздел указанного или текущего проекта.
    pub heading: Option<String>,
    pub area: Option<String>,
    /// Убрать из проекта и области.
    pub no_project: bool,
    pub deadline: Option<String>,
    pub no_deadline: bool,
}

/// Перенос и позиция относительно живых соседей в одной транзакции.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DropTask {
    pub task: TaskId,
    pub expected_updated_at: Timestamp,
    pub source: Option<TaskLane>,
    pub destination: TaskLane,
    pub position: DropPosition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskLane {
    Inbox,
    Today { evening: bool },
    Anytime,
    Someday,
    Project(ProjectId, Option<HeadingId>),
    Area(AreaId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropPosition {
    Before(DropAnchor),
    After(DropAnchor),
    End,
    /// Назначение через боковую панель с вставкой в конец.
    Sidebar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropAnchor {
    Task(TaskId),
    Project(ProjectId),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewProject {
    pub title: String,
    pub area: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewArea {
    pub title: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewHeading {
    pub title: String,
    pub project: String,
}

/// Сегодняшний день (для дат «завтра», «пт» и списков) и момент команды
/// (время создания, закрытия и правки записей). В ядре `now()` нет – время
/// всегда приходит снаружи.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clock {
    pub today: Date,
    pub now: Timestamp,
}

/// Что сделала команда, и база сразу после неё (прочитана в той же
/// транзакции).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Done {
    pub effect: Effect,
    pub snapshot: Snapshot,
    /// Номер строки журнала действий, которую записала команда (или которую
    /// отменили и повторили undo и redo). `None` – команда ничего не
    /// изменила, журнал не тронут.
    pub seq: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Added(TaskId),
    /// Задачи в порядке аргументов, без повторов.
    Closed {
        tasks: Vec<TaskId>,
        cancel: bool,
    },
    Moved(TaskId),
    Edited(TaskId),
    Reopened(TaskId),
    ProjectAdded {
        title: String,
    },
    AreaAdded {
        title: String,
    },
    HeadingAdded {
        title: String,
        project: String,
    },
    ProjectRenamed {
        title: String,
    },
    AreaRenamed {
        title: String,
    },
    ProjectClosed {
        title: String,
        cancel: bool,
    },
    TrashChanged {
        title: String,
        trashed: bool,
    },
    TrashEmptied,
    ProjectRepeatSet,
    ScheduledRepeats,
    Undone(Step),
    Redone(Step),
}

/// Отменённое или повторённое действие.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// Название действия: «перенос задачи «Отчёт»».
    pub action: String,
    /// Задачи, которые оно затронуло (удалённых отменой уже нет в базе).
    pub tasks: Vec<TaskId>,
}
