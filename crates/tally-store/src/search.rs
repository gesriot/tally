//! Поиск по `search_text`: каждое слово запроса – подстрока нормализованного текста.
//!
//! Для v1 – `instr()` по колонке (полный просмотр таблиц, на тысячах записей это
//! миллисекунды). При росте объёма поверх той же колонки встанет FTS5.

use rusqlite::{Connection, params_from_iter};
use tally_core::{AreaId, ProjectId, TagId, TaskId};

use crate::codec::Decoder;
use crate::error::{Kind, Result};
use crate::normalize::{normalize, query_words};
use crate::table::{table_of, trash};

/// Что найдено.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Found {
    Area(AreaId),
    Project(ProjectId),
    Tag(TagId),
    Task(TaskId),
}

impl Found {
    /// Порядок групп в выдаче, как в быстром поиске Things: области,
    /// проекты, теги, затем задачи.
    fn group(self) -> u8 {
        match self {
            Self::Area(_) => 0,
            Self::Project(_) => 1,
            Self::Tag(_) => 2,
            Self::Task(_) => 3,
        }
    }
}

/// Результат поиска.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub found: Found,
    pub title: String,
    /// Запись не выполнена и не отменена (у областей и тегов – всегда `true`).
    pub open: bool,
    /// Все слова запроса есть в самом названии, а не только в заметках.
    pub in_title: bool,
}

/// Таблица поиска: вид записи (по нему – цепочка удаления из
/// [`crate::table::trash`]), выражение «открыта» и способ собрать [`Found`].
struct Source {
    kind: Kind,
    open: &'static str,
    found: fn(uuid::Uuid) -> Found,
}

const SOURCES: [Source; 4] = [
    Source {
        kind: Kind::Area,
        open: "1",
        found: |id| Found::Area(AreaId::from_uuid(id)),
    },
    Source {
        kind: Kind::Project,
        open: "a.status = 'open'",
        found: |id| Found::Project(ProjectId::from_uuid(id)),
    },
    Source {
        kind: Kind::Tag,
        open: "1",
        found: |id| Found::Tag(TagId::from_uuid(id)),
    },
    Source {
        kind: Kind::Task,
        open: "a.status = 'open' AND coalesce(p.status, 'open') = 'open'",
        found: |id| Found::Task(TaskId::from_uuid(id)),
    },
];

/// Поиск без учёта регистра и «ё». Пустой запрос – пустой результат.
///
/// Порядок: совпадение в названии, затем открытые, затем по группам
/// ([`Found`]), внутри – по названию.
pub(crate) fn search(conn: &Connection, query: &str) -> Result<Vec<Hit>> {
    let words = query_words(query);
    if words.is_empty() {
        return Ok(Vec::new());
    }
    let condition: String = (1..=words.len())
        .map(|i| format!(" AND instr(a.search_text, ?{i}) > 0"))
        .collect();
    let mut hits = Vec::new();
    for source in &SOURCES {
        let d = Decoder {
            table: table_of(source.kind),
        };
        let chain = trash::chain(source.kind);
        let sql = format!(
            "SELECT a.id, a.title, {}, {} {} WHERE 1{condition}",
            source.open, chain.flags, chain.from
        );
        let mut stmt = conn.prepare_cached(&sql)?;
        let rows = stmt
            .query_map(params_from_iter(words.iter()), |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, bool>(2)?,
                    trash::deleted(r, 3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        // Удалённое – своё или через раздел, проект, область – не находится:
        // правило то же, что у списков (`Deleted::is_trashed`).
        for (id, title, open, _) in rows.into_iter().filter(|r| !r.3.is_trashed()) {
            let normalized = normalize(&title);
            hits.push(Hit {
                found: (source.found)(d.parse("id", &id)?),
                in_title: words.iter().all(|w| normalized.contains(w.as_str())),
                open,
                title,
            });
        }
    }
    hits.sort_by_cached_key(|h| (!h.in_title, !h.open, h.found.group(), normalize(&h.title)));
    Ok(hits)
}
