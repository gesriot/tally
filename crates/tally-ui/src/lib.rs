//! Окно Tally на Slint: боковая панель (стандартные списки, области,
//! проекты), список задач, отметка выполнения, отмена и повтор.
//!
//! * [`view`] – строки боковой панели и списка из снимка базы (правила
//!   списков – `tally-core`, как у `tally list`);
//! * [`controller`] – состояние окна и решения по нажатиям;
//! * [`keys`] – сочетания клавиш;
//! * [`backend`] – поток – владелец базы и канал к нему;
//! * [`strings`] – все строки интерфейса;
//! * [`bind`] – перенос всего этого в окно Slint (`ui/*.slint`).

pub mod backend;
pub mod bind;
pub mod controller;
pub mod keys;
pub mod strings;
pub mod taskbar;
pub mod view;

/// Код, сгенерированный Slint; в нём есть `unwrap()` и `todo!()`, поэтому
/// линт отключён только здесь.
#[allow(clippy::unwrap_used, clippy::todo)]
pub mod ui {
    slint::include_modules!();
}

#[cfg(not(any(feature = "femtovg", feature = "skia")))]
compile_error!("включите фичу `skia` или `femtovg`");

/// Рендерер выбирается фичей сборки; Skia – по умолчанию и важнее, если
/// включены обе.
pub const RENDERER: &str = if cfg!(feature = "skia") {
    "skia"
} else {
    "femtovg"
};
