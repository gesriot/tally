//! Целостность: раздел и проект задачи, каскад удаления, надгробия чек-листа,
//! очистка только из корзины, атомарность операций, миграция и чтение
//! при нескольких соединениях.

mod common;

use std::sync::{Arc, Barrier};

use common::*;
use jiff::civil::{Date, date};
use tally_core::lists::Item;
use tally_core::{AreaId, ChecklistItem, ChecklistItemId, Status, TagId};
use tally_store::{RecordId, SCHEMA_VERSION, Snapshot, Store, StoreError};

const TODAY: Date = date(2026, 11, 19);

fn store() -> Store {
    Store::open_in_memory().unwrap_or_else(|e| panic!("база в памяти: {e}"))
}

fn titles(items: &[Item<'_>]) -> Vec<String> {
    items.iter().map(|i| i.title().to_owned()).collect()
}

fn snapshot(s: &Store) -> Snapshot {
    s.read()
        .snapshot()
        .unwrap_or_else(|e| panic!("снимок: {e}"))
}

fn found(s: &Store, q: &str) -> Vec<String> {
    s.read()
        .search(q)
        .unwrap_or_default()
        .into_iter()
        .map(|h| h.title)
        .collect()
}

fn task_titles(s: &Store) -> Vec<String> {
    s.read()
        .tasks()
        .unwrap_or_default()
        .into_iter()
        .map(|t| t.title)
        .collect()
}

// --- 1. Раздел задачи – раздел её проекта ------------------------------------

/// Задача проекта Б с разделом проекта А. Раньше она
/// сохранялась и пропадала после удаления А и очистки корзины.
#[test]
fn task_cannot_take_a_heading_of_another_project() {
    let mut s = store();
    let a = project(1, "А");
    let b = project(2, "Б");
    let h = heading(10, a.id, "Раздел А");
    let mut stray = task(100, "Задача Б");
    stray.project = Some(b.id);
    stray.heading = Some(h.id);
    assert!(
        s.write(|w| {
            w.insert_project(&a)?;
            w.insert_project(&b)?;
            w.insert_heading(&h)
        })
        .is_ok()
    );
    assert!(matches!(
        s.write(|w| w.insert_task(&stray)),
        Err(StoreError::HeadingOutsideProject { task, heading })
            if task == stray.id.0 && heading == h.id.0
    ));
    // Без проекта раздел тоже нельзя.
    let mut no_project = task(101, "Без проекта");
    no_project.heading = Some(h.id);
    assert!(matches!(
        s.write(|w| w.insert_task(&no_project)),
        Err(StoreError::HeadingOutsideProject { .. })
    ));
    // Обновление: задача раздела А переезжает в Б, а раздел остаётся прежним.
    let mut moved = task(102, "Была в А");
    moved.project = Some(a.id);
    moved.heading = Some(h.id);
    assert!(s.write(|w| w.insert_task(&moved)).is_ok());
    moved.project = Some(b.id);
    assert!(matches!(
        s.write(|w| w.update_task(&moved)),
        Err(StoreError::HeadingOutsideProject { .. })
    ));
    // Корректно: в Б без раздела. Удаление А и очистка корзины её не трогают.
    moved.heading = None;
    stray.heading = None;
    assert!(
        s.write(|w| {
            w.update_task(&moved)?;
            w.insert_task(&stray)?;
            w.delete(RecordId::Project(a.id), t0())?;
            w.empty_trash()
        })
        .is_ok()
    );
    let mut left = task_titles(&s);
    left.sort();
    assert_eq!(left, ["Была в А", "Задача Б"]);
}

/// Мимо проверки в Rust – прямым SQL: связь держит сама схема.
#[test]
fn schema_rejects_heading_of_another_project() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("heading.db");
    {
        let mut s = Store::open(&path).unwrap_or_else(|e| panic!("{e}"));
        let a = project(1, "А");
        let mut t = task(100, "Задача А");
        t.project = Some(a.id);
        assert!(
            s.write(|w| {
                w.insert_project(&a)?;
                w.insert_project(&project(2, "Б"))?;
                w.insert_heading(&heading(10, a.id, "Раздел А"))?;
                w.insert_task(&t)
            })
            .is_ok()
        );
    }
    let conn = rusqlite::Connection::open(&path).unwrap_or_else(|e| panic!("{e}"));
    assert!(conn.pragma_update(None, "foreign_keys", true).is_ok());
    let a = tally_core::ProjectId::from_u128(1).0.to_string();
    let b = tally_core::ProjectId::from_u128(2).0.to_string();
    let h = tally_core::HeadingId::from_u128(10).0.to_string();
    // Раздел А у задачи проекта Б.
    assert!(
        conn.execute(
            "UPDATE tasks SET project_id = ?1, heading_id = ?2",
            [&b, &h]
        )
        .is_err()
    );
    // Раздел без проекта.
    assert!(
        conn.execute("UPDATE tasks SET project_id = NULL, heading_id = ?1", [&h])
            .is_err()
    );
    // Свой раздел – можно.
    assert!(
        conn.execute(
            "UPDATE tasks SET project_id = ?1, heading_id = ?2",
            [&a, &h]
        )
        .is_ok()
    );
}

/// Перенос раздела в другой проект переносит и его задачи (как в Things):
/// после удаления старого проекта и очистки корзины они на месте.
#[test]
fn moving_a_heading_moves_its_tasks() {
    let mut s = store();
    let a = project(1, "А");
    let b = project(2, "Б");
    let mut h = heading(10, a.id, "Покупки");
    let mut t = task(100, "Купить краску");
    t.project = Some(a.id);
    t.heading = Some(h.id);
    assert!(
        s.write(|w| {
            w.insert_project(&a)?;
            w.insert_project(&b)?;
            w.insert_heading(&h)?;
            w.insert_task(&t)
        })
        .is_ok()
    );
    h.project = b.id;
    assert!(s.write(|w| w.update_heading(&h)).is_ok());
    let moved = s.read().task(t.id).ok().flatten();
    assert_eq!(
        moved.as_ref().map(|t| (t.project, t.heading)),
        Some((Some(b.id), Some(h.id)))
    );
    // Задача из снимка сохраняется обратно без ошибки.
    if let Some(moved) = moved {
        assert!(s.write(|w| w.update_task(&moved)).is_ok());
    }
    assert!(
        s.write(|w| {
            w.delete(RecordId::Project(a.id), t0())?;
            w.empty_trash()
        })
        .is_ok()
    );
    assert_eq!(task_titles(&s), ["Купить краску"]);
    assert_eq!(
        s.read().heading(h.id).ok().flatten().map(|h| h.project),
        Some(b.id)
    );
}

// --- 2. Удаление области: списки, поиск, очистка ------------------------------

#[test]
fn area_delete_restore_purge_cycle() {
    let mut s = store();
    let home = area(1, "Дом");
    let mut repair = project(10, "Ремонт кухни");
    repair.area = Some(home.id);
    let buy = heading(20, repair.id, "Покупки");
    let mut paint = task(100, "Купить краску");
    paint.project = Some(repair.id);
    paint.heading = Some(buy.id);
    paint.date = Some(TODAY);
    let mut plants = task(101, "Полить цветы");
    plants.area = Some(home.id);
    plants.date = Some(TODAY);
    let mut done = task(102, "Снять плитку");
    done.project = Some(repair.id);
    done.status = Status::Completed { at: t0() };
    let mut inbox = task(103, "Позвонить маме");
    inbox.date = Some(TODAY);
    assert!(
        s.write(|w| {
            w.insert_area(&home)?;
            w.insert_project(&repair)?;
            w.insert_heading(&buy)?;
            for t in [&paint, &plants, &done, &inbox] {
                w.insert_task(t)?;
            }
            Ok(())
        })
        .is_ok()
    );
    let before = snapshot(&s);
    let lists = before.lists(TODAY);
    assert_eq!(
        titles(&lists.today.day),
        ["Купить краску", "Полить цветы", "Позвонить маме"]
    );
    assert_eq!(titles(&lists.logbook), ["Снять плитку"]);
    assert_eq!(found(&s, "кра"), ["Купить краску"]);

    // Удалили область – всё внутри сразу ушло из списков и поиска.
    assert!(s.write(|w| w.delete(RecordId::Area(home.id), t0())).is_ok());
    let deleted = snapshot(&s);
    let lists = deleted.lists(TODAY);
    assert_eq!(titles(&lists.today.day), ["Позвонить маме"]);
    assert!(lists.logbook.is_empty());
    assert_eq!(titles(&lists.trash), ["Дом"]);
    assert!(found(&s, "кра").is_empty());
    assert!(found(&s, "цвет").is_empty());
    assert!(found(&s, "ремонт").is_empty());

    // Вернули – всё как было.
    assert!(
        s.write(|w| w.restore(RecordId::Area(home.id), t0()))
            .is_ok()
    );
    assert_eq!(snapshot(&s), before);
    assert_eq!(found(&s, "кра"), ["Купить краску"]);

    // Снова удалили и стёрли саму область – содержимое ушло вместе с ней.
    assert!(
        s.write(|w| {
            w.delete(RecordId::Area(home.id), t0())?;
            w.purge(RecordId::Area(home.id))
        })
        .is_ok()
    );
    let after = snapshot(&s);
    assert!(after.areas.is_empty() && after.projects.is_empty() && after.headings.is_empty());
    assert_eq!(task_titles(&s), ["Позвонить маме"]);
    assert!(after.lists(TODAY).trash.is_empty());
}

// --- 3. Удалённый раздел -------------------------------------------------------

#[test]
fn deleted_heading_hides_its_tasks_in_lists_and_search() {
    let mut s = store();
    let repair = project(10, "Ремонт кухни");
    let buy = heading(20, repair.id, "Покупки");
    let mut paint = task(100, "Купить краску");
    paint.project = Some(repair.id);
    paint.heading = Some(buy.id);
    paint.date = Some(TODAY);
    let mut plan = task(101, "Начертить план");
    plan.project = Some(repair.id);
    plan.date = Some(TODAY);
    assert!(
        s.write(|w| {
            w.insert_project(&repair)?;
            w.insert_heading(&buy)?;
            w.insert_task(&paint)?;
            w.insert_task(&plan)
        })
        .is_ok()
    );
    let before = snapshot(&s);
    assert_eq!(
        titles(&before.lists(TODAY).today.day),
        ["Купить краску", "Начертить план"]
    );

    assert!(
        s.write(|w| w.delete(RecordId::Heading(buy.id), t0()))
            .is_ok()
    );
    let deleted = snapshot(&s);
    let lists = deleted.lists(TODAY);
    assert_eq!(titles(&lists.today.day), ["Начертить план"]);
    assert_eq!(titles(&lists.trash), ["Покупки"]);
    assert!(found(&s, "краск").is_empty());
    assert_eq!(found(&s, "план"), ["Начертить план"]);

    assert!(
        s.write(|w| w.restore(RecordId::Heading(buy.id), t0()))
            .is_ok()
    );
    assert_eq!(snapshot(&s), before);
    assert_eq!(found(&s, "краск"), ["Купить краску"]);
}

// --- 4. Чек-лист и очистка -------------------------------------------------------

#[test]
fn removed_checklist_item_is_a_tombstone_until_trash_is_emptied() {
    let mut s = store();
    let mut t = task(100, "Собрать шкаф");
    t.checklist = vec![
        ChecklistItem::new(
            ChecklistItemId::from_u128(1),
            "Найти шуруповёрт",
            rank("a0"),
            t0(),
        ),
        ChecklistItem::new(
            ChecklistItemId::from_u128(2),
            "Прочитать схему",
            rank("a1"),
            t0(),
        ),
    ];
    assert!(s.write(|w| w.insert_task(&t)).is_ok());
    let later = at("2026-11-19T15:00:00Z");
    t.checklist.remove(0);
    t.sync.touch(later);
    assert!(s.write(|w| w.update_task(&t)).is_ok());

    let stored = s.read().task(t.id).ok().flatten();
    let checklist = stored.map(|t| t.checklist).unwrap_or_default();
    assert_eq!(checklist.len(), 2, "надгробие осталось в базе");
    assert_eq!(checklist[0].sync.deleted_at, Some(later));
    assert_eq!(checklist[0].sync.updated_at, later);
    assert_eq!(checklist[1].sync.deleted_at, None);
    // Удалённый пункт поиск не находит.
    assert!(found(&s, "шуруповерт").is_empty());
    assert_eq!(found(&s, "схему"), ["Собрать шкаф"]);

    // Сохранение задачи со снимка (с надгробием в массиве) ничего не меняет.
    if let Some(again) = s.read().task(t.id).ok().flatten() {
        assert!(s.write(|w| w.update_task(&again)).is_ok());
        assert_eq!(s.read().task(t.id).ok().flatten(), Some(again));
    }

    // Очистка корзины стирает надгробия и у живой задачи.
    let purged = s.write(|w| w.empty_trash()).unwrap_or_default();
    assert_eq!((purged.checklist_items, purged.tasks), (1, 0));
    let left: Vec<String> = s
        .read()
        .task(t.id)
        .ok()
        .flatten()
        .map(|t| t.checklist.into_iter().map(|c| c.title).collect())
        .unwrap_or_default();
    assert_eq!(left, ["Прочитать схему"]);
}

#[test]
fn checklist_item_cannot_jump_to_another_task() {
    let mut s = store();
    let item = ChecklistItem::new(ChecklistItemId::from_u128(1), "Пункт", rank("a0"), t0());
    let mut first = task(100, "Первая");
    first.checklist = vec![item.clone()];
    let mut second = task(101, "Вторая");
    second.checklist = vec![item];
    assert!(s.write(|w| w.insert_task(&first)).is_ok());
    assert!(matches!(
        s.write(|w| w.insert_task(&second)),
        Err(StoreError::AlreadyExists { .. })
    ));
}

#[test]
fn purge_takes_only_what_is_in_the_trash() {
    let mut s = store();
    let home = area(1, "Дом");
    let mut repair = project(10, "Ремонт");
    repair.area = Some(home.id);
    let buy = heading(20, repair.id, "Покупки");
    let mut paint = task(100, "Краска");
    paint.project = Some(repair.id);
    paint.heading = Some(buy.id);
    let live = task(101, "Живая");
    let urgent = tag(5, "Срочно");
    assert!(
        s.write(|w| {
            w.insert_area(&home)?;
            w.insert_project(&repair)?;
            w.insert_heading(&buy)?;
            w.insert_task(&paint)?;
            w.insert_task(&live)?;
            w.insert_tag(&urgent)
        })
        .is_ok()
    );
    for id in [
        RecordId::Task(live.id),
        RecordId::Task(paint.id),
        RecordId::Heading(buy.id),
        RecordId::Project(repair.id),
        RecordId::Area(home.id),
        RecordId::Tag(urgent.id),
    ] {
        assert!(
            matches!(s.write(|w| w.purge(id)), Err(StoreError::NotInTrash { .. })),
            "{id:?}"
        );
    }
    assert!(matches!(
        s.write(|w| w.purge(RecordId::Task(tally_core::TaskId::from_u128(999)))),
        Err(StoreError::NotFound { .. })
    ));
    assert_eq!(task_titles(&s).len(), 2);

    // Через удалённую область в корзине и проект, и раздел, и задача.
    assert!(s.write(|w| w.delete(RecordId::Area(home.id), t0())).is_ok());
    assert!(s.write(|w| w.purge(RecordId::Task(paint.id))).is_ok());
    assert!(s.write(|w| w.purge(RecordId::Heading(buy.id))).is_ok());
    assert!(s.write(|w| w.purge(RecordId::Project(repair.id))).is_ok());
    assert!(
        s.write(|w| w.delete(RecordId::Tag(urgent.id), t0()))
            .is_ok()
    );
    assert!(s.write(|w| w.purge(RecordId::Tag(urgent.id))).is_ok());
    assert_eq!(task_titles(&s), ["Живая"]);
    // Область всё ещё в корзине.
    assert!(s.write(|w| w.purge(RecordId::Area(home.id))).is_ok());
}

// --- 5. Миграция при одновременном открытии ------------------------------------

#[test]
fn two_connections_migrate_a_fresh_file_once() {
    for round in 0..20 {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join(format!("race-{round}.db"));
        let start = Arc::new(Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let path = path.clone();
                let start = Arc::clone(&start);
                std::thread::spawn(move || {
                    start.wait();
                    Store::open(&path).and_then(|s| s.schema_version())
                })
            })
            .collect();
        for h in handles {
            let version = h.join().map_err(|_| "поток упал");
            assert!(
                matches!(version, Ok(Ok(v)) if v == SCHEMA_VERSION),
                "раунд {round}: {version:?}"
            );
        }
    }
}

// --- 6. Согласованный снимок при параллельной записи ---------------------------

#[test]
fn snapshot_and_search_see_one_state_while_another_connection_writes() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("wal.db");
    let reader = Store::open(&path).unwrap_or_else(|e| panic!("{e}"));
    let writer_path = path.clone();
    const PAIRS: u128 = 150;
    let writer = std::thread::spawn(move || {
        let mut s = Store::open(&writer_path).unwrap_or_else(|e| panic!("{e}"));
        for n in 1..=PAIRS {
            // Проект и его задача – одной транзакцией.
            let p = project(n, &format!("Пара {n}"));
            let mut t = task(n, &format!("Пара {n}"));
            t.project = Some(p.id);
            let ok = s.write(|w| {
                w.insert_project(&p)?;
                w.insert_task(&t)
            });
            assert!(ok.is_ok(), "{ok:?}");
        }
    });
    let mut checks = 0;
    loop {
        let done = writer.is_finished();
        let snap = snapshot(&reader);
        assert_eq!(snap.projects.len(), snap.tasks.len(), "снимок {checks}");
        for t in &snap.tasks {
            assert!(
                snap.projects.iter().any(|p| Some(p.id) == t.project),
                "задача без проекта в снимке {checks}"
            );
        }
        let hits = reader.read().search("пара").unwrap_or_default();
        let tasks = hits
            .iter()
            .filter(|h| matches!(h.found, tally_store::Found::Task(_)))
            .count();
        assert_eq!(hits.len(), tasks * 2, "поиск {checks}");
        checks += 1;
        if done {
            break;
        }
    }
    assert!(writer.join().is_ok());
    assert_eq!(snapshot(&reader).tasks.len() as u128, PAIRS);
}

// --- 7. Атомарность операций внутри write ---------------------------------------

/// Ошибка `update_task` поймана внутри `write`, замыкание
/// вернуло `Ok`. Раньше фиксировалась половина: новое название и пропавшие
/// старые теги.
#[test]
fn caught_error_does_not_commit_half_an_operation() {
    let mut s = store();
    let urgent = tag(1, "Срочно");
    let mut t = task(100, "Старое название");
    t.tags = vec![urgent.id];
    assert!(
        s.write(|w| {
            w.insert_tag(&urgent)?;
            w.insert_task(&t)
        })
        .is_ok()
    );
    let before = s.read().task(t.id).ok().flatten();

    let mut edited = t.clone();
    edited.title = "Новое название".into();
    edited.tags = vec![urgent.id, TagId::from_u128(404)]; // второго тега нет
    let caught = s.write(|w| match w.update_task(&edited) {
        Err(StoreError::MissingReference { .. }) => Ok(true),
        other => other.map(|()| false),
    });
    assert_eq!(caught.ok(), Some(true));
    assert_eq!(s.read().task(t.id).ok().flatten(), before);
    assert_eq!(
        s.read()
            .task(t.id)
            .ok()
            .flatten()
            .map(|t| (t.title, t.tags)),
        Some(("Старое название".to_owned(), vec![urgent.id]))
    );
}

// --- Ревью 2. Область задачи в проекте – область проекта ----------------------

/// Две области, по проекту в каждой; задача в разделе проекта «Дома».
fn two_areas_with_a_heading() -> (Store, tally_core::Heading, tally_core::Task) {
    let mut s = store();
    let home = area(1, "Дом");
    let work = area(2, "Работа");
    let mut repair = project(10, "Ремонт");
    repair.area = Some(home.id);
    let mut office = project(11, "Офис");
    office.area = Some(work.id);
    let h = heading(20, repair.id, "Покупки");
    let mut t = task(100, "Купить краску");
    t.project = Some(repair.id);
    t.heading = Some(h.id);
    t.date = Some(TODAY);
    // Исходная область задано явно – так раньше сохранял и CRUD-тест.
    t.area = Some(home.id);
    assert!(
        s.write(|w| {
            w.insert_area(&home)?;
            w.insert_area(&work)?;
            w.insert_project(&repair)?;
            w.insert_project(&office)?;
            w.insert_heading(&h)
        })
        .is_ok()
    );
    assert!(matches!(
        s.write(|w| w.insert_task(&t)),
        Err(StoreError::AreaWithProject { task }) if task == t.id.0
    ));
    t.area = None;
    assert!(s.write(|w| w.insert_task(&t)).is_ok());

    // Раздел с задачей переезжает в «Офис» (область «Работа»).
    let mut moved = h.clone();
    moved.project = office.id;
    moved.sync.touch(at("2026-11-19T15:00:00Z"));
    assert!(s.write(|w| w.update_heading(&moved)).is_ok());
    let t = s
        .read()
        .task(t.id)
        .ok()
        .flatten()
        .unwrap_or_else(|| panic!("задача пропала при переносе"));
    (s, moved, t)
}

#[test]
fn moving_a_heading_to_another_area_and_deleting_the_old_area_keeps_the_task() {
    let (mut s, moved, t) = two_areas_with_a_heading();
    assert_eq!(t.project, Some(moved.project));
    assert_eq!(t.area, None);
    // Перенос виден выборке изменений по времени.
    assert_eq!(t.sync.updated_at, at("2026-11-19T15:00:00Z"));

    // Удалили старую область «Дом» и очистили корзину – задача на месте.
    assert!(
        s.write(|w| {
            w.delete(RecordId::Area(AreaId::from_u128(1)), t0())?;
            w.empty_trash()
        })
        .is_ok()
    );
    assert_eq!(task_titles(&s), ["Купить краску"]);
    assert_eq!(
        titles(&snapshot(&s).lists(TODAY).today.day),
        ["Купить краску"]
    );
    assert_eq!(found(&s, "краск"), ["Купить краску"]);
}

#[test]
fn moving_a_heading_to_another_area_and_deleting_the_new_area_trashes_the_task_at_once() {
    let (mut s, _, _) = two_areas_with_a_heading();
    assert!(
        s.write(|w| w.delete(RecordId::Area(AreaId::from_u128(2)), t0()))
            .is_ok()
    );
    // Сразу, до очистки: ни в «Сегодня», ни в поиске; в корзине – «Работа».
    let snap = snapshot(&s);
    let lists = snap.lists(TODAY);
    assert!(lists.today.day.is_empty());
    assert_eq!(titles(&lists.trash), ["Работа"]);
    assert!(found(&s, "краск").is_empty());
    // Очистка уносит задачу вместе с областью, в которой она теперь.
    let purged = s.write(|w| w.empty_trash()).unwrap_or_default();
    assert_eq!((purged.areas, purged.projects, purged.tasks), (1, 1, 1));
    assert!(task_titles(&s).is_empty());
}

#[test]
fn moving_a_project_to_another_area_takes_its_tasks() {
    let mut s = store();
    let home = area(1, "Дом");
    let work = area(2, "Работа");
    let mut repair = project(10, "Ремонт");
    repair.area = Some(home.id);
    let mut t = task(100, "Купить краску");
    t.project = Some(repair.id);
    t.date = Some(TODAY);
    assert!(
        s.write(|w| {
            w.insert_area(&home)?;
            w.insert_area(&work)?;
            w.insert_project(&repair)?;
            w.insert_task(&t)
        })
        .is_ok()
    );
    repair.area = Some(work.id);
    assert!(s.write(|w| w.update_project(&repair)).is_ok());
    // Строка задачи не меняется: область она берёт у проекта.
    assert_eq!(s.read().task(t.id).ok().flatten(), Some(t));
    assert!(
        s.write(|w| {
            w.delete(RecordId::Area(home.id), t0())?;
            w.empty_trash()
        })
        .is_ok()
    );
    assert_eq!(task_titles(&s), ["Купить краску"]);
    assert!(s.write(|w| w.delete(RecordId::Area(work.id), t0())).is_ok());
    assert!(snapshot(&s).lists(TODAY).today.day.is_empty());
    assert!(found(&s, "краск").is_empty());
}

#[test]
fn schema_rejects_own_area_of_a_task_in_a_project() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("area.db");
    {
        let mut s = Store::open(&path).unwrap_or_else(|e| panic!("{e}"));
        let p = project(10, "Ремонт");
        let mut t = task(100, "Краска");
        t.project = Some(p.id);
        assert!(
            s.write(|w| {
                w.insert_area(&area(1, "Дом"))?;
                w.insert_project(&p)?;
                w.insert_task(&t)
            })
            .is_ok()
        );
    }
    let conn = rusqlite::Connection::open(&path).unwrap_or_else(|e| panic!("{e}"));
    let home = AreaId::from_u128(1).0.to_string();
    assert!(
        conn.execute("UPDATE tasks SET area_id = ?1", [&home])
            .is_err()
    );
    assert!(
        conn.execute("UPDATE tasks SET project_id = NULL, area_id = ?1", [&home])
            .is_ok()
    );
}

// --- Ревью 2.1. Повтор id пункта чек-листа -----------------------------------

#[test]
fn duplicate_checklist_ids_are_rejected_and_nothing_is_saved() {
    let mut s = store();
    let same = ChecklistItemId::from_u128(1);
    let mut t = task(100, "Покрасить стену");
    t.checklist = vec![
        ChecklistItem::new(same, "Кисть", rank("a0"), t0()),
        ChecklistItem::new(same, "Валик", rank("a1"), t0()),
    ];
    assert!(matches!(
        s.write(|w| w.insert_task(&t)),
        Err(StoreError::DuplicateChecklistItem { task, item }) if task == t.id.0 && item == same.0
    ));
    assert_eq!(s.read().task(t.id).ok(), Some(None));
    assert!(found(&s, "кисть").is_empty());

    // Обновление: ошибка откатывает всю операцию, и название, и чек-лист.
    t.checklist.truncate(1);
    assert!(s.write(|w| w.insert_task(&t)).is_ok());
    let before = s.read().task(t.id).ok().flatten();
    let mut edited = t.clone();
    edited.title = "Покрасить потолок".into();
    edited.checklist = vec![
        ChecklistItem::new(same, "Валик", rank("a0"), t0()),
        ChecklistItem::new(same, "Кисть", rank("a1"), t0()),
    ];
    let caught = s.write(|w| match w.update_task(&edited) {
        Err(StoreError::DuplicateChecklistItem { .. }) => Ok(true),
        other => other.map(|()| false),
    });
    assert_eq!(caught.ok(), Some(true));
    assert_eq!(s.read().task(t.id).ok().flatten(), before);
    assert!(found(&s, "валик").is_empty());
    assert_eq!(found(&s, "кисть"), ["Покрасить стену"]);
}

// --- Ревью 2.3. Области и проекты читаются одним снимком ----------------------

#[test]
fn area_and_project_reads_see_one_state_while_another_connection_writes() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("records.db");
    let even = tag(1, "чёт");
    let odd = tag(2, "нечет");
    let mut a = area(1, "0");
    a.tags = vec![even.id];
    let mut p = project(10, "0");
    p.tags = vec![even.id];
    let mut reader = Store::open(&path).unwrap_or_else(|e| panic!("{e}"));
    assert!(
        reader
            .write(|w| {
                w.insert_tag(&even)?;
                w.insert_tag(&odd)?;
                w.insert_area(&a)?;
                w.insert_project(&p)
            })
            .is_ok()
    );
    let writer_path = path.clone();
    let writer = std::thread::spawn(move || {
        let mut s = Store::open(&writer_path).unwrap_or_else(|e| panic!("{e}"));
        for n in 1..=300u32 {
            // Название и тег меняются вместе, одной операцией.
            let parity = if n.is_multiple_of(2) { even.id } else { odd.id };
            a.title = n.to_string();
            a.tags = vec![parity];
            p.title = n.to_string();
            p.tags = vec![parity];
            let ok = s.write(|w| {
                w.update_area(&a)?;
                w.update_project(&p)
            });
            assert!(ok.is_ok(), "{ok:?}");
        }
    });
    // Каждый метод обязан прочитать успешно и ровно одну ожидаемую запись:
    // ошибка чтения или пустой результат – провал, а не пропуск проверки.
    let check = |what: &str, checks: u32, title: &str, tags: &[TagId]| {
        let n: u32 = title
            .parse()
            .unwrap_or_else(|_| panic!("{what}, чтение {checks}: название «{title}»"));
        let parity = TagId::from_u128(if n.is_multiple_of(2) { 1 } else { 2 });
        assert_eq!(tags, [parity], "{what}, чтение {checks}: «{title}»");
        n
    };
    let area_id = AreaId::from_u128(1);
    let project_id = tally_core::ProjectId::from_u128(10);
    let mut checks = 0;
    loop {
        let done = writer.is_finished();
        let r = reader.read();
        let one_area = r
            .area(area_id)
            .unwrap_or_else(|e| panic!("area(), чтение {checks}: {e}"))
            .unwrap_or_else(|| panic!("area(), чтение {checks}: области нет"));
        let all_areas = r
            .areas()
            .unwrap_or_else(|e| panic!("areas(), чтение {checks}: {e}"));
        let one_project = r
            .project(project_id)
            .unwrap_or_else(|e| panic!("project(), чтение {checks}: {e}"))
            .unwrap_or_else(|| panic!("project(), чтение {checks}: проекта нет"));
        let all_projects = r
            .projects()
            .unwrap_or_else(|e| panic!("projects(), чтение {checks}: {e}"));
        assert_eq!(
            all_areas.iter().map(|a| a.id).collect::<Vec<_>>(),
            [area_id],
            "areas(), чтение {checks}"
        );
        assert_eq!(
            all_projects.iter().map(|p| p.id).collect::<Vec<_>>(),
            [project_id],
            "projects(), чтение {checks}"
        );
        assert_eq!(one_area.id, area_id);
        assert_eq!(one_project.id, project_id);
        check("area()", checks, &one_area.title, &one_area.tags);
        check("areas()", checks, &all_areas[0].title, &all_areas[0].tags);
        check("project()", checks, &one_project.title, &one_project.tags);
        check(
            "projects()",
            checks,
            &all_projects[0].title,
            &all_projects[0].tags,
        );
        checks += 1;
        if done {
            break;
        }
    }
    assert!(writer.join().is_ok());
    // Последнее чтение – после завершения писателя: видно итоговое состояние.
    let last = reader
        .read()
        .project(project_id)
        .unwrap_or_else(|e| panic!("project(): {e}"))
        .unwrap_or_else(|| panic!("project(): проекта нет"));
    assert_eq!(last.title, "300");
}

// --- Ревью 3. Метка переноса раздела не старше изменений его задач -----------

#[test]
fn heading_move_with_a_stale_timestamp_is_refused_and_retry_succeeds() {
    let mut s = store();
    let a = project(1, "А");
    let b = project(2, "Б");
    let h = heading(10, a.id, "Покупки");
    assert!(
        s.write(|w| {
            w.insert_project(&a)?;
            w.insert_project(&b)?;
            w.insert_heading(&h)
        })
        .is_ok()
    );
    // Перенос подготовлен в 15:00…
    let mut moved = h.clone();
    moved.project = b.id;
    moved.sync.touch(at("2026-11-19T15:00:00Z"));
    // …в 15:01 в разделе создана задача…
    let mut t = tally_core::Task::new(
        tally_core::TaskId::from_u128(100),
        "Купить краску",
        rank("a0"),
        at("2026-11-19T15:01:00Z"),
    );
    t.project = Some(a.id);
    t.heading = Some(h.id);
    assert!(s.write(|w| w.insert_task(&t)).is_ok());
    let before = snapshot(&s);

    // …а перенос записан после неё: отказ, ничего не изменилось.
    assert!(matches!(
        s.write(|w| w.update_heading(&moved)),
        Err(StoreError::StaleTimestamp { kind: tally_store::Kind::Task, id, latest })
            if id == t.id.0 && latest == at("2026-11-19T15:01:00Z")
    ));
    assert_eq!(snapshot(&s), before);

    // Повтор с текущим временем проходит; у задачи время правки не уходит назад.
    moved.sync.touch(at("2026-11-19T15:02:00Z"));
    assert!(s.write(|w| w.update_heading(&moved)).is_ok());
    let stored = s
        .read()
        .task(t.id)
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|| panic!("задача пропала"));
    assert_eq!(stored.project, Some(b.id));
    assert_eq!(stored.sync.created_at, at("2026-11-19T15:01:00Z"));
    assert_eq!(stored.sync.updated_at, at("2026-11-19T15:02:00Z"));
}

#[test]
fn heading_move_older_than_the_heading_itself_is_refused() {
    let mut s = store();
    let a = project(1, "А");
    let b = project(2, "Б");
    let mut h = heading(10, a.id, "Покупки");
    assert!(
        s.write(|w| {
            w.insert_project(&a)?;
            w.insert_project(&b)?;
            w.insert_heading(&h)
        })
        .is_ok()
    );
    // Раздел переименовали в 15:05, а перенос подготовлен в 15:00.
    let mut moved = h.clone();
    moved.project = b.id;
    moved.sync.touch(at("2026-11-19T15:00:00Z"));
    h.title = "Закупки".into();
    h.sync.touch(at("2026-11-19T15:05:00Z"));
    assert!(s.write(|w| w.update_heading(&h)).is_ok());
    assert!(matches!(
        s.write(|w| w.update_heading(&moved)),
        Err(StoreError::StaleTimestamp { kind: tally_store::Kind::Heading, id, .. }) if id == h.id.0
    ));
    assert_eq!(s.read().heading(h.id).ok().flatten(), Some(h));
    // Ровно та же метка, что у последнего изменения, допустима.
    moved.title = "Закупки".into();
    moved.sync.touch(at("2026-11-19T15:05:00Z"));
    assert!(s.write(|w| w.update_heading(&moved)).is_ok());
}
