//! Списки для небольшого набора данных в два соседних дня:
//! `cargo run -p tally-core --example lists`.

use jiff::civil::{Date, Weekday, date};
use tally_core::lists::{Group, Item, Lists, Reason, project_progress};
use tally_core::{Area, AreaId, Project, ProjectId, Rank, Start, Status, Task, TaskId, Timestamp};

const MONTHS: [&str; 12] = [
    "января",
    "февраля",
    "марта",
    "апреля",
    "мая",
    "июня",
    "июля",
    "августа",
    "сентября",
    "октября",
    "ноября",
    "декабря",
];
const MONTHS_NOM: [&str; 12] = [
    "Январь",
    "Февраль",
    "Март",
    "Апрель",
    "Май",
    "Июнь",
    "Июль",
    "Август",
    "Сентябрь",
    "Октябрь",
    "Ноябрь",
    "Декабрь",
];

fn weekday(d: Date) -> &'static str {
    match d.weekday() {
        Weekday::Monday => "пн",
        Weekday::Tuesday => "вт",
        Weekday::Wednesday => "ср",
        Weekday::Thursday => "чт",
        Weekday::Friday => "пт",
        Weekday::Saturday => "сб",
        Weekday::Sunday => "вс",
    }
}

fn month_index(m: i8) -> usize {
    usize::try_from(m - 1).unwrap_or(0)
}

fn day(d: Date) -> String {
    format!(
        "{}, {} {}",
        weekday(d),
        d.day(),
        MONTHS[month_index(d.month())]
    )
}

fn ts(s: &str) -> Timestamp {
    s.parse().unwrap_or(Timestamp::UNIX_EPOCH)
}

struct Data {
    areas: Vec<Area>,
    tasks: Vec<Task>,
    projects: Vec<Project>,
}

fn sample() -> Data {
    let at = ts("2026-11-10T09:00:00Z");
    let keys = Rank::spread(40);
    let key = |i: usize| keys[i].clone();

    let home = Area::new(AreaId::from_u128(1), "Дом", key(0), at);

    let mut repair = Project::new(ProjectId::from_u128(10), "Ремонт кухни", key(1), at);
    repair.area = Some(home.id);
    repair.deadline = Some(date(2026, 12, 15));

    let mut spanish = Project::new(ProjectId::from_u128(11), "Выучить испанский", key(2), at);
    spanish.start = Start::Someday;

    let mut trip = Project::new(ProjectId::from_u128(12), "Поездка в Казань", key(3), at);
    trip.date = Some(date(2026, 12, 4));

    let mut n = 100u128;
    let mut mk = |title: &str, i: usize| {
        n += 1;
        Task::new(TaskId::from_u128(n), title, key(i), at)
    };

    let mut tasks = Vec::new();

    tasks.push(mk("Разобрать почту", 4)); // Входящие

    let mut t = mk("Позвонить маме", 5);
    t.start = Start::Anytime;
    t.date = Some(date(2026, 11, 19));
    tasks.push(t);

    let mut t = mk("Купить продукты на неделю", 6);
    t.start = Start::Anytime;
    t.date = Some(date(2026, 11, 19));
    t.evening = true;
    tasks.push(t);

    let mut t = mk("Оплатить интернет", 7);
    t.start = Start::Anytime;
    t.date = Some(date(2026, 11, 17)); // просроченная дата начала
    tasks.push(t);

    let mut t = mk("Сдать отчёт за квартал", 8);
    t.start = Start::Anytime;
    t.deadline = Some(date(2026, 11, 20)); // срок завтра
    tasks.push(t);

    let mut t = mk("Тренировка", 9);
    t.start = Start::Anytime;
    t.date = Some(date(2026, 11, 20));
    t.evening = true;
    tasks.push(t);

    let mut t = mk("Записаться к стоматологу", 10);
    t.date = Some(date(2026, 11, 24)); // из «Входящих» сразу в «Планы»
    tasks.push(t);

    let mut t = mk("Купить плитку", 11);
    t.project = Some(repair.id);
    tasks.push(t);

    let mut t = mk("Вызвать электрика", 12);
    t.project = Some(repair.id);
    t.start = Start::Anytime;
    t.date = Some(date(2026, 11, 26));
    tasks.push(t);

    let mut t = mk("Снять старые шкафы", 13);
    t.project = Some(repair.id);
    t.status = Status::Completed {
        at: ts("2026-11-18T17:30:00Z"),
    };
    tasks.push(t);

    let mut t = mk("Найти учебник", 14);
    t.project = Some(spanish.id);
    tasks.push(t);

    let mut t = mk("Купить билеты", 15);
    t.project = Some(trip.id);
    tasks.push(t);

    let mut t = mk("Прочитать «Мастера и Маргариту»", 16);
    t.start = Start::Someday;
    tasks.push(t);

    let mut t = mk("Старая идея", 17);
    t.sync.delete(ts("2026-11-15T12:00:00Z"));
    tasks.push(t);

    let mut t = mk("Подать декларацию", 18);
    t.start = Start::Anytime;
    t.deadline = Some(date(2027, 4, 30));
    tasks.push(t);

    let mut t = mk("Продлить страховку", 19);
    t.start = Start::Anytime;
    t.date = Some(date(2026, 11, 21)); // начать собирались позже,
    t.deadline = Some(date(2026, 11, 19)); // но срок уже сегодня
    tasks.push(t);

    let mut t = mk("Вернуть книгу в библиотеку", 20);
    t.start = Start::Anytime;
    t.date = Some(date(2026, 11, 23));
    t.deadline = Some(date(2026, 11, 23)); // дата и срок в один день
    tasks.push(t);

    let mut t = mk("Записаться на курсы", 21);
    t.project = Some(spanish.id);
    t.deadline = Some(date(2026, 11, 19)); // срок сегодня в проекте «Когда-нибудь»
    tasks.push(t);

    let mut t = mk("Забронировать гостиницу", 22);
    t.project = Some(trip.id);
    t.date = Some(date(2026, 11, 19)); // своя дата в проекте с будущим стартом
    tasks.push(t);

    let mut garage = Project::new(ProjectId::from_u128(13), "Разбор гаража", key(23), at);
    garage.sync.delete(ts("2026-11-16T10:00:00Z"));

    let mut t = mk("Вывезти старые шины", 24);
    t.project = Some(garage.id);
    t.status = Status::Completed {
        at: ts("2026-11-12T15:00:00Z"),
    }; // выполнена, но проект в корзине – в «Журнале» её нет
    tasks.push(t);

    Data {
        areas: vec![home],
        tasks,
        projects: vec![repair, spanish, trip, garage],
    }
}

fn item(i: &Item<'_>) -> String {
    match i {
        Item::Task(t) => t.title.clone(),
        Item::Project(p) => format!("[проект] {}", p.title),
        Item::Heading(h) => format!("[раздел] {}", h.title),
        Item::Area(a) => format!("[область] {}", a.title),
    }
}

fn group(g: Group, today: Date) -> String {
    match g {
        Group::Day(d) if d == today.tomorrow().unwrap_or(d) => format!("Завтра – {}", day(d)),
        Group::Day(d) => day(d),
        Group::RestOfMonth { month, .. } => {
            format!("Остаток {}", MONTHS[month_index(month)])
        }
        Group::Month { month, year } => format!("{} {year}", MONTHS_NOM[month_index(month)]),
        Group::Year(y) => format!("{y}"),
    }
}

fn print(data: &Data, today: Date) {
    let lists = Lists::compute(&data.tasks, &data.projects, &data.areas, &[], today);
    println!("════════ Сегодня: {} {} ════════", day(today), today.year());

    println!("\nВходящие");
    for t in &lists.inbox {
        println!("  ○ {}", t.title);
    }

    println!("\nСегодня");
    for i in &lists.today.day {
        println!("  ○ {}", item(i));
    }
    if !lists.today.evening.is_empty() {
        println!("  Вечер");
        for i in &lists.today.evening {
            println!("    ○ {}", item(i));
        }
    }

    println!("\nПланы");
    for g in &lists.upcoming {
        println!("  {}", group(g.group, today));
        for e in &g.entries {
            let mark = match e.reason {
                Reason::Scheduled => "○",
                Reason::ScheduledAndDeadline => "○ ⚑ срок:",
                Reason::Deadline => "⚑ срок:",
            };
            println!("    {mark} {} ({})", item(&e.item), day(e.date));
        }
    }

    println!("\nВ любое время");
    for e in &lists.anytime {
        let star = if e.in_today { "★" } else { "○" };
        println!("  {star} {}", e.task.title);
    }

    println!("\nКогда-нибудь");
    for p in &lists.someday.projects {
        println!("  [проект] {}", p.title);
    }
    for t in &lists.someday.tasks {
        println!("  ○ {}", t.title);
    }

    println!("\nЖурнал");
    for i in &lists.logbook {
        println!("  ✓ {}", item(i));
    }

    println!("\nКорзина");
    for i in &lists.trash {
        println!("  ✗ {}", item(i));
    }

    println!("\nПрогресс проектов");
    for p in &data.projects {
        let pr = project_progress(p.id, &data.tasks, &[]);
        println!("  {} – {}/{}", p.title, pr.done, pr.total);
    }
    println!();
}

fn main() {
    let data = sample();
    print(&data, date(2026, 11, 19));
    print(&data, date(2026, 11, 20));
}
