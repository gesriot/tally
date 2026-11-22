use jiff::{Timestamp, civil::date};
use tally_app::{
    Clock, Command, Done, DropAnchor, DropPosition, DropTask, Error, NewArea, NewProject, NewTask,
    TaskLane, execute,
};
use tally_core::{Rank, Start, Task, lists::Item};
use tally_store::Store;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const CLOCK: Clock = Clock {
    today: date(2026, 11, 20),
    now: Timestamp::UNIX_EPOCH,
};
const DAY: TaskLane = TaskLane::Today { evening: false };
const EVENING: TaskLane = TaskLane::Today { evening: true };

fn run(store: &mut Store, cmd: Command) -> Result<Done> {
    Ok(execute(store, &cmd, CLOCK)?)
}

fn add(store: &mut Store, text: &str, when: Option<&str>, deadline: Option<&str>) -> Result<Task> {
    let done = run(
        store,
        Command::AddTask(NewTask {
            text: text.into(),
            when: when.map(Into::into),
            deadline: deadline.map(Into::into),
            ..NewTask::default()
        }),
    )?;
    Ok(done
        .snapshot
        .tasks
        .into_iter()
        .find(|task| task.title == text)
        .ok_or("нет задачи")?)
}

fn drop_task(
    task: &Task,
    source: TaskLane,
    destination: TaskLane,
    position: DropPosition,
) -> Command {
    Command::DropTask(DropTask {
        task: task.id,
        expected_updated_at: task.sync.updated_at,
        source: Some(source),
        destination,
        position,
    })
}

fn titles<'a>(items: &[Item<'a>]) -> Vec<&'a str> {
    items.iter().map(Item::title).collect()
}

#[test]
fn today_uses_its_own_rank_and_resolves_ties_with_project_rows() -> Result {
    let mut store = Store::open_in_memory()?;
    let first = add(&mut store, "Первая", Some("сегодня"), None)?;
    let moved = add(&mut store, "Переносимая", Some("сегодня"), None)?;
    let added = run(
        &mut store,
        Command::AddProject(NewProject {
            title: "Проект".into(),
            area: None,
        }),
    )?;
    let mut project = added
        .snapshot
        .projects
        .into_iter()
        .next()
        .ok_or("нет проекта")?;
    project.date = Some(CLOCK.today);
    project.today_index = Rank::initial();
    assert_eq!(first.today_index, project.today_index);
    store.write(|w| w.update_project(&project))?;

    let before = store.read().snapshot()?;
    assert_eq!(
        titles(&before.lists(CLOCK.today).today.day),
        ["Первая", "Проект", "Переносимая"]
    );
    let cmd = drop_task(
        &moved,
        DAY,
        DAY,
        DropPosition::Before(DropAnchor::Project(project.id)),
    );
    let result = run(&mut store, cmd.clone())?;
    let step = result.seq.ok_or("нет шага переноса")?;
    assert_eq!(
        titles(&result.snapshot.lists(CLOCK.today).today.day),
        ["Первая", "Переносимая", "Проект"]
    );
    let actual = store.read().task(moved.id)?.ok_or("нет задачи")?;
    assert_eq!(actual.index, moved.index);
    assert_eq!(actual.date, moved.date);
    assert_eq!(result.snapshot.projects[0].index, project.index);
    assert_eq!(store.read().task(first.id)?.as_ref(), Some(&first));
    assert!(matches!(
        execute(&mut store, &cmd, CLOCK),
        Err(Error::Conflict(_))
    ));
    assert_eq!(store.read().snapshot()?, result.snapshot);
    let undone = run(&mut store, Command::Undo)?;
    assert_eq!(undone.seq, Some(step));
    assert_eq!(
        titles(&undone.snapshot.lists(CLOCK.today).today.day),
        ["Первая", "Проект", "Переносимая"]
    );
    let redone = run(&mut store, Command::Redo)?;
    assert_eq!(redone.seq, Some(step));
    assert_eq!(
        titles(&redone.snapshot.lists(CLOCK.today).today.day),
        ["Первая", "Переносимая", "Проект"]
    );
    Ok(())
}

#[test]
fn project_section_and_area_moves_keep_the_schedule_and_use_exact_anchors() -> Result {
    let mut store = Store::open_in_memory()?;
    let added = run(
        &mut store,
        Command::AddArea(NewArea {
            title: "Дом".into(),
        }),
    )?;
    let area = added.snapshot.areas[0].id;
    let added = run(
        &mut store,
        Command::AddProject(NewProject {
            title: "Переезд".into(),
            area: None,
        }),
    )?;
    let project = added.snapshot.projects[0].id;
    let mut headings = Vec::new();
    for title in ["Упаковать", "Отвезти"] {
        let done = run(
            &mut store,
            Command::AddProjectSection {
                title: title.into(),
                project,
            },
        )?;
        headings.push(
            done.snapshot
                .headings
                .into_iter()
                .find(|h| h.title == title)
                .ok_or("нет раздела")?,
        );
    }
    let from = TaskLane::Project(project, Some(headings[0].id));
    let to = TaskLane::Project(project, Some(headings[1].id));
    let first = add(&mut store, "Коробки", Some("сегодня вечером"), Some("пн"))?;
    let anchor = add(&mut store, "Книги", Some("завтра"), None)?;
    run(
        &mut store,
        drop_task(&first, EVENING, from, DropPosition::End),
    )?;
    let anchor = run(
        &mut store,
        Command::DropTask(DropTask {
            task: anchor.id,
            expected_updated_at: anchor.sync.updated_at,
            source: None,
            destination: to,
            position: DropPosition::End,
        }),
    )?
    .snapshot
    .tasks
    .into_iter()
    .find(|t| t.id == anchor.id)
    .ok_or("нет задачи")?;
    let original = store.read().task(first.id)?.ok_or("нет задачи")?;
    let result = run(
        &mut store,
        drop_task(
            &original,
            from,
            to,
            DropPosition::Before(DropAnchor::Task(anchor.id)),
        ),
    )?;
    let moved = store.read().task(first.id)?.ok_or("нет задачи")?;
    assert_eq!(moved.heading, Some(headings[1].id));
    assert!(moved.index < anchor.index);
    assert_eq!(
        (
            moved.date,
            moved.deadline,
            moved.evening,
            &moved.today_index
        ),
        (
            first.date,
            first.deadline,
            first.evening,
            &first.today_index
        )
    );
    let undone = run(&mut store, Command::Undo)?;
    assert_eq!(undone.seq, result.seq);
    assert_eq!(
        store.read().task(first.id)?.ok_or("нет задачи")?.heading,
        Some(headings[0].id)
    );
    run(&mut store, Command::Redo)?;
    let current = store.read().task(first.id)?.ok_or("нет задачи")?;
    let valid = drop_task(&current, to, TaskLane::Area(area), DropPosition::End);
    run(&mut store, valid)?;
    let moved = store.read().task(first.id)?.ok_or("нет задачи")?;
    assert_eq!(
        (moved.project, moved.heading, moved.area),
        (None, None, Some(area))
    );
    assert_eq!(moved.deadline, first.deadline);

    let mut archived = headings.remove(0);
    archived.archived = true;
    store.write(|w| w.update_heading(&archived))?;
    let state = store.read().snapshot()?;
    let history = store.read().history()?;
    let cmd = drop_task(&moved, TaskLane::Area(area), from, DropPosition::End);
    assert!(matches!(
        execute(&mut store, &cmd, CLOCK),
        Err(Error::Conflict(_))
    ));
    let cmd = drop_task(
        &moved,
        TaskLane::Area(area),
        TaskLane::Area(area),
        DropPosition::Before(DropAnchor::Task(anchor.id)),
    );
    assert!(matches!(
        execute(&mut store, &cmd, CLOCK),
        Err(Error::Conflict(_))
    ));
    assert_eq!(store.read().snapshot()?, state);
    assert_eq!(store.read().history()?, history);
    Ok(())
}

#[test]
fn schedule_transitions_respect_deadlines_and_no_op_does_not_write() -> Result {
    let mut store = Store::open_in_memory()?;
    let task = add(&mut store, "Позвонить", None, None)?;
    let start = run(
        &mut store,
        drop_task(&task, TaskLane::Inbox, EVENING, DropPosition::End),
    )?;
    assert_eq!(
        titles(&start.snapshot.lists(CLOCK.today).today.evening),
        ["Позвонить"]
    );
    let current = store.read().task(task.id)?.ok_or("нет задачи")?;
    assert_eq!(current.index, task.index);
    let day = run(
        &mut store,
        drop_task(&current, EVENING, DAY, DropPosition::End),
    )?;
    assert_eq!(
        titles(&day.snapshot.lists(CLOCK.today).today.day),
        ["Позвонить"]
    );
    let current = store.read().task(task.id)?.ok_or("нет задачи")?;
    let history = store.read().history()?;
    let no_op = run(&mut store, drop_task(&current, DAY, DAY, DropPosition::End))?;
    assert_eq!(no_op.seq, None);
    assert_eq!(store.read().history()?, history);
    run(
        &mut store,
        drop_task(&current, DAY, TaskLane::Anytime, DropPosition::End),
    )?;
    let current = store.read().task(task.id)?.ok_or("нет задачи")?;
    assert_eq!(
        (current.date, current.evening, current.start),
        (None, false, Start::Anytime)
    );
    assert_eq!(current.today_index, task.today_index);

    let due = add(&mut store, "Срок", None, Some("сегодня"))?;
    let history = store.read().history()?;
    let cmd = drop_task(&due, DAY, TaskLane::Someday, DropPosition::End);
    assert!(matches!(
        execute(&mut store, &cmd, CLOCK),
        Err(Error::Rejected(_))
    ));
    assert_eq!(store.read().task(due.id)?.as_ref(), Some(&due));
    assert_eq!(store.read().history()?, history);
    Ok(())
}
