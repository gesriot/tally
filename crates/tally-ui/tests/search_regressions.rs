//! Быстрый поиск и фильтр тегов: регрессии.

mod common;
#[path = "common/projects.rs"]
mod projects;

use std::{convert::identity, time::Duration};

use common::{Result, clock, run, task};
use i_slint_backend_testing::{init_integration_test_with_mock_time, mock_elapsed_time};
use projects::{open_window, project};
use slint::{Model, platform::Key};
use tally_app::{Command, NewArea};
use tally_core::{Rank, Tag, TagId};
use tally_store::Store;
use tally_ui::ui::{AppWindow, RowKind};

fn press(window: &AppWindow, key: Key) {
    assert!(window.invoke_key(key.into(), false, false, false, false));
}

fn search(window: &AppWindow, query: &str) {
    window.invoke_search_begin();
    window.invoke_search_query(query.into());
    mock_elapsed_time(Duration::from_millis(150));
}

fn filter(window: &AppWindow, name: &str) -> Result {
    let index = window
        .get_tag_filter_items()
        .iter()
        .position(|label| label.as_str() == name)
        .ok_or("нет тега в фильтре")?;
    window.invoke_tag_filter(i32::try_from(index)?);
    Ok(())
}

fn tag(id: u128, title: &str) -> Tag {
    Tag::new(TagId::from_u128(id), title, Rank::initial(), clock().now)
}

fn refresh_preserves_the_selected_search_result() -> Result {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let mut console = Store::open(&path)?;
    for title in ["Письмо Б", "Письмо В"] {
        run(&mut console, Command::AddTask(task(title)))?;
    }
    let (window, _bound) = open_window(&path, identity)?;
    search(&window, "письмо");
    press(&window, Key::DownArrow);
    let selected = window
        .get_rows()
        .iter()
        .find(|r| r.title == "Письмо В")
        .ok_or("нет выбранной задачи")?;
    assert_eq!(window.get_selected_result(), selected.row_key);

    run(&mut console, Command::AddTask(task("Письмо А")))?;
    press(&window, Key::F5);
    assert_eq!(window.get_rows().row_count(), 3);
    press(&window, Key::Return);
    assert_eq!(window.get_edit_task_id(), selected.key);
    Ok(())
}

fn escaping_an_empty_search_preserves_the_checkmark_in_today() -> Result {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let mut console = Store::open(&path)?;
    let mut entry = task("Позвонить");
    entry.when = Some("сегодня".into());
    run(&mut console, Command::AddTask(entry))?;
    let id = common::id_of(&console.read().snapshot()?, "Позвонить")?;
    let key = id.to_string();
    let (window, _bound) = open_window(&path, identity)?;
    window.invoke_toggle(key.as_str().into());
    let kept = || {
        window
            .get_rows()
            .iter()
            .any(|row| row.key.as_str() == key.as_str() && row.checkable)
    };
    assert!(kept());
    assert!(window.invoke_key("f".into(), true, false, false, false));
    assert!(window.get_search_active());
    press(&window, Key::Escape);
    assert_eq!(window.get_page_title(), "Сегодня");
    assert!(kept(), "выполненная строка исчезла после отмены поиска");
    window.invoke_toggle(key.as_str().into());
    let reopened = console.read().task(id)?.ok_or("нет задачи")?;
    assert!(reopened.status.is_open());
    Ok(())
}

fn tagged_closed_project_is_visible_from_the_tag_as_well_as_the_logbook() -> Result {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let mut console = Store::open(&path)?;
    let id = project(&mut console, "Ремонт")?;
    let mut record = console.read().project(id)?.ok_or("нет проекта")?;
    let label = tag(1, "Бюджет");
    record.tags.push(label.id);
    console.write(|w| {
        w.insert_tag(&label)?;
        w.update_project(&record)
    })?;
    run(
        &mut console,
        Command::CloseProject {
            project: id,
            expected_updated_at: record.sync.updated_at,
            cancel: false,
        },
    )?;
    let (window, _bound) = open_window(&path, identity)?;
    window.invoke_select("logbook".into());
    filter(&window, "Бюджет")?;
    let shows_project = || window.get_rows().iter().any(|r| r.title == "Ремонт");
    assert!(shows_project());
    search(&window, "Бюджет");
    press(&window, Key::Return);
    assert_eq!(window.get_page_title(), "Бюджет");
    assert!(shows_project());
    Ok(())
}

fn tag_attached_to_an_empty_area_opens_a_page_containing_that_area() -> Result {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let mut console = Store::open(&path)?;
    let area_command = Command::AddArea(NewArea {
        title: "Дом".into(),
    });
    run(&mut console, area_command)?;
    let mut area = console.read().areas()?.pop().ok_or("нет области")?;
    let label = tag(1, "Личное");
    area.tags.push(label.id);
    console.write(|w| {
        w.insert_tag(&label)?;
        w.update_area(&area)
    })?;
    let (window, _bound) = open_window(&path, identity)?;
    search(&window, "Личное");
    press(&window, Key::Return);
    assert_eq!(window.get_page_title(), "Личное");
    assert!(window.get_rows().iter().any(|r| r.title == "Дом"));
    Ok(())
}

fn tag_filter_preserves_the_seven_day_calendar() -> Result {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let mut console = Store::open(&path)?;
    let mut tagged = task("Письмо");
    tagged.when = Some("завтра".into());
    tagged.tags = vec!["Работа".into()];
    run(&mut console, Command::AddTask(tagged))?;
    let mut other = task("Прогулка");
    other.when = Some("послезавтра".into());
    run(&mut console, Command::AddTask(other))?;
    let (window, _bound) = open_window(&path, identity)?;
    window.invoke_select("upcoming".into());
    let days = || {
        window
            .get_rows()
            .iter()
            .filter(|r| r.kind == RowKind::Section)
            .map(|r| r.title.to_string())
            .collect::<Vec<_>>()
    };
    let original = days();
    assert_eq!(original.len(), 7);
    filter(&window, "Работа")?;
    assert!(window.get_rows().iter().any(|r| r.title == "Письмо"));
    assert!(!window.get_rows().iter().any(|r| r.title == "Прогулка"));
    assert_eq!(days(), original);
    Ok(())
}

fn same_named_tags_in_different_branches_have_distinct_visible_labels() -> Result {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let mut console = Store::open(&path)?;
    let work = tag(1, "Работа");
    let home = tag(2, "Дом");
    let mut work_child = tag(3, "Срочно");
    let mut home_child = tag(4, "Срочно");
    work_child.parent = Some(work.id);
    home_child.parent = Some(home.id);
    console.write(|w| {
        for t in [&work, &home, &work_child, &home_child] {
            w.insert_tag(t)?;
        }
        Ok(())
    })?;
    let (window, _bound) = open_window(&path, identity)?;
    let choices: Vec<_> = window
        .get_tag_filter_items()
        .iter()
        .filter(|value| value.contains("Срочно"))
        .collect();
    search(&window, "Срочно");
    let results: Vec<_> = window
        .get_rows()
        .iter()
        .map(|row| (row.title, row.meta, row.when))
        .collect();
    assert_eq!(choices.len(), 2);
    assert_eq!(results.len(), 2);
    assert!(
        choices[0] != choices[1] && results[0] != results[1],
        "фильтр: {choices:?}, поиск: {results:?}"
    );
    Ok(())
}

#[test]
fn search_regressions() -> Result {
    init_integration_test_with_mock_time();
    refresh_preserves_the_selected_search_result()?;
    escaping_an_empty_search_preserves_the_checkmark_in_today()?;
    tagged_closed_project_is_visible_from_the_tag_as_well_as_the_logbook()?;
    tag_attached_to_an_empty_area_opens_a_page_containing_that_area()?;
    tag_filter_preserves_the_seven_day_calendar()?;
    same_named_tags_in_different_branches_have_distinct_visible_labels()
}
