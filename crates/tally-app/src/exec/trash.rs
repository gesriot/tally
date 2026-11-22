//! Общие команды перемещения в корзину, восстановления и очистки.

use super::*;
use crate::TrashPreview;

pub fn trash_preview(s: &Snapshot) -> TrashPreview {
    let mut preview = TrashPreview::default();
    let mut areas = HashSet::new();
    let mut projects = HashSet::new();
    let mut headings = HashSet::new();
    for area in s.areas.iter().filter(|a| a.sync.is_deleted()) {
        areas.insert(area.id);
        preview
            .records
            .push((RecordId::Area(area.id), area.sync.updated_at));
    }
    for project in &s.projects {
        if project.sync.is_deleted() || project.area.is_some_and(|id| areas.contains(&id)) {
            projects.insert(project.id);
            preview
                .records
                .push((RecordId::Project(project.id), project.sync.updated_at));
        }
    }
    for heading in &s.headings {
        if heading.sync.is_deleted() || projects.contains(&heading.project) {
            headings.insert(heading.id);
            preview
                .records
                .push((RecordId::Heading(heading.id), heading.sync.updated_at));
        }
    }
    for tag in s.tags.iter().filter(|t| t.sync.is_deleted()) {
        preview
            .records
            .push((RecordId::Tag(tag.id), tag.sync.updated_at));
    }
    for task in &s.tasks {
        let purged = task.sync.is_deleted()
            || task.area.is_some_and(|id| areas.contains(&id))
            || task.project.is_some_and(|id| projects.contains(&id))
            || task.heading.is_some_and(|id| headings.contains(&id));
        if purged {
            preview
                .records
                .push((RecordId::Task(task.id), task.sync.updated_at));
        }
        preview.checklist_items.extend(
            task.checklist
                .iter()
                .filter(|item| purged || item.sync.is_deleted())
                .map(|item| (item.id, item.sync.updated_at)),
        );
    }
    preview.records.sort_unstable();
    preview.checklist_items.sort_unstable();
    preview
}

pub(super) fn set(
    store: &mut Store,
    id: RecordId,
    expected: Timestamp,
    trashed: bool,
    clock: Clock,
) -> Result<Done> {
    change(store, clock, |s, now| {
        let mut record = history::find(s, id)
            .ok_or_else(|| Error::Conflict("запись больше недоступна".into()))?;
        if record.sync().updated_at != expected {
            return Err(Error::Conflict(
                "запись изменилась. Повторите действие".into(),
            ));
        }
        let ix = Index::new(s, clock.today);
        let (title, in_deleted_container) = match &record {
            Record::Task(t) => {
                let inherited =
                    Deleted::of_task(t, ix.project_of(t), ix.heading_of(t), ix.area_of(t));
                (
                    &t.title,
                    inherited.heading || inherited.project || inherited.area,
                )
            }
            Record::Project(p) => (
                &p.title,
                ix.area_of_project(p).is_some_and(|a| a.sync.is_deleted()),
            ),
            Record::Heading(h) => (
                &h.title,
                ix.project(h.project)
                    .is_some_and(|p| Deleted::of_project(p, ix.area_of_project(p)).is_trashed()),
            ),
            Record::Area(a) => (&a.title, false),
            Record::Tag(_) => reject!("перемещение тегов в корзину недоступно"),
        };
        if in_deleted_container {
            return Err(Error::Conflict(
                "сначала восстановите проект, раздел или область".into(),
            ));
        }
        let title = title.clone();
        let verb = if trashed {
            if !record.sync().is_deleted() {
                record.sync_mut().delete(now);
            }
            "перенос в корзину"
        } else {
            record.sync_mut().restore(now);
            "восстановление из корзины"
        };
        Ok(Changed {
            action: format!("{verb} «{title}»"),
            effect: Effect::TrashChanged { title, trashed },
            records: vec![record],
        })
    })
}

pub(super) fn empty(store: &mut Store, preview: &TrashPreview) -> Result<Done> {
    store.write_with(|w| {
        let before = w.read().snapshot()?;
        if trash_preview(&before) != *preview {
            return Err(Error::Conflict(
                "содержимое корзины изменилось. Подтвердите очистку ещё раз".into(),
            ));
        }
        w.empty_trash()?;
        let after = w.read().snapshot()?;
        history::forget_purged(w, &before, &after)?;
        Ok(Done {
            snapshot: after,
            seq: None,
            effect: Effect::TrashEmptied,
        })
    })
}
