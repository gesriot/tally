use i_slint_backend_testing::init_integration_test_with_mock_time;
use slint::ComponentHandle;
use tally_app::{App, Appearance, Clock, Command, NewTask, Service, Settings};
use tally_store::Store;
use tally_ui::backend::{Calendar, Inline};
use tally_ui::bind::Bound;
use tally_ui::ui::{AppWindow, QuickEntry, ReminderNotice, SettingsWindow, Theme};

#[test]
fn settings_apply_to_each_window_and_json_import_refreshes_the_visible_list()
-> Result<(), Box<dyn std::error::Error>> {
    init_integration_test_with_mock_time();
    let path = tempfile::tempdir()?;
    let db = path.path().join("work.db");
    let calendar = Calendar::default();
    let window = AppWindow::new()?;
    let entry = QuickEntry::new()?;
    let notice = ReminderNotice::new()?;
    let settings = SettingsWindow::new()?;
    let store = Store::open(&db)?;
    let bound = Bound::attach(
        &window,
        Box::new(Inline::new(App::new(store), calendar)),
        calendar,
    );
    bound.attach_quick_entry(&entry);
    bound.attach_settings(
        &settings,
        &notice,
        Ok::<_, String>(path.path().join("backups")),
    );
    assert!(settings.get_ready());
    assert!(window.get_settings_available());
    settings.invoke_set_appearance(2);
    assert_eq!(settings.get_appearance(), 2);
    assert!(window.global::<Theme<'_>>().get_dark());
    assert!(entry.global::<Theme<'_>>().get_dark());
    assert!(notice.global::<Theme<'_>>().get_dark());
    assert!(settings.global::<Theme<'_>>().get_dark());
    settings.invoke_set_automatic_backups(false);
    assert_eq!(
        Store::open(&db)?.settings()?,
        Settings {
            appearance: Appearance::Dark,
            automatic_backups: false,
        }
    );

    let sync_file = path.path().join("copy.json");
    let mut source = App::new(Store::open_in_memory()?);
    let clock: Clock = calendar.clock();
    source.execute(
        &Command::AddTask(NewTask {
            text: "Задача из файла".into(),
            when: Some("сегодня".into()),
            ..NewTask::default()
        }),
        clock,
    )?;
    source.data(tally_app::DataRequest::Export(sync_file.clone()), clock)?;
    settings.set_json_path(sync_file.to_string_lossy().as_ref().into());
    settings.invoke_import_json();
    assert_eq!(settings.get_status(), "Добавлено записей: 1");
    assert_eq!(Store::open(&db)?.read().snapshot()?.tasks.len(), 1);
    let rows = slint::Model::iter(&window.get_rows()).collect::<Vec<_>>();
    assert!(rows.iter().any(|row| row.title == "Задача из файла"));
    settings.invoke_export_json();
    assert!(
        settings.get_status_error(),
        "экспорт должен сохранить существующий файл"
    );

    settings.invoke_set_appearance(1);
    assert!(!window.global::<Theme<'_>>().get_dark());
    Ok(())
}
