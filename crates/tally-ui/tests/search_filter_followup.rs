//! Быстрый поиск и фильтр тегов: дополнительные случаи.

mod common;

use std::{path::Path, time::Duration};

use common::{Result, TODAY, clock, id_of, run, task};
use i_slint_backend_testing::{init_integration_test_with_mock_time, mock_elapsed_time};
use slint::{ComponentHandle, LogicalSize, Model, platform::Key};
use tally_app::{App, Command};
use tally_core::{Rank, Task, TaskId};
use tally_store::Store;
use tally_ui::backend::{Calendar, Inline};
use tally_ui::bind::Bound;
use tally_ui::ui::{AppWindow, Mark};

fn open(path: &Path) -> Result<(AppWindow, Bound)> {
    let window = AppWindow::new()?;
    window.window().set_size(LogicalSize::new(900.0, 600.0));
    let calendar = Calendar {
        fixed_today: Some(TODAY),
    };
    let backend = Inline::new(App::new(Store::open(path)?), calendar);
    let bound = Bound::attach(&window, Box::new(backend), calendar);
    window.show()?;
    Ok((window, bound))
}

fn key(window: &AppWindow, key: Key) {
    assert!(window.invoke_key(key.into(), false, false, false, false));
}

fn search(window: &AppWindow, text: &str) {
    window.invoke_search_begin();
    window.invoke_search_query(text.into());
    mock_elapsed_time(Duration::from_millis(150));
}

fn close_from_search_above_logbook() -> Result {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let mut store = Store::open(&path)?;
    run(&mut store, Command::AddTask(task("Позвонить маме")))?;
    let id = id_of(&store.read().snapshot()?, "Позвонить маме")?;
    let (window, _bound) = open(&path)?;
    window.invoke_select("logbook".into());
    search(&window, "Позвонить маме");
    let result = window.get_rows().row_data(0).ok_or("нет результата")?;
    assert_eq!(result.key.as_str(), id.to_string());
    assert!(result.checkable);
    window.invoke_toggle(result.key.clone());
    let task = store.read().task(id)?.ok_or("нет задачи")?;
    assert!(!task.status.is_open());
    key(&window, Key::Escape);
    assert_eq!(window.get_page_title(), "Журнал");
    let listed = || {
        window
            .get_rows()
            .iter()
            .any(|row| row.key == result.key && row.mark == Mark::Completed && !row.checkable)
    };
    let immediately = listed();
    key(&window, Key::F5);
    assert!(
        immediately && listed(),
        "выполненная задача пропала из журнала"
    );
    Ok(())
}

fn refresh_keeps_the_selected_match_in_view() -> Result {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let mut store = Store::open(&path)?;
    let id = TaskId::from_u128(1000);
    let mut target = Task::new(id, "Письмо А", Rank::initial(), clock().now);
    store.write(|w| {
        w.insert_task(&target)?;
        for (i, rank) in Rank::spread(96).into_iter().enumerate() {
            w.insert_task(&Task::new(
                TaskId::from_u128(i as u128 + 1),
                format!("Письмо М{i:03}"),
                rank,
                clock().now,
            ))?;
        }
        Ok(())
    })?;
    let (window, _bound) = open(&path)?;
    search(&window, "Письмо");
    let selected = window.get_selected_result();
    assert_eq!(selected.as_str(), id.to_string());
    let visible = || {
        let top = -window.get_task_scroll_y();
        let bottom = top + window.get_task_view_height();
        window
            .get_visible_rows()
            .iter()
            .any(|row| row.item.row_key == selected && row.top >= top - 0.5 && row.top < bottom)
    };
    assert!(visible(), "до обновления выбранная строка была видна");
    target.title = "Письмо Я".into();
    store.write(|w| w.update_task(&target))?;
    key(&window, Key::F5);
    mock_elapsed_time(Duration::from_millis(32));
    assert_eq!(window.get_rows().row_count(), 97);
    assert_eq!(
        window.get_rows().row_data(96).ok_or("нет строки")?.row_key,
        selected
    );
    assert_eq!(window.get_selected_result(), selected);
    assert!(
        visible(),
        "после обновления выделение оказалось за пределами экрана"
    );
    Ok(())
}

fn tag_named_all_tags_is_distinct_from_the_reset_option() -> Result {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");
    let mut store = Store::open(&path)?;
    let mut tagged = task("Сверить остатки");
    tagged.when = Some("сегодня".into());
    tagged.tags = vec!["Все теги".into()];
    run(&mut store, Command::AddTask(tagged))?;
    let mut other = task("Прогулка");
    other.when = Some("сегодня".into());
    run(&mut store, Command::AddTask(other))?;
    let (window, _bound) = open(&path)?;
    let options: Vec<_> = window.get_tag_filter_items().iter().collect();
    assert_eq!(options.len(), 2);
    let contains_other = || window.get_rows().iter().any(|r| r.title == "Прогулка");
    window.invoke_tag_filter(1);
    assert!(!contains_other());
    window.invoke_tag_filter(0);
    assert!(contains_other());
    assert_ne!(
        options[0], options[1],
        "два одинаковых названия у разных действий"
    );
    Ok(())
}

#[test]
fn search_and_filter_followup() -> Result {
    init_integration_test_with_mock_time();
    close_from_search_above_logbook()?;
    refresh_keeps_the_selected_match_in_view()?;
    tag_named_all_tags_is_distinct_from_the_reset_option()
}
