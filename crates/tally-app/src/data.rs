//! Настройки базы, перенос записей и резервные копии SQLite.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, BufReader, ErrorKind, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tally_core::{RepeatFrom, Tag, TagId};
use tally_store::{Settings, Snapshot, Store};
use tempfile::NamedTempFile;

use crate::exec::edited;
use crate::history::{self, Record};
use crate::{Clock, Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataRequest {
    Settings,
    SaveSettings(Settings),
    Export(PathBuf),
    Import(PathBuf),
    DailyBackup(PathBuf),
}

#[derive(Debug)]
pub enum DataReply {
    Settings(Settings),
    Exported(PathBuf),
    Imported(usize),
    DailyBackup,
}

#[derive(Serialize)]
struct Export<'a> {
    format: &'static str,
    version: u32,
    data: &'a Snapshot,
}

#[derive(Deserialize)]
struct Import {
    format: String,
    version: u32,
    data: Snapshot,
}

const FORMAT: &str = "tally";
const VERSION: u32 = 1;

pub(crate) fn execute(store: &mut Store, request: DataRequest, clock: Clock) -> Result<DataReply> {
    match request {
        DataRequest::Settings => Ok(DataReply::Settings(store.settings()?)),
        DataRequest::SaveSettings(settings) => {
            store.save_settings(settings)?;
            Ok(DataReply::Settings(settings))
        }
        DataRequest::Export(path) => {
            export(store, &path)?;
            Ok(DataReply::Exported(path))
        }
        DataRequest::Import(path) => {
            let file = File::open(&path).map_err(|e| file_error(&path, e))?;
            let imported: Import =
                serde_json::from_reader(BufReader::new(file)).map_err(Error::DataJson)?;
            if imported.format != FORMAT || imported.version != VERSION {
                return Err(Error::Rejected(
                    "этот формат или версия JSON не поддерживается".into(),
                ));
            }
            Ok(DataReply::Imported(import(store, imported.data, clock)?))
        }
        DataRequest::DailyBackup(directory) => {
            if store.settings()?.automatic_backups {
                daily_backup(store, &directory, clock)?;
            }
            Ok(DataReply::DailyBackup)
        }
    }
}

fn file_error(path: &Path, source: io::Error) -> Error {
    Error::File {
        path: path.display().to_string(),
        source,
    }
}

fn temporary_file(path: &Path) -> Result<NamedTempFile> {
    let directory = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    NamedTempFile::new_in(directory).map_err(|e| file_error(path, e))
}

fn export(store: &Store, path: &Path) -> Result<()> {
    let snapshot = store.read().snapshot()?;
    let mut file = temporary_file(path)?;
    serde_json::to_writer_pretty(
        &mut file,
        &Export {
            format: FORMAT,
            version: VERSION,
            data: &snapshot,
        },
    )
    .map_err(Error::DataJson)?;
    file.write_all(b"\n").map_err(|e| file_error(path, e))?;
    file.as_file().sync_all().map_err(|e| file_error(path, e))?;
    file.persist_noclobber(path)
        .map_err(|e| file_error(path, e.error))?;
    Ok(())
}

fn daily_backup(store: &Store, directory: &Path, clock: Clock) -> Result<()> {
    let path = directory.join(format!("{}.sqlite3", clock.today));
    match fs::symlink_metadata(&path) {
        Ok(_) => return verify_daily_backup(store, &path),
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(file_error(&path, e)),
    }
    fs::create_dir_all(directory).map_err(|e| file_error(directory, e))?;
    let temporary = temporary_file(&path)?;
    store.backup_into(temporary.path())?;
    store
        .verify_backup(temporary.path())
        .map_err(|e| unusable_backup(&path, e))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|e| file_error(&path, e))?;
    match temporary.persist_noclobber(&path) {
        Ok(_) => Ok(()),
        Err(e) if e.error.kind() == ErrorKind::AlreadyExists => verify_daily_backup(store, &path),
        Err(e) => Err(file_error(&path, e.error)),
    }
}

fn verify_daily_backup(store: &Store, path: &Path) -> Result<()> {
    let result = if fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file()) {
        store.verify_backup(path).map_err(|e| e.to_string())
    } else {
        Err("путь занят".into())
    };
    result.map_err(|reason| unusable_backup(path, reason))
}

fn unusable_backup(path: &Path, reason: impl ToString) -> Error {
    Error::UnusableBackup {
        path: path.display().to_string(),
        reason: reason.to_string(),
    }
}

fn check_calendar_repeat(record: &Record) -> Result<()> {
    let (label, title, date, from, rule) = match record {
        Record::Task(t) => ("задача", &t.title, t.date, t.repeat_from, t.repeat_rule),
        Record::Project(p) => ("проект", &p.title, p.date, p.repeat_from, p.repeat_rule),
        _ => return Ok(()),
    };
    if date.is_none() && from == RepeatFrom::Date && rule.is_some() {
        return Err(Error::Rejected(format!(
            "импорт JSON: {label} «{title}» с календарным повтором без даты начала"
        )));
    }
    Ok(())
}

fn records(s: Snapshot) -> impl Iterator<Item = Record> {
    s.tags
        .into_iter()
        .map(Record::Tag)
        .chain(s.areas.into_iter().map(Record::Area))
        .chain(s.projects.into_iter().map(Record::Project))
        .chain(s.headings.into_iter().map(Record::Heading))
        .chain(s.tasks.into_iter().map(Record::Task))
}

fn import(store: &mut Store, mut incoming: Snapshot, clock: Clock) -> Result<usize> {
    store.write_with(|writer| {
        let before = writer.read().snapshot()?;
        let existing: HashMap<_, _> = records(before.clone()).map(|r| (r.id(), r)).collect();
        incoming.tags = parent_first(incoming.tags, &before.tags)?;
        let mut seen = HashSet::new();
        let mut added = Vec::new();
        for record in records(incoming) {
            check_calendar_repeat(&record)?;
            let id = record.id();
            if !seen.insert(id) {
                return Err(Error::Rejected(format!(
                    "в JSON повторяется идентификатор {id:?}"
                )));
            }
            if let Some(previous) = existing.get(&id) {
                if previous != &record {
                    return Err(Error::Conflict(format!(
                        "в базе уже есть другая запись с идентификатором {id:?}"
                    )));
                }
                continue;
            }
            edited(clock.now, record.sync())?;
            history::save(writer, &record, true)?;
            added.push(id);
        }
        if !added.is_empty() {
            history::record(writer, &before, clock.now, "импорт JSON", &added)?;
        }
        Ok(added.len())
    })
}

// Обходим входные теги от корней. Родитель может быть в файле или уже в базе.
fn parent_first(tags: Vec<Tag>, existing: &[Tag]) -> Result<Vec<Tag>> {
    let mut known: HashSet<_> = existing.iter().map(|tag| tag.id).collect();
    let mut children: HashMap<TagId, Vec<Tag>> = HashMap::new();
    let mut ordered = Vec::with_capacity(tags.len());
    for tag in tags {
        match tag.parent {
            Some(parent) if !known.contains(&parent) => {
                children.entry(parent).or_default().push(tag)
            }
            _ => ordered.push(tag),
        }
    }
    let mut index = 0;
    while let Some(tag) = ordered.get(index) {
        if known.insert(tag.id)
            && let Some(waiting) = children.remove(&tag.id)
        {
            ordered.extend(waiting);
        }
        index += 1;
    }
    if !children.is_empty() {
        return Err(Error::Rejected(
            "в JSON отсутствует родительский тег или есть цикл тегов".into(),
        ));
    }
    Ok(ordered)
}
