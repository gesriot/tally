//! Хранилище на настоящем файле: запись, повторное открытие, списки, поиск,
//! корзина, разделы, атомарность операций и надгробия чек-листа.
//! `cargo run -p tally-store --example store`.

use jiff::civil::date;
use tally_core::lists::Item;
use tally_core::{
    Area, AreaId, ChecklistItem, ChecklistItemId, Heading, HeadingId, Project, ProjectId, Rank,
    Start, Status, Task, TaskId, Timestamp,
};
use tally_store::{Found, RecordId, Store, StoreError};

fn ts(s: &str) -> Timestamp {
    s.parse().unwrap_or(Timestamp::UNIX_EPOCH)
}

fn title(i: &Item<'_>) -> String {
    match i {
        Item::Task(t) => t.title.clone(),
        Item::Project(p) => format!("[проект] {}", p.title),
        Item::Heading(h) => format!("[раздел] {}", h.title),
        Item::Area(a) => format!("[область] {}", a.title),
    }
}

/// Через « · », пустое – «–».
fn joined(items: &[String]) -> String {
    if items.is_empty() {
        "–".to_owned()
    } else {
        items.join(" · ")
    }
}

fn fill(store: &mut Store) -> Result<(), StoreError> {
    let at = ts("2026-11-10T09:00:00Z");
    let keys = Rank::spread(16);
    let key = |i: usize| keys[i].clone();

    let home = Area::new(AreaId::from_u128(1), "Дом", key(0), at);
    let mut tree = Project::new(ProjectId::from_u128(10), "Новогодняя ёлка", key(1), at);
    tree.area = Some(home.id);
    tree.deadline = Some(date(2026, 12, 30));
    let garage = Project::new(ProjectId::from_u128(11), "Разбор гаража", key(2), at);

    let mut n = 100u128;
    let mut mk = |title: &str, i: usize| {
        n += 1;
        Task::new(TaskId::from_u128(n), title, key(i), at)
    };
    let mut tasks = Vec::new();
    tasks.push(mk("Разобрать почту", 3));
    let mut t = mk("Купить ЁЛКУ", 4);
    t.project = Some(tree.id);
    t.date = Some(date(2026, 11, 19));
    t.notes = "Живую, не меньше двух метров".into();
    tasks.push(t);
    let mut t = mk("Позвонить Алёне", 5);
    t.start = Start::Anytime;
    t.checklist = vec![ChecklistItem::new(
        ChecklistItemId::from_u128(1),
        "Спросить про гирлянду",
        key(0),
        at,
    )];
    tasks.push(t);
    let mut t = mk("Ёлочные игрушки с антресоли", 6);
    t.project = Some(tree.id);
    t.date = Some(date(2026, 11, 22));
    tasks.push(t);
    let mut t = mk("Вывезти старые шины", 7);
    t.project = Some(garage.id);
    t.status = Status::Completed {
        at: ts("2026-11-12T15:00:00Z"),
    };
    tasks.push(t);

    store.write(|w| {
        w.insert_area(&home)?;
        w.insert_project(&tree)?;
        w.insert_project(&garage)?;
        for t in &tasks {
            w.insert_task(t)?;
        }
        Ok(())
    })
}

fn show_lists(store: &Store) -> Result<(), StoreError> {
    let snap = store.read().snapshot()?;
    let lists = snap.lists(date(2026, 11, 19));
    let line = |name: &str, items: Vec<String>| println!("  {name}: {}", joined(&items));
    line(
        "Входящие",
        lists.inbox.iter().map(|t| t.title.clone()).collect(),
    );
    line("Сегодня", lists.today.day.iter().map(title).collect());
    line(
        "Планы",
        lists
            .upcoming
            .iter()
            .flat_map(|g| g.entries.iter().map(|e| title(&e.item)))
            .collect(),
    );
    line(
        "В любое время",
        lists.anytime.iter().map(|e| e.task.title.clone()).collect(),
    );
    line("Журнал", lists.logbook.iter().map(title).collect());
    line("Корзина", lists.trash.iter().map(title).collect());
    Ok(())
}

fn show_search(store: &Store, query: &str) -> Result<(), StoreError> {
    let hits = store.read().search(query)?;
    let found: Vec<String> = hits
        .iter()
        .map(|h| {
            let kind = match h.found {
                Found::Area(_) => "область",
                Found::Project(_) => "проект",
                Found::Tag(_) => "тег",
                Found::Task(_) => "задача",
            };
            let place = if h.in_title {
                ""
            } else {
                ", в заметках"
            };
            let done = if h.open { "" } else { ", выполнено" };
            format!("{} ({kind}{place}{done})", h.title)
        })
        .collect();
    println!("  «{query}» → {}", joined(&found));
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("tally.db");

    {
        let mut store = Store::open(&path)?;
        println!("Новая база, схема v{}", store.schema_version()?);
        fill(&mut store)?;
    }
    let mut store = Store::open(&path)?;
    println!(
        "Открыта повторно: схема v{}, задач {}, проектов {}",
        store.schema_version()?,
        store.read().tasks()?.len(),
        store.read().projects()?.len()
    );

    println!("\nСписки на чт, 19 ноября 2026");
    show_lists(&store)?;

    println!("\nПоиск (регистр и «ё» не важны)");
    for q in [
        "ёлк",
        "ЕЛКУ",
        "гирлянду",
        "алён",
        "шины",
        "почту разобрать",
        "сосна",
    ] {
        show_search(&store, q)?;
    }

    let at = ts("2026-11-19T18:00:00Z");
    println!("\nПроект «Разбор гаража» удалён");
    store.write(|w| w.delete(RecordId::Project(ProjectId::from_u128(11)), at))?;
    show_lists(&store)?;
    show_search(&store, "шины")?;

    println!("\nПроект возвращён из корзины");
    store.write(|w| w.restore(RecordId::Project(ProjectId::from_u128(11)), at))?;
    show_lists(&store)?;

    println!("\nОбласть «Дом» удалена – с ней в корзине и проект «Новогодняя ёлка»");
    store.write(|w| w.delete(RecordId::Area(AreaId::from_u128(1)), at))?;
    show_lists(&store)?;
    show_search(&store, "ёлк")?;

    println!("\nОбласть возвращена");
    store.write(|w| w.restore(RecordId::Area(AreaId::from_u128(1)), at))?;
    show_lists(&store)?;
    show_search(&store, "ёлк")?;

    println!("\nПроект «Разбор гаража» удалён снова, корзина очищена");
    let purged = store.write(|w| {
        w.delete(RecordId::Project(ProjectId::from_u128(11)), at)?;
        w.empty_trash()
    })?;
    println!(
        "  удалено: проектов {}, задач {}, разделов {}",
        purged.projects, purged.tasks, purged.headings
    );
    show_lists(&store)?;

    println!("\nНеудачная транзакция откатывается целиком");
    let result = store.write(|w| {
        w.insert_task(&Task::new(
            TaskId::from_u128(900),
            "Не сохранится",
            Rank::initial(),
            at,
        ))?;
        w.insert_task(&Task::new(
            TaskId::from_u128(101),
            "Дубликат",
            Rank::initial(),
            at,
        ))
    });
    println!(
        "  ошибка: {}",
        result.err().map(|e| e.to_string()).unwrap_or_default()
    );
    println!(
        "  «Не сохранится» в базе: {}",
        store.read().task(TaskId::from_u128(900))?.is_some()
    );

    println!("\nРаздел «Украшения» в проекте «Новогодняя ёлка» удалён");
    let decor = Heading::new(
        HeadingId::from_u128(20),
        ProjectId::from_u128(10),
        "Украшения",
        Rank::initial(),
        at,
    );
    let toys_id = TaskId::from_u128(104);
    let mut toys = store.read().task(toys_id)?.ok_or(StoreError::NotFound {
        kind: tally_store::Kind::Task,
        id: toys_id.0,
    })?;
    toys.heading = Some(decor.id);
    store.write(|w| {
        w.insert_heading(&decor)?;
        w.update_task(&toys)?;
        w.delete(RecordId::Heading(decor.id), at)
    })?;
    show_lists(&store)?;
    show_search(&store, "игрушки")?;
    println!("\nРаздел возвращён");
    store.write(|w| w.restore(RecordId::Heading(decor.id), at))?;
    show_lists(&store)?;

    println!("\nЗадача проекта «Подарки» с разделом «Украшения» не сохраняется");
    let mut stray = Task::new(TaskId::from_u128(901), "Чужой раздел", Rank::initial(), at);
    let gifts = Project::new(ProjectId::from_u128(12), "Подарки", Rank::initial(), at);
    stray.project = Some(gifts.id);
    stray.heading = Some(decor.id);
    store.write(|w| w.insert_project(&gifts))?;
    let refused = store.write(|w| w.insert_task(&stray));
    println!(
        "  ошибка: {}",
        refused.err().map(|e| e.to_string()).unwrap_or_default()
    );

    println!("\nОшибка операции поймана внутри write – операция откатилась целиком");
    let mut renamed = toys.clone();
    renamed.title = "Переименовано".into();
    renamed.tags = vec![tally_core::TagId::from_u128(404)];
    store.write(|w| {
        let caught = w.update_task(&renamed);
        println!(
            "  поймано: {}",
            caught.err().map(|e| e.to_string()).unwrap_or_default()
        );
        Ok(())
    })?;
    println!(
        "  название в базе: {}",
        store
            .read()
            .task(toys_id)?
            .map(|t| t.title)
            .unwrap_or_default()
    );

    println!("\nПункт чек-листа убран – надгробие до очистки корзины");
    let call_id = TaskId::from_u128(103);
    if let Some(mut call) = store.read().task(call_id)? {
        call.checklist.clear();
        call.sync.touch(ts("2026-11-19T10:00:00Z"));
        store.write(|w| w.update_task(&call))?;
    }
    let kept = store
        .read()
        .task(call_id)?
        .map(|t| t.checklist.len())
        .unwrap_or_default();
    println!("  пунктов в базе (с надгробиями): {kept}");
    show_search(&store, "гирлянд")?;
    let purged = store.write(|w| w.empty_trash())?;
    println!(
        "  очистка корзины: пунктов чек-листа {}",
        purged.checklist_items
    );
    Ok(())
}
