//! Метки времени, одноимённые области и теги, вечер проекта.

use std::path::Path;

use anyhow::{Context, Result};
use clap::Parser;
use jiff::civil::date;
use tally_cli::{app::App, cli::Cli};
use tally_core::{Project, ProjectId, Rank, Status, Tag, TagId, Timestamp};
use tally_store::{RecordId, Store};

const NOW: &str = "2026-11-19T12:00:00Z";

fn command(db: &Path, args: &[&str]) -> Result<String> {
    let cli = Cli::try_parse_from(std::iter::once("tally").chain(args.iter().copied()))?;
    let mut output = Vec::new();
    App {
        store: Store::open(db)?,
        today: date(2026, 11, 19),
        now: NOW.parse()?,
        out: &mut output,
    }
    .run(cli.command)?;
    Ok(String::from_utf8(output)?)
}

#[test]
fn unrelated_future_edit_does_not_date_new_tasks_or_completions() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let db = dir.path().join("tally.db");
    command(&db, &["add", "Отчёт"])?;
    let mut store = Store::open(&db)?;
    let now: Timestamp = NOW.parse()?;
    let mut tag = Tag::new(TagId::from_u128(1), "Архив", Rank::initial(), now);
    tag.sync.touch("2036-11-19T12:00:00Z".parse()?);
    store.write(|w| w.insert_tag(&tag))?;

    command(&db, &["done", "Отчёт"])?;
    command(&db, &["add", "Письмо"])?;
    let tasks = store.read().tasks()?;
    let report = tasks
        .iter()
        .find(|t| t.title == "Отчёт")
        .context("нет отчёта")?;
    let letter = tasks
        .iter()
        .find(|t| t.title == "Письмо")
        .context("нет письма")?;
    assert_eq!(
        (report.status, letter.sync.created_at),
        (Status::Completed { at: now }, now)
    );
    Ok(())
}

#[test]
fn restoring_a_homonymous_area_does_not_allow_creating_a_third() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let db = dir.path().join("tally.db");
    command(&db, &["area", "Дом"])?;
    let mut store = Store::open(&db)?;
    let first = store.read().areas()?.pop().context("нет области")?.id;
    let now = NOW.parse()?;
    store.write(|w| w.delete(RecordId::Area(first), now))?;
    command(&db, &["area", "ДОМ"])?;
    store.write(|w| w.restore(RecordId::Area(first), now))?;

    let result = command(&db, &["area", "дом"]);
    assert!(result.is_err(), "{result:?}");
    assert_eq!(store.read().areas()?.len(), 2);
    Ok(())
}

#[test]
fn add_does_not_pick_the_first_of_two_live_tags() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let db = dir.path().join("tally.db");
    command(&db, &["add", "А", "-t", "Ёлка"])?;
    let mut store = Store::open(&db)?;
    let first = store.read().tags()?.pop().context("нет тега")?.id;
    let now = NOW.parse()?;
    store.write(|w| w.delete(RecordId::Tag(first), now))?;
    command(&db, &["add", "Б", "-t", "ЕЛКА"])?;
    store.write(|w| w.restore(RecordId::Tag(first), now))?;

    let result = command(&db, &["add", "В", "-t", "елка"]);
    assert!(result.is_err(), "{result:?}");
    let snapshot = store.read().snapshot()?;
    assert_eq!(snapshot.tags.len(), 2);
    assert_eq!(snapshot.tasks.len(), 2);
    Ok(())
}

#[test]
fn soft_deleted_tag_is_hidden_on_a_live_task_until_restored() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let db = dir.path().join("tally.db");
    command(&db, &["add", "Письмо", "-t", "Дом"])?;
    let mut store = Store::open(&db)?;
    let tag = store.read().tags()?.pop().context("нет тега")?.id;
    let now = NOW.parse()?;
    store.write(|w| w.delete(RecordId::Tag(tag), now))?;
    let output = command(&db, &["list", "inbox"])?;
    assert!(
        output.contains("Письмо") && !output.contains("#Дом"),
        "{output}"
    );
    store.write(|w| w.restore(RecordId::Tag(tag), now))?;
    assert!(command(&db, &["list", "inbox"])?.contains("#Дом"));
    Ok(())
}

#[test]
fn project_evening_is_visible_on_its_start_date_only() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let db = dir.path().join("tally.db");
    let mut store = Store::open(&db)?;
    let mut project = Project::new(
        ProjectId::from_u128(1),
        "Ремонт",
        Rank::initial(),
        NOW.parse()?,
    );
    project.date = Some(date(2026, 11, 20));
    project.evening = true;
    project.deadline = Some(date(2026, 12, 5));
    store.write(|w| w.insert_project(&project))?;

    let output = command(&db, &["list", "upcoming"])?;
    let rows: Vec<_> = output
        .lines()
        .filter(|line| line.contains("Ремонт"))
        .collect();
    assert_eq!(rows.len(), 2, "{output}");
    assert!(rows[0].contains("вечер"), "{output}");
    assert!(
        rows[1].contains("⚑ срок") && !rows[1].contains("вечер"),
        "{output}"
    );
    Ok(())
}
