//! «Планы»: будущие даты начала и сроки, сгруппированные как в Things.

use jiff::civil::Date;

use super::Containers;
use super::item::Item;
use super::placement::List;
use crate::area::Area;
use crate::heading::Heading;
use crate::project::Project;
use crate::schedule::days_between;
use crate::task::Task;

/// Почему запись стоит на эту дату.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reason {
    /// Дата начала.
    Scheduled,
    /// Дата начала, и в тот же день срок: нужны обе пометки (○ и ⚑).
    ScheduledAndDeadline,
    /// Крайний срок (задача сама может быть в другом списке).
    Deadline,
}

impl Reason {
    /// Запись начинается в этот день.
    pub fn is_scheduled(self) -> bool {
        matches!(self, Self::Scheduled | Self::ScheduledAndDeadline)
    }

    /// В этот день срок.
    pub fn has_deadline(self) -> bool {
        matches!(self, Self::Deadline | Self::ScheduledAndDeadline)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpcomingEntry<'a> {
    pub date: Date,
    pub item: Item<'a>,
    pub reason: Reason,
}

/// Группа в «Планах».
///
/// Ближайшие 7 дней (с завтрашнего) – по дням, причём, как в Things, все семь
/// групп есть всегда, даже пустые. Остаток текущего месяца – одной группой,
/// дальше по месяцам на год вперёд (12 месяцев, считая текущий), затем по годам.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Day(Date),
    RestOfMonth { year: i16, month: i8 },
    Month { year: i16, month: i8 },
    Year(i16),
}

#[derive(Debug)]
pub struct UpcomingGroup<'a> {
    pub group: Group,
    pub entries: Vec<UpcomingEntry<'a>>,
}

/// Число дней, которые в «Планах» всегда показаны отдельными группами.
pub const DAY_GROUPS: i32 = 7;

/// Группа для даты `date > today`.
pub fn group_of(date: Date, today: Date) -> Group {
    if days_between(today, date) <= DAY_GROUPS {
        return Group::Day(date);
    }
    let months_ahead = month_number(date) - month_number(today);
    match months_ahead {
        0 => Group::RestOfMonth {
            year: date.year(),
            month: date.month(),
        },
        1..=11 => Group::Month {
            year: date.year(),
            month: date.month(),
        },
        _ => Group::Year(date.year()),
    }
}

fn month_number(d: Date) -> i32 {
    i32::from(d.year()) * 12 + i32::from(d.month())
}

pub fn upcoming<'a>(
    tasks: &'a [Task],
    projects: &'a [Project],
    areas: &'a [Area],
    headings: &'a [Heading],
    today: Date,
) -> Vec<UpcomingGroup<'a>> {
    let c = Containers::new(projects, areas, headings);
    let mut entries = Vec::new();

    for t in tasks {
        let placement = c.place(t, today);
        if matches!(placement.list, List::Logbook | List::Trash) {
            continue;
        }
        let scheduled = (placement.list == List::Upcoming && placement.is_own())
            .then_some(t.date)
            .flatten();
        push_entries(&mut entries, Item::Task(t), scheduled, t.deadline, today);
    }
    for p in projects {
        let list = c.place_project(p, today);
        if matches!(list, List::Logbook | List::Trash) {
            continue;
        }
        let scheduled = (list == List::Upcoming).then_some(p.date).flatten();
        push_entries(&mut entries, Item::Project(p), scheduled, p.deadline, today);
    }

    // Внутри дня: всё, что на этот день запланировано (с ⚑ или без), – в порядке
    // `index`, а записи только со сроком – после них. Пометка ⚑ порядок не меняет.
    entries.sort_by(|a, b| {
        (a.date, !a.reason.is_scheduled(), a.item.index()).cmp(&(
            b.date,
            !b.reason.is_scheduled(),
            b.item.index(),
        ))
    });
    // Семь дней с завтрашнего – всегда, даже пустые.
    let mut groups: Vec<UpcomingGroup<'a>> = (1..=DAY_GROUPS)
        .filter_map(|n| today.checked_add(jiff::Span::new().days(n)).ok())
        .map(|d| UpcomingGroup {
            group: Group::Day(d),
            entries: Vec::new(),
        })
        .collect();
    for entry in entries {
        let group = group_of(entry.date, today);
        if let Some(g) = groups.iter_mut().find(|g| g.group == group) {
            g.entries.push(entry);
        } else {
            groups.push(UpcomingGroup {
                group,
                entries: vec![entry],
            });
        }
    }
    groups
}

/// Записи одного элемента: на дату начала (`scheduled`, если элемент сам
/// в «Планах») и на будущий срок. Если дата и срок совпадают – одна запись
/// с обеими пометками.
fn push_entries<'a>(
    entries: &mut Vec<UpcomingEntry<'a>>,
    item: Item<'a>,
    scheduled: Option<Date>,
    deadline: Option<Date>,
    today: Date,
) {
    let deadline = deadline.filter(|&d| d > today);
    if let Some(date) = scheduled {
        let reason = if deadline == Some(date) {
            Reason::ScheduledAndDeadline
        } else {
            Reason::Scheduled
        };
        entries.push(UpcomingEntry { date, item, reason });
    }
    if let Some(date) = deadline
        && scheduled != Some(date)
    {
        entries.push(UpcomingEntry {
            date,
            item,
            reason: Reason::Deadline,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn grouping() {
        let today = date(2026, 11, 19);
        assert_eq!(
            group_of(date(2026, 11, 20), today),
            Group::Day(date(2026, 11, 20))
        );
        assert_eq!(
            group_of(date(2026, 11, 26), today),
            Group::Day(date(2026, 11, 26))
        );
        assert_eq!(
            group_of(date(2026, 11, 27), today),
            Group::RestOfMonth {
                year: 2026,
                month: 11
            }
        );
        assert_eq!(
            group_of(date(2027, 1, 5), today),
            Group::Month {
                year: 2027,
                month: 1
            }
        );
        assert_eq!(
            group_of(date(2027, 10, 1), today),
            Group::Month {
                year: 2027,
                month: 10
            }
        );
        assert_eq!(group_of(date(2027, 11, 1), today), Group::Year(2027));
    }

    #[test]
    fn first_week_crosses_month_boundary() {
        let today = date(2026, 11, 28);
        assert_eq!(
            group_of(date(2026, 12, 5), today),
            Group::Day(date(2026, 12, 5))
        );
        assert_eq!(
            group_of(date(2026, 12, 6), today),
            Group::Month {
                year: 2026,
                month: 12
            }
        );
    }
}
