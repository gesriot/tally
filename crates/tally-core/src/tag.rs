//! Тег. Теги могут быть вложенными (`parent`).

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use tally_rank::Rank;

use crate::ids::TagId;
use crate::meta::SyncMeta;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    pub id: TagId,
    pub title: String,
    pub parent: Option<TagId>,
    pub index: Rank,
    pub sync: SyncMeta,
}

impl Tag {
    pub fn new(id: TagId, title: impl Into<String>, index: Rank, at: Timestamp) -> Self {
        Self {
            id,
            title: title.into(),
            parent: None,
            index,
            sync: SyncMeta::new(at),
        }
    }
}
