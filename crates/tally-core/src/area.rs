//! Область ответственности («Работа», «Дом»). Области всегда активны.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use tally_rank::Rank;

use crate::ids::{AreaId, TagId};
use crate::meta::SyncMeta;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Area {
    pub id: AreaId,
    pub title: String,
    pub tags: Vec<TagId>,
    pub index: Rank,
    pub sync: SyncMeta,
}

impl Area {
    pub fn new(id: AreaId, title: impl Into<String>, index: Rank, at: Timestamp) -> Self {
        Self {
            id,
            title: title.into(),
            tags: Vec::new(),
            index,
            sync: SyncMeta::new(at),
        }
    }
}
