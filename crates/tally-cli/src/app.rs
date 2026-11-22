//! Команды консоли: разбор аргументов – в [`crate::cli`], выполнение –
//! `tally-app` (транзакции, журнал действий, отмена и повтор), здесь –
//! перевод аргументов в команды `tally-app` и вывод результата.

use std::io::Write;
use std::ops::Deref;

use anyhow::Result;
use jiff::Timestamp;
use jiff::civil::Date;
use tally_app::{Clock, Effect, Index, Step};
use tally_core::lists::{Deleted, Group, Item, project_progress};
use tally_core::{Area, AreaId, Project, Status, Task};
use tally_store::{Found, Snapshot, Store};

use crate::cli::{
    AddArgs, AreaArgs, Command, DoneArgs, HeadingArgs, ListArgs, ListName, MoveArgs, ProjectArgs,
    SearchArgs,
};
use tally_client::{desktop, env, show};

/// Открытая база, сегодняшний день, момент операции и куда писать вывод.
pub struct App<'o> {
    pub store: Store,
    pub today: Date,
    pub now: Timestamp,
    pub out: &'o mut dyn Write,
}

impl App<'_> {
    pub fn run(&mut self, command: Command) -> Result<()> {
        let command = match command {
            Command::QuickAdd => {
                let path = env::db_path(None, env::var(env::DB_VAR).as_deref())?;
                return desktop::open_quick_add(&path);
            }
            Command::List(args) => return self.list(&args),
            Command::Search(args) => return self.search(&args),
            Command::Add(args) => add(args),
            Command::Done(args) => done(args),
            Command::Move(args) => move_task(args),
            Command::Project(args) => project(args),
            Command::Area(args) => area(args),
            Command::Heading(args) => heading(args),
            Command::Undo => tally_app::Command::Undo,
            Command::Redo => tally_app::Command::Redo,
        };
        self.change(&command)
    }

    #[cfg(test)]
    fn snapshot(&self) -> Result<Snapshot> {
        Ok(tally_app::snapshot(&self.store)?)
    }

    /// Команда, меняющая базу: выполняет `tally-app` в одной транзакции
    /// вместе с журналом действий; вывод печатается только после фиксации.
    fn change(&mut self, command: &tally_app::Command) -> Result<()> {
        let clock = Clock {
            today: self.today,
            now: self.now,
        };
        if matches!(command, tally_app::Command::Close(_)) {
            tally_app::snapshot_for_day(&mut self.store, clock)?;
        }
        let done = tally_app::execute(&mut self.store, command, clock)?;
        let v = View::new(&done.snapshot, self.today);
        let task = |id| done.snapshot.tasks.iter().find(|t| t.id == id);
        let text = match &done.effect {
            Effect::Added(id) => format!("Добавлено: {}\n", line(&v, task(*id))),
            Effect::Moved(id) => format!("Перенесено: {}\n", line(&v, task(*id))),
            Effect::Edited(id) => format!("Изменено: {}\n", line(&v, task(*id))),
            Effect::Reopened(id) => format!("Открыто заново: {}\n", line(&v, task(*id))),
            Effect::Closed { tasks, cancel } => {
                let verb = if *cancel {
                    "Отменено"
                } else {
                    "Выполнено"
                };
                tasks
                    .iter()
                    .filter_map(|id| task(*id))
                    .map(|t| format!("{verb}: {}\n", v.label(t)))
                    .collect()
            }
            Effect::ProjectAdded { title } => format!("Создан проект «{title}»\n"),
            Effect::AreaAdded { title } => format!("Создана область «{title}»\n"),
            Effect::HeadingAdded { title, project } => {
                format!("Создан раздел «{title}» в проекте «{project}»\n")
            }
            Effect::ProjectRenamed { title } => format!("Переименован проект «{title}»\n"),
            Effect::AreaRenamed { title } => format!("Переименована область «{title}»\n"),
            Effect::ProjectClosed { title, cancel } => {
                let action = if *cancel {
                    "Отменён"
                } else {
                    "Завершён"
                };
                format!("{action} проект «{title}»\n")
            }
            Effect::TrashChanged { title, trashed } => {
                let verb = if *trashed {
                    "В корзину"
                } else {
                    "Восстановлено"
                };
                format!("{verb}: «{title}»\n")
            }
            Effect::ScheduledRepeats => String::new(),
            Effect::ProjectRepeatSet => "Повтор проекта настроен\n".into(),
            Effect::TrashEmptied => "Корзина очищена\n".into(),
            Effect::Undone(step) => stepped(&v, "Отменено действие", step),
            Effect::Redone(step) => stepped(&v, "Повторено действие", step),
        };
        write!(self.out, "{text}")?;
        Ok(())
    }

    fn list(&mut self, args: &ListArgs) -> Result<()> {
        let s = tally_app::snapshot_for_day(
            &mut self.store,
            Clock {
                today: self.today,
                now: self.now,
            },
        )?;
        let v = View::new(&s, self.today);
        let lists = s.lists(self.today);
        let today = self.today;
        let mut text = String::new();
        // `blank` – что показать вместо пустого раздела: «пусто» или ничего.
        let mut section = |title: String, rows: Vec<String>, blank: bool| {
            text.push_str(&title);
            text.push('\n');
            if rows.is_empty() && blank {
                text.push_str("  пусто\n");
            }
            for row in rows {
                text.push_str("  ");
                text.push_str(&row);
                text.push('\n');
            }
        };
        match args.list {
            ListName::Today => {
                let rows = |items: &[Item<'_>]| items.iter().map(|i| v.item_line(*i)).collect();
                section(
                    format!("Сегодня – {}", show::date(today, today)),
                    rows(&lists.today.day),
                    true,
                );
                if !lists.today.evening.is_empty() {
                    section("Вечер".to_owned(), rows(&lists.today.evening), true);
                }
            }
            ListName::Inbox => {
                let rows = lists
                    .inbox
                    .iter()
                    .map(|t| v.item_line(Item::Task(t)))
                    .collect();
                section("Входящие".to_owned(), rows, true);
            }
            ListName::Upcoming => {
                // Семь дней с завтрашнего – всегда, даже пустые: тогда только
                // заголовок дня. Дальше – только группы с записями.
                for g in &lists.upcoming {
                    let rows: Vec<String> = g
                        .entries
                        .iter()
                        .map(|e| {
                            // Вечер – только в строке начала, не в строке срока.
                            let evening = e.reason.is_scheduled() && e.item.evening();
                            // В группе месяца или года – и сам день.
                            let day = match (g.group, evening) {
                                (Group::Day(_), false) => None,
                                (Group::Day(_), true) => Some("вечер".to_owned()),
                                (_, false) => Some(show::date(e.date, today)),
                                (_, true) => Some(format!("{}, вечер", show::date(e.date, today))),
                            };
                            let mut line = v.item_line_with(e.item, !e.reason.has_deadline(), day);
                            if e.reason.has_deadline() {
                                line.push_str(" ⚑ срок");
                            }
                            line
                        })
                        .collect();
                    if !rows.is_empty() || matches!(g.group, Group::Day(_)) {
                        section(show::group(g.group, today), rows, false);
                    }
                }
            }
            ListName::Anytime => {
                let rows = lists
                    .anytime
                    .iter()
                    .map(|e| {
                        let line = v.item_line(Item::Task(e.task));
                        if e.in_today {
                            format!("{line} ★")
                        } else {
                            line
                        }
                    })
                    .collect();
                section("В любое время".to_owned(), rows, true);
            }
            ListName::Someday => {
                let rows = lists
                    .someday
                    .tasks
                    .iter()
                    .map(|t| v.item_line(Item::Task(t)))
                    .chain(
                        lists
                            .someday
                            .projects
                            .iter()
                            .map(|p| v.item_line(Item::Project(p))),
                    )
                    .collect();
                section("Когда-нибудь".to_owned(), rows, true);
            }
            ListName::Logbook => {
                let rows = lists.logbook.iter().map(|i| v.item_line(*i)).collect();
                section("Журнал".to_owned(), rows, true);
            }
            ListName::Trash => {
                let rows = lists.trash.iter().map(|i| v.item_line(*i)).collect();
                section("Корзина".to_owned(), rows, true);
            }
            ListName::Projects => {
                let open = |p: &&Project| {
                    p.status.is_open() && !Deleted::of_project(p, v.area_of_project(p)).is_trashed()
                };
                let mut areas: Vec<&Area> =
                    s.areas.iter().filter(|a| !a.sync.is_deleted()).collect();
                areas.sort_by(|a, b| a.index.cmp(&b.index));
                let mut projects: Vec<&Project> = s.projects.iter().filter(open).collect();
                projects.sort_by(|a, b| a.index.cmp(&b.index));
                let rows = |area: Option<AreaId>| {
                    projects
                        .iter()
                        .filter(|p| p.area == area)
                        .map(|p| v.project_line(p))
                        .collect::<Vec<_>>()
                };
                let loose = rows(None);
                if !loose.is_empty() || areas.is_empty() {
                    section("Проекты".to_owned(), loose, true);
                }
                for a in areas {
                    section(format!("Область «{}»", a.title), rows(Some(a.id)), true);
                }
            }
        }
        write!(self.out, "{text}")?;
        Ok(())
    }

    fn search(&mut self, args: &SearchArgs) -> Result<()> {
        let query = args.query.join(" ");
        // Совпадения и данные для их вывода – из одного состояния базы.
        let (hits, s) = tally_app::search(&self.store, &query)?;
        let v = View::new(&s, self.today);
        if hits.is_empty() {
            writeln!(self.out, "Ничего не найдено")?;
        }
        for hit in hits {
            let closed = if hit.open { "" } else { " (закрыто)" };
            let line = match hit.found {
                Found::Task(id) => match s.tasks.iter().find(|t| t.id == id) {
                    Some(t) => v.task_line(t),
                    None => hit.title.clone(),
                },
                Found::Project(_) => format!("проект «{}»", hit.title),
                Found::Area(_) => format!("область «{}»", hit.title),
                Found::Tag(_) => format!("тег «{}»", hit.title),
            };
            writeln!(self.out, "{line}{closed}")?;
        }
        Ok(())
    }
}

fn add(args: AddArgs) -> tally_app::Command {
    tally_app::Command::AddTask(tally_app::NewTask {
        text: args.text.join(" "),
        when: args.when,
        deadline: args.deadline,
        project: args.project,
        heading: args.heading,
        area: args.area,
        notes: args.notes,
        tags: args.tags,
    })
}

fn done(args: DoneArgs) -> tally_app::Command {
    tally_app::Command::Close(tally_app::Close {
        tasks: args.tasks,
        cancel: args.cancel,
    })
}

fn move_task(args: MoveArgs) -> tally_app::Command {
    tally_app::Command::MoveTask(tally_app::MoveTask {
        task: args.task,
        when: args.when,
        inbox: args.inbox,
        project: args.project,
        heading: args.heading,
        area: args.area,
        no_project: args.no_project,
        deadline: args.deadline,
        no_deadline: args.no_deadline,
    })
}

fn project(args: ProjectArgs) -> tally_app::Command {
    tally_app::Command::AddProject(tally_app::NewProject {
        title: args.title.join(" "),
        area: args.area,
    })
}

fn area(args: AreaArgs) -> tally_app::Command {
    tally_app::Command::AddArea(tally_app::NewArea {
        title: args.title.join(" "),
    })
}

fn heading(args: HeadingArgs) -> tally_app::Command {
    tally_app::Command::AddHeading(tally_app::NewHeading {
        title: args.title.join(" "),
        project: args.project,
    })
}

/// Строка задачи после команды; задачи нет только при ошибке базы.
fn line(v: &View<'_>, task: Option<&Task>) -> String {
    task.map(|t| v.task_line(t)).unwrap_or_default()
}

/// «Отменено действие: перенос задачи «Отчёт»» и строки задач, которые
/// остались в базе.
fn stepped(v: &View<'_>, verb: &str, step: &Step) -> String {
    let mut text = format!("{verb}: {}\n", step.action);
    for id in &step.tasks {
        if let Some(t) = v.snapshot().tasks.iter().find(|t| t.id == *id) {
            text.push_str(&format!("  {}\n", v.task_line(t)));
        }
    }
    text
}

/// Снимок для вывода: поиск записей – [`Index`] из `tally-app`, здесь –
/// строки списков.
struct View<'a> {
    ix: Index<'a>,
}

impl<'a> Deref for View<'a> {
    type Target = Index<'a>;

    fn deref(&self) -> &Index<'a> {
        &self.ix
    }
}

impl<'a> View<'a> {
    fn new(s: &'a Snapshot, today: Date) -> Self {
        Self {
            ix: Index::new(s, today),
        }
    }

    /// «[3f9a2c] Купить хлеб – пт, 20 ноября · Дом ⚑ пт, 27 ноября».
    fn task_line(&self, t: &Task) -> String {
        let mut line = self.label(t);
        line.push_str(" – ");
        line.push_str(&show::place(
            self.list_of(t),
            t.date,
            t.evening,
            self.today(),
        ));
        self.details(t, &mut line, true);
        line
    }

    /// Проект или область, теги и срок: « · Дом #срочно ⚑ пт, 27 ноября».
    fn details(&self, t: &Task, line: &mut String, with_deadline: bool) {
        if let Some(container) = self.container(t) {
            line.push_str(" · ");
            line.push_str(&container);
        }
        for tag in &t.tags {
            // Тег в корзине не показывается; связь остаётся – после
            // восстановления тега пометка вернётся.
            if let Some(tag) = self
                .snapshot()
                .tags
                .iter()
                .find(|x| x.id == *tag && !x.sync.is_deleted())
            {
                line.push_str(" #");
                line.push_str(&tag.title);
            }
        }
        if let Some(d) = t.deadline.filter(|_| t.status.is_open() && with_deadline) {
            line.push(' ');
            line.push_str(&show::deadline(d, self.today()));
        }
    }

    /// Проект (и раздел) или область задачи.
    fn container(&self, t: &Task) -> Option<String> {
        match (self.project_of(t), self.heading_of(t)) {
            (Some(p), Some(h)) => Some(format!("{} / {}", p.title, h.title)),
            (Some(p), None) => Some(p.title.clone()),
            (None, _) => self.area_of(t).map(|a| a.title.clone()),
        }
    }

    /// Строка задачи в «Сегодня», «Планах» и «Журнале»: без названия
    /// списка, в котором она и так показана.
    fn item_line(&self, item: Item<'_>) -> String {
        self.item_line_with(item, true, None)
    }

    /// `with_deadline` – с пометкой срока; в «Планах» в день срока вместо
    /// неё – «⚑ срок». `day` – когда, после названия: «– ср, 15 декабря 2027»,
    /// «– вечер».
    fn item_line_with(&self, item: Item<'_>, with_deadline: bool, day: Option<String>) -> String {
        let mut line = self.item_head(item);
        if let Some(day) = day {
            line.push_str(" – ");
            line.push_str(&day);
        }
        match item {
            Item::Task(t) => self.details(t, &mut line, with_deadline),
            Item::Project(p) if with_deadline => self.project_deadline(p, &mut line),
            _ => {}
        }
        line
    }

    /// Начало строки: отметка, номер и название.
    fn item_head(&self, item: Item<'_>) -> String {
        match item {
            Item::Task(t) => match t.status {
                Status::Open => self.label(t),
                Status::Completed { .. } => format!("✓ {}", self.label(t)),
                Status::Canceled { .. } => format!("✕ {}", self.label(t)),
            },
            Item::Project(p) => {
                let mark = match p.status {
                    Status::Open => "",
                    Status::Completed { .. } => "✓ ",
                    Status::Canceled { .. } => "✕ ",
                };
                let progress =
                    project_progress(p.id, &self.snapshot().tasks, &self.snapshot().headings);
                format!(
                    "{mark}проект «{}» – {} из {}",
                    p.title, progress.done, progress.total
                )
            }
            Item::Heading(h) => {
                let project = self.project(h.project).map_or("", |p| p.title.as_str());
                format!("раздел «{}» проекта «{project}»", h.title)
            }
            Item::Area(a) => format!("область «{}»", a.title),
        }
    }

    /// «проект «Ремонт кухни» – 2 из 5».
    fn project_line(&self, p: &Project) -> String {
        self.item_line(Item::Project(p))
    }

    fn project_deadline(&self, p: &Project, line: &mut String) {
        if let Some(d) = p.deadline.filter(|_| p.status.is_open()) {
            line.push(' ');
            line.push_str(&show::deadline(d, self.today()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use clap::Parser;
    use jiff::SignedDuration;
    use jiff::civil::date;
    use tally_app::candidate_id_len;
    use tally_core::{Rank, Start, Tag, TagId, TaskId};
    use uuid::Uuid;

    use crate::cli::Cli;

    const TODAY: Date = date(2026, 11, 19);

    fn app(out: &mut Vec<u8>) -> App<'_> {
        App {
            store: Store::open_in_memory().unwrap_or_else(|e| panic!("{e}")),
            today: TODAY,
            now: Timestamp::UNIX_EPOCH,
            out,
        }
    }

    /// Выполнить команду; вывод – текст, ошибка – её сообщение.
    fn run(app: &mut App<'_>, args: &[&str]) -> Result<(), String> {
        let cli = Cli::try_parse_from(std::iter::once("tally").chain(args.iter().copied()))
            .map_err(|e| e.to_string())?;
        app.run(cli.command).map_err(|e| format!("{e:#}"))
    }

    fn task(id: u128, title: &str) -> Task {
        Task::new(
            TaskId::from_u128(id),
            title,
            Rank::initial(),
            Timestamp::UNIX_EPOCH,
        )
    }

    #[test]
    fn quick_entry_date_goes_to_today_and_evening() {
        let mut out = Vec::new();
        let mut a = app(&mut out);
        for args in [
            ["add", "Позвонить", "маме", "сегодня"].as_slice(),
            &["add", "Заказать ужин вечером"],
            &["add", "Купить хлеб завтра"],
            &["add", "Сдать отчёт до пятницы"],
        ] {
            assert_eq!(run(&mut a, args), Ok(()));
        }
        let s = a.snapshot().unwrap_or_default();
        let lists = s.lists(TODAY);
        fn titles<'a>(items: &[Item<'a>]) -> Vec<&'a str> {
            items.iter().map(Item::title).collect()
        }
        assert_eq!(titles(&lists.today.day), ["Позвонить маме"]);
        assert_eq!(titles(&lists.today.evening), ["Заказать ужин"]);
        let inbox: Vec<&str> = lists.inbox.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(inbox, ["Сдать отчёт до пятницы"]);
        let bread = s.tasks.iter().find(|t| t.title == "Купить хлеб");
        assert_eq!(bread.and_then(|t| t.date), Some(date(2026, 11, 20)));
    }

    #[test]
    fn explicit_when_keeps_the_text_whole() {
        let mut out = Vec::new();
        let mut a = app(&mut out);
        assert_eq!(
            run(&mut a, &["add", "Фильм «Завтра» завтра", "-w", "пт"]),
            Ok(())
        );
        let s = a.snapshot().unwrap_or_default();
        assert_eq!(s.tasks[0].title, "Фильм «Завтра» завтра");
        assert_eq!(s.tasks[0].date, Some(date(2026, 11, 20)));
    }

    #[test]
    fn deadline_is_a_day() {
        let mut out = Vec::new();
        let mut a = app(&mut out);
        assert_eq!(
            run(&mut a, &["add", "Отчёт", "-d", "завтра вечером"]),
            Err("срок: «вечером» у срока не бывает".to_owned())
        );
        assert_eq!(
            run(&mut a, &["add", "Отчёт", "-d", "когда-нибудь"]),
            Err("срок – это день: «когда-нибудь»".to_owned())
        );
        assert_eq!(
            run(&mut a, &["add", "Отчёт", "-d", "вчера"]),
            Err("срок: «вчера» – в прошлом; прошедший день задаётся датой".to_owned())
        );
        assert_eq!(run(&mut a, &["add", "Отчёт", "-d", "15 дек"]), Ok(()));
        let s = a.snapshot().unwrap_or_default();
        assert_eq!(s.tasks.len(), 1);
        assert_eq!(s.tasks[0].deadline, Some(date(2026, 12, 15)));
    }

    #[test]
    fn tasks_are_found_by_id_prefix_or_title() {
        let mut out = Vec::new();
        let mut a = app(&mut out);
        let tasks = [
            task(0xabcd_1000_0000_0000_0000_0000_0000_0000, "Купить хлеб"),
            task(0xabcd_2000_0000_0000_0000_0000_0000_0000, "Купить молоко"),
            task(0x1234_0000_0000_0000_0000_0000_0000_0000, "Хлеб"),
        ];
        let written = a
            .store
            .write(|w| tasks.iter().try_for_each(|t| w.insert_task(t)));
        assert!(written.is_ok());
        let s = a.snapshot().unwrap_or_default();
        let v = View::new(&s, TODAY);
        let title = |arg: &str| {
            v.resolve(arg, true)
                .map(|t| t.title.clone())
                .map_err(|e| e.to_string())
        };
        assert_eq!(title("abcd1"), Ok("Купить хлеб".to_owned()));
        assert_eq!(title("ABCD-2"), Ok("Купить молоко".to_owned()));
        assert_eq!(title("1234"), Ok("Хлеб".to_owned()));
        // Точное название важнее части.
        assert_eq!(title("хлеб"), Ok("Хлеб".to_owned()));
        assert_eq!(title("молоко"), Ok("Купить молоко".to_owned()));
        let twin = task(0x5678_0000_0000_0000_0000_0000_0000_0000, "хлеб");
        assert!(a.store.write(|w| w.insert_task(&twin)).is_ok());
        let s = a.snapshot().unwrap_or_default();
        let v = View::new(&s, TODAY);
        let title = |arg: &str| {
            v.resolve(arg, true)
                .map(|t| t.title.clone())
                .map_err(|e| e.to_string())
        };
        // Два совпадения целиком – в списке только они, без «Купить хлеб».
        assert_eq!(
            title("Хлеб"),
            Err(
                "«Хлеб» подходит к нескольким задачам:\n  [123400] Хлеб\n  [567800] хлеб"
                    .to_owned()
            )
        );
        assert_eq!(
            title("abcd"),
            Err("«abcd» подходит к нескольким задачам:\n  [abcd10] Купить хлеб\n  [abcd20] Купить молоко".to_owned())
        );
        assert_eq!(title("купить"), Err("«купить» подходит к нескольким задачам:\n  [abcd10] Купить хлеб\n  [abcd20] Купить молоко".to_owned()));
        assert_eq!(title("ffff"), Err("задача «ffff» не найдена".to_owned()));
    }

    #[test]
    fn done_closes_once() {
        let mut out = Vec::new();
        let mut a = app(&mut out);
        assert_eq!(run(&mut a, &["add", "Полить цветы"]), Ok(()));
        assert_eq!(run(&mut a, &["done", "цветы"]), Ok(()));
        let s = a.snapshot().unwrap_or_default();
        assert!(matches!(s.tasks[0].status, Status::Completed { .. }));
        // Закрытая задача по названию не ищется, по номеру – закрыта.
        assert_eq!(
            run(&mut a, &["done", "цветы"]),
            Err("задача «цветы» не найдена".to_owned())
        );
        let id = show::short(s.tasks[0].id.0, 6);
        assert_eq!(
            run(&mut a, &["done", &id]),
            Err(format!("задача [{id}] Полить цветы уже закрыта"))
        );
    }

    #[test]
    fn move_between_lists_and_containers() {
        let mut out = Vec::new();
        let mut a = app(&mut out);
        for args in [
            ["area", "Дом"].as_slice(),
            &["project", "Ремонт", "-a", "Дом"],
            &["heading", "Покупки", "-p", "ремонт"],
            &["add", "Купить краску"],
        ] {
            assert_eq!(run(&mut a, args), Ok(()));
        }
        assert_eq!(
            run(
                &mut a,
                &["move", "краску", "-p", "Ремонт", "--heading", "покупки"]
            ),
            Ok(())
        );
        let s = a.snapshot().unwrap_or_default();
        let t = &s.tasks[0];
        assert!(t.project.is_some() && t.heading.is_some() && t.area.is_none());
        assert_eq!(t.start, Start::Anytime);
        assert_eq!(
            run(
                &mut a,
                &["move", "краску", "-w", "когда-нибудь", "-d", "пт"]
            ),
            Ok(())
        );
        assert_eq!(run(&mut a, &["move", "краску", "-a", "дом"]), Ok(()));
        let s = a.snapshot().unwrap_or_default();
        let t = &s.tasks[0];
        assert!(t.project.is_none() && t.heading.is_none() && t.area.is_some());
        assert_eq!(
            (t.start, t.deadline),
            (Start::Someday, Some(date(2026, 11, 20)))
        );
        assert_eq!(
            run(&mut a, &["move", "краску", "--heading", "покупки"]),
            Err("задача не в проекте – укажите проект: --project".to_owned())
        );
        assert_eq!(
            run(&mut a, &["move", "краску", "--inbox", "--no-deadline"]),
            Ok(())
        );
        let s = a.snapshot().unwrap_or_default();
        let t = &s.tasks[0];
        assert_eq!((t.start, t.area, t.deadline), (Start::Inbox, None, None));
        assert_eq!(
            run(&mut a, &["move", "краску"]),
            Err("не указано, куда перенести задачу".to_owned())
        );
        assert_eq!(
            run(&mut a, &["project", "ремонт"]),
            Err("проект «ремонт» уже есть".to_owned())
        );
        assert_eq!(
            run(&mut a, &["add", "X", "-p", "Нет"]),
            Err("проекта «Нет» нет".to_owned())
        );
    }

    #[test]
    fn edit_time_never_goes_back_for_the_edited_record_only() {
        let mut out = Vec::new();
        let mut a = app(&mut out);
        let later = Timestamp::UNIX_EPOCH + SignedDuration::from_hours(1);
        let next = later + SignedDuration::from_nanos(1);
        let mut t = task(7, "Из будущего");
        t.sync.touch(later);
        let plain = task(8, "Обычная");
        assert!(a.store.write(|w| w.insert_task(&t)).is_ok());
        assert!(a.store.write(|w| w.insert_task(&plain)).is_ok());
        // Часы этого клиента отстают от правки задачи: время выполнения –
        // время команды, время правки – на наносекунду позже прежней.
        assert_eq!(run(&mut a, &["done", "будущего"]), Ok(()));
        assert_eq!(run(&mut a, &["move", "Обычная", "-w", "пт"]), Ok(()));
        assert_eq!(run(&mut a, &["add", "Новая"]), Ok(()));
        let s = a.snapshot().unwrap_or_default();
        let get = |title: &str| s.tasks.iter().find(|t| t.title == title).cloned();
        let future = get("Из будущего").unwrap_or_else(|| panic!("нет задачи"));
        assert_eq!(future.status, Status::Completed { at: a.now });
        assert_eq!(future.sync.updated_at, next);
        // Остальные записи живут по времени команды, чужая метка не в счёт.
        let plain = get("Обычная").unwrap_or_else(|| panic!("нет задачи"));
        assert_eq!(plain.sync.updated_at, a.now + SignedDuration::from_nanos(1));
        let new = get("Новая").unwrap_or_else(|| panic!("нет задачи"));
        assert_eq!((new.sync.created_at, new.sync.updated_at), (a.now, a.now));
    }

    #[test]
    fn moving_a_future_edited_task_advances_its_saved_timestamp() -> Result<()> {
        let mut out = Vec::new();
        let mut a = app(&mut out);
        let mut original = task(7, "Отчёт");
        a.now = "2026-11-19T12:00:00Z".parse()?;
        let future: Timestamp = "2036-11-19T12:00:00Z".parse()?;
        original.sync.touch(future);
        a.store.write(|w| w.insert_task(&original))?;

        assert_eq!(run(&mut a, &["move", "Отчёт", "-w", "завтра"]), Ok(()));
        let saved = a
            .store
            .read()
            .task(original.id)?
            .ok_or_else(|| anyhow!("нет задачи"))?;
        assert_eq!(saved.date, Some(date(2026, 11, 20)));
        assert_eq!(saved.sync.created_at, original.sync.created_at);
        assert_eq!(
            saved.sync.updated_at,
            future + SignedDuration::from_nanos(1)
        );
        Ok(())
    }

    #[test]
    fn moving_a_previously_edited_task_uses_the_command_time_when_later() -> Result<()> {
        let mut out = Vec::new();
        let mut a = app(&mut out);
        let mut original = task(8, "Письмо");
        original.sync.touch("2026-11-18T12:00:00Z".parse()?);
        a.store.write(|w| w.insert_task(&original))?;
        a.now = "2026-11-19T12:00:00Z".parse()?;

        assert_eq!(run(&mut a, &["move", "Письмо", "-w", "завтра"]), Ok(()));
        let saved = a
            .store
            .read()
            .task(original.id)?
            .ok_or_else(|| anyhow!("нет задачи"))?;
        assert_eq!(saved.date, Some(date(2026, 11, 20)));
        assert_eq!(saved.sync.updated_at, a.now);
        Ok(())
    }

    #[test]
    fn homonym_ids_are_long_enough_to_differ() {
        let id = |hex: u128| Uuid::from_u128(hex);
        let a = id(0x1234_5678_9a00_0000_0000_0000_0000_0000);
        let b = id(0x1234_5678_9b00_0000_0000_0000_0000_0000);
        let c = id(0xffff_0000_0000_0000_0000_0000_0000_0000);
        // Минимум – 8 знаков, даже когда хватило бы меньше.
        assert_eq!(candidate_id_len([a, c]), 8);
        // Общие 9 знаков – номер из 10.
        assert_eq!(candidate_id_len([a, b, c]), 10);
        let mut out = Vec::new();
        let mut app = app(&mut out);
        let tags: Vec<Tag> = [a, b]
            .into_iter()
            .map(|i| {
                Tag::new(
                    TagId::from_uuid(i),
                    "Дом",
                    Rank::initial(),
                    Timestamp::UNIX_EPOCH,
                )
            })
            .collect();
        assert!(
            app.store
                .write(|w| tags.iter().try_for_each(|t| w.insert_tag(t)))
                .is_ok()
        );
        assert_eq!(
            run(&mut app, &["add", "Письмо", "-t", "дом"]),
            Err("тегов «дом» несколько:\n  [123456789a] Дом\n  [123456789b] Дом".to_owned())
        );
        let s = app.snapshot().unwrap_or_default();
        assert!(s.tasks.is_empty());
    }

    /// Ни одна правка задачи «из будущего» не отводит её время правки назад:
    /// с любым набором параметров move и в done. Перенос, который ничего не
    /// меняет (задача и так во «Входящих», без проекта и срока), метку не
    /// трогает.
    #[test]
    fn every_edit_keeps_a_future_stamp_moving_forward() {
        let future = Timestamp::UNIX_EPOCH + SignedDuration::from_hours(24 * 365 * 10);
        let next = future + SignedDuration::from_nanos(1);
        for args in [
            ["-w", "завтра"].as_slice(),
            &["-w", "когда-нибудь"],
            &["--inbox"],
            &["-p", "Ремонт"],
            &["-a", "Дом"],
            &["--no-project"],
            &["-d", "пт"],
            &["--no-deadline"],
            &["-w", "пт", "-p", "Ремонт", "-d", "15 дек"],
        ] {
            let mut out = Vec::new();
            let mut a = app(&mut out);
            for setup in [["area", "Дом"].as_slice(), &["project", "Ремонт"]] {
                assert_eq!(run(&mut a, setup), Ok(()));
            }
            let mut t = task(9, "Отчёт");
            t.sync.touch(future);
            assert!(a.store.write(|w| w.insert_task(&t)).is_ok());
            let mut argv = vec!["move", "Отчёт"];
            argv.extend_from_slice(args);
            assert_eq!(run(&mut a, &argv), Ok(()), "{args:?}");
            let s = a.snapshot().unwrap_or_default();
            let saved = s.tasks.iter().find(|t| t.title == "Отчёт");
            let noop = matches!(args, ["--inbox"] | ["--no-project"] | ["--no-deadline"]);
            let next = if noop { future } else { next };
            assert_eq!(saved.map(|t| t.sync.updated_at), Some(next), "{args:?}");
            assert_eq!(run(&mut a, &["done", "Отчёт"]), Ok(()));
            let s = a.snapshot().unwrap_or_default();
            let saved = s.tasks.iter().find(|t| t.title == "Отчёт");
            let done = saved.map(|t| (t.status, t.sync.updated_at));
            let after = next + SignedDuration::from_nanos(1);
            assert_eq!(
                done,
                Some((Status::Completed { at: a.now }, after)),
                "{args:?}"
            );
        }
    }

    #[test]
    fn tags_are_created_once() {
        let mut out = Vec::new();
        let mut a = app(&mut out);
        assert_eq!(
            run(&mut a, &["add", "А", "-t", "Работа", "-t", "работа"]),
            Ok(())
        );
        assert_eq!(run(&mut a, &["add", "Б", "-t", "РАБОТА"]), Ok(()));
        let s = a.snapshot().unwrap_or_default();
        assert_eq!(s.tags.len(), 1);
        assert!(s.tasks.iter().all(|t| t.tags == [s.tags[0].id]));
    }
}
