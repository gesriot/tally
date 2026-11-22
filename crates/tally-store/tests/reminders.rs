mod common;

use common::{area, at, heading, project, t0, task};
use tally_core::Status;
use tally_store::{Result, Store};

#[test]
fn confirms_open_task_reminders_and_respects_closed_and_trashed_containers() -> Result<()> {
    let at = at("2026-11-19T09:00:00.000000001Z");
    let mut store = Store::open_in_memory()?;
    let mut area = area(1, "Скрытая область");
    area.sync.deleted_at = Some(t0());
    let mut closed = project(2, "Завершённый проект");
    closed.status = Status::Completed { at: t0() };
    let open = project(3, "Открытый проект");
    let mut heading = heading(4, open.id, "Скрытый раздел");
    heading.sync.deleted_at = Some(t0());
    let mut eligible = task(5, "Напомнить");
    let mut in_project = task(6, "В завершённом проекте");
    in_project.project = Some(closed.id);
    let mut in_heading = task(7, "В удалённом разделе");
    in_heading.project = Some(open.id);
    in_heading.heading = Some(heading.id);
    let mut in_area = task(8, "В удалённой области");
    in_area.area = Some(area.id);
    let mut completed = task(9, "Завершённая задача");
    completed.status = Status::Completed { at: t0() };
    let mut trashed = task(10, "Удалённая задача");
    trashed.sync.deleted_at = Some(t0());
    store.write(|w| {
        w.insert_area(&area)?;
        w.insert_project(&closed)?;
        w.insert_project(&open)?;
        w.insert_heading(&heading)?;
        for t in [
            &mut eligible,
            &mut in_project,
            &mut in_heading,
            &mut in_area,
            &mut completed,
            &mut trashed,
        ] {
            t.reminder_at = Some(at);
            w.insert_task(t)?;
        }
        Ok(())
    })?;
    assert!(store.claim_reminders(t0())?.is_empty());
    let due = store.claim_reminders(at)?;
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].task, eligible.id);
    assert_eq!(due[0].at, at);
    assert_eq!(store.claim_reminders(at)?, due);
    store.confirm_reminders(&due)?;
    assert!(store.claim_reminders(at)?.is_empty());
    Ok(())
}
