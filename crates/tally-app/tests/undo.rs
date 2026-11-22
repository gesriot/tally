//! Отмена и повтор через `tally-app`: журнал в базе, метки времени,
//! конфликты, поток-владелец.

use jiff::civil::{Date, date};
use jiff::{SignedDuration, Timestamp};
use tally_app::{
    App, Clock, Close, Command, Effect, Error, HISTORY_LIMIT, MoveTask, NewArea, NewHeading,
    NewProject, NewTask, Service, Worker,
};
use tally_core::{Project, ProjectId, Rank, Start, Status, Task, TaskId};
use tally_store::{RecordId, Snapshot, Store};

const TODAY: Date = date(2026, 11, 19);

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn at(s: &str) -> Timestamp {
    s.parse().unwrap_or(Timestamp::UNIX_EPOCH)
}

/// Часы: каждая следующая команда – на секунду позже.
struct Session {
    app: App,
    now: Timestamp,
}

impl Session {
    fn new() -> Result<Self> {
        Ok(Self {
            app: App::new(Store::open_in_memory()?),
            now: at("2026-11-19T12:00:00Z"),
        })
    }

    fn clock(&mut self) -> Clock {
        self.now += SignedDuration::from_secs(1);
        Clock {
            today: TODAY,
            now: self.now,
        }
    }

    fn run(&mut self, command: Command) -> tally_app::Result<Effect> {
        let clock = self.clock();
        self.app.execute(&command, clock).map(|d| d.effect)
    }

    fn ok(&mut self, command: Command) -> Result<Effect> {
        Ok(self.run(command)?)
    }

    fn snapshot(&mut self) -> Result<Snapshot> {
        Ok(self.app.snapshot()?)
    }

    fn task(&mut self, title: &str) -> Result<Task> {
        self.snapshot()?
            .tasks
            .into_iter()
            .find(|t| t.title == title)
            .ok_or_else(|| format!("нет задачи «{title}»").into())
    }
}

fn add(text: &str) -> Command {
    Command::AddTask(NewTask {
        text: text.to_owned(),
        ..NewTask::default()
    })
}

fn project(title: &str) -> Command {
    Command::AddProject(NewProject {
        title: title.to_owned(),
        area: None,
    })
}

fn heading(title: &str, project: &str) -> Command {
    Command::AddHeading(NewHeading {
        title: title.to_owned(),
        project: project.to_owned(),
    })
}

/// Место задачи: проект, раздел, область, «когда», вечер и срок.
fn place(t: &Task) -> impl PartialEq + std::fmt::Debug {
    (
        t.project, t.heading, t.area, t.start, t.date, t.evening, t.deadline,
    )
}

/// Приёмочный сценарий: задача из раздела проекта переносится в другой
/// проект на новую дату; отмена возвращает проект, раздел и дату, повтор –
/// снова выбранный проект и новую дату.
#[test]
fn move_to_another_project_is_undone_and_redone() -> Result {
    let mut s = Session::new()?;
    s.ok(project("Ремонт"))?;
    s.ok(heading("Кухня", "Ремонт"))?;
    s.ok(project("Дача"))?;
    s.ok(Command::AddTask(NewTask {
        text: "Купить плитку".into(),
        when: Some("пт".into()),
        deadline: Some("15 дек".into()),
        project: Some("Ремонт".into()),
        heading: Some("Кухня".into()),
        ..NewTask::default()
    }))?;
    let original = s.task("Купить плитку")?;
    let snap = s.snapshot()?;
    let repair = snap
        .projects
        .iter()
        .find(|p| p.title == "Ремонт")
        .map(|p| p.id);
    let dacha = snap
        .projects
        .iter()
        .find(|p| p.title == "Дача")
        .map(|p| p.id);
    let kitchen = snap.headings.first().map(|h| h.id);
    assert_eq!(original.project, repair);
    assert_eq!(original.heading, kitchen);
    assert_eq!(original.date, Some(date(2026, 11, 20)));

    let moved = s.ok(Command::MoveTask(MoveTask {
        task: "плитку".into(),
        project: Some("Дача".into()),
        when: Some("через неделю вечером".into()),
        ..MoveTask::default()
    }))?;
    assert_eq!(moved, Effect::Moved(original.id));
    let after_move = s.task("Купить плитку")?;
    assert_eq!(after_move.project, dacha);
    assert_eq!(after_move.heading, None);
    assert_eq!(
        (after_move.date, after_move.evening),
        (Some(date(2026, 11, 26)), true)
    );

    let Effect::Undone(step) = s.ok(Command::Undo)? else {
        return Err("не отмена".into());
    };
    assert_eq!(step.action, "перенос задачи «Купить плитку»");
    assert_eq!(step.tasks, [original.id]);
    let undone = s.task("Купить плитку")?;
    assert_eq!(place(&undone), place(&original));
    assert_eq!(undone.sync.created_at, original.sync.created_at);
    assert!(undone.sync.updated_at > after_move.sync.updated_at);

    let Effect::Redone(step) = s.ok(Command::Redo)? else {
        return Err("не повтор".into());
    };
    assert_eq!(step.action, "перенос задачи «Купить плитку»");
    let redone = s.task("Купить плитку")?;
    assert_eq!(place(&redone), place(&after_move));
    assert!(redone.sync.updated_at > undone.sync.updated_at);
    // Кроме времени правки, повтор возвращает ровно то, что оставил перенос.
    let mut expected = after_move.clone();
    expected.sync.updated_at = redone.sync.updated_at;
    assert_eq!(redone, expected);
    Ok(())
}

#[test]
fn undo_of_add_removes_the_task_and_its_new_tags_only() -> Result {
    let mut s = Session::new()?;
    s.ok(Command::AddTask(NewTask {
        text: "Первая".into(),
        tags: vec!["Дом".into()],
        ..NewTask::default()
    }))?;
    s.ok(Command::AddTask(NewTask {
        text: "Письмо".into(),
        tags: vec!["дом".into(), "Работа".into()],
        ..NewTask::default()
    }))?;
    let letter = s.task("Письмо")?;
    assert_eq!(s.snapshot()?.tags.len(), 2);

    s.ok(Command::Undo)?;
    let snap = s.snapshot()?;
    assert!(snap.tasks.iter().all(|t| t.title != "Письмо"));
    // «Работу» создало отменённое действие – её нет; «Дом» был раньше.
    let tags: Vec<&str> = snap.tags.iter().map(|t| t.title.as_str()).collect();
    assert_eq!(tags, ["Дом"]);

    s.ok(Command::Redo)?;
    let back = s.task("Письмо")?;
    assert_eq!(back.id, letter.id);
    assert_eq!(back.tags, letter.tags);
    assert_eq!(back.sync.created_at, letter.sync.created_at);
    assert!(back.sync.updated_at > letter.sync.updated_at);
    assert_eq!(s.snapshot()?.tags.len(), 2);
    Ok(())
}

#[test]
fn undo_of_done_reopens_and_redo_keeps_the_completion_time() -> Result {
    let mut s = Session::new()?;
    s.ok(add("Отчёт"))?;
    s.ok(add("Письмо"))?;
    s.ok(Command::Close(Close {
        tasks: vec!["Отчёт".into(), "Письмо".into()],
        cancel: false,
    }))?;
    let done = s.task("Отчёт")?;
    let Status::Completed { at: completed } = done.status else {
        return Err("не выполнена".into());
    };
    let Effect::Undone(step) = s.ok(Command::Undo)? else {
        return Err("не отмена".into());
    };
    assert_eq!(step.action, "выполнение задач «Отчёт», «Письмо»");
    assert_eq!(step.tasks.len(), 2);
    assert_eq!(s.task("Отчёт")?.status, Status::Open);
    assert_eq!(s.task("Письмо")?.status, Status::Open);
    s.ok(Command::Redo)?;
    assert_eq!(s.task("Отчёт")?.status, Status::Completed { at: completed });

    s.ok(add("Звонок"))?;
    s.ok(Command::Close(Close {
        tasks: vec!["Звонок".into()],
        cancel: true,
    }))?;
    let Effect::Undone(step) = s.ok(Command::Undo)? else {
        return Err("не отмена".into());
    };
    assert_eq!(step.action, "отмена задачи «Звонок»");
    assert_eq!(s.task("Звонок")?.status, Status::Open);
    Ok(())
}

#[test]
fn containers_are_created_and_removed_by_undo() -> Result {
    let mut s = Session::new()?;
    s.ok(Command::AddArea(NewArea {
        title: "Дом".into(),
    }))?;
    s.ok(Command::AddProject(NewProject {
        title: "Ремонт".into(),
        area: Some("Дом".into()),
    }))?;
    s.ok(heading("Кухня", "Ремонт"))?;
    let full = s.snapshot()?;
    for action in [
        "создание раздела «Кухня» в проекте «Ремонт»",
        "создание проекта «Ремонт»",
        "создание области «Дом»",
    ] {
        let Effect::Undone(step) = s.ok(Command::Undo)? else {
            return Err("не отмена".into());
        };
        assert_eq!(step.action, action);
        assert!(step.tasks.is_empty());
    }
    let empty = s.snapshot()?;
    assert!(empty.areas.is_empty() && empty.projects.is_empty() && empty.headings.is_empty());
    for _ in 0..3 {
        s.ok(Command::Redo)?;
    }
    let again = s.snapshot()?;
    let ids = |s: &Snapshot| {
        (
            s.areas.iter().map(|a| a.id).collect::<Vec<_>>(),
            s.projects
                .iter()
                .map(|p| (p.id, p.area))
                .collect::<Vec<_>>(),
            s.headings
                .iter()
                .map(|h| (h.id, h.project))
                .collect::<Vec<_>>(),
        )
    };
    assert_eq!(ids(&again), ids(&full));
    Ok(())
}

#[test]
fn empty_history_is_an_error_that_writes_nothing() -> Result {
    let mut s = Session::new()?;
    assert!(matches!(s.run(Command::Undo), Err(Error::NothingToUndo)));
    assert!(matches!(s.run(Command::Redo), Err(Error::NothingToRedo)));
    assert_eq!(
        s.run(Command::Undo).map_err(|e| e.to_string()),
        Err("нечего отменять".to_owned())
    );
    assert_eq!(
        s.run(Command::Redo).map_err(|e| e.to_string()),
        Err("нечего повторять".to_owned())
    );
    s.ok(add("Отчёт"))?;
    // Ничего не отменено – повторять нечего; отменять – одно действие.
    assert!(matches!(s.run(Command::Redo), Err(Error::NothingToRedo)));
    s.ok(Command::Undo)?;
    assert!(matches!(s.run(Command::Undo), Err(Error::NothingToUndo)));
    assert_eq!(s.app.store().read().history()?.len(), 1);
    Ok(())
}

#[test]
fn a_new_command_clears_redo_and_failed_commands_are_not_recorded() -> Result {
    let mut s = Session::new()?;
    s.ok(add("Первая"))?;
    s.ok(add("Вторая"))?;
    s.ok(Command::Undo)?;
    s.ok(add("Третья"))?;
    assert!(matches!(s.run(Command::Redo), Err(Error::NothingToRedo)));
    let history = s.app.store().read().history()?;
    let actions: Vec<&str> = history.iter().map(|e| e.action.as_str()).collect();
    assert_eq!(
        actions,
        ["добавление задачи «Первая»", "добавление задачи «Третья»"]
    );
    // Ошибка команды – ни записи, ни строки журнала.
    assert!(
        s.run(Command::MoveTask(MoveTask {
            task: "Нет такой".into(),
            inbox: true,
            ..MoveTask::default()
        }))
        .is_err()
    );
    assert!(s.run(project("")).is_err());
    assert_eq!(s.app.store().read().history()?.len(), 2);
    s.ok(Command::Undo)?;
    s.ok(Command::Undo)?;
    assert!(s.snapshot()?.tasks.is_empty());
    Ok(())
}

#[test]
fn history_keeps_the_last_actions_only() -> Result {
    let mut s = Session::new()?;
    for n in 0..=HISTORY_LIMIT {
        s.ok(add(&format!("Задача {n}")))?;
    }
    let history = s.app.store().read().history()?;
    assert_eq!(history.len(), HISTORY_LIMIT);
    assert_eq!(history[0].action, "добавление задачи «Задача 1»");
    for _ in 0..HISTORY_LIMIT {
        s.ok(Command::Undo)?;
    }
    assert!(matches!(s.run(Command::Undo), Err(Error::NothingToUndo)));
    let left: Vec<String> = s.snapshot()?.tasks.into_iter().map(|t| t.title).collect();
    assert_eq!(left, ["Задача 0"]);
    Ok(())
}

/// Запись изменили в обход журнала – отмена и повтор отклоняются, база и
/// журнал не меняются.
#[test]
fn changes_outside_history_are_conflicts() -> Result {
    let mut s = Session::new()?;
    s.ok(add("Отчёт"))?;
    s.ok(Command::MoveTask(MoveTask {
        task: "Отчёт".into(),
        when: Some("завтра".into()),
        ..MoveTask::default()
    }))?;
    let mut task = s.task("Отчёт")?;
    let label = format!("[{}] Отчёт", tally_app::short(task.id.0, 6));
    task.title = "Отчёт за ноябрь".into();
    task.sync.touch(s.now + SignedDuration::from_secs(1));
    s.app.store_mut().write(|w| w.update_task(&task))?;
    let before = (s.snapshot()?, s.app.store().read().history()?);
    let err = s.run(Command::Undo);
    assert!(matches!(err, Err(Error::Conflict(_))), "{err:?}");
    assert_eq!(
        err.map_err(|e| e.to_string()),
        Err(format!(
            "нельзя отменить перенос задачи «Отчёт»: задачу {label} за ноябрь изменили после этого действия"
        ))
    );
    assert_eq!((s.snapshot()?, s.app.store().read().history()?), before);

    // Вернули как было (только метка новее) – отмена снова возможна.
    task.title = "Отчёт".into();
    s.app.store_mut().write(|w| w.update_task(&task))?;
    s.ok(Command::Undo)?;
    assert_eq!(s.task("Отчёт")?.date, None);

    // Отменённое действие задачу убрало бы – а её удалили сами.
    s.ok(Command::Undo)?;
    s.ok(Command::Redo)?;
    let id = s.task("Отчёт")?.id;
    let now = s.now + SignedDuration::from_secs(1);
    s.app.store_mut().write(|w| {
        w.delete(RecordId::Task(id), now)?;
        w.purge(RecordId::Task(id))
    })?;
    assert_eq!(
        s.run(Command::Undo).map_err(|e| e.to_string()),
        Err(format!(
            "нельзя отменить добавление задачи «Отчёт»: задачу {label} удалили после этого действия"
        ))
    );
    Ok(())
}

/// Отмена создания проекта не уносит задачу, которую положили в него в
/// обход журнала.
#[test]
fn undo_does_not_remove_a_record_others_refer_to() -> Result {
    let mut s = Session::new()?;
    s.ok(project("Ремонт"))?;
    let p = s.snapshot()?.projects[0].id;
    let mut t = Task::new(TaskId::from_u128(7), "Плитка", Rank::initial(), s.now);
    t.project = Some(p);
    t.start = Start::Anytime;
    s.app.store_mut().write(|w| w.insert_task(&t))?;
    let label = format!("[{}] Плитка", tally_app::short(t.id.0, 6));
    assert_eq!(
        s.run(Command::Undo).map_err(|e| e.to_string()),
        Err(format!(
            "нельзя отменить создание проекта «Ремонт»: на проект «Ремонт» ссылается задача {label}"
        ))
    );
    assert_eq!(s.snapshot()?.projects.len(), 1);
    assert_eq!(s.snapshot()?.tasks.len(), 1);
    Ok(())
}

/// Время правки не идёт назад и при отмене: запись, правленная «из
/// будущего», получает +1 нс на каждом шаге.
#[test]
fn undo_and_redo_keep_future_stamps_moving_forward() -> Result {
    let mut s = Session::new()?;
    let future = at("2036-11-19T12:00:00Z");
    let mut t = Task::new(TaskId::from_u128(9), "Отчёт", Rank::initial(), s.now);
    t.sync.touch(future);
    s.app.store_mut().write(|w| w.insert_task(&t))?;
    s.ok(Command::MoveTask(MoveTask {
        task: "Отчёт".into(),
        when: Some("когда-нибудь".into()),
        ..MoveTask::default()
    }))?;
    let ns = SignedDuration::from_nanos;
    assert_eq!(s.task("Отчёт")?.sync.updated_at, future + ns(1));
    s.ok(Command::Undo)?;
    let undone = s.task("Отчёт")?;
    assert_eq!(undone.start, Start::Inbox);
    assert_eq!(undone.sync.updated_at, future + ns(2));
    s.ok(Command::Redo)?;
    let redone = s.task("Отчёт")?;
    assert_eq!(redone.start, Start::Someday);
    assert_eq!(redone.sync.updated_at, future + ns(3));
    s.ok(Command::Undo)?;
    assert_eq!(s.task("Отчёт")?.sync.updated_at, future + ns(4));
    Ok(())
}

/// Отменённая задача возвращается повтором позже метки, с которой её
/// убрала отмена, даже если часы ушли назад.
#[test]
fn recreated_records_are_stamped_after_their_removal() -> Result {
    let mut s = Session::new()?;
    s.ok(add("Отчёт"))?;
    let created = s.task("Отчёт")?.sync;
    s.ok(Command::Undo)?;
    let removed_at = s.now;
    let clock = Clock {
        today: TODAY,
        now: at("2026-01-01T00:00:00Z"),
    };
    s.app.execute(&Command::Redo, clock)?;
    let back = s.task("Отчёт")?;
    assert_eq!(back.sync.created_at, created.created_at);
    assert_eq!(
        back.sync.updated_at,
        removed_at + SignedDuration::from_nanos(1)
    );
    Ok(())
}

/// Журнал – в базе: отмена работает после перезапуска и из другого
/// соединения.
#[test]
fn history_survives_reopening_and_is_shared() -> Result {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let clock = |sec: i64| Clock {
        today: TODAY,
        now: at("2026-11-19T12:00:00Z") + SignedDuration::from_secs(sec),
    };
    {
        let mut app = App::new(Store::open(&path)?);
        app.execute(&project("Дача"), clock(1))?;
        app.execute(&add("Полить"), clock(2))?;
    }
    let mut a = App::new(Store::open(&path)?);
    let mut b = App::new(Store::open(&path)?);
    a.execute(
        &Command::MoveTask(MoveTask {
            task: "Полить".into(),
            project: Some("Дача".into()),
            ..MoveTask::default()
        }),
        clock(3),
    )?;
    let done = b.execute(&Command::Undo, clock(4))?;
    let task = done.snapshot.tasks.first().ok_or("нет задачи")?;
    assert_eq!(task.project, None);
    let done = a.execute(&Command::Redo, clock(5))?;
    assert!(done.snapshot.tasks.iter().all(|t| t.project.is_some()));
    Ok(())
}

/// Две отмены одновременно из разных соединений отменяют два разных
/// действия: журнал читается и пишется в одной транзакции записи.
#[test]
fn parallel_undos_take_different_actions() -> Result {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let clock = Clock {
        today: TODAY,
        now: at("2026-11-19T12:00:00Z"),
    };
    {
        let mut app = App::new(Store::open(&path)?);
        app.execute(&add("Первая"), clock)?;
        app.execute(&add("Вторая"), clock)?;
    }
    let barrier = std::sync::Barrier::new(2);
    let results: Vec<String> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..2)
            .map(|_| {
                scope.spawn(|| {
                    let store = Store::open(&path).map_err(|e| e.to_string())?;
                    let mut app = App::new(store);
                    barrier.wait();
                    match app.execute(&Command::Undo, clock) {
                        Ok(d) => match d.effect {
                            Effect::Undone(step) => Ok(step.action),
                            other => Err(format!("{other:?}")),
                        },
                        Err(e) => Err(e.to_string()),
                    }
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| match h.join() {
                Ok(Ok(action)) => action,
                Ok(Err(e)) => e,
                Err(_) => "паника".to_owned(),
            })
            .collect()
    });
    let mut results = results;
    results.sort();
    assert_eq!(
        results,
        ["добавление задачи «Вторая»", "добавление задачи «Первая»"]
    );
    let store = Store::open(&path)?;
    assert!(store.read().tasks()?.is_empty());
    Ok(())
}

/// Поток – владелец базы отвечает так же, как синхронный вариант.
#[test]
fn worker_serves_the_same_commands() -> Result {
    let mut worker = Worker::spawn(Store::open_in_memory()?);
    let clock = Clock {
        today: TODAY,
        now: at("2026-11-19T12:00:00Z"),
    };
    let added = worker.execute(&add("Купить хлеб завтра"), clock)?;
    let Effect::Added(id) = added.effect else {
        return Err("не добавлено".into());
    };
    let task = added.snapshot.tasks.iter().find(|t| t.id == id);
    assert_eq!(task.and_then(|t| t.date), Some(date(2026, 11, 20)));
    let (hits, _) = std::thread::scope(|scope| scope.spawn(|| worker.search("хлеб")).join())
        .map_err(|_| "паника")??;
    assert_eq!(hits.len(), 1);
    worker.execute(&Command::Undo, clock)?;
    assert!(worker.snapshot()?.tasks.is_empty());
    assert!(matches!(
        worker.execute(&Command::Undo, clock),
        Err(Error::NothingToUndo)
    ));
    Ok(())
}

/// Ошибки команд и сообщения о неоднозначности – те же, что были в CLI.
#[test]
fn command_errors_are_unchanged() -> Result {
    let mut s = Session::new()?;
    let p = Project::new(ProjectId::from_u128(1), "Дом", Rank::initial(), s.now);
    let twin = Project::new(ProjectId::from_u128(2), "дом", Rank::initial(), s.now);
    s.app.store_mut().write(|w| {
        w.insert_project(&p)?;
        w.insert_project(&twin)
    })?;
    let err = |s: &mut Session, c: Command| s.run(c).err().map(|e| e.to_string());
    assert_eq!(
        err(
            &mut s,
            Command::AddTask(NewTask {
                text: "Письмо".into(),
                project: Some("Дом".into()),
                ..NewTask::default()
            })
        ),
        Some(format!(
            "проектов «Дом» несколько:\n  [{}] Дом\n  [{}] дом",
            tally_app::short(p.id.0, 32),
            tally_app::short(twin.id.0, 32)
        ))
    );
    assert_eq!(
        err(&mut s, Command::MoveTask(MoveTask::default())),
        Some("не указано, куда перенести задачу".to_owned())
    );
    assert_eq!(
        err(
            &mut s,
            Command::AddTask(NewTask {
                text: "Отчёт".into(),
                deadline: Some("завтра вечером".into()),
                ..NewTask::default()
            })
        ),
        Some("срок: «вечером» у срока не бывает".to_owned())
    );
    assert!(s.app.store().read().history()?.is_empty());
    Ok(())
}

/// Повтор вставил бы задачу в проект, которого уже нет, – конфликт.
#[test]
fn redo_into_a_vanished_project_is_a_conflict() -> Result {
    let mut s = Session::new()?;
    s.ok(project("Дача"))?;
    s.ok(Command::AddTask(NewTask {
        text: "Полить".into(),
        project: Some("Дача".into()),
        ..NewTask::default()
    }))?;
    s.ok(Command::Undo)?;
    let p = RecordId::Project(s.snapshot()?.projects[0].id);
    let id_of = |r: RecordId| match r {
        RecordId::Project(p) => p.0,
        _ => uuid::Uuid::nil(),
    };
    let now = s.now + SignedDuration::from_secs(1);
    s.app.store_mut().write(|w| {
        w.delete(p, now)?;
        w.purge(p)
    })?;
    let err = s.run(Command::Redo);
    assert!(matches!(err, Err(Error::Conflict(_))), "{err:?}");
    assert_eq!(
        err.map_err(|e| e.to_string()),
        Err(format!(
            "нельзя повторить добавление задачи «Полить»: проекта [{}] нет в базе",
            tally_app::short(id_of(p), 8)
        ))
    );
    assert!(s.snapshot()?.tasks.is_empty());
    assert!(s.app.store().read().first_undone()?.is_some());
    Ok(())
}
