//! Пункт чек-листа внутри задачи.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use tally_rank::Rank;

use crate::ids::ChecklistItemId;
use crate::meta::SyncMeta;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChecklistItem {
    pub id: ChecklistItemId,
    pub title: String,
    pub done: bool,
    pub index: Rank,
    pub sync: SyncMeta,
}

impl ChecklistItem {
    pub fn new(id: ChecklistItemId, title: impl Into<String>, index: Rank, at: Timestamp) -> Self {
        Self {
            id,
            title: title.into(),
            done: false,
            index,
            sync: SyncMeta::new(at),
        }
    }
}
