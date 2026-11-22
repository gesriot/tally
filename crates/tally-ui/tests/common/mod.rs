//! Общее для тестов окна: демо-база через команды `tally-app`.

#![allow(dead_code)]

use jiff::Timestamp;
use jiff::civil::{Date, date};
use tally_app::{Clock, Command, NewArea, NewHeading, NewProject, NewTask, execute};
use tally_core::{Task, TaskId};
use tally_store::{Snapshot, Store};

pub type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Пятница, 20 ноября 2026.
pub const TODAY: Date = date(2026, 11, 20);

pub fn clock() -> Clock {
    Clock {
        today: TODAY,
        now: "2026-11-20T09:00:00Z"
            .parse()
            .unwrap_or(Timestamp::UNIX_EPOCH),
    }
}

pub fn run(store: &mut Store, command: Command) -> Result {
    execute(store, &command, clock())?;
    Ok(())
}

pub fn task(text: &str) -> NewTask {
    NewTask {
        text: text.into(),
        ..NewTask::default()
    }
}

/// Как `scripts/seed-demo.sh`, но короче: области «Дом» и «Работа»,
/// «Ремонт» с разделами «Кухня» и «Ванная», задачи на сегодня и вечер.
pub fn seed(store: &mut Store) -> Result {
    for area in ["Дом", "Работа"] {
        run(store, Command::AddArea(NewArea { title: area.into() }))?;
    }
    let project = |title: &str, area: Option<&str>| {
        Command::AddProject(NewProject {
            title: title.into(),
            area: area.map(Into::into),
        })
    };
    run(store, project("Ремонт", Some("Дом")))?;
    run(store, project("Отпуск", None))?;
    for heading in ["Кухня", "Ванная"] {
        run(
            store,
            Command::AddHeading(NewHeading {
                title: heading.into(),
                project: "Ремонт".into(),
            }),
        )?;
    }
    let add = |text: &str, f: &dyn Fn(&mut NewTask)| {
        let mut t = task(text);
        f(&mut t);
        Command::AddTask(t)
    };
    let cmds = [
        add("Записаться к стоматологу", &|_| {}),
        add("Купить плитку", &|t| {
            t.project = Some("Ремонт".into());
            t.heading = Some("Кухня".into());
            t.when = Some("сегодня".into());
            t.deadline = Some("пн".into());
        }),
        add("Выбрать краску", &|t| {
            t.project = Some("Ремонт".into());
            t.heading = Some("Кухня".into());
        }),
        add("Заказать смеситель", &|t| {
            t.project = Some("Ремонт".into());
            t.heading = Some("Ванная".into());
            t.when = Some("завтра".into());
        }),
        add("Позвонить электрику", &|t| {
            t.project = Some("Ремонт".into());
            t.when = Some("сегодня".into());
        }),
        add("Оплатить интернет", &|t| {
            t.area = Some("Дом".into());
            t.when = Some("сегодня".into());
            t.deadline = Some("сегодня".into());
            t.tags = vec!["счета".into()];
        }),
        add("Позвонить маме", &|t| {
            t.when = Some("сегодня вечером".into())
        }),
        add("Посмотреть фильм", &|t| {
            t.when = Some("в любое время".into())
        }),
    ];
    for c in cmds {
        run(store, c)?;
    }
    Ok(())
}

pub fn snapshot(store: &Store) -> Result<Snapshot> {
    Ok(store.read().snapshot()?)
}

pub fn find<'a>(s: &'a Snapshot, title: &str) -> Result<&'a Task> {
    Ok(s.tasks
        .iter()
        .find(|t| t.title == title)
        .ok_or_else(|| format!("нет задачи «{title}»"))?)
}

pub fn id_of(s: &Snapshot, title: &str) -> Result<TaskId> {
    Ok(find(s, title)?.id)
}
