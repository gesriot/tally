use anyhow::{Context, Result};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use slint::winit_030::winit::window::Window;
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle,
    backend::{Backend, ObjectId},
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_registry, wl_surface::WlSurface},
};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1;

struct Activation;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Activation {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

wayland_client::delegate_noop!(Activation: ignore XdgActivationV1);

// Winit 0.30 cannot activate an existing Wayland window. Send the compositor's
// token to the surface Winit already owns, using that surface's connection.
#[allow(unsafe_code)]
pub(super) fn activate(window: &Window, token: &str) -> Result<()> {
    let (RawDisplayHandle::Wayland(display), RawWindowHandle::Wayland(surface)) = (
        window.display_handle()?.as_raw(),
        window.window_handle()?.as_raw(),
    ) else {
        return Ok(());
    };
    // SAFETY: the borrowed winit Window keeps both the display and the
    // wl_surface alive on this thread. Nothing from the foreign connection
    // escapes this call, and its backend does not own/disconnect the display.
    let connection = Connection::from_backend(unsafe {
        Backend::from_foreign_display(display.display.as_ptr().cast())
    });
    let id =
        unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.surface.as_ptr().cast()) }?;
    let surface = WlSurface::from_id(&connection, id)?;
    let (globals, queue) = registry_queue_init::<Activation>(&connection)?;
    let activation: XdgActivationV1 = globals
        .bind(&queue.handle(), 1..=1, ())
        .context("композитор не объявил xdg_activation_v1")?;
    activation.activate(token.to_owned(), &surface);
    activation.destroy();
    connection.flush()?;
    Ok(())
}
