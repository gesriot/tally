//! CRUD, миграции, транзакции и ошибки.

mod common;

use common::*;
use jiff::civil::date;
use tally_core::lists::List;
use tally_core::{
    ChecklistItem, ChecklistItemId, RepeatFrom, RepeatRule, RepeatUnit, Start, Status, TaskId,
};
use tally_store::{RecordId, SCHEMA_VERSION, Store, StoreError};

fn store() -> Store {
    Store::open_in_memory().unwrap_or_else(|e| panic!("база в памяти: {e}"))
}

/// Задача, в которой заполнено всё.
fn full_task(store: &mut Store) -> tally_core::Task {
    let home = area(1, "Дом");
    let mut urgent = tag(2, "Срочно");
    urgent.index = rank("a1");
    let errands = tag(3, "Дела");
    let mut p = project(10, "Ремонт кухни");
    p.area = Some(home.id);
    let h = heading(20, p.id, "Покупки");

    let mut t = task(100, "Купить плитку");
    t.notes = "Белая, 20×20\nВторая строка".into();
    t.start = Start::Someday;
    t.date = Some(date(2026, 11, 20));
    t.evening = true;
    t.deadline = Some(date(2026, 12, 1));
    t.status = Status::Completed {
        at: at("2026-11-19T10:15:30.123456789Z"),
    };
    t.project = Some(p.id);
    t.heading = Some(h.id);
    t.tags = vec![urgent.id, errands.id]; // порядок не по алфавиту и не по id
    t.tags.reverse();
    t.checklist = vec![
        ChecklistItem::new(
            ChecklistItemId::from_u128(501),
            "Замерить",
            rank("a0"),
            t0(),
        ),
        ChecklistItem::new(
            ChecklistItemId::from_u128(502),
            "Выбрать цвет",
            rank("a1"),
            t0(),
        ),
    ];
    t.checklist[0].done = true;
    t.index = rank("a0V");
    t.today_index = rank("Zz");
    t.repeat_rule = Some(RepeatRule {
        interval: 2,
        unit: RepeatUnit::Week,
    });
    t.repeat_from = RepeatFrom::Completion;
    t.sync.touch(at("2026-11-19T11:00:00Z"));

    let result = store.write(|w| {
        w.insert_area(&home)?;
        w.insert_tag(&urgent)?;
        w.insert_tag(&errands)?;
        w.insert_project(&p)?;
        w.insert_heading(&h)?;
        w.insert_task(&t)
    });
    assert!(result.is_ok(), "{result:?}");
    t
}

#[test]
fn fresh_database_has_current_schema() {
    let s = store();
    assert_eq!(s.schema_version().ok(), Some(SCHEMA_VERSION));
    const { assert!(SCHEMA_VERSION >= 1) };
    assert!(s.read().tasks().is_ok_and(|t| t.is_empty()));
}

#[test]
fn task_round_trips_with_every_field() {
    let mut s = store();
    let t = full_task(&mut s);
    let back = s.read().task(t.id).ok().flatten();
    assert_eq!(back.as_ref(), Some(&t));
    assert_eq!(s.read().tasks().ok(), Some(vec![t]));
}

#[test]
fn other_records_round_trip() {
    let mut s = store();
    let _ = full_task(&mut s);
    let mut child = tag(4, "Магазин");
    child.parent = Some(tally_core::TagId::from_u128(3));
    let mut a = area(5, "Работа");
    a.tags = vec![child.id];
    let mut p = project(11, "Отчёт");
    p.notes = "Квартальный".into();
    p.start = Start::Someday;
    p.deadline = Some(date(2026, 12, 31));
    p.status = Status::Canceled { at: t0() };
    p.tags = vec![child.id];
    p.repeat_rule = Some(RepeatRule {
        interval: 1,
        unit: RepeatUnit::Year,
    });
    let mut h = heading(21, p.id, "Архив");
    h.archived = true;
    let ok = s.write(|w| {
        w.insert_tag(&child)?;
        w.insert_area(&a)?;
        w.insert_project(&p)?;
        w.insert_heading(&h)
    });
    assert!(ok.is_ok(), "{ok:?}");
    let r = s.read();
    assert_eq!(r.tag(child.id).ok().flatten(), Some(child));
    assert_eq!(r.area(a.id).ok().flatten(), Some(a));
    assert_eq!(r.project(p.id).ok().flatten(), Some(p));
    assert_eq!(r.heading(h.id).ok().flatten(), Some(h));
    assert_eq!(
        r.project(tally_core::ProjectId::from_u128(999)).ok(),
        Some(None)
    );
}

#[test]
fn update_replaces_tags_and_checklist() {
    let mut s = store();
    let mut t = full_task(&mut s);
    t.title = "Купить плитку и клей".into();
    t.tags = vec![tally_core::TagId::from_u128(2)];
    let removed = t.checklist.remove(0);
    t.status = Status::Open;
    t.date = None;
    t.repeat_rule = None;
    t.sync.touch(at("2026-11-19T12:00:00Z"));
    assert!(s.write(|w| w.update_task(&t)).is_ok());
    // Убранный пункт не исчезает, а становится надгробием с временем правки задачи.
    let mut tombstone = removed;
    tombstone.sync.delete(at("2026-11-19T12:00:00Z"));
    let mut expected = t.clone();
    expected.checklist.insert(0, tombstone);
    assert_eq!(s.read().task(t.id).ok().flatten(), Some(expected));
}

#[test]
fn errors_are_specific() {
    let mut s = store();
    let t = full_task(&mut s);
    assert!(matches!(
        s.write(|w| w.insert_task(&t)),
        Err(StoreError::AlreadyExists { .. })
    ));
    let ghost = task(999, "Нет в базе");
    assert!(matches!(
        s.write(|w| w.update_task(&ghost)),
        Err(StoreError::NotFound { .. })
    ));
    let mut orphan = task(998, "Без проекта");
    orphan.project = Some(tally_core::ProjectId::from_u128(404));
    assert!(matches!(
        s.write(|w| w.insert_task(&orphan)),
        Err(StoreError::MissingReference { .. })
    ));
    let mut twice = task(997, "Тег дважды");
    twice.tags = vec![tally_core::TagId::from_u128(2); 2];
    assert!(matches!(
        s.write(|w| w.insert_task(&twice)),
        Err(StoreError::DuplicateTag { .. })
    ));
    assert!(matches!(
        s.write(|w| w.delete(RecordId::Task(TaskId::from_u128(404)), t0())),
        Err(StoreError::NotFound { .. })
    ));
    // Ничего из неудачного не записалось.
    assert_eq!(s.read().tasks().map(|v| v.len()).ok(), Some(1));
}

#[test]
fn failed_write_rolls_back_everything() {
    let mut s = store();
    let a = task(1, "Первая");
    let b = task(2, "Вторая");
    let result: Result<(), StoreError> = s.write(|w| {
        w.insert_task(&a)?;
        w.insert_task(&b)?;
        // Внутри транзакции записи уже видны.
        assert_eq!(w.read().tasks().map(|v| v.len()).ok(), Some(2));
        w.insert_task(&a) // дубликат – вся транзакция откатывается
    });
    assert!(result.is_err());
    assert!(s.read().tasks().is_ok_and(|v| v.is_empty()));
}

#[test]
fn tasks_come_back_in_rank_order() {
    let mut s = store();
    let keys = [
        "a0V",
        "Zz",
        "a1",
        "a0",
        "A0000000000000000000000000V",
        "b00",
    ];
    let ok = s.write(|w| {
        for (i, k) in keys.iter().enumerate() {
            let mut t = task(i as u128 + 1, k);
            t.index = rank(k);
            w.insert_task(&t)?;
        }
        Ok(())
    });
    assert!(ok.is_ok());
    let from_db: Vec<String> = s
        .read()
        .tasks()
        .unwrap_or_default()
        .into_iter()
        .map(|t| t.index.to_string())
        .collect();
    let mut expected: Vec<tally_core::Rank> = keys.iter().map(|k| rank(k)).collect();
    expected.sort();
    let expected: Vec<String> = expected.iter().map(ToString::to_string).collect();
    assert_eq!(from_db, expected);
}

#[test]
fn soft_delete_restore_and_lists() {
    let mut s = store();
    let p = project(10, "Разбор гаража");
    let mut done = task(1, "Вывезти шины");
    done.project = Some(p.id);
    done.status = Status::Completed { at: t0() };
    let mut today = task(2, "Позвонить");
    today.date = Some(date(2026, 11, 19));
    assert!(
        s.write(|w| {
            w.insert_project(&p)?;
            w.insert_task(&done)?;
            w.insert_task(&today)
        })
        .is_ok()
    );
    let day = date(2026, 11, 19);
    let logbook_len = |s: &Store| {
        s.read()
            .snapshot()
            .map(|snap| snap.lists(day).logbook.len())
            .unwrap_or(99)
    };
    assert_eq!(logbook_len(&s), 1);

    let later = at("2026-11-19T15:00:00Z");
    assert!(
        s.write(|w| w.delete(RecordId::Project(p.id), later))
            .is_ok()
    );
    let snap = s.read().snapshot().unwrap_or_default();
    let stored = snap
        .projects
        .first()
        .map(|p| (p.sync.deleted_at, p.sync.updated_at));
    assert_eq!(stored, Some((Some(later), later)));
    let lists = snap.lists(day);
    assert!(lists.logbook.is_empty());
    assert_eq!(lists.trash.len(), 1);
    assert_eq!(lists.today.len(), 1);

    assert!(
        s.write(|w| w.restore(RecordId::Project(p.id), later))
            .is_ok()
    );
    assert_eq!(logbook_len(&s), 1);
    let restored = s
        .read()
        .project(p.id)
        .ok()
        .flatten()
        .and_then(|p| p.sync.deleted_at);
    assert_eq!(restored, None);
}

#[test]
fn empty_trash_cascades_through_projects_and_areas() {
    let mut s = store();
    let home = area(1, "Дом");
    let work = area(2, "Работа");
    let mut p = project(10, "Старый проект");
    p.area = Some(work.id);
    let h = heading(20, p.id, "Раздел");
    let mut inside = task(1, "Внутри проекта");
    inside.project = Some(p.id);
    inside.heading = Some(h.id);
    let mut home_project = project(11, "Ремонт");
    home_project.area = Some(home.id);
    let mut in_home_project = task(2, "В проекте области");
    in_home_project.project = Some(home_project.id);
    let mut in_area = task(3, "В области");
    in_area.area = Some(home.id);
    let loose = task(4, "Сама удалена");
    let mut keep = task(5, "Остаётся");
    keep.area = Some(work.id);
    assert!(
        s.write(|w| {
            w.insert_area(&home)?;
            w.insert_area(&work)?;
            w.insert_project(&p)?;
            w.insert_project(&home_project)?;
            w.insert_heading(&h)?;
            for t in [&inside, &in_home_project, &in_area, &loose, &keep] {
                w.insert_task(t)?;
            }
            w.delete(RecordId::Project(p.id), t0())?;
            w.delete(RecordId::Task(loose.id), t0())?;
            w.delete(RecordId::Area(home.id), t0())
        })
        .is_ok()
    );
    let purged = s.write(|w| w.empty_trash()).unwrap_or_default();
    // Проекты: удалённый сам + проект удалённой области; задачи: внутри них,
    // прямо в области и удалённая сама.
    assert_eq!(
        (purged.areas, purged.projects, purged.headings, purged.tasks),
        (1, 2, 1, 4)
    );
    let r = s.read();
    let left: Vec<String> = r
        .tasks()
        .unwrap_or_default()
        .into_iter()
        .map(|t| t.title)
        .collect();
    assert_eq!(left, ["Остаётся"]);
    let areas: Vec<String> = r
        .areas()
        .unwrap_or_default()
        .into_iter()
        .map(|a| a.title)
        .collect();
    assert_eq!(areas, ["Работа"]);
    assert!(r.projects().is_ok_and(|v| v.is_empty()));

    // Физическое удаление удалённой области уносит и её содержимое.
    assert!(s.write(|w| w.delete(RecordId::Area(work.id), t0())).is_ok());
    assert!(s.write(|w| w.purge(RecordId::Area(work.id))).is_ok());
    assert!(s.read().tasks().is_ok_and(|v| v.is_empty()));
}

#[test]
fn deleted_area_hides_contents_until_restored() {
    let mut s = store();
    let home = area(1, "Дом");
    let mut repair = project(10, "Ремонт кухни");
    repair.area = Some(home.id);
    let mut paint = task(1, "Купить краску");
    paint.project = Some(repair.id);
    paint.date = Some(date(2026, 11, 19));
    let mut tiles = task(2, "Снять плитку");
    tiles.project = Some(repair.id);
    tiles.status = Status::Completed { at: t0() };
    let mut plants = task(3, "Полить цветы");
    plants.area = Some(home.id);
    plants.start = Start::Anytime;
    let mut old = task(4, "Старая краска");
    old.area = Some(home.id);
    assert!(
        s.write(|w| {
            w.insert_area(&home)?;
            w.insert_project(&repair)?;
            for t in [&paint, &tiles, &plants, &old] {
                w.insert_task(t)?;
            }
            w.delete(RecordId::Task(old.id), at("2026-11-18T09:00:00Z"))
        })
        .is_ok()
    );
    let day = date(2026, 11, 19);
    let view = |s: &Store| {
        let snap = s.read().snapshot().unwrap_or_default();
        let l = snap.lists(day);
        let names = |items: &[tally_core::lists::Item<'_>]| -> Vec<String> {
            items.iter().map(|i| i.title().to_owned()).collect()
        };
        let mut found: Vec<String> = ["краск", "плитку", "цветы", "ремонт", "дом"]
            .iter()
            .flat_map(|q| s.read().search(q).unwrap_or_default())
            .map(|h| h.title)
            .collect();
        found.sort();
        (
            names(&l.today.day),
            l.anytime.len(),
            names(&l.logbook),
            names(&l.trash),
            found,
        )
    };
    let before = view(&s);
    assert_eq!(before.0, ["Купить краску"]);
    assert_eq!(before.2, ["Снять плитку"]);
    assert_eq!(before.3, ["Старая краска"]);
    assert_eq!(
        before.4,
        [
            "Дом",
            "Купить краску",
            "Полить цветы",
            "Ремонт кухни",
            "Снять плитку"
        ]
    );

    let later = at("2026-11-19T15:00:00Z");
    assert!(
        s.write(|w| w.delete(RecordId::Area(home.id), later))
            .is_ok()
    );
    let deleted = view(&s);
    assert!(deleted.0.is_empty());
    assert_eq!(deleted.1, 0);
    assert!(deleted.2.is_empty());
    assert_eq!(deleted.3, ["Дом", "Старая краска"]);
    assert!(deleted.4.is_empty(), "{:?}", deleted.4);

    assert!(
        s.write(|w| w.restore(RecordId::Area(home.id), later))
            .is_ok()
    );
    assert_eq!(view(&s), before);
}

#[test]
fn snapshot_lists_match_in_memory_lists() {
    let mut s = store();
    let mut tasks = Vec::new();
    for (i, (date_, start)) in [
        (Some(date(2026, 11, 19)), Start::Anytime),
        (Some(date(2026, 11, 25)), Start::Inbox),
        (None, Start::Someday),
        (None, Start::Inbox),
    ]
    .into_iter()
    .enumerate()
    {
        let mut t = task(i as u128 + 1, &format!("Задача {i}"));
        t.date = date_;
        t.start = start;
        t.index = tally_core::Rank::spread(5)[i].clone();
        tasks.push(t);
    }
    assert!(
        s.write(|w| {
            for t in &tasks {
                w.insert_task(t)?;
            }
            Ok(())
        })
        .is_ok()
    );
    let snap = s.read().snapshot().unwrap_or_default();
    assert_eq!(snap.tasks, tasks);
    let day = date(2026, 11, 19);
    let from_db = snap.lists(day);
    let in_memory = tally_core::lists::Lists::compute(&tasks, &[], &[], &[], day);
    assert_eq!(format!("{from_db:?}"), format!("{in_memory:?}"));
    assert_eq!(
        tally_core::lists::list_of(&snap.tasks[3], None, None, None, day),
        List::Inbox
    );
}

#[test]
fn file_database_persists_and_migrates_once() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("tally.db");
    {
        let mut s = Store::open(&path).unwrap_or_else(|e| panic!("{e}"));
        assert!(s.write(|w| w.insert_task(&task(1, "Сохранится"))).is_ok());
    }
    let s = Store::open(&path).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(s.schema_version().ok(), Some(SCHEMA_VERSION));
    let titles: Vec<String> = s
        .read()
        .tasks()
        .unwrap_or_default()
        .into_iter()
        .map(|t| t.title)
        .collect();
    assert_eq!(titles, ["Сохранится"]);
}

#[test]
fn newer_schema_is_refused() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("future.db");
    drop(Store::open(&path));
    {
        let conn = rusqlite::Connection::open(&path).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
                .is_ok()
        );
    }
    assert!(matches!(
        Store::open(&path),
        Err(StoreError::NewerSchema { found, known }) if found == SCHEMA_VERSION + 1 && known == SCHEMA_VERSION
    ));
}

#[test]
fn corrupt_values_are_reported_not_panicked() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("broken.db");
    {
        let mut s = Store::open(&path).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            s.write(|w| w.insert_task(&task(1, "Ключ испортят")))
                .is_ok()
        );
    }
    {
        let conn = rusqlite::Connection::open(&path).unwrap_or_else(|e| panic!("{e}"));
        // Запрещённый ключ порядка (см. tally-rank) – схема его не ловит, ловит разбор.
        let bad = format!("A{}", "0".repeat(26));
        assert!(
            conn.execute("UPDATE tasks SET sort_index = ?1", [&bad])
                .is_ok()
        );
    }
    let s = Store::open(&path).unwrap_or_else(|e| panic!("{e}"));
    assert!(matches!(
        s.read().tasks(),
        Err(StoreError::Corrupt {
            table: "tasks",
            column: "sort_index",
            ..
        })
    ));
}

#[test]
fn schema_rejects_inconsistent_rows() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("checks.db");
    {
        let mut s = Store::open(&path).unwrap_or_else(|e| panic!("{e}"));
        assert!(s.write(|w| w.insert_task(&task(1, "Проверки"))).is_ok());
    }
    let conn = rusqlite::Connection::open(&path).unwrap_or_else(|e| panic!("{e}"));
    for sql in [
        "UPDATE tasks SET status = 'completed'", // без closed_at
        "UPDATE tasks SET start = 'tomorrow'",   // нет такого start
        "UPDATE tasks SET repeat_interval = 3",  // без repeat_unit
        "UPDATE tasks SET evening = 2",          // не 0/1
        "UPDATE tasks SET date = 'завтра'",      // не YYYY-MM-DD
    ] {
        assert!(conn.execute(sql, []).is_err(), "{sql}");
    }
}

#[test]
fn store_can_move_to_its_own_thread() {
    fn is_send<T: Send>(_: &T) {}
    let s = store();
    is_send(&s);
    let handle = std::thread::spawn(move || s.schema_version().ok());
    assert_eq!(handle.join().ok().flatten(), Some(SCHEMA_VERSION));
}

/// `write_with`: ошибка вызывающего кода откатывает всю транзакцию.
#[test]
fn write_with_rolls_back_on_caller_error() {
    #[derive(Debug)]
    enum AppError {
        Store(tally_store::StoreError),
        Refused,
    }
    impl From<tally_store::StoreError> for AppError {
        fn from(e: tally_store::StoreError) -> Self {
            Self::Store(e)
        }
    }
    let mut store = tally_store::Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
    let at = jiff::Timestamp::UNIX_EPOCH;
    let task = tally_core::Task::new(
        tally_core::TaskId::from_u128(1),
        "Задача",
        tally_core::Rank::initial(),
        at,
    );
    let result: Result<(), AppError> = store.write_with(|w| {
        w.insert_task(&task)?;
        assert_eq!(w.read().tasks()?.len(), 1);
        Err(AppError::Refused)
    });
    assert!(matches!(result, Err(AppError::Refused)));
    assert!(
        store
            .read()
            .tasks()
            .unwrap_or_else(|e| panic!("{e}"))
            .is_empty()
    );
    let result: Result<usize, AppError> = store.write_with(|w| {
        w.insert_task(&task)?;
        Ok(w.read().tasks()?.len())
    });
    assert!(matches!(result, Ok(1)));
    let dup: Result<(), AppError> = store.write_with(|w| Ok(w.insert_task(&task)?));
    assert!(matches!(
        dup,
        Err(AppError::Store(
            tally_store::StoreError::AlreadyExists { .. }
        ))
    ));
}
