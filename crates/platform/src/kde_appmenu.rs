//! Tell KWin where a window's menu lives (fork, owner 2026-10-13: the menu in Plasma's Global Menu /
//! macOS-style top panel, as Qt and LibreOffice do it).
//!
//! The menu itself is a `com.canonical.dbusmenu` object the app serves on the session bus; on
//! Wayland the compositor links it to the window through KDE's `org_kde_kwin_appmenu` protocol:
//! one `set_address(bus name, object path)` on the window's `wl_surface`. The window belongs to
//! winit, so this talks to winit's `wl_display` through a second, private event queue (libwayland
//! is thread-safe per queue) and wraps winit's `wl_surface` as a proxy without taking it over.
//!
//! Compositors without the protocol (GNOME, Sway…) simply don't advertise it: [`register`] returns
//! an error and the app keeps its menu bar in the window.
//!
//! This is the only `unsafe` in it: adopting two pointers winit hands out through
//! `raw-window-handle` (ADR 0001, `crates/platform` is the FFI crate).

use std::ffi::c_void;
use std::ptr::NonNull;

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry, wl_surface::WlSurface};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_plasma::appmenu::client::org_kde_kwin_appmenu::OrgKdeKwinAppmenu;
use wayland_protocols_plasma::appmenu::client::org_kde_kwin_appmenu_manager::OrgKdeKwinAppmenuManager;

/// The registration: keep it alive as long as the window shows the menu. Dropping it releases the
/// appmenu object (the panel stops showing the menu).
pub struct AppMenu {
    menu: OrgKdeKwinAppmenu,
    conn: Connection,
}

impl Drop for AppMenu {
    fn drop(&mut self) {
        self.menu.release();
        let _ = self.conn.flush();
    }
}

struct State;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<OrgKdeKwinAppmenuManager, ()> for State {
    fn event(_: &mut Self, _: &OrgKdeKwinAppmenuManager, _: <OrgKdeKwinAppmenuManager as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<OrgKdeKwinAppmenu, ()> for State {
    fn event(_: &mut Self, _: &OrgKdeKwinAppmenu, _: <OrgKdeKwinAppmenu as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

/// Link the window's surface to the dbusmenu object at `service` / `path`.
///
/// `display` and `surface` are the `wl_display` / `wl_surface` pointers from the window's
/// `raw-window-handle` (Wayland variants). They must stay valid while the returned [`AppMenu`]
/// lives: the window outlives it as long as the app keeps it next to the window (eframe drops the
/// app before it closes the window).
pub fn register(display: NonNull<c_void>, surface: NonNull<c_void>, service: &str, path: &str) -> Result<AppMenu, String> {
    // SAFETY: `display` is winit's live `wl_display` (raw-window-handle's Wayland display handle);
    // a foreign-display backend never closes it, and the caller keeps the window (and so the
    // display) alive longer than the returned value.
    let backend = unsafe { wayland_backend::client::Backend::from_foreign_display(display.as_ptr().cast()) };
    let conn = Connection::from_backend(backend);
    // SAFETY: `surface` is winit's live `wl_surface` proxy on that display; `from_ptr` checks the
    // interface and fails on a mismatch. The id is only used while the window exists (see above).
    let id =
        unsafe { wayland_backend::client::ObjectId::from_ptr(WlSurface::interface(), surface.as_ptr().cast()) }.map_err(|e| format!("window surface: {e}"))?;
    let surface = WlSurface::from_id(&conn, id).map_err(|e| format!("window surface: {e}"))?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn).map_err(|e| format!("Wayland registry: {e}"))?;
    let qh = queue.handle();
    let manager: OrgKdeKwinAppmenuManager = globals.bind(&qh, 1..=2, ()).map_err(|_| "the compositor has no org_kde_kwin_appmenu (not KWin)".to_string())?;
    let menu = manager.create(&surface, &qh, ());
    menu.set_address(service.into(), path.into());
    queue.roundtrip(&mut State).map_err(|e| format!("Wayland roundtrip: {e}"))?;
    // the manager is only needed to create the menu object
    if manager.version() >= 2 {
        manager.release();
    }
    let _ = conn.flush();
    Ok(AppMenu { menu, conn })
}
