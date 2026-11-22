//! Цепочка удаления в SQL: те же признаки, что у `tally_core::lists::Deleted`,
//! чтобы «в корзине ли запись» решало одно правило – и для списков, и для
//! поиска, и для очистки.
//!
//! Алиасы: `a` – сама запись, `h` – её раздел, `p` – проект, `ar` – область
//! (у задачи в проекте – область проекта, вне проекта – своя, как в `tally-core`).

use rusqlite::Row;
use tally_core::lists::Deleted;

use crate::error::Kind;

pub(crate) struct Chain {
    /// `FROM … LEFT JOIN …` с алиасами из описания модуля.
    pub(crate) from: &'static str,
    /// Четыре колонки-признака: своё удаление, раздела, проекта, области.
    pub(crate) flags: &'static str,
}

pub(crate) fn chain(kind: Kind) -> Chain {
    match kind {
        Kind::Task => Chain {
            from: "FROM tasks a \
                   LEFT JOIN headings h ON h.id = a.heading_id \
                   LEFT JOIN projects p ON p.id = a.project_id \
                   LEFT JOIN areas ar ON ar.id = CASE WHEN a.project_id IS NULL \
                   THEN a.area_id ELSE p.area_id END",
            flags: "a.deleted_at IS NOT NULL, h.deleted_at IS NOT NULL, \
                    p.deleted_at IS NOT NULL, ar.deleted_at IS NOT NULL",
        },
        Kind::Heading => Chain {
            from: "FROM headings a \
                   JOIN projects p ON p.id = a.project_id \
                   LEFT JOIN areas ar ON ar.id = p.area_id",
            flags: "a.deleted_at IS NOT NULL, 0, \
                    p.deleted_at IS NOT NULL, ar.deleted_at IS NOT NULL",
        },
        Kind::Project => Chain {
            from: "FROM projects a LEFT JOIN areas ar ON ar.id = a.area_id",
            flags: "a.deleted_at IS NOT NULL, 0, 0, ar.deleted_at IS NOT NULL",
        },
        Kind::Area => Chain {
            from: "FROM areas a",
            flags: "a.deleted_at IS NOT NULL, 0, 0, 0",
        },
        Kind::Tag => Chain {
            from: "FROM tags a",
            flags: "a.deleted_at IS NOT NULL, 0, 0, 0",
        },
        Kind::ChecklistItem => Chain {
            from: "FROM checklist_items a JOIN tasks t ON t.id = a.task_id",
            flags: "a.deleted_at IS NOT NULL, 0, 0, 0",
        },
    }
}

/// Признаки из четырёх колонок, начиная с `start`.
pub(crate) fn deleted(row: &Row<'_>, start: usize) -> rusqlite::Result<Deleted> {
    Ok(Deleted {
        own: row.get(start)?,
        heading: row.get(start + 1)?,
        project: row.get(start + 2)?,
        area: row.get(start + 3)?,
    })
}
