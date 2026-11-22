//! Идентификаторы записей: отдельный тип на каждую сущность, внутри – UUID.
//!
//! Ядро не генерирует UUID само (это случайность, то есть внешний мир):
//! их создаёт вызывающий код и передаёт в конструкторы.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            pub const fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            /// Детерминированный id – для тестов и примеров.
            pub const fn from_u128(v: u128) -> Self {
                Self(Uuid::from_u128(v))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

id_type!(
    /// Id задачи.
    TaskId
);
id_type!(
    /// Id проекта.
    ProjectId
);
id_type!(
    /// Id области.
    AreaId
);
id_type!(
    /// Id раздела внутри проекта.
    HeadingId
);
id_type!(
    /// Id тега.
    TagId
);
id_type!(
    /// Id пункта чек-листа.
    ChecklistItemId
);
