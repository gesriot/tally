//! Правила списков в духе Things 3, включая смену суток.

use jiff::civil::{Date, date};
use tally_core::lists::{self, DAY_GROUPS, Group, Item, List, Reason, Via};
use tally_core::{Area, AreaId, Project, ProjectId, Rank, Start, Status, Task, TaskId, Timestamp};

const TODAY: Date = date(2026, 11, 19);
const TOMORROW: Date = date(2026, 11, 20);
const YESTERDAY: Date = date(2026, 11, 18);

fn at() -> Timestamp {
    "2026-11-19T09:00:00Z"
        .parse()
        .unwrap_or(Timestamp::UNIX_EPOCH)
}

fn rank(n: usize) -> Rank {
    Rank::spread(n + 1).pop().unwrap_or_else(Rank::initial)
}

fn task(n: u128, title: &str) -> Task {
    Task::new(TaskId::from_u128(n), title, rank(n as usize), at())
}

fn project(n: u128, title: &str) -> Project {
    Project::new(ProjectId::from_u128(n), title, rank(n as usize), at())
}

fn anytime(n: u128, title: &str) -> Task {
    let mut t = task(n, title);
    t.start = Start::Anytime;
    t
}

fn list(t: &Task, today: Date) -> List {
    lists::list_of(t, None, None, None, today)
}

#[test]
fn new_task_is_in_inbox() {
    assert_eq!(list(&task(1, "Новая"), TODAY), List::Inbox);
}

#[test]
fn inbox_task_with_date_leaves_inbox() {
    let mut t = task(1, "Из входящих");
    t.date = Some(TODAY);
    assert_eq!(list(&t, TODAY), List::Today { evening: false });
    t.date = Some(TOMORROW);
    assert_eq!(list(&t, TODAY), List::Upcoming);
}

#[test]
fn crossing_midnight_moves_task_from_upcoming_to_today() {
    let mut t = anytime(1, "Завтрашнее");
    t.date = Some(TOMORROW);
    t.evening = true;
    assert_eq!(list(&t, TODAY), List::Upcoming);
    assert_eq!(list(&t, TOMORROW), List::Today { evening: true });

    let tasks = vec![t];
    assert!(lists::today(&tasks, &[], &[], &[], TODAY).is_empty());
    let next_day = lists::today(&tasks, &[], &[], &[], TOMORROW);
    assert!(next_day.day.is_empty());
    assert_eq!(next_day.evening.len(), 1);
}

#[test]
fn overdue_start_date_stays_in_today() {
    let mut t = anytime(1, "Вчерашнее");
    t.date = Some(YESTERDAY);
    assert_eq!(list(&t, TODAY), List::Today { evening: false });
}

#[test]
fn deadline_brings_task_into_today_on_the_day() {
    let mut t = anytime(1, "Сдать отчёт");
    t.deadline = Some(TOMORROW);
    assert_eq!(list(&t, TODAY), List::Anytime);
    assert_eq!(list(&t, TOMORROW), List::Today { evening: false });
    // И остаётся там, пока просрочена.
    assert_eq!(list(&t, date(2026, 11, 25)), List::Today { evening: false });
}

#[test]
fn someday_rules() {
    let mut t = task(1, "Когда-нибудь");
    t.start = Start::Someday;
    assert_eq!(list(&t, TODAY), List::Someday);
    t.date = Some(date(2026, 12, 1));
    assert_eq!(list(&t, TODAY), List::Upcoming);
    t.date = None;
    t.deadline = Some(TODAY);
    assert_eq!(list(&t, TODAY), List::Today { evening: false });
}

#[test]
fn evening_only_matters_in_today() {
    let mut t = anytime(1, "Вечером");
    t.evening = true;
    assert_eq!(list(&t, TODAY), List::Anytime);
    t.date = Some(TODAY);
    assert_eq!(list(&t, TODAY), List::Today { evening: true });
}

#[test]
fn completed_goes_to_logbook_and_deleted_to_trash() {
    let mut t = anytime(1, "Сделано");
    t.date = Some(TODAY);
    t.status = Status::Completed { at: at() };
    assert_eq!(list(&t, TODAY), List::Logbook);
    t.sync.delete(at());
    assert_eq!(list(&t, TODAY), List::Trash);
}

#[test]
fn inbox_start_inside_project_counts_as_anytime() {
    let p = project(10, "Ремонт");
    let mut t = task(1, "Купить краску");
    t.project = Some(p.id);
    assert_eq!(
        lists::list_of(&t, Some(&p), None, None, TODAY),
        List::Anytime
    );
}

#[test]
fn someday_project_hides_its_tasks() {
    let mut p = project(10, "Выучить испанский");
    p.start = Start::Someday;
    let mut loose = anytime(1, "Найти учебник");
    loose.project = Some(p.id);
    let mut dated = anytime(2, "Пробный урок");
    dated.project = Some(p.id);
    dated.date = Some(TODAY);

    let placement = lists::place_task(&loose, Some(&p), None, None, TODAY);
    assert_eq!(placement.list, List::Someday);
    assert!(placement.via == Via::Project);
    // Своя дата важнее проекта «Когда-нибудь».
    assert_eq!(
        lists::list_of(&dated, Some(&p), None, None, TODAY),
        List::Today { evening: false }
    );

    let projects = vec![p];
    let tasks = vec![loose, dated];
    let someday = lists::someday(&tasks, &projects, &[], &[], TODAY);
    assert!(someday.tasks.is_empty());
    assert_eq!(someday.projects.len(), 1);
    // Задачи отложенного проекта не попадают во «В любое время»,
    // даже та, что по своей дате в «Сегодня».
    assert!(lists::anytime(&tasks, &projects, &[], &[], TODAY).is_empty());
    assert_eq!(
        lists::today(&tasks, &projects, &[], &[], TODAY).day.len(),
        1
    );
}

#[test]
fn future_project_shows_in_upcoming_as_project() {
    let mut p = project(10, "Отпуск");
    p.date = Some(date(2026, 12, 20));
    let mut t = anytime(1, "Собрать чемодан");
    t.project = Some(p.id);

    let placement = lists::place_task(&t, Some(&p), None, None, TODAY);
    assert_eq!(placement.list, List::Upcoming);
    assert!(placement.via == Via::Project);

    let projects = vec![p];
    let tasks = vec![t];
    let groups = lists::upcoming(&tasks, &projects, &[], &[], TODAY);
    let filled: Vec<_> = groups.iter().filter(|g| !g.entries.is_empty()).collect();
    assert_eq!(filled.len(), 1);
    assert_eq!(filled[0].entries.len(), 1);
    assert!(matches!(filled[0].entries[0].item, Item::Project(_)));
    // В день начала проекта задача становится доступной, а проект – в «Сегодня».
    let day = date(2026, 12, 20);
    assert_eq!(
        lists::list_of(&tasks[0], Some(&projects[0]), None, None, day),
        List::Anytime
    );
    assert_eq!(lists::today(&tasks, &projects, &[], &[], day).day.len(), 1);
}

#[test]
fn closed_or_deleted_project_takes_its_tasks() {
    let mut p = project(10, "Переезд");
    let mut t = anytime(1, "Заказать грузчиков");
    t.project = Some(p.id);
    p.status = Status::Completed { at: at() };
    let placement = lists::place_task(&t, Some(&p), None, None, TODAY);
    assert_eq!(
        (placement.list, placement.via == Via::Project),
        (List::Logbook, true)
    );
    p.sync.delete(at());
    assert_eq!(lists::list_of(&t, Some(&p), None, None, TODAY), List::Trash);
}

#[test]
fn today_has_its_own_order() {
    let mut a = anytime(1, "Первая в проекте");
    let mut b = anytime(2, "Вторая в проекте");
    a.date = Some(TODAY);
    b.date = Some(TODAY);
    // В «Сегодня» b стоит выше a, хотя в проекте ниже.
    b.today_index = Rank::between(None, Some(&a.today_index));
    let tasks = vec![a, b];
    let view = lists::today(&tasks, &[], &[], &[], TODAY);
    let titles: Vec<&str> = view.day.iter().map(Item::title).collect();
    assert_eq!(titles, ["Вторая в проекте", "Первая в проекте"]);
    let anytime_titles: Vec<&str> = lists::anytime(&tasks, &[], &[], &[], TODAY)
        .iter()
        .map(|e| e.task.title.as_str())
        .collect();
    assert_eq!(anytime_titles, ["Первая в проекте", "Вторая в проекте"]);
}

#[test]
fn upcoming_groups_dates_and_deadlines() {
    let mut a = anytime(1, "Через два дня");
    a.date = Some(date(2026, 11, 21));
    let mut b = anytime(2, "Срок в декабре");
    b.deadline = Some(date(2026, 12, 15));
    let mut c = anytime(3, "Конец месяца");
    c.date = Some(date(2026, 11, 30));
    let tasks = vec![a, b, c];
    let groups = lists::upcoming(&tasks, &[], &[], &[], TODAY);
    let summary: Vec<(Group, usize)> = groups.iter().map(|g| (g.group, g.entries.len())).collect();
    assert_eq!(
        summary,
        [
            (Group::Day(date(2026, 11, 20)), 0),
            (Group::Day(date(2026, 11, 21)), 1),
            (Group::Day(date(2026, 11, 22)), 0),
            (Group::Day(date(2026, 11, 23)), 0),
            (Group::Day(date(2026, 11, 24)), 0),
            (Group::Day(date(2026, 11, 25)), 0),
            (Group::Day(date(2026, 11, 26)), 0),
            (
                Group::RestOfMonth {
                    year: 2026,
                    month: 11
                },
                1
            ),
            (
                Group::Month {
                    year: 2026,
                    month: 12
                },
                1
            ),
        ]
    );
    assert_eq!(groups[8].entries[0].reason, Reason::Deadline);
    // Задача со сроком при этом остаётся в «В любое время».
    assert_eq!(list(&tasks[1], TODAY), List::Anytime);
}

#[test]
fn due_deadline_beats_future_start_date() {
    // Сегодня 19-е, начать 20-го, срок 19-го (сегодня) или 18-го (просрочен).
    for deadline in [TODAY, YESTERDAY] {
        let mut t = anytime(1, "Продлить страховку");
        t.date = Some(TOMORROW);
        t.deadline = Some(deadline);
        assert_eq!(list(&t, TODAY), List::Today { evening: false });

        let tasks = vec![t];
        assert_eq!(lists::today(&tasks, &[], &[], &[], TODAY).day.len(), 1);
        // В «Планах» на день начала запись не повторяется: она уже в «Сегодня».
        assert!(
            lists::upcoming(&tasks, &[], &[], &[], TODAY)
                .iter()
                .all(|g| g.entries.is_empty())
        );
        // В день начала – по-прежнему в «Сегодня».
        assert_eq!(list(&tasks[0], TOMORROW), List::Today { evening: false });
    }
}

#[test]
fn due_deadline_beats_future_evening_start() {
    // «Вечер» относился к будущему дню начала – сегодня задача в дневном блоке.
    let mut t = anytime(1, "Вечером завтра, но срок сегодня");
    t.date = Some(TOMORROW);
    t.evening = true;
    t.deadline = Some(TODAY);
    assert_eq!(list(&t, TODAY), List::Today { evening: false });
    assert_eq!(list(&t, TOMORROW), List::Today { evening: true });
}

#[test]
fn due_deadline_beats_future_start_for_projects() {
    for deadline in [TODAY, YESTERDAY] {
        let mut p = project(10, "Отчёт");
        p.date = Some(TOMORROW);
        p.deadline = Some(deadline);
        assert_eq!(
            lists::place_project(&p, None, TODAY),
            List::Today { evening: false }
        );

        // Проект в «Сегодня» уже не отложен: его задачи доступны.
        let mut t = anytime(1, "Собрать цифры");
        t.project = Some(p.id);
        assert_eq!(
            lists::list_of(&t, Some(&p), None, None, TODAY),
            List::Anytime
        );

        let projects = vec![p];
        let tasks = vec![t];
        let today = lists::today(&tasks, &projects, &[], &[], TODAY);
        assert_eq!(today.day.len(), 1);
        assert!(matches!(today.day[0], Item::Project(_)));
        assert!(
            lists::upcoming(&tasks, &projects, &[], &[], TODAY)
                .iter()
                .all(|g| g.entries.is_empty())
        );
    }
}

#[test]
fn completed_task_of_deleted_project_leaves_logbook_and_returns() {
    let mut p = project(10, "Разбор гаража");
    let mut t = anytime(1, "Вывезти шины");
    t.project = Some(p.id);
    t.status = Status::Completed { at: at() };
    let mut projects = vec![p.clone()];
    let tasks = vec![t];
    let titles = |projects: &[Project]| -> Vec<String> {
        lists::logbook(&tasks, projects, &[], &[])
            .iter()
            .map(|i| i.title().to_owned())
            .collect()
    };
    assert_eq!(titles(&projects), ["Вывезти шины"]);

    p.sync.delete(at());
    projects[0] = p.clone();
    assert_eq!(
        lists::list_of(&tasks[0], Some(&p), None, None, TODAY),
        List::Trash
    );
    assert!(titles(&projects).is_empty());
    // В «Корзине» – сам проект, задача внутри него.
    let trash = lists::trash(&tasks, &projects, &[], &[]);
    assert_eq!(trash.len(), 1);
    assert!(matches!(trash[0], Item::Project(_)));

    p.sync.restore(at());
    projects[0] = p;
    assert_eq!(titles(&projects), ["Вывезти шины"]);
    assert!(lists::trash(&tasks, &projects, &[], &[]).is_empty());
}

#[test]
fn deferred_project_task_due_today_is_in_today_not_anytime() {
    let mut someday = project(10, "Выучить испанский");
    someday.start = Start::Someday;
    let mut future = project(11, "Отпуск");
    future.date = Some(date(2026, 12, 20));

    for p in [someday, future] {
        // Своя дата сегодня и свой срок сегодня – оба варианта.
        let mut by_date = anytime(1, "Своя дата");
        by_date.project = Some(p.id);
        by_date.date = Some(TODAY);
        let mut by_deadline = anytime(2, "Свой срок");
        by_deadline.project = Some(p.id);
        by_deadline.deadline = Some(TODAY);

        let projects = vec![p];
        let tasks = vec![by_date, by_deadline];
        let today = lists::today(&tasks, &projects, &[], &[], TODAY);
        let today_titles: Vec<&str> = today.day.iter().map(Item::title).collect();
        assert_eq!(today_titles.len(), 2, "{}", projects[0].title);
        assert!(today_titles.contains(&"Своя дата") && today_titles.contains(&"Свой срок"));
        assert!(
            lists::anytime(&tasks, &projects, &[], &[], TODAY).is_empty(),
            "{}",
            projects[0].title
        );
    }
}

#[test]
fn deferred_project_task_with_future_deadline_shows_in_upcoming() {
    let mut p = project(10, "Выучить испанский");
    p.start = Start::Someday;
    let mut t = anytime(1, "Сдать экзамен");
    t.project = Some(p.id);
    t.deadline = Some(date(2026, 11, 22));
    let projects = vec![p];
    let tasks = vec![t];
    let entries: Vec<_> = lists::upcoming(&tasks, &projects, &[], &[], TODAY)
        .into_iter()
        .flat_map(|g| g.entries)
        .collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].reason, Reason::Deadline);
    assert!(lists::anytime(&tasks, &projects, &[], &[], TODAY).is_empty());
}

#[test]
fn anytime_contains_today_except_deferred_projects() {
    let mut someday = project(10, "Когда-нибудь");
    someday.start = Start::Someday;
    let active = project(11, "Активный");
    let mut a = anytime(1, "В активном");
    a.project = Some(active.id);
    a.date = Some(TODAY);
    let mut b = anytime(2, "В отложенном");
    b.project = Some(someday.id);
    b.date = Some(TODAY);
    let mut c = anytime(3, "Сама по себе");
    c.deadline = Some(TODAY);
    let projects = vec![someday, active];
    let tasks = vec![a, b, c];
    let anytime_titles: Vec<&str> = lists::anytime(&tasks, &projects, &[], &[], TODAY)
        .iter()
        .filter(|e| e.in_today)
        .map(|e| e.task.title.as_str())
        .collect();
    assert_eq!(anytime_titles, ["В активном", "Сама по себе"]);
    assert_eq!(
        lists::today(&tasks, &projects, &[], &[], TODAY).day.len(),
        3
    );
}

#[test]
fn same_day_date_and_deadline_keep_both_marks() {
    let mut t = anytime(1, "Вернуть книгу");
    t.date = Some(date(2026, 11, 23));
    t.deadline = Some(date(2026, 11, 23));
    let tasks = vec![t];
    let entries: Vec<_> = lists::upcoming(&tasks, &[], &[], &[], TODAY)
        .into_iter()
        .flat_map(|g| g.entries)
        .collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].reason, Reason::ScheduledAndDeadline);
    assert!(entries[0].reason.is_scheduled() && entries[0].reason.has_deadline());
    assert!(!Reason::Scheduled.has_deadline() && !Reason::Deadline.is_scheduled());
}

#[test]
fn deadline_mark_does_not_reorder_scheduled_day() {
    // Обе на 23 ноября; у книги ключ меньше и срок в тот же день.
    let mut book = anytime(1, "Вернуть книгу");
    book.index = "a0".parse().unwrap_or_else(|_| Rank::initial());
    book.date = Some(date(2026, 11, 23));
    book.deadline = Some(date(2026, 11, 23));
    let mut mail = anytime(2, "Разобрать почту");
    mail.index = "a1".parse().unwrap_or_else(|_| Rank::initial());
    mail.date = Some(date(2026, 11, 23));
    // И запись только со сроком в тот же день – после запланированных,
    // хотя ключ у неё самый маленький.
    let mut due = anytime(3, "Только срок");
    due.index = "Zz".parse().unwrap_or_else(|_| Rank::initial());
    due.deadline = Some(date(2026, 11, 23));
    let tasks = vec![mail, due, book];
    let groups = lists::upcoming(&tasks, &[], &[], &[], TODAY);
    let day = groups
        .iter()
        .find(|g| g.group == Group::Day(date(2026, 11, 23)))
        .map(|g| {
            g.entries
                .iter()
                .map(|e| (e.item.title(), e.reason))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    assert_eq!(
        day,
        [
            ("Вернуть книгу", Reason::ScheduledAndDeadline),
            ("Разобрать почту", Reason::Scheduled),
            ("Только срок", Reason::Deadline),
        ]
    );
}

#[test]
fn upcoming_always_has_seven_days_from_tomorrow() {
    let empty = lists::upcoming(&[], &[], &[], &[], TODAY);
    let days: Vec<Group> = empty.iter().map(|g| g.group).collect();
    let expected: Vec<Group> = (20..=26).map(|d| Group::Day(date(2026, 11, d))).collect();
    assert_eq!(days, expected);
    assert_eq!(empty.len(), usize::try_from(DAY_GROUPS).unwrap_or(0));
    assert!(empty.iter().all(|g| g.entries.is_empty()));

    // Одна задача на неделе – всё равно семь дней, и она в своём.
    let mut t = anytime(1, "Через пять дней");
    t.date = Some(date(2026, 11, 24));
    let tasks = vec![t];
    let groups = lists::upcoming(&tasks, &[], &[], &[], TODAY);
    assert_eq!(groups.len(), 7);
    assert_eq!(groups[4].group, Group::Day(date(2026, 11, 24)));
    assert_eq!(groups[4].entries.len(), 1);

    // Через границу месяца.
    let groups = lists::upcoming(&[], &[], &[], &[], date(2026, 11, 28));
    assert_eq!(groups[0].group, Group::Day(date(2026, 11, 29)));
    assert_eq!(groups[6].group, Group::Day(date(2026, 12, 5)));
}

#[test]
fn clearing_date_moves_to_anytime() {
    for start in [Start::Inbox, Start::Someday] {
        let mut t = task(1, "Без даты");
        t.start = start;
        t.date = Some(TOMORROW);
        t.evening = true;
        t.deadline = Some(date(2026, 12, 1));
        t.clear_date(at());
        assert_eq!((t.date, t.evening, t.start), (None, false, Start::Anytime));
        assert_eq!(t.deadline, Some(date(2026, 12, 1)));
        assert_eq!(list(&t, TODAY), List::Anytime);
    }
    let mut p = project(10, "Проект");
    p.start = Start::Someday;
    p.date = Some(TOMORROW);
    p.clear_date(at());
    assert_eq!(lists::place_project(&p, None, TODAY), List::Anytime);
}

#[test]
fn project_progress_counts() {
    let p = project(10, "Сайт");
    let mut tasks: Vec<Task> = (1..=5).map(|i| anytime(i, "шаг")).collect();
    for t in &mut tasks {
        t.project = Some(p.id);
    }
    tasks[0].status = Status::Completed { at: at() };
    tasks[1].status = Status::Completed { at: at() };
    tasks[2].status = Status::Canceled { at: at() };
    tasks[3].sync.delete(at());
    // Правило Tally: отменённые не входят ни в done, ни в total.
    let progress = lists::project_progress(p.id, &tasks, &[]);
    assert_eq!((progress.done, progress.total), (2, 3));
}

#[test]
fn logbook_and_trash_are_newest_first() {
    let mut a = anytime(1, "Раньше");
    let mut b = anytime(2, "Позже");
    a.status = Status::Completed {
        at: "2026-11-18T10:00:00Z"
            .parse()
            .unwrap_or(Timestamp::UNIX_EPOCH),
    };
    b.status = Status::Completed { at: at() };
    let tasks = vec![a, b];
    let titles: Vec<&str> = lists::logbook(&tasks, &[], &[], &[])
        .iter()
        .map(Item::title)
        .collect();
    assert_eq!(titles, ["Позже", "Раньше"]);
    assert!(lists::trash(&tasks, &[], &[], &[]).is_empty());
}

#[test]
fn serde_dates_are_plain_iso() {
    let mut t = anytime(1, "Сериализация");
    t.date = Some(TOMORROW);
    t.deadline = Some(date(2026, 12, 1));
    let json = serde_json::to_value(&t).unwrap_or_default();
    assert_eq!(json["date"], "2026-11-20");
    assert_eq!(json["deadline"], "2026-12-01");
    assert_eq!(json["start"], "anytime");
    assert_eq!(json["status"]["state"], "open");
    let back: Task = serde_json::from_value(json).unwrap_or_else(|_| task(0, ""));
    assert_eq!(back, t);
}

#[test]
fn someday_task_with_deadline_comes_to_today_and_goes_back() {
    // Правило Tally: «Когда-нибудь» – день начала не выбран, срок – день, к
    // которому задачу надо закончить. В этот день она в «Сегодня».
    let nov = |d| date(2026, 11, d);
    let mut t = task(1, "Вернуть книгу");
    t.start = Start::Someday;
    t.evening = true; // без даты «Вечер» ни на что не влияет
    t.deadline = Some(nov(23));

    let where_is = |t: &Task, day: Date| {
        let tasks = vec![t.clone()];
        let in_someday = lists::someday(&tasks, &[], &[], &[], day).tasks.len() == 1;
        let today = lists::today(&tasks, &[], &[], &[], day);
        let anytime_star: Vec<bool> = lists::anytime(&tasks, &[], &[], &[], day)
            .iter()
            .map(|e| e.in_today)
            .collect();
        let flags: Vec<(Date, Reason)> = lists::upcoming(&tasks, &[], &[], &[], day)
            .into_iter()
            .flat_map(|g| g.entries)
            .map(|e| (e.date, e.reason))
            .collect();
        (
            in_someday,
            today.day.len(),
            today.evening.len(),
            anytime_star,
            flags,
        )
    };

    // До срока: в «Когда-нибудь», в «Планах» – ⚑ на 23-е.
    assert_eq!(
        where_is(&t, nov(19)),
        (true, 0, 0, vec![], vec![(nov(23), Reason::Deadline)])
    );
    // В день срока: дневной блок «Сегодня» и звезда во «В любое время».
    assert_eq!(where_is(&t, nov(23)), (false, 1, 0, vec![true], vec![]));
    // Просрочена – там же, пока открыта.
    assert_eq!(where_is(&t, nov(24)), (false, 1, 0, vec![true], vec![]));
    assert_eq!(t.start, Start::Someday);

    // 23-го срок перенесли на 30-е – сразу обратно в «Когда-нибудь», ⚑ на 30-е.
    t.deadline = Some(nov(30));
    assert_eq!(
        where_is(&t, nov(23)),
        (true, 0, 0, vec![], vec![(nov(30), Reason::Deadline)])
    );
    // Срок сняли – только «Когда-нибудь».
    t.deadline = None;
    assert_eq!(where_is(&t, nov(23)), (true, 0, 0, vec![], vec![]));
    assert_eq!(t.start, Start::Someday);
}

#[test]
fn deleting_an_area_trashes_everything_inside_and_restore_brings_it_back() {
    let mut home = Area::new(AreaId::from_u128(50), "Дом", rank(50), at());
    let mut repair = project(10, "Ремонт");
    repair.area = Some(home.id);
    let mut moved = project(11, "Переезд");
    moved.area = Some(home.id);
    moved.status = Status::Completed { at: at() };

    let mut paint = anytime(1, "Купить краску");
    paint.project = Some(repair.id);
    paint.date = Some(TODAY);
    let mut tiles = anytime(2, "Снять плитку");
    tiles.project = Some(repair.id);
    tiles.status = Status::Completed { at: at() };
    let mut plants = anytime(3, "Полить цветы");
    plants.area = Some(home.id);
    let mut bulb = anytime(4, "Вкрутить лампочку");
    bulb.area = Some(home.id);
    bulb.status = Status::Completed { at: at() };
    let mut old = anytime(5, "Старая идея");
    old.area = Some(home.id);
    old.sync.delete(
        "2026-11-18T09:00:00Z"
            .parse()
            .unwrap_or(Timestamp::UNIX_EPOCH),
    );
    let outside = anytime(6, "Вне области");

    let tasks = vec![paint, tiles, plants, bulb, old, outside];
    let snapshot = |areas: &[Area], projects: &[Project]| {
        let titles = |items: &[Item<'_>]| -> Vec<String> {
            items.iter().map(|i| i.title().to_owned()).collect()
        };
        let mut logbook = titles(&lists::logbook(&tasks, projects, areas, &[]));
        logbook.sort();
        let anytime: Vec<String> = lists::anytime(&tasks, projects, areas, &[], TODAY)
            .iter()
            .map(|e| e.task.title.clone())
            .collect();
        let mut today = lists::today(&tasks, projects, areas, &[], TODAY).day;
        today.sort_by_key(|i| i.title().to_owned());
        (
            titles(&today),
            anytime,
            logbook,
            titles(&lists::trash(&tasks, projects, areas, &[])),
        )
    };
    let projects = vec![repair, moved];

    let before = snapshot(std::slice::from_ref(&home), &projects);
    assert_eq!(before.0, ["Купить краску"]);
    assert_eq!(before.1, ["Купить краску", "Полить цветы", "Вне области"]);
    assert_eq!(before.2, ["Вкрутить лампочку", "Переезд", "Снять плитку"]);
    assert_eq!(before.3, ["Старая идея"]);

    // Удаляем только саму область.
    home.sync.delete(at());
    let deleted = snapshot(std::slice::from_ref(&home), &projects);
    assert!(deleted.0.is_empty());
    assert_eq!(deleted.1, ["Вне области"]);
    assert!(deleted.2.is_empty());
    // В корзине – область одной строкой и то, что было удалено отдельно раньше.
    assert_eq!(deleted.3, ["Дом", "Старая идея"]);
    let p = lists::place_task(&tasks[0], Some(&projects[0]), None, Some(&home), TODAY);
    assert_eq!((p.list, p.via), (List::Trash, Via::Area));
    let p = lists::place_task(&tasks[2], None, None, Some(&home), TODAY);
    assert_eq!((p.list, p.via), (List::Trash, Via::Area));
    assert_eq!(
        lists::place_project(&projects[0], Some(&home), TODAY),
        List::Trash
    );

    // Возвращаем: всё на своих местах, отдельно удалённое – в корзине.
    home.sync.restore(at());
    assert_eq!(snapshot(std::slice::from_ref(&home), &projects), before);
}

#[test]
fn deleting_a_heading_trashes_its_tasks_at_once_and_restore_brings_them_back() {
    let p = project(10, "Ремонт кухни");
    let mut buy = tally_core::Heading::new(
        tally_core::HeadingId::from_u128(20),
        p.id,
        "Покупки",
        rank(20),
        at(),
    );
    let mut paint = anytime(1, "Купить краску");
    paint.project = Some(p.id);
    paint.heading = Some(buy.id);
    paint.date = Some(TODAY);
    let mut glue = anytime(2, "Купить клей");
    glue.project = Some(p.id);
    glue.heading = Some(buy.id);
    glue.status = Status::Completed { at: at() };
    let mut brush = anytime(3, "Купить кисть");
    brush.project = Some(p.id);
    brush.heading = Some(buy.id);
    brush.sync.delete(
        "2026-11-18T09:00:00Z"
            .parse()
            .unwrap_or(Timestamp::UNIX_EPOCH),
    );
    let mut other = anytime(4, "Без раздела");
    other.project = Some(p.id);
    let tasks = vec![paint, glue, brush, other];
    let projects = vec![p];

    let view = |headings: &[tally_core::Heading]| {
        let titles = |items: &[Item<'_>]| -> Vec<String> {
            items.iter().map(|i| i.title().to_owned()).collect()
        };
        (
            titles(&lists::today(&tasks, &projects, &[], headings, TODAY).day),
            lists::anytime(&tasks, &projects, &[], headings, TODAY)
                .iter()
                .map(|e| e.task.title.clone())
                .collect::<Vec<_>>(),
            titles(&lists::logbook(&tasks, &projects, &[], headings)),
            titles(&lists::trash(&tasks, &projects, &[], headings)),
        )
    };
    let before = view(std::slice::from_ref(&buy));
    assert_eq!(before.0, ["Купить краску"]);
    assert_eq!(before.1, ["Купить краску", "Без раздела"]);
    assert_eq!(before.2, ["Купить клей"]);
    assert_eq!(before.3, ["Купить кисть"]);

    buy.sync.delete(at());
    let deleted = view(std::slice::from_ref(&buy));
    // Сразу, без очистки корзины: из «Сегодня» и остальных списков ушли.
    assert!(deleted.0.is_empty());
    assert_eq!(deleted.1, ["Без раздела"]);
    assert!(deleted.2.is_empty());
    // В корзине – раздел одной строкой и отдельно удалённая раньше задача.
    assert_eq!(deleted.3, ["Покупки", "Купить кисть"]);
    let placed = lists::place_task(&tasks[0], Some(&projects[0]), Some(&buy), None, TODAY);
    assert_eq!((placed.list, placed.via), (List::Trash, Via::Heading));

    buy.sync.restore(at());
    assert_eq!(view(std::slice::from_ref(&buy)), before);
}

#[test]
fn deletion_chain_is_one_rule() {
    use tally_core::lists::Deleted;
    let none = Deleted::default();
    assert_eq!(none.via(), None);
    let all = Deleted {
        own: true,
        heading: true,
        project: true,
        area: true,
    };
    assert_eq!(all.via(), Some(Via::Own));
    assert_eq!(Deleted { own: false, ..all }.via(), Some(Via::Heading));
    assert_eq!(
        Deleted {
            project: true,
            ..none
        }
        .via(),
        Some(Via::Project)
    );
    assert_eq!(Deleted { area: true, ..none }.via(), Some(Via::Area));
}

#[test]
fn progress_drops_tasks_of_a_deleted_heading_and_restore_returns_them() {
    let p = project(10, "Ремонт");
    let mut buy = tally_core::Heading::new(
        tally_core::HeadingId::from_u128(20),
        p.id,
        "Покупки",
        rank(20),
        at(),
    );
    let mut done = anytime(1, "Снять плитку");
    done.project = Some(p.id);
    done.status = Status::Completed { at: at() };
    let mut open = anytime(2, "Купить краску");
    open.project = Some(p.id);
    open.heading = Some(buy.id);
    let tasks = vec![done, open];
    let count = |h: &tally_core::Heading| {
        let pr = lists::project_progress(p.id, &tasks, std::slice::from_ref(h));
        (pr.done, pr.total)
    };
    assert_eq!(count(&buy), (1, 2));
    buy.sync.delete(at());
    assert_eq!(count(&buy), (1, 1));
    buy.sync.restore(at());
    assert_eq!(count(&buy), (1, 2));
}

#[test]
fn task_in_a_project_takes_the_project_area_not_its_own() {
    // Своя область у задачи в проекте – устаревшее значение: решает область проекта.
    let mut home = Area::new(AreaId::from_u128(50), "Дом", rank(50), at());
    let mut work = Area::new(AreaId::from_u128(51), "Работа", rank(51), at());
    let mut office = project(10, "Офис");
    office.area = Some(work.id);
    let mut t = anytime(1, "Купить краску");
    t.project = Some(office.id);
    t.area = Some(home.id);
    let tasks = vec![t];
    let projects = vec![office];
    let anytime_titles = |areas: &[Area]| -> Vec<String> {
        lists::anytime(&tasks, &projects, areas, &[], TODAY)
            .iter()
            .map(|e| e.task.title.clone())
            .collect()
    };
    home.sync.delete(at());
    assert_eq!(
        anytime_titles(&[home.clone(), work.clone()]),
        ["Купить краску"]
    );
    home.sync.restore(at());
    work.sync.delete(at());
    assert!(anytime_titles(&[home, work]).is_empty());
}
