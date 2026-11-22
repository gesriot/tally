//! Модель отображения: боковая панель и страницы из снимка.

mod common;

use common::{Result, TODAY, find, id_of, run, seed, snapshot};
use tally_app::{Close, Command};
use tally_core::Status;
use tally_store::Store;
use tally_ui::view::{self, Icon, Mark, RowKind, Selection, SideKind};

fn seeded() -> Result<Store> {
    let mut store = Store::open_in_memory()?;
    seed(&mut store)?;
    Ok(store)
}

fn titles(page: &view::Page) -> Vec<String> {
    page.rows
        .iter()
        .map(|r| match r.kind {
            RowKind::Section => format!("# {}", r.title),
            _ => r.title.clone(),
        })
        .collect()
}

#[test]
fn sidebar_lists_then_loose_projects_then_areas() -> Result {
    let s = snapshot(&seeded()?)?;
    let side = view::sidebar(&s, TODAY, Selection::Today);
    let shown: Vec<String> = side
        .iter()
        .map(|r| match r.kind {
            SideKind::Gap => "|".to_owned(),
            SideKind::Area => format!("[{}]", r.title),
            _ => r.title.clone(),
        })
        .collect();
    assert_eq!(
        shown,
        [
            "Входящие",
            "Сегодня",
            "Планы",
            "В любое время",
            "Когда-нибудь",
            "|",
            "Журнал",
            "Корзина",
            "|",
            "Отпуск",
            "|",
            "[Дом]",
            "Ремонт",
            "|",
            "[Работа]",
        ]
    );
    let today = &side[1];
    assert!(today.selected && today.due, "{today:?}");
    assert_eq!(today.count, "4");
    assert_eq!(side[0].count, "1");
    assert_eq!(side[2].count, "");
    assert!(!side[9].nested);
    assert!(side[12].nested);
    assert_eq!(side[12].icon, Icon::Project);
    assert_eq!(side[12].key, format!("project:{}", s.projects[0].id));
    Ok(())
}

#[test]
fn closed_and_trashed_containers_leave_the_sidebar() -> Result {
    let mut store = seeded()?;
    let s = snapshot(&store)?;
    let mut repair = s
        .projects
        .iter()
        .find(|p| p.title == "Ремонт")
        .cloned()
        .ok_or("нет")?;
    repair.status = Status::Completed {
        at: common::clock().now,
    };
    let work = s
        .areas
        .iter()
        .find(|a| a.title == "Работа")
        .ok_or("нет")?
        .id;
    store.write(|w| {
        w.update_project(&repair)?;
        w.delete(tally_store::RecordId::Area(work), common::clock().now)
    })?;
    let s = snapshot(&store)?;
    let side = view::sidebar(&s, TODAY, Selection::Project(repair.id));
    let titles: Vec<&str> = side.iter().map(|r| r.title.as_str()).collect();
    assert!(
        !titles.contains(&"Ремонт") && !titles.contains(&"Работа"),
        "{titles:?}"
    );
    // Выбранного проекта больше нет – открывается «Сегодня».
    assert_eq!(
        view::valid(&s, TODAY, Selection::Project(repair.id)),
        Selection::Today
    );
    assert_eq!(
        view::valid(&s, TODAY, Selection::Area(work)),
        Selection::Today
    );
    Ok(())
}

#[test]
fn today_has_an_evening_block_and_containers() -> Result {
    let s = snapshot(&seeded()?)?;
    let page = view::page(&s, TODAY, Selection::Today, &[]);
    assert_eq!(page.title, "Сегодня");
    assert_eq!(
        titles(&page),
        [
            "Купить плитку",
            "Позвонить электрику",
            "Оплатить интернет",
            "# Вечер",
            "Позвонить маме",
        ]
    );
    let tile = &page.rows[0];
    assert_eq!(tile.meta, "Ремонт / Кухня");
    assert_eq!(tile.deadline, "пн, 23 ноября");
    assert!(!tile.due && tile.checkable && tile.mark == Mark::Open);
    let internet = &page.rows[2];
    assert_eq!(internet.meta, "Дом · #счета");
    assert_eq!(internet.deadline, "сегодня");
    assert!(internet.due);
    assert_eq!(page.rows[3].kind, RowKind::Section);
    Ok(())
}

#[test]
fn a_project_page_is_grouped_by_headings() -> Result {
    let mut store = seeded()?;
    run(
        &mut store,
        Command::Close(Close {
            tasks: vec!["Выбрать краску".into()],
            cancel: false,
        }),
    )?;
    let s = snapshot(&store)?;
    let repair = s
        .projects
        .iter()
        .find(|p| p.title == "Ремонт")
        .ok_or("нет")?
        .id;
    let page = view::page(&s, TODAY, Selection::Project(repair), &[]);
    assert_eq!(page.title, "Ремонт");
    // Выполненная «Выбрать краску» – в «Журнале», а не здесь.
    assert_eq!(
        titles(&page),
        [
            "Позвонить электрику",
            "# Кухня",
            "Купить плитку",
            "# Ванная",
            "Заказать смеситель",
        ]
    );
    // Проект не повторяется в строках, «когда» – там, где это не
    // «В любое время».
    assert_eq!(page.rows[0].meta, "");
    assert_eq!(page.rows[0].when, "Сегодня");
    assert_eq!(page.rows[4].when, "сб, 21 ноября");
    Ok(())
}

#[test]
fn an_area_page_shows_its_projects_then_its_tasks() -> Result {
    let s = snapshot(&seeded()?)?;
    let home = s.areas.iter().find(|a| a.title == "Дом").ok_or("нет")?.id;
    let page = view::page(&s, TODAY, Selection::Area(home), &[]);
    assert_eq!(titles(&page), ["Ремонт", "Оплатить интернет"]);
    assert_eq!(page.rows[0].kind, RowKind::Project);
    assert_eq!(page.rows[0].meta, "0 из 4");
    assert_eq!(page.rows[1].meta, "#счета");
    Ok(())
}

#[test]
fn upcoming_always_shows_seven_days() -> Result {
    let s = snapshot(&seeded()?)?;
    let page = view::page(&s, TODAY, Selection::Upcoming, &[]);
    assert_eq!(
        titles(&page),
        [
            "# сб, 21 ноября",
            "Заказать смеситель",
            "# вс, 22 ноября",
            "# пн, 23 ноября",
            "Купить плитку",
            "# вт, 24 ноября",
            "# ср, 25 ноября",
            "# чт, 26 ноября",
            "# пт, 27 ноября",
        ]
    );
    // В день срока – пометка «срок» вместо даты срока.
    assert_eq!(page.rows[4].deadline, "срок");
    Ok(())
}

#[test]
fn anytime_marks_tasks_that_are_also_in_today() -> Result {
    let s = snapshot(&seeded()?)?;
    let page = view::page(&s, TODAY, Selection::Anytime, &[]);
    let stars: Vec<(&str, bool)> = page
        .rows
        .iter()
        .map(|r| (r.title.as_str(), r.star))
        .collect();
    assert!(stars.contains(&("Купить плитку", true)), "{stars:?}");
    assert!(stars.contains(&("Посмотреть фильм", false)), "{stars:?}");
    Ok(())
}

#[test]
fn a_task_closed_in_this_window_stays_checked_in_place() -> Result {
    let mut store = seeded()?;
    run(
        &mut store,
        Command::Close(Close {
            tasks: vec!["Позвонить электрику".into()],
            cancel: false,
        }),
    )?;
    let s = snapshot(&store)?;
    let id = id_of(&s, "Позвонить электрику")?;
    let plain = view::page(&s, TODAY, Selection::Today, &[]);
    assert!(!titles(&plain).contains(&"Позвонить электрику".to_owned()));
    let kept = view::page(&s, TODAY, Selection::Today, &[id]);
    let row = &kept.rows[1];
    assert_eq!(row.title, "Позвонить электрику");
    assert_eq!(row.mark, Mark::Completed);
    assert!(row.checkable);
    // В «Журнале» отметка не нажимается: задачу оттуда не открыть.
    let logbook = view::page(&s, TODAY, Selection::Logbook, &[]);
    assert_eq!(titles(&logbook), ["Позвонить электрику"]);
    assert!(!logbook.rows[0].checkable);
    assert_eq!(logbook.rows[0].mark, Mark::Completed);
    Ok(())
}

#[test]
fn trash_shows_deleted_records_without_checkboxes() -> Result {
    let mut store = seeded()?;
    let s = snapshot(&store)?;
    let film = find(&s, "Посмотреть фильм")?.id;
    let kitchen = s
        .headings
        .iter()
        .find(|h| h.title == "Кухня")
        .ok_or("нет")?
        .id;
    store.write(|w| {
        w.delete(tally_store::RecordId::Task(film), common::clock().now)?;
        w.delete(tally_store::RecordId::Heading(kitchen), common::clock().now)
    })?;
    let s = snapshot(&store)?;
    let page = view::page(&s, TODAY, Selection::Trash, &[]);
    assert_eq!(page.title, "Корзина");
    let rows: Vec<(&str, RowKind, bool)> = page
        .rows
        .iter()
        .map(|r| (r.title.as_str(), r.kind, r.checkable))
        .collect();
    assert!(
        rows.contains(&("Посмотреть фильм", RowKind::Task, false)),
        "{rows:?}"
    );
    assert!(
        rows.contains(&("Кухня – раздел проекта «Ремонт»", RowKind::Other, false)),
        "{rows:?}"
    );
    Ok(())
}

#[test]
fn selection_keys_round_trip() -> Result {
    let s = snapshot(&seeded()?)?;
    let p = s.projects[0].id;
    let a = s.areas[0].id;
    for sel in [
        Selection::Inbox,
        Selection::Today,
        Selection::Upcoming,
        Selection::Anytime,
        Selection::Someday,
        Selection::Logbook,
        Selection::Trash,
        Selection::Project(p),
        Selection::Area(a),
    ] {
        assert_eq!(Selection::parse(&sel.key()), Some(sel));
    }
    assert_eq!(Selection::parse("project:не-uuid"), None);
    assert_eq!(Selection::parse("всё"), None);
    Ok(())
}
