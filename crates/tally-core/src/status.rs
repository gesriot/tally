//! Состояние выполнения.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// Открыта, выполнена или отменена (с моментом закрытия).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Status {
    #[default]
    Open,
    Completed {
        at: Timestamp,
    },
    Canceled {
        at: Timestamp,
    },
}

impl Status {
    pub fn is_open(self) -> bool {
        matches!(self, Self::Open)
    }

    /// Момент выполнения или отмены.
    pub fn closed_at(self) -> Option<Timestamp> {
        match self {
            Self::Open => None,
            Self::Completed { at } | Self::Canceled { at } => Some(at),
        }
    }
}
