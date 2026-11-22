//! Правила перетаскивания: порядок «Сегодня», назначение даты, срок, линия
//! вставки в «Когда-нибудь».

mod common;
#[path = "common/projects.rs"]
mod projects;

use common::{Result, TODAY, clock, find, run};
use i_slint_backend_testing::ElementHandle;
use projects::{open_window, project};
use slint::SharedString;
use slint::platform::Key;
use slint::{ComponentHandle, LogicalSize, Model};
use tally_app::{
    Command, DropAnchor, DropPosition, DropTask, MoveTask, NewTask, TaskLane, execute,
};
use tally_core::{Start, Task};
use tally_store::Store;
use tally_ui::ui::{DropSurface, Theme};

const DAY: TaskLane = TaskLane::Today { evening: false };

fn add(
    store: &mut Store,
    title: &str,
    project: Option<&str>,
    when: Option<&str>,
    deadline: Option<&str>,
) -> Result<Task> {
    let result = execute(
        store,
        &Command::AddTask(NewTask {
            text: title.into(),
            project: project.map(Into::into),
            when: when.map(Into::into),
            deadline: deadline.map(Into::into),
            ..Default::default()
        }),
        clock(),
    )?;
    Ok(find(&result.snapshot, title)?.clone())
}

fn drop_at(task: &Task, from: TaskLane, to: TaskLane, position: DropPosition) -> Command {
    Command::DropTask(DropTask {
        task: task.id,
        expected_updated_at: task.sync.updated_at,
        source: Some(from),
        destination: to,
        position,
    })
}

fn daytime(store: &Store) -> Result<Vec<String>> {
    let snapshot = store.read().snapshot()?;
    Ok(snapshot
        .lists(TODAY)
        .today
        .day
        .iter()
        .map(|item| item.title().to_owned())
        .collect())
}

#[test]
fn sorting_a_project_keeps_the_visible_today_order_after_merging_day_and_evening() -> Result {
    let mut store = Store::open_in_memory()?;
    let lane = TaskLane::Project(project(&mut store, "Работа")?, None);
    let a = add(
        &mut store,
        "А",
        Some("Работа"),
        Some("сегодня вечером"),
        None,
    )?;
    let b = add(&mut store, "Б", Some("Работа"), Some("в любое время"), None)?;
    // Дневной блок пока пуст. Вечерняя задача уже имеет собственный ключ.
    run(&mut store, drop_at(&b, lane, DAY, DropPosition::End))?;
    let schedule_a = MoveTask {
        task: a.id.to_string(),
        when: Some("сегодня".into()),
        ..Default::default()
    };
    run(&mut store, Command::MoveTask(schedule_a))?;
    let before = daytime(&store)?;
    assert_eq!(before.len(), 2);
    let b = store.read().task(b.id)?.ok_or("нет Б")?;
    let position = DropPosition::Before(DropAnchor::Task(a.id));
    run(&mut store, drop_at(&b, lane, lane, position))?;
    let b = store.read().task(b.id)?.ok_or("нет Б")?;
    let a = store.read().task(a.id)?.ok_or("нет А")?;
    assert!(b.index < a.index);
    assert_eq!(daytime(&store)?, before);
    Ok(())
}

#[test]
fn scheduling_an_inbox_task_by_drag_sets_the_same_fields_as_move_when() -> Result {
    let mut store = Store::open_in_memory()?;
    let mouse = add(&mut store, "Мышь", None, None, None)?;
    let cli = add(&mut store, "Консоль", None, None, None)?;
    let schedule_mouse = drop_at(&mouse, TaskLane::Inbox, DAY, DropPosition::End);
    run(&mut store, schedule_mouse)?;
    let schedule_cli = MoveTask {
        task: cli.id.to_string(),
        when: Some("сегодня".into()),
        ..Default::default()
    };
    run(&mut store, Command::MoveTask(schedule_cli))?;
    let mouse = store.read().task(mouse.id)?.ok_or("нет задачи")?;
    let cli = store.read().task(cli.id)?.ok_or("нет задачи")?;
    assert_eq!(
        (mouse.start, mouse.date, mouse.evening),
        (cli.start, cli.date, cli.evening)
    );
    Ok(())
}

#[test]
fn explicit_drop_on_today_keeps_the_task_there_when_its_deadline_is_postponed() -> Result {
    let mut store = Store::open_in_memory()?;
    let lane = TaskLane::Project(project(&mut store, "Работа")?, None);
    let task = add(
        &mut store,
        "Отчёт",
        Some("Работа"),
        Some("завтра"),
        Some("сегодня"),
    )?;
    run(&mut store, drop_at(&task, lane, DAY, DropPosition::End))?;
    let shift_deadline = MoveTask {
        task: task.id.to_string(),
        deadline: Some("через неделю".into()),
        ..Default::default()
    };
    run(&mut store, Command::MoveTask(shift_deadline))?;
    let saved = store.read().task(task.id)?.ok_or("нет задачи")?;
    assert_eq!(saved.date, Some(TODAY));
    assert_eq!(daytime(&store)?, ["Отчёт"]);
    Ok(())
}

#[test]
fn a_line_after_the_last_someday_project_agrees_with_the_actual_drop() -> Result {
    i_slint_backend_testing::init_integration_test_with_mock_time();
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let mut store = Store::open(&path)?;
    let id = project(&mut store, "Проект")?;
    let mut p = store.read().project(id)?.ok_or("нет проекта")?;
    p.start = Start::Someday;
    store.write(|w| w.update_project(&p))?;
    add(&mut store, "Первая", None, Some("когда-нибудь"), None)?;
    add(&mut store, "Вторая", None, Some("когда-нибудь"), None)?;
    let (window, _bound) = open_window(&path, std::convert::identity)?;
    window.window().set_size(LogicalSize::new(1000.0, 760.0));
    window.invoke_select("someday".into());
    let rows = window.get_rows();
    let titles: Vec<_> = rows.iter().map(|r| r.title.to_string()).collect();
    assert_eq!(titles, ["Первая", "Вторая", "Проект"]);
    let theme = window.global::<Theme<'_>>();
    let height: f32 = rows
        .iter()
        .map(|r| {
            if r.meta.is_empty() {
                theme.get_row_height()
            } else {
                theme.get_row_height_meta()
            }
        })
        .sum();
    let blank_space = height + 12.0;
    assert!(window.invoke_drag_begin(0));
    window.invoke_drag_hover(DropSurface::List, blank_space);
    assert!((window.get_drag_line() - (height - theme.get_row_height_meta())).abs() < 0.5);
    window.invoke_drag_end(DropSurface::List, blank_space);
    let titles: Vec<_> = window
        .get_rows()
        .iter()
        .map(|r| r.title.to_string())
        .collect();
    assert_eq!(titles, ["Вторая", "Первая", "Проект"]);

    let scheduled = add(&mut store, "Первая вечерняя", None, Some("сегодня"), None)?;
    assert!(window.invoke_key(SharedString::from(Key::F5), false, false, false, false));
    window.invoke_select("today".into());
    assert_eq!(window.get_rows().row_count(), 1);
    assert!(
        ElementHandle::find_by_accessible_label(&window, "Вечер")
            .next()
            .is_none()
    );
    assert!(window.invoke_drag_begin(0));
    assert!(
        ElementHandle::find_by_accessible_label(&window, "Вечер")
            .next()
            .is_some()
    );
    let boundary = window.get_task_extent() + theme.get_section_height();
    window.invoke_drag_hover(DropSurface::List, boundary + 10.0);
    assert!((window.get_drag_line() - boundary).abs() < 0.5);
    window.invoke_drag_end(DropSurface::List, boundary + 10.0);
    let snapshot = store.read().snapshot()?;
    let evening = snapshot.lists(TODAY).today.evening;
    assert_eq!(
        evening.iter().map(|item| item.title()).collect::<Vec<_>>(),
        ["Первая вечерняя"]
    );
    let saved = store.read().task(scheduled.id)?.ok_or("нет задачи")?;
    assert!(saved.evening);
    assert_eq!(saved.date, Some(TODAY));
    Ok(())
}
