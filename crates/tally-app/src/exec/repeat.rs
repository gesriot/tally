//! Экземпляры по календарю и после завершения.

use jiff::Span;
use tally_core::{RepeatFrom, RepeatRule, schedule::days_between};

use super::*;
use crate::Repeat;

fn validate(repeat: Option<Repeat>) -> Result<(Option<RepeatRule>, RepeatFrom)> {
    let Some(repeat) = repeat else {
        return Ok((None, RepeatFrom::Date));
    };
    if repeat.rule.interval == 0 {
        reject!("интервал повтора должен быть больше нуля");
    }
    Ok((Some(repeat.rule), repeat.from))
}

pub(super) fn set_task(task: &mut Task, repeat: Option<Repeat>, today: Date) -> Result<()> {
    if !task.status.is_open() {
        reject!("настроить повтор можно у открытой задачи");
    }
    let (rule, from) = validate(repeat)?;
    if (task.repeat_rule, task.repeat_from) != (rule, from) {
        task.repeat_advanced = false;
        task.repeat_rule = rule;
        task.repeat_from = from;
        if rule.is_some() && from == RepeatFrom::Date && task.date.is_none() {
            When::On {
                date: today,
                evening: false,
            }
            .apply_to_task(task, task.sync.updated_at);
        }
        task.repeat_anchor = task
            .date
            .filter(|_| rule.is_some() && from == RepeatFrom::Date);
    }
    Ok(())
}

pub(super) fn schedule_changed(task: &mut Task, old: &Task) -> Result<()> {
    if task.repeat_rule.is_some() && task.repeat_from == RepeatFrom::Date && task.date != old.date {
        let Some(date) = task.date else {
            reject!("сначала уберите календарный повтор, чтобы снять дату начала");
        };
        task.repeat_anchor = Some(date);
    }
    Ok(())
}

pub(super) fn set_project(
    store: &mut Store,
    id: ProjectId,
    expected: Timestamp,
    repeat: Option<Repeat>,
    clock: Clock,
) -> Result<Done> {
    change(store, clock, |s, _| {
        let mut project = live_project(s, id)?.clone();
        if project.sync.updated_at != expected {
            return Err(Error::Conflict(
                "проект изменился. Откройте настройку повтора снова".into(),
            ));
        }
        let (rule, from) = validate(repeat)?;
        if (project.repeat_rule, project.repeat_from) != (rule, from) {
            project.repeat_rule = rule;
            project.repeat_from = from;
            project.repeat_advanced = false;
            if rule.is_some() && from == RepeatFrom::Date && project.date.is_none() {
                project.date = Some(clock.today);
                project.start = Start::Anytime;
            }
            project.repeat_anchor = project
                .date
                .filter(|_| rule.is_some() && from == RepeatFrom::Date);
        }
        Ok(Changed {
            effect: Effect::ProjectRepeatSet,
            action: format!("настройка повтора проекта «{}»", project.title),
            records: vec![Record::Project(project)],
        })
    })
}

struct DateOutsideCalendar;

struct Occurrence {
    date: Date,
    anchor: Option<Date>,
    offset: i32,
}

impl Occurrence {
    fn next(
        rule: RepeatRule,
        from: RepeatFrom,
        scheduled: Option<Date>,
        anchor: Option<Date>,
        today: Date,
    ) -> Option<Self> {
        let base = scheduled.unwrap_or(today);
        let (start, anchor) = match from {
            RepeatFrom::Date => (base, anchor.or(Some(base))),
            RepeatFrom::Completion => (today, None),
        };
        let date = rule.next(start, today, anchor.unwrap_or(start))?;
        Some(Self {
            date,
            anchor,
            offset: days_between(base, date),
        })
    }

    fn due(
        rule: Option<RepeatRule>,
        from: RepeatFrom,
        base: Option<Date>,
        anchor: Option<Date>,
        today: Date,
    ) -> Option<Self> {
        if from != RepeatFrom::Date {
            return None;
        }
        let base = base?;
        let anchor = anchor.unwrap_or(base);
        let date = rule?.latest(base, today, anchor)?;
        Some(Self {
            date,
            anchor: Some(anchor),
            offset: days_between(base, date),
        })
    }

    fn shift(&self, date: Option<Date>) -> Result<Option<Date>, DateOutsideCalendar> {
        shift_date(date, self.offset)
    }
}

fn shift_date(date: Option<Date>, days: i32) -> Result<Option<Date>, DateOutsideCalendar> {
    date.map(|date| {
        let shift = Span::new()
            .try_days(days)
            .map_err(|_| DateOutsideCalendar)?;
        date.checked_add(shift).map_err(|_| DateOutsideCalendar)
    })
    .transpose()
}

fn live_tags(s: &Snapshot, tags: &mut Vec<TagId>) {
    tags.retain(|id| s.tags.iter().any(|t| t.id == *id && !t.sync.is_deleted()));
}

fn copy_task(task: &Task, s: &Snapshot, now: Timestamp) -> Task {
    let mut next = task.clone();
    next.id = TaskId::from_uuid(Uuid::new_v4());
    next.sync = SyncMeta::new(now);
    next.status = Status::Open;
    next.repeat_advanced = false;
    next.reminder_at = None;
    live_tags(s, &mut next.tags);
    next.checklist.retain(|item| !item.sync.is_deleted());
    for item in &mut next.checklist {
        item.id = ChecklistItemId::from_uuid(Uuid::new_v4());
        item.sync = SyncMeta::new(now);
        item.done = false;
    }
    next
}

pub(super) fn advance_task(task: &mut Task, s: &Snapshot, clock: Clock) -> Option<Task> {
    let rule = task.repeat_rule.filter(|_| !task.repeat_advanced)?;
    let index = Index::new(s, clock.today);
    if index.project_of(task).is_some_and(|p| !p.status.is_open()) {
        return None;
    }
    let occurrence = Occurrence::next(
        rule,
        task.repeat_from,
        task.date,
        task.repeat_anchor,
        clock.today,
    )?;
    let archived = index.heading_of(task).is_some_and(|h| h.archived);
    task_at(task, &occurrence, s, archived, clock.now).ok()
}

fn task_at(
    task: &mut Task,
    occurrence: &Occurrence,
    s: &Snapshot,
    archived: bool,
    now: Timestamp,
) -> Result<Task, DateOutsideCalendar> {
    let mut next = copy_task(task, s, now);
    next.date = Some(occurrence.date);
    next.start = Start::Anytime;
    next.deadline = occurrence.shift(task.deadline)?;
    next.repeat_anchor = occurrence.anchor;
    if archived {
        next.heading = None;
    }
    task.repeat_rule = None;
    task.repeat_from = RepeatFrom::Date;
    task.repeat_anchor = None;
    task.repeat_advanced = true;
    Ok(next)
}

pub(super) fn advance_project(project: &mut Project, s: &Snapshot, clock: Clock) -> Vec<Record> {
    let Some(rule) = project.repeat_rule.filter(|_| !project.repeat_advanced) else {
        return Vec::new();
    };
    let Some(occurrence) = Occurrence::next(
        rule,
        project.repeat_from,
        project.date,
        project.repeat_anchor,
        clock.today,
    ) else {
        return Vec::new();
    };
    project_at(project, &occurrence, s, clock).unwrap_or_default()
}

fn project_at(
    project: &mut Project,
    occurrence: &Occurrence,
    s: &Snapshot,
    clock: Clock,
) -> Result<Vec<Record>, DateOutsideCalendar> {
    let now = clock.now;
    let mut next = project.clone();
    next.id = ProjectId::from_uuid(Uuid::new_v4());
    next.sync = SyncMeta::new(now);
    next.status = Status::Open;
    next.date = Some(occurrence.date);
    next.start = Start::Anytime;
    next.deadline = occurrence.shift(project.deadline)?;
    next.repeat_anchor = occurrence.anchor;
    live_tags(s, &mut next.tags);
    let id = next.id;
    let mut records = vec![Record::Project(next)];
    let mut sections = std::collections::HashMap::new();
    for section in s
        .headings
        .iter()
        .filter(|h| h.project == project.id && !h.archived && !h.sync.is_deleted())
    {
        let heading = HeadingId::from_uuid(Uuid::new_v4());
        sections.insert(section.id, heading);
        records.push(Record::Heading(Heading::new(
            heading,
            id,
            &section.title,
            section.index.clone(),
            now,
        )));
    }
    for task in s
        .tasks
        .iter()
        .filter(|t| t.project == Some(project.id) && !t.sync.is_deleted() && !t.repeat_advanced)
    {
        let heading = match task.heading {
            Some(old) => match sections.get(&old) {
                Some(&new) => Some(new),
                None => continue,
            },
            None => None,
        };
        let mut next = copy_task(task, s, now);
        next.project = Some(id);
        next.heading = heading;
        // При индивидуальном повторе дата задачи сдвигается, а начало
        // её календарной серии остаётся опорой для следующего проекта.
        let offset = match (
            task.repeat_from,
            task.repeat_rule,
            task.date,
            task.repeat_anchor,
        ) {
            (RepeatFrom::Date, Some(_), Some(date), Some(anchor)) => {
                occurrence.offset + days_between(date, anchor)
            }
            _ => occurrence.offset,
        };
        next.date = shift_date(task.date, offset)?;
        next.deadline = shift_date(task.deadline, offset)?;
        next.repeat_anchor = next
            .date
            .filter(|_| next.repeat_rule.is_some() && next.repeat_from == RepeatFrom::Date);
        // У новой копии задачи уже мог наступить собственный повтор.
        // Сразу переносим правило на последнюю копию в этом же проекте.
        let due = Occurrence::due(
            next.repeat_rule,
            next.repeat_from,
            next.date,
            next.repeat_anchor,
            clock.today,
        );
        let successor = due.and_then(|date| task_at(&mut next, &date, s, false, now).ok());
        records.push(Record::Task(next));
        if let Some(successor) = successor {
            records.push(Record::Task(successor));
        }
    }
    project.repeat_rule = None;
    project.repeat_from = RepeatFrom::Date;
    project.repeat_anchor = None;
    project.repeat_advanced = true;
    Ok(records)
}

enum Scheduled<'a> {
    Task(&'a Task, Occurrence),
    Project(&'a Project, Occurrence),
}

fn due(s: &Snapshot, today: Date) -> impl Iterator<Item = Scheduled<'_>> {
    let index = std::cell::OnceCell::new();
    let projects = s.projects.iter().filter_map(move |project| {
        if project.repeat_advanced || !project.status.is_open() {
            return None;
        }
        let date = Occurrence::due(
            project.repeat_rule,
            project.repeat_from,
            project.date,
            project.repeat_anchor,
            today,
        )?;
        (!project.sync.is_deleted()
            && !project
                .area
                .and_then(|id| s.areas.iter().find(|a| a.id == id))
                .is_some_and(|a| a.sync.is_deleted()))
        .then_some(Scheduled::Project(project, date))
    });
    let tasks = s.tasks.iter().filter_map(move |task| {
        if task.repeat_advanced || !task.status.is_open() {
            return None;
        }
        let date = Occurrence::due(
            task.repeat_rule,
            task.repeat_from,
            task.date,
            task.repeat_anchor,
            today,
        )?;
        let ix = index.get_or_init(|| Index::new(s, today));
        (!ix.trashed(task) && ix.project_of(task).is_none_or(|p| p.status.is_open()))
            .then_some(Scheduled::Task(task, date))
    });
    projects.chain(tasks)
}

pub(crate) fn has_due_repeats(s: &Snapshot, today: Date) -> bool {
    due(s, today).next().is_some()
}

pub(super) fn advance_due(store: &mut Store, clock: Clock) -> Result<Done> {
    change(store, clock, |s, now| {
        let index = Index::new(s, clock.today);
        let mut records = Vec::new();
        for scheduled in due(s, clock.today) {
            match scheduled {
                Scheduled::Task(task, date) => {
                    let mut previous = task.clone();
                    let archived = index.heading_of(task).is_some_and(|h| h.archived);
                    if edited(now, &task.sync).is_ok()
                        && let Ok(next) = task_at(&mut previous, &date, s, archived, now)
                    {
                        records.push(Record::Task(next));
                        records.push(Record::Task(previous));
                    }
                }
                Scheduled::Project(project, date) => {
                    let mut previous = project.clone();
                    if edited(now, &project.sync).is_ok()
                        && let Ok(next) = project_at(&mut previous, &date, s, clock)
                    {
                        records.extend(next);
                        records.push(Record::Project(previous));
                    }
                }
            }
        }
        Ok(Changed {
            effect: Effect::ScheduledRepeats,
            action: "создание повторов по расписанию".into(),
            records,
        })
    })
}
