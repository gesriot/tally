//! Значки окна в Windows: маленький (заголовок) и большой (панель задач,
//! Alt+Tab).
//!
//! Slint отдаёт winit только `set_window_icon`, а в Windows это лишь маленький
//! значок. Без большого панель задач остаётся с пустой кнопкой, даже если в
//! самом exe значок есть. Поэтому после каждого показа окна оба значка
//! выставляются здесь.
//!
//! Размер выбирается по масштабу экрана, как принято в Windows: 16 и 32
//! логических пикселей (при 125 % – 20 и 40, при 150 % – 24 и 48 и так далее).
//! Берётся кадр нужного размера из значка, вшитого в exe (`1 ICON` в
//! `scripts/package-windows.ps1`). В сборке без него (например, `cargo run`)
//! используется картинка `Theme.app-icon`, которую система сама уменьшит.

use slint::{ComponentHandle, Global};

use crate::ui;

/// Поставить окну значки. Вызывать после `show()`: окно ОС создаётся там.
/// Вне Windows ничего не делает.
pub fn set_icons<'a, C>(component: &'a C)
where
    C: ComponentHandle,
    ui::Theme<'a>: Global<'a, C>,
{
    #[cfg(target_os = "windows")]
    windows::apply(
        component.window(),
        &ui::Theme::get(component).get_app_icon(),
    );
    #[cfg(not(target_os = "windows"))]
    let _ = component;
}

#[cfg(target_os = "windows")]
mod windows {
    use slint::winit_030::WinitWindowAccessor;
    use slint::winit_030::winit::dpi::PhysicalSize;
    use slint::winit_030::winit::platform::windows::{IconExtWindows, WindowExtWindows};
    use slint::winit_030::winit::window::Icon;

    /// Номер ресурса значка в exe (`1 ICON "app.ico"`).
    const RESOURCE_ID: u16 = 1;
    /// Размеры значков в логических пикселях при масштабе 100 %.
    const SMALL: u32 = 16;
    const BIG: u32 = 32;

    pub fn apply(window: &slint::Window, fallback: &slint::Image) {
        window.with_winit_window(|native| {
            let scale = native.scale_factor();
            let side = |logical: u32| (f64::from(logical) * scale).round() as u32;
            native.set_window_icon(icon(side(SMALL), fallback));
            native.set_taskbar_icon(icon(side(BIG), fallback));
        });
    }

    fn icon(side: u32, fallback: &slint::Image) -> Option<Icon> {
        Icon::from_resource(RESOURCE_ID, Some(PhysicalSize::new(side, side)))
            .ok()
            .or_else(|| from_image(fallback))
    }

    fn from_image(image: &slint::Image) -> Option<Icon> {
        let pixels = image.to_rgba8()?;
        Icon::from_rgba(pixels.as_bytes().to_vec(), pixels.width(), pixels.height()).ok()
    }
}
