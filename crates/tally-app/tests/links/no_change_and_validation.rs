//! Команды без изменений, проверки входных значений, связи при отмене и
//! повторе, метки пунктов чек-листа, предел меток.

use jiff::civil::{Date, date};
use jiff::{SignedDuration, Timestamp};
use tally_app::{
    Clock, Close, Command, Effect, Error, MoveTask, NewArea, NewHeading, NewProject, NewTask,
    execute,
};
use tally_core::{ChecklistItem, ChecklistItemId, Rank, Status, Task, TaskId};
use tally_store::{HistoryEntry, RecordId, Snapshot, Store};

const TODAY: Date = date(2026, 11, 19);

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn at(s: &str) -> Timestamp {
    s.parse().unwrap_or(Timestamp::UNIX_EPOCH)
}

fn clock() -> Clock {
    Clock {
        today: TODAY,
        now: at("2026-11-19T12:00:00Z"),
    }
}

fn run(s: &mut Store, c: Command) -> tally_app::Result<Effect> {
    execute(s, &c, clock()).map(|d| d.effect)
}

fn state(s: &Store) -> Result<(Snapshot, Vec<HistoryEntry>)> {
    Ok((s.read().snapshot()?, s.read().history()?))
}

/// Ошибка с текстом `message`, база и журнал не меняются.
fn rejected(s: &mut Store, c: Command, message: &str) -> Result {
    let before = state(s)?;
    let r = run(s, c);
    assert!(matches!(&r, Err(Error::Rejected(_))), "{r:?}");
    assert_eq!(r.map_err(|e| e.to_string()), Err(message.to_owned()));
    assert_eq!(state(s)?, before);
    Ok(())
}

/// Конфликт с текстом `message`, база и журнал не меняются.
fn conflict(s: &mut Store, c: Command, message: &str) -> Result {
    let before = state(s)?;
    let r = run(s, c);
    assert!(matches!(&r, Err(Error::Conflict(_))), "{r:?}");
    assert_eq!(r.map_err(|e| e.to_string()), Err(message.to_owned()));
    assert_eq!(state(s)?, before);
    Ok(())
}

fn add(text: &str) -> Command {
    Command::AddTask(NewTask {
        text: text.into(),
        ..NewTask::default()
    })
}

fn add_in(text: &str, project: &str, heading: Option<&str>) -> Command {
    Command::AddTask(NewTask {
        text: text.into(),
        project: Some(project.into()),
        heading: heading.map(Into::into),
        ..NewTask::default()
    })
}

fn project(title: &str) -> Command {
    Command::AddProject(NewProject {
        title: title.into(),
        area: None,
    })
}

fn heading(title: &str, project: &str) -> Command {
    Command::AddHeading(NewHeading {
        title: title.into(),
        project: project.into(),
    })
}

fn moved(task: &str, f: impl FnOnce(&mut MoveTask)) -> Command {
    let mut m = MoveTask {
        task: task.into(),
        ..MoveTask::default()
    };
    f(&mut m);
    Command::MoveTask(m)
}

fn close(task: &str) -> Command {
    Command::Close(Close {
        tasks: vec![task.into()],
        cancel: false,
    })
}

fn task(s: &Store, title: &str) -> Result<Task> {
    s.read()
        .tasks()?
        .into_iter()
        .find(|t| t.title == title)
        .ok_or_else(|| format!("нет задачи «{title}»").into())
}

fn short(id: uuid::Uuid) -> String {
    tally_app::short(id, 6)
}

#[test]
fn commands_without_changes_write_nothing() -> Result {
    let mut s = Store::open_in_memory()?;
    run(&mut s, project("Ремонт"))?;
    run(
        &mut s,
        Command::AddTask(NewTask {
            text: "Отчёт".into(),
            when: Some("пт".into()),
            deadline: Some("15 дек".into()),
            project: Some("Ремонт".into()),
            ..NewTask::default()
        }),
    )?;
    let id = task(&s, "Отчёт")?.id;
    for c in [
        moved("Отчёт", |m| m.when = Some("пт".into())),
        moved("Отчёт", |m| m.project = Some("ремонт".into())),
        moved("Отчёт", |m| m.deadline = Some("15 декабря".into())),
        moved("Отчёт", |m| {
            m.when = Some("20 ноября".into());
            m.project = Some("Ремонт".into());
        }),
    ] {
        let before = state(&s)?;
        // Успех с обычным результатом, но ни метки, ни строки журнала.
        assert_eq!(run(&mut s, c)?, Effect::Moved(id));
        assert_eq!(state(&s)?, before);
    }
    // Изменение есть – запись и журнал.
    run(&mut s, moved("Отчёт", |m| m.no_deadline = true))?;
    assert_eq!(s.read().history()?.len(), 3);
    assert_eq!(task(&s, "Отчёт")?.deadline, None);
    Ok(())
}

#[test]
fn a_no_op_keeps_the_redo_branch_and_the_future_stamp() -> Result {
    let mut s = Store::open_in_memory()?;
    let mut t = Task::new(TaskId::from_u128(9), "Отчёт", Rank::initial(), clock().now);
    let future = at("2036-11-19T12:00:00Z");
    t.sync.touch(future);
    s.write(|w| w.insert_task(&t))?;
    run(&mut s, moved("Отчёт", |m| m.when = Some("завтра".into())))?;
    run(&mut s, Command::Undo)?;
    let before = state(&s)?;
    run(&mut s, moved("Отчёт", |m| m.inbox = true))?;
    assert_eq!(state(&s)?, before);
    let Effect::Redone(step) = run(&mut s, Command::Redo)? else {
        return Err("не повтор".into());
    };
    assert_eq!(step.action, "перенос задачи «Отчёт»");
    assert_eq!(task(&s, "Отчёт")?.date, Some(date(2026, 11, 20)));
    assert_eq!(
        task(&s, "Отчёт")?.sync.updated_at,
        future + SignedDuration::from_nanos(3)
    );
    Ok(())
}

#[test]
fn the_api_checks_what_the_command_line_checks() -> Result {
    let mut s = Store::open_in_memory()?;
    run(&mut s, project("Ремонт"))?;
    run(
        &mut s,
        Command::AddArea(NewArea {
            title: "Дом".into(),
        }),
    )?;
    run(&mut s, add("Отчёт"))?;
    let new = |f: fn(&mut NewTask)| {
        let mut n = NewTask {
            text: "Письмо".into(),
            ..NewTask::default()
        };
        f(&mut n);
        Command::AddTask(n)
    };
    for (c, message) in [
        (
            new(|n| n.heading = Some("Кухня".into())),
            "раздел указывается вместе с проектом",
        ),
        (
            new(|n| {
                n.project = Some("Ремонт".into());
                n.area = Some("Дом".into());
            }),
            "задача не может быть сразу в проекте и в области",
        ),
        (
            new(|n| n.tags = vec!["Работа".into(), " ".into()]),
            "пустое название тега",
        ),
        (
            new(|n| n.tags = vec![String::new()]),
            "пустое название тега",
        ),
        (
            new(|n| n.when = Some(" ".into())),
            "пустое значение «когда»",
        ),
        (new(|n| n.deadline = Some(String::new())), "пустой срок"),
        (
            new(|n| n.project = Some("  ".into())),
            "пустое название проекта",
        ),
        (new(|n| n.area = Some("".into())), "пустое название области"),
        (
            new(|n| {
                n.project = Some("Ремонт".into());
                n.heading = Some(" ".into());
            }),
            "пустое название раздела",
        ),
        (new(|n| n.text = "  ".into()), "пустое название задачи"),
        (Command::Close(Close::default()), "не указаны задачи"),
        (
            Command::Close(Close {
                tasks: vec!["  ".into()],
                cancel: true,
            }),
            "пустой номер задачи",
        ),
        (
            moved("Отчёт", |m| {
                m.inbox = true;
                m.when = Some("завтра".into());
            }),
            "несовместимые изменения: во «Входящие» и дата",
        ),
        (
            moved("Отчёт", |m| {
                m.project = Some("Ремонт".into());
                m.area = Some("Дом".into());
            }),
            "несовместимые изменения: проект и область",
        ),
        (
            moved("Отчёт", |m| {
                m.heading = Some("Кухня".into());
                m.no_project = true;
            }),
            "несовместимые изменения: раздел и без проекта",
        ),
        (
            moved("Отчёт", |m| {
                m.deadline = Some("пт".into());
                m.no_deadline = true;
            }),
            "несовместимые изменения: срок и без срока",
        ),
        (
            moved("Отчёт", |m| m.project = Some(" ".into())),
            "пустое название проекта",
        ),
        (moved(" ", |m| m.inbox = true), "пустой номер задачи"),
        (project(" "), "пустое название"),
        (
            Command::AddProject(NewProject {
                title: "Дача".into(),
                area: Some(" ".into()),
            }),
            "пустое название области",
        ),
        (
            Command::AddArea(NewArea { title: "\t".into() }),
            "пустое название",
        ),
        (heading("Кухня", " "), "пустое название проекта"),
        (heading(" ", "Ремонт"), "пустое название"),
    ] {
        rejected(&mut s, c, message)?;
    }
    Ok(())
}

/// Повтор в проект из корзины – конфликт; проект вернули – повтор проходит.
#[test]
fn redo_waits_until_the_project_is_back_from_trash() -> Result {
    let mut s = Store::open_in_memory()?;
    run(&mut s, project("Дача"))?;
    let p = s.read().projects()?[0].id;
    run(&mut s, add_in("Полить", "Дача", None))?;
    run(&mut s, Command::Undo)?;
    s.write(|w| w.delete(RecordId::Project(p), clock().now))?;
    conflict(
        &mut s,
        Command::Redo,
        "нельзя повторить добавление задачи «Полить»: проект «Дача» в корзине",
    )?;
    s.write(|w| w.restore(RecordId::Project(p), clock().now))?;
    run(&mut s, Command::Redo)?;
    assert_eq!(task(&s, "Полить")?.project, Some(p));
    // Закрытый проект – тоже конфликт.
    run(&mut s, Command::Undo)?;
    let mut closed = s.read().project(p)?.ok_or("нет проекта")?;
    closed.status = Status::Completed { at: clock().now };
    s.write(|w| w.update_project(&closed))?;
    conflict(
        &mut s,
        Command::Redo,
        "нельзя повторить добавление задачи «Полить»: проект «Дача» закрыт",
    )
}

/// Раздел перенесли в другой проект – отмена переноса задачи в него не
/// вернёт; раздел вернули – отмена проходит.
#[test]
fn undo_waits_until_the_heading_is_back_in_its_project() -> Result {
    let mut s = Store::open_in_memory()?;
    run(&mut s, project("Ремонт"))?;
    run(&mut s, project("Дача"))?;
    run(&mut s, heading("Кухня", "Ремонт"))?;
    run(&mut s, add_in("Купить плитку", "Ремонт", Some("Кухня")))?;
    run(&mut s, moved("плитку", |m| m.project = Some("Дача".into())))?;
    let snap = s.read().snapshot()?;
    let project_of = |title: &str| {
        snap.projects
            .iter()
            .find(|p| p.title == title)
            .map(|p| p.id)
            .ok_or("нет проекта")
    };
    let (repair, dacha) = (project_of("Ремонт")?, project_of("Дача")?);
    let mut h = snap.headings[0].clone();
    h.project = dacha;
    h.sync.touch(at("2026-11-19T13:00:00Z"));
    s.write(|w| w.update_heading(&h))?;
    conflict(
        &mut s,
        Command::Undo,
        "нельзя отменить перенос задачи «Купить плитку»: раздел «Кухня» теперь в проекте «Дача»",
    )?;
    h.project = repair;
    h.sync.touch(at("2026-11-19T14:00:00Z"));
    s.write(|w| w.update_heading(&h))?;
    run(&mut s, Command::Undo)?;
    let t = task(&s, "Купить плитку")?;
    assert_eq!((t.project, t.heading), (Some(repair), Some(h.id)));
    // Раздел в корзине – повтор переноса возможен (он уводит из раздела),
    // а отмена обратно в раздел – нет.
    run(&mut s, Command::Redo)?;
    s.write(|w| w.delete(RecordId::Heading(h.id), at("2026-11-19T15:00:00Z")))?;
    conflict(
        &mut s,
        Command::Undo,
        "нельзя отменить перенос задачи «Купить плитку»: раздел «Кухня» в корзине",
    )
}

#[test]
fn area_and_tag_links_are_checked_too() -> Result {
    let mut s = Store::open_in_memory()?;
    run(
        &mut s,
        Command::AddArea(NewArea {
            title: "Дом".into(),
        }),
    )?;
    let area = s.read().areas()?[0].id;
    run(
        &mut s,
        Command::AddTask(NewTask {
            text: "Полить".into(),
            area: Some("Дом".into()),
            ..NewTask::default()
        }),
    )?;
    run(&mut s, moved("Полить", |m| m.no_project = true))?;
    s.write(|w| w.delete(RecordId::Area(area), clock().now))?;
    conflict(
        &mut s,
        Command::Undo,
        "нельзя отменить перенос задачи «Полить»: область «Дом» в корзине",
    )?;
    s.write(|w| w.restore(RecordId::Area(area), clock().now))?;
    run(&mut s, Command::Undo)?;

    // Тег был до действия; повтор добавления задачи с ним, пока тег в
    // корзине, – конфликт.
    run(
        &mut s,
        Command::AddTask(NewTask {
            text: "Счёт".into(),
            tags: vec!["Срочно".into()],
            ..NewTask::default()
        }),
    )?;
    run(
        &mut s,
        Command::AddTask(NewTask {
            text: "Письмо".into(),
            tags: vec!["срочно".into()],
            ..NewTask::default()
        }),
    )?;
    run(&mut s, Command::Undo)?;
    let tag = s.read().tags()?[0].id;
    s.write(|w| w.delete(RecordId::Tag(tag), clock().now))?;
    conflict(
        &mut s,
        Command::Redo,
        "нельзя повторить добавление задачи «Письмо»: тег «Срочно» в корзине",
    )?;
    s.write(|w| w.restore(RecordId::Tag(tag), clock().now))?;
    run(&mut s, Command::Redo)?;
    assert_eq!(task(&s, "Письмо")?.tags, [tag]);
    Ok(())
}

/// Связь, которую шаг не меняет, не мешает: выполненная задача в
/// закрытом потом проекте открывается отменой.
#[test]
fn unchanged_links_are_not_rechecked() -> Result {
    let mut s = Store::open_in_memory()?;
    run(&mut s, project("Ремонт"))?;
    run(&mut s, add_in("Плитка", "Ремонт", None))?;
    run(&mut s, close("Плитка"))?;
    let mut p = s.read().projects()?[0].clone();
    p.status = Status::Completed { at: clock().now };
    s.write(|w| w.update_project(&p))?;
    run(&mut s, Command::Undo)?;
    assert_eq!(task(&s, "Плитка")?.status, Status::Open);
    Ok(())
}

/// Тег, созданный добавлением, уже нужен другой записи – отмена добавления
/// целиком конфликт; прежний тег отмена не трогает.
#[test]
fn undo_of_add_keeps_old_tags_and_refuses_to_drop_a_used_new_one() -> Result {
    let mut s = Store::open_in_memory()?;
    run(
        &mut s,
        Command::AddTask(NewTask {
            text: "Счёт".into(),
            tags: vec!["Дом".into()],
            ..NewTask::default()
        }),
    )?;
    run(
        &mut s,
        Command::AddTask(NewTask {
            text: "Письмо".into(),
            tags: vec!["дом".into(), "Работа".into()],
            ..NewTask::default()
        }),
    )?;
    let work = s
        .read()
        .tags()?
        .into_iter()
        .find(|t| t.title == "Работа")
        .ok_or("нет тега")?
        .id;
    let mut other = Task::new(TaskId::from_u128(5), "Чужая", Rank::initial(), clock().now);
    other.tags = vec![work];
    s.write(|w| w.insert_task(&other))?;
    conflict(
        &mut s,
        Command::Undo,
        &format!(
            "нельзя отменить добавление задачи «Письмо»: на тег «Работа» ссылается задача [{}] Чужая",
            short(other.id.0)
        ),
    )?;
    s.write(|w| {
        w.delete(RecordId::Task(other.id), clock().now)?;
        w.purge(RecordId::Task(other.id))
    })?;
    run(&mut s, Command::Undo)?;
    let tags: Vec<String> = s.read().tags()?.into_iter().map(|t| t.title).collect();
    assert_eq!(tags, ["Дом"]);
    Ok(())
}

/// Чек-лист: метку пункта трогать можно, текст и отметку – нет.
#[test]
fn checklist_content_still_conflicts() -> Result {
    let mut s = Store::open_in_memory()?;
    let id = TaskId::from_u128(1);
    let mut t = Task::new(id, "Сборы", Rank::initial(), clock().now);
    t.checklist.push(ChecklistItem::new(
        ChecklistItemId::from_u128(2),
        "Паспорт",
        Rank::initial(),
        clock().now,
    ));
    s.write(|w| w.insert_task(&t))?;
    run(&mut s, close("Сборы"))?;
    let label = format!("[{}] Сборы", short(id.0));
    for change in [
        (|t: &mut Task| t.checklist[0].title = "Билеты".into()) as fn(&mut Task),
        |t: &mut Task| t.checklist[0].done = true,
    ] {
        let original = s.read().task(id)?.ok_or("нет задачи")?;
        let mut edited = original.clone();
        change(&mut edited);
        s.write(|w| w.update_task(&edited))?;
        conflict(
            &mut s,
            Command::Undo,
            &format!(
                "нельзя отменить выполнение задачи «Сборы»: задачу {label} изменили после этого действия"
            ),
        )?;
        s.write(|w| w.update_task(&original))?;
    }
    run(&mut s, Command::Undo)?;
    assert!(task(&s, "Сборы")?.status.is_open());
    Ok(())
}

/// На `Timestamp::MAX` метке расти некуда: команда, отмена и повтор
/// отказываются и ничего не пишут.
#[test]
fn exhausted_stamps_are_errors() -> Result {
    let mut s = Store::open_in_memory()?;
    let mut t = Task::new(TaskId::from_u128(1), "Отчёт", Rank::initial(), clock().now);
    t.sync.touch(Timestamp::MAX);
    s.write(|w| w.insert_task(&t))?;
    let message = format!(
        "время правки записи достигло предела ({}) – отметить правку позже нельзя",
        Timestamp::MAX
    );
    for c in [
        moved("Отчёт", |m| m.when = Some("завтра".into())),
        close("Отчёт"),
    ] {
        let before = state(&s)?;
        let r = run(&mut s, c);
        assert!(matches!(r, Err(Error::NoLaterTime(m)) if m == Timestamp::MAX));
        assert_eq!(state(&s)?, before);
    }
    assert_eq!(
        run(&mut s, close("Отчёт")).map_err(|e| e.to_string()),
        Err(message.clone())
    );

    // Действие записано раньше, потом метку довели до предела в обход.
    run(&mut s, add("Письмо"))?;
    let mut letter = task(&s, "Письмо")?;
    letter.sync.touch(Timestamp::MAX);
    s.write(|w| w.update_task(&letter))?;
    let before = state(&s)?;
    assert_eq!(
        run(&mut s, Command::Undo).map_err(|e| e.to_string()),
        Err(message)
    );
    assert_eq!(state(&s)?, before);
    Ok(())
}
