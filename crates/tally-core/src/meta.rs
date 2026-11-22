//! Поля синхронизации, общие для всех записей.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// Когда запись создана, изменена и (мягко) удалена.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncMeta {
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// `Some` – запись в корзине. Физически её удаляет только очистка корзины.
    pub deleted_at: Option<Timestamp>,
}

impl SyncMeta {
    /// Новая запись: создана и изменена в момент `at`.
    pub fn new(at: Timestamp) -> Self {
        Self {
            created_at: at,
            updated_at: at,
            deleted_at: None,
        }
    }

    pub fn is_deleted(&self) -> bool {
        self.deleted_at.is_some()
    }

    /// Отметить изменение в момент `at`.
    pub fn touch(&mut self, at: Timestamp) {
        self.updated_at = at;
    }

    /// Переместить в корзину в момент `at`.
    pub fn delete(&mut self, at: Timestamp) {
        self.deleted_at = Some(at);
        self.updated_at = at;
    }

    /// Вернуть из корзины в момент `at`.
    pub fn restore(&mut self, at: Timestamp) {
        self.deleted_at = None;
        self.updated_at = at;
    }
}
