//! Перенос выбранной задачи и запись позиции одним шагом истории.

use tally_core::lists::{Item, List, place_project};
use tally_core::{Rank, Start, Task, TaskId};
use tally_store::Store;
use tally_when::When;

use super::{Changed, change, live_area, live_project};
use crate::Index;
use crate::command::{Clock, Done, DropAnchor, DropPosition, DropTask, Effect, TaskLane};
use crate::error::{Error, Result, reject};
use crate::history::{self, Record};

pub(super) fn execute(store: &mut Store, request: &DropTask, clock: Clock) -> Result<Done> {
    change(store, clock, |snapshot, _| {
        let index = Index::new(snapshot, clock.today);
        let current = snapshot
            .tasks
            .iter()
            .find(|task| task.id == request.task)
            .filter(|task| !matches!(index.list_of(task), List::Trash | List::Logbook))
            .ok_or(Error::TaskUnavailable)?;
        if current.sync.updated_at != request.expected_updated_at
            || request
                .source
                .is_some_and(|lane| !contains(&index, lane, current))
        {
            return Err(Error::Conflict(
                "задача изменилась. Перетащите её снова".into(),
            ));
        }
        let lane = request.destination;
        if let Some(source) = request.source {
            validate_lane(&index, source)?;
        }
        let today = matches!(lane, TaskLane::Today { .. });
        let schedule_from_sidebar =
            request.position == DropPosition::Sidebar && (today || lane == TaskLane::Anytime);
        let sorting = request.source == Some(lane) && !schedule_from_sidebar;
        let mut task = current.clone();
        if !sorting {
            validate_lane(&index, lane)?;
            assign(&index, lane, &mut task);
        }
        if !contains(&index, lane, &task) {
            reject!(
                "задача не попадёт в выбранный список при её текущем сроке или расписании проекта"
            );
        }

        let mut peers: Vec<_> = snapshot
            .tasks
            .iter()
            .filter(|peer| contains(&index, lane, peer))
            .map(Item::Task)
            .collect();
        if let TaskLane::Today { evening } = lane {
            peers.extend(
                snapshot
                    .projects
                    .iter()
                    .filter(|project| {
                        place_project(project, index.area_of_project(project), index.today())
                            == List::Today { evening }
                    })
                    .map(Item::Project),
            );
        }
        if today {
            peers.sort_by(Item::cmp_today);
        } else {
            peers.sort_by(|a, b| a.index().cmp(b.index()));
        }
        let current_position = peers
            .iter()
            .position(|item| anchor(*item) == Some(DropAnchor::Task(task.id)));
        if let Some(position) = current_position {
            peers.remove(position);
        }
        let position = position_in(&peers, request.position, task.id, current_position)?;
        let mut records = Vec::new();
        if current_position != Some(position) {
            let before = position
                .checked_sub(1)
                .and_then(|i| peers.get(i))
                .map(|p| rank(*p, today));
            let after = peers.get(position).map(|p| rank(*p, today));
            let next = match (before, after) {
                (Some(left), Some(right)) if left == right => {
                    // При совпадающих ключах раздвигаем только соседей с этим ключом.
                    let tied = peers[position..]
                        .iter()
                        .take_while(|p| rank(**p, today) == left);
                    let count = tied.clone().count();
                    let upper = snapshot
                        .tasks
                        .iter()
                        .map(Item::Task)
                        .chain(snapshot.projects.iter().map(Item::Project))
                        .map(|p| rank(p, today))
                        .filter(|p| *p > left)
                        .min();
                    let mut ranks = Rank::n_between(Some(left), upper, count + 1).into_iter();
                    let Some(inserted) = ranks.next() else {
                        unreachable!("пустой набор ключей")
                    };
                    records.extend(tied.zip(ranks).map(|(p, key)| ranked(*p, today, key)));
                    inserted
                }
                _ => Rank::try_between(before, after).map_err(|error| {
                    Error::Rejected(format!("не удалось задать порядок задачи: {error}"))
                })?,
            };
            if today {
                task.today_index = next;
            } else {
                task.index = next;
            }
        }
        let action = if sorting {
            "сортировка"
        } else {
            "перенос"
        };
        let effect = Effect::Moved(task.id);
        let action = format!("{action} задачи «{}»", task.title);
        records.push(Record::Task(task));
        Ok(Changed {
            effect,
            action,
            records,
        })
    })
}

fn contains(index: &Index<'_>, lane: TaskLane, task: &Task) -> bool {
    let list = index.list_of(task);
    if matches!(list, List::Trash | List::Logbook) {
        return false;
    }
    match lane {
        TaskLane::Inbox => list == List::Inbox,
        TaskLane::Today { evening } => list == List::Today { evening },
        TaskLane::Anytime => {
            matches!(list, List::Anytime | List::Today { .. })
                && index.project_of(task).is_none_or(|p| {
                    !matches!(
                        place_project(p, None, index.today()),
                        List::Upcoming | List::Someday
                    )
                })
        }
        TaskLane::Someday => {
            list == List::Someday
                && index
                    .project_of(task)
                    .is_none_or(|p| place_project(p, None, index.today()) != List::Someday)
        }
        TaskLane::Project(project, heading) => {
            task.project == Some(project) && task.heading == heading
        }
        TaskLane::Area(area) => task.project.is_none() && task.area == Some(area),
    }
}

fn validate_lane(index: &Index<'_>, lane: TaskLane) -> Result<()> {
    let snapshot = index.snapshot();
    match lane {
        TaskLane::Project(id, heading) => {
            live_project(snapshot, id)?;
            if let Some(heading) = heading
                && !index
                    .heading(heading)
                    .is_some_and(|h| h.project == id && history::heading_available(snapshot, h))
            {
                return Err(Error::Conflict("раздел больше недоступен".into()));
            }
        }
        TaskLane::Area(id) => {
            live_area(snapshot, id)?;
        }
        _ => {}
    }
    Ok(())
}

fn assign(index: &Index<'_>, lane: TaskLane, task: &mut Task) {
    match lane {
        TaskLane::Inbox => {
            task.start = Start::Inbox;
            task.date = None;
            task.evening = false;
            task.project = None;
            task.heading = None;
            task.area = None;
        }
        TaskLane::Today { evening } => When::On {
            date: index.today(),
            evening,
        }
        .apply_to_task(task, task.sync.updated_at),
        TaskLane::Anytime => When::Anytime.apply_to_task(task, task.sync.updated_at),
        TaskLane::Someday => When::Someday.apply_to_task(task, task.sync.updated_at),
        TaskLane::Project(id, heading) => {
            task.project = Some(id);
            task.heading = heading;
            task.area = None;
            if task.start == Start::Inbox {
                task.start = Start::Anytime;
            }
        }
        TaskLane::Area(id) => {
            task.area = Some(id);
            task.project = None;
            task.heading = None;
            if task.start == Start::Inbox {
                task.start = Start::Anytime;
            }
        }
    }
}

fn position_in(
    peers: &[Item<'_>],
    position: DropPosition,
    source: TaskId,
    current: Option<usize>,
) -> Result<usize> {
    let (target, offset) = match position {
        DropPosition::Before(target) => (target, 0),
        DropPosition::After(target) => (target, 1),
        DropPosition::End | DropPosition::Sidebar => return Ok(peers.len()),
    };
    if target == DropAnchor::Task(source)
        && let Some(current) = current
    {
        return Ok(current);
    }
    peers
        .iter()
        .position(|item| anchor(*item) == Some(target))
        .map(|position| position + offset)
        .ok_or_else(|| Error::Conflict("место вставки изменилось. Перетащите задачу снова".into()))
}

fn anchor(item: Item<'_>) -> Option<DropAnchor> {
    match item {
        Item::Task(task) => Some(DropAnchor::Task(task.id)),
        Item::Project(project) => Some(DropAnchor::Project(project.id)),
        _ => None,
    }
}

fn rank(item: Item<'_>, today: bool) -> &Rank {
    if today {
        item.today_index()
    } else {
        item.index()
    }
}

fn ranked(item: Item<'_>, today: bool, rank: Rank) -> Record {
    match item {
        Item::Task(task) => {
            let mut task = task.clone();
            if today {
                task.today_index = rank;
            } else {
                task.index = rank;
            }
            Record::Task(task)
        }
        Item::Project(project) => {
            let mut project = project.clone();
            project.today_index = rank;
            Record::Project(project)
        }
        _ => unreachable!("в списке порядка только задачи и проекты"),
    }
}
