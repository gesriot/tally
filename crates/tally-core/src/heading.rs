//! Раздел внутри проекта.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use tally_rank::Rank;

use crate::ids::{HeadingId, ProjectId};
use crate::meta::SyncMeta;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Heading {
    pub id: HeadingId,
    pub project: ProjectId,
    pub title: String,
    /// Архивный раздел (в Things – «Архивировать» после выполнения всех задач).
    pub archived: bool,
    pub index: Rank,
    pub sync: SyncMeta,
}

impl Heading {
    pub fn new(
        id: HeadingId,
        project: ProjectId,
        title: impl Into<String>,
        index: Rank,
        at: Timestamp,
    ) -> Self {
        Self {
            id,
            project,
            title: title.into(),
            archived: false,
            index,
            sync: SyncMeta::new(at),
        }
    }
}
