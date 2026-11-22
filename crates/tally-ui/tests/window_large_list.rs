//! Окно с длинными списками создаёт компоненты только около видимой области.

mod common;

use std::time::Duration;

use common::{Result, TODAY};
use jiff::Timestamp;
use slint::{ComponentHandle, LogicalSize, Model};
use tally_app::App;
use tally_core::{Area, AreaId, Rank, Start, Task, TaskId};
use tally_store::Store;
use tally_ui::backend::{Calendar, Inline};
use tally_ui::bind::Bound;
use tally_ui::ui::AppWindow;

#[test]
fn large_lists_only_mount_the_visible_range_and_can_reveal_the_last_task() -> Result {
    i_slint_backend_testing::init_integration_test_with_mock_time();
    let mut store = Store::open_in_memory()?;
    store.write(|writer| {
        for (i, rank) in Rank::spread(5000).into_iter().enumerate() {
            let mut task = Task::new(
                TaskId::from_u128(i as u128 + 1),
                format!("Задача {i:04}"),
                rank,
                Timestamp::UNIX_EPOCH,
            );
            task.start = Start::Anytime;
            writer.insert_task(&task)?;
        }
        for (i, rank) in Rank::spread(1000).into_iter().enumerate() {
            writer.insert_area(&Area::new(
                AreaId::from_u128(i as u128 + 1),
                format!("Область {i:04}"),
                rank,
                Timestamp::UNIX_EPOCH,
            ))?;
        }
        Ok(())
    })?;
    let window = AppWindow::new()?;
    window.window().set_size(LogicalSize::new(1000.0, 680.0));
    let calendar = Calendar {
        fixed_today: Some(TODAY),
    };
    let _bound = Bound::attach(
        &window,
        Box::new(Inline::new(App::new(store), calendar)),
        calendar,
    );
    window.show()?;
    window.invoke_select("anytime".into());

    let rows = window.get_visible_rows();
    let sidebar = window.get_visible_sidebar();
    assert_eq!(window.get_rows().row_count(), 5000);
    assert!(window.get_sidebar().row_count() >= 1000);
    assert!((1..64).contains(&rows.row_count()));
    assert!((1..64).contains(&sidebar.row_count()));
    assert!(rows.iter().any(|row| row.index == 0));
    assert!(rows.iter().all(|row| row.index < 100));

    let end = window.get_task_extent();
    window.invoke_reveal_row(end - 34.0, end);
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(16));
    assert!((1..64).contains(&rows.row_count()));
    assert!(rows.iter().any(|row| row.index == 4999));
    assert!(rows.iter().all(|row| row.index > 4900));
    Ok(())
}
