//! Повторы и история при обновлении базы.

use jiff::{
    Timestamp,
    civil::{Date, date},
};
use serde_json::json;
use tally_app::{
    Clock, Command, Done, EditTask, Effect, NewProject, NewTask, Repeat, execute, snapshot_for_day,
};
use tally_core::{Project, RepeatFrom, RepeatRule, RepeatUnit, Start, Status, Task};
use tally_store::Store;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn clock(day: i8) -> Clock {
    Clock {
        today: date(2027, 1, day),
        now: Timestamp::UNIX_EPOCH,
    }
}

fn run(store: &mut Store, command: Command) -> Result<Done> {
    Ok(execute(store, &command, clock(1))?)
}

fn rule(unit: RepeatUnit, from: RepeatFrom) -> Repeat {
    Repeat {
        rule: RepeatRule { interval: 1, unit },
        from,
    }
}

fn add_task(store: &mut Store, new: NewTask) -> Result<Task> {
    let done = run(store, Command::AddTask(new))?;
    let Effect::Added(id) = done.effect else {
        return Err("ожидалась новая задача".into());
    };
    Ok(store.read().task(id)?.ok_or("нет новой задачи")?)
}

fn add_project(store: &mut Store, repeat: Repeat) -> Result<Project> {
    let created = run(
        store,
        Command::AddProject(NewProject {
            title: "Неделя".into(),
            area: None,
        }),
    )?;
    let first = &created.snapshot.projects[0];
    let configured = run(
        store,
        Command::SetProjectRepeat {
            project: first.id,
            expected_updated_at: first.sync.updated_at,
            repeat: Some(repeat),
        },
    )?;
    Ok(configured
        .snapshot
        .projects
        .into_iter()
        .next()
        .ok_or("нет проекта")?)
}

fn configure_task(
    store: &mut Store,
    task: &Task,
    repeat: Repeat,
    deadline: Option<Date>,
) -> Result {
    run(
        store,
        Command::EditTaskRepeat {
            edit: EditTask {
                task: task.id,
                expected_updated_at: task.sync.updated_at,
                title: task.title.clone(),
                notes: task.notes.clone(),
                when: None,
                deadline,
                checklist: Vec::new(),
                tags: Vec::new(),
            },
            repeat: Some(repeat),
        },
    )?;
    Ok(())
}

#[test]
fn copied_project_uses_child_series_start_independent_of_earlier_loads() -> Result {
    for opened_previous_day in [false, true] {
        let mut store = Store::open_in_memory()?;
        let project = add_project(&mut store, rule(RepeatUnit::Week, RepeatFrom::Date))?;
        let task = add_task(
            &mut store,
            NewTask {
                text: "Сверить выписку".into(),
                when: Some("сегодня".into()),
                project: Some(project.title),
                ..NewTask::default()
            },
        )?;
        configure_task(
            &mut store,
            &task,
            rule(RepeatUnit::Day, RepeatFrom::Date),
            Some(clock(2).today),
        )?;
        if opened_previous_day {
            snapshot_for_day(&mut store, clock(7))?;
        }
        let snapshot = snapshot_for_day(&mut store, clock(8))?;
        let new = snapshot
            .projects
            .iter()
            .find(|p| p.id != project.id)
            .ok_or("нет копии проекта")?;
        let mut children = snapshot.tasks.iter().filter(|t| t.project == Some(new.id));
        let child = children.next().ok_or("нет копии задачи")?;
        assert_eq!(new.date, Some(clock(8).today));
        assert_eq!(
            child.date,
            Some(clock(8).today),
            "открывали накануне: {opened_previous_day}"
        );
        assert_eq!(child.deadline, Some(clock(9).today));
        assert_eq!(child.repeat_anchor, child.date);
        assert!(children.next().is_none());
        assert_eq!(snapshot_for_day(&mut store, clock(8))?, snapshot);
    }
    Ok(())
}

#[test]
fn task_completion_is_undoable_when_the_next_deadline_is_outside_the_calendar() -> Result {
    let mut store = Store::open_in_memory()?;
    let task = add_task(
        &mut store,
        NewTask {
            text: "Крайний срок".into(),
            ..NewTask::default()
        },
    )?;
    let repeat = rule(RepeatUnit::Day, RepeatFrom::Completion);
    configure_task(&mut store, &task, repeat, Some(Date::MAX))?;
    let closed = run(&mut store, Command::CloseTask(task.id))?;
    assert_eq!(closed.snapshot.tasks.len(), 1);
    assert!(matches!(
        closed.snapshot.tasks[0].status,
        Status::Completed { .. }
    ));
    assert_eq!(closed.snapshot.tasks[0].deadline, Some(Date::MAX));
    let undone = run(
        &mut store,
        Command::UndoStep(closed.seq.ok_or("нет шага закрытия")?),
    )?;
    assert!(undone.snapshot.tasks[0].status.is_open());
    assert_eq!(undone.snapshot.tasks[0].repeat_rule, Some(repeat.rule));
    let redone = run(&mut store, Command::Redo)?;
    assert_eq!(redone.snapshot.tasks.len(), 1);
    assert_eq!(
        redone.snapshot.tasks[0].status,
        closed.snapshot.tasks[0].status
    );
    Ok(())
}

#[test]
fn project_completion_is_undoable_when_the_next_deadline_is_outside_the_calendar() -> Result {
    let mut store = Store::open_in_memory()?;
    let mut project = add_project(&mut store, rule(RepeatUnit::Day, RepeatFrom::Completion))?;
    project.deadline = Some(Date::MAX);
    store.write(|w| w.update_project(&project))?;
    let child = add_task(
        &mut store,
        NewTask {
            text: "Собрать данные".into(),
            project: Some(project.title.clone()),
            ..NewTask::default()
        },
    )?;
    let closed = run(
        &mut store,
        Command::CloseProject {
            project: project.id,
            expected_updated_at: project.sync.updated_at,
            cancel: false,
        },
    )?;
    assert_eq!(closed.snapshot.projects.len(), 1);
    assert_eq!(closed.snapshot.tasks, [child]);
    assert!(matches!(
        closed.snapshot.projects[0].status,
        Status::Completed { .. }
    ));
    let undone = run(
        &mut store,
        Command::UndoStep(closed.seq.ok_or("нет шага закрытия")?),
    )?;
    assert!(undone.snapshot.projects[0].status.is_open());
    assert_eq!(undone.snapshot.projects[0].repeat_rule, project.repeat_rule);
    let redone = run(&mut store, Command::Redo)?;
    assert_eq!(redone.snapshot.projects.len(), 1);
    assert_eq!(
        redone.snapshot.projects[0].status,
        closed.snapshot.projects[0].status
    );
    Ok(())
}

#[test]
fn an_old_history_step_can_undo_and_redo_clearing_the_start_after_upgrade() -> Result {
    let mut store = Store::open_in_memory()?;
    let mut before = add_task(
        &mut store,
        NewTask {
            text: "Старая задача".into(),
            when: Some("сегодня".into()),
            ..NewTask::default()
        },
    )?;
    before.repeat_rule = Some(rule(RepeatUnit::Day, RepeatFrom::Date).rule);
    before.repeat_anchor = before.date;
    let mut old_after = before.clone();
    old_after.date = None;
    old_after.repeat_anchor = None;
    old_after.start = Start::Anytime;
    let history = json!([{"before": {"Task": before}, "after": {"Task": old_after}, "stamp": before.sync.updated_at}]);
    let mut migrated = old_after.clone();
    migrated.repeat_rule = None;
    let seq = store.write(|w| {
        w.update_task(&migrated)?;
        w.push_history(clock(1).now, "снятие даты", &history.to_string(), 100)
    })?;
    let undone = run(&mut store, Command::UndoStep(seq))?;
    assert_eq!(undone.snapshot.tasks[0].date, before.date);
    assert_eq!(undone.snapshot.tasks[0].repeat_rule, before.repeat_rule);
    let redone = run(&mut store, Command::Redo)?;
    assert_eq!(redone.snapshot.tasks[0].date, None);
    assert_eq!(redone.snapshot.tasks[0].repeat_rule, None);
    assert_eq!(redone.snapshot.tasks[0].start, Start::Anytime);
    Ok(())
}
