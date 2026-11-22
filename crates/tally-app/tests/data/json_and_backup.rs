//! JSON-экспорт и импорт одним шагом истории, откат конфликтного импорта,
//! суточная копия базы.

use std::fs;
use std::path::PathBuf;

use jiff::{Timestamp, civil::date};
use tally_app::{
    App, Appearance, Clock, Command, DataReply, DataRequest, NewArea, NewProject, NewTask, Service,
    Settings,
};
use tally_store::Store;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn clock() -> Clock {
    Clock {
        now: Timestamp::UNIX_EPOCH,
        today: date(2026, 11, 22),
    }
}

fn graph() -> Result<App> {
    let mut app = App::new(Store::open_in_memory()?);
    let clock = clock();
    app.execute(
        &Command::AddArea(NewArea {
            title: "Дом".into(),
        }),
        clock,
    )?;
    app.execute(
        &Command::AddProject(NewProject {
            title: "Ремонт".into(),
            area: Some("Дом".into()),
        }),
        clock,
    )?;
    let saved = app.execute(
        &Command::AddTask(NewTask {
            text: "Покрасить стену".into(),
            project: Some("Ремонт".into()),
            notes: Some("Цвет — шалфей".into()),
            tags: vec!["Малярные работы".into()],
            ..NewTask::default()
        }),
        clock,
    )?;
    let project = &saved.snapshot.projects[0];
    app.execute(
        &Command::CloseProject {
            project: project.id,
            expected_updated_at: project.sync.updated_at,
            cancel: false,
        },
        clock,
    )?;
    Ok(app)
}

#[test]
fn json_preserves_relationships_and_import_is_one_undoable_step() -> Result {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("перенос.json");
    let mut source = graph()?;
    let baseline = source.snapshot()?;
    source.data(DataRequest::Export(path.clone()), clock())?;
    let bytes = fs::read(&path)?;
    assert!(
        source
            .data(DataRequest::Export(path.clone()), clock())
            .is_err()
    );
    assert_eq!(fs::read(&path)?, bytes);

    let mut target = App::new(Store::open_in_memory()?);
    assert!(matches!(
        target.data(DataRequest::Import(path.clone()), clock())?,
        DataReply::Imported(4)
    ));
    assert_eq!(target.snapshot()?, baseline);
    assert!(matches!(
        target.data(DataRequest::Import(path), clock())?,
        DataReply::Imported(0)
    ));
    assert_eq!(
        target.execute(&Command::Undo, clock())?.snapshot,
        Default::default()
    );
    let redone = target.execute(&Command::Redo, clock())?.snapshot;
    assert_eq!(redone.tasks[0].project, baseline.tasks[0].project);
    assert_eq!(redone.tasks[0].tags, baseline.tasks[0].tags);
    assert_eq!(redone.projects[0].status, baseline.projects[0].status);
    Ok(())
}

#[test]
fn conflicting_import_rolls_back_records_inserted_before_the_conflict() -> Result {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("export.json");
    let mut source = graph()?;
    source.data(DataRequest::Export(path.clone()), clock())?;
    let mut target = App::new(Store::open_in_memory()?);
    target.data(DataRequest::Import(path.clone()), clock())?;
    let baseline = target.snapshot()?;

    source.execute(
        &Command::AddArea(NewArea {
            title: "Запасная".into(),
        }),
        clock(),
    )?;
    let changed = dir.path().join("changed.json");
    source.data(DataRequest::Export(changed.clone()), clock())?;
    let mut document: serde_json::Value = serde_json::from_slice(&fs::read(&changed)?)?;
    document["data"]["tasks"][0]["title"] = "Другая задача".into();
    fs::write(&changed, serde_json::to_vec(&document)?)?;

    assert!(matches!(
        target.data(DataRequest::Import(changed), clock()),
        Err(tally_app::Error::Conflict(_))
    ));
    assert_eq!(target.snapshot()?, baseline);
    assert_eq!(
        target.execute(&Command::Undo, clock())?.snapshot,
        Default::default()
    );
    Ok(())
}

#[test]
fn daily_backup_includes_open_wal_history_and_settings_and_obeys_toggle() -> Result {
    let directory = tempfile::tempdir()?;
    let folder = directory.path().join("backups");
    let mut app = App::new(Store::open(directory.path().join("tally.db"))?);
    let mut settings = Settings {
        appearance: Appearance::Dark,
        automatic_backups: false,
    };
    let request = DataRequest::DailyBackup(folder.clone());
    app.data(DataRequest::SaveSettings(settings), clock())?;
    app.data(request.clone(), clock())?;
    assert!(!folder.exists());

    settings.automatic_backups = true;
    app.data(DataRequest::SaveSettings(settings), clock())?;
    let first = app
        .execute(
            &Command::AddArea(NewArea {
                title: "Первая".into(),
            }),
            clock(),
        )?
        .snapshot;
    app.data(request.clone(), clock())?;
    app.execute(
        &Command::AddArea(NewArea {
            title: "Вторая".into(),
        }),
        clock(),
    )?;
    app.data(request.clone(), clock())?;
    let tomorrow = Clock {
        today: date(2026, 11, 23),
        ..clock()
    };
    app.data(request, tomorrow)?;
    let open = |day| -> Result<Store> {
        let path: PathBuf = folder.join(format!("{day}.sqlite3"));
        assert!(path.is_file());
        Ok(Store::open(path)?)
    };
    let mut copy = App::new(open(clock().today)?);
    assert_eq!(copy.snapshot()?, first);
    assert_eq!(copy.store().settings()?, settings);
    assert!(
        copy.execute(&Command::Undo, clock())?
            .snapshot
            .areas
            .is_empty()
    );
    assert_eq!(open(tomorrow.today)?.read().snapshot()?, app.snapshot()?);
    Ok(())
}
