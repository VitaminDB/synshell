//! `gtk_shell1` (protocols/gtk-shell.xml): GTK3-программа сообщает, где на
//! D-Bus её меню (`org.gtk.Menus`) и действия (`org.gtk.Actions`), — так
//! глобальное меню оболочки показывает меню GTK (GIMP с `GIMP_GTK_MENUBAR`).
//! Возможность `global_menu_bar` объявляется, только пока на панели есть
//! апплет `appmenu`: по ней GTK убирает строку меню из окна.

use std::sync::Mutex;

use smithay::reexports::wayland_server::{
    backend::ClientId, protocol::wl_surface::WlSurface, Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New,
    Resource,
};
use smithay::wayland::compositor::with_states;
use syndesktop_common::ipc::GtkMenu;

use crate::state::State;

#[allow(non_upper_case_globals, non_camel_case_types, unused_imports, missing_docs, clippy::all)]
pub mod protocol {
    use wayland_server;
    use wayland_server::protocol::*;

    pub mod __interfaces {
        use wayland_server::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/gtk-shell.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_server_code!("protocols/gtk-shell.xml");
}

use protocol::gtk_shell1::{self, GtkShell1};
use protocol::gtk_surface1::{self, GtkSurface1};

#[derive(Default)]
struct GtkMenuAddr(Mutex<Option<GtkMenu>>);

pub fn init(dh: &DisplayHandle) {
    dh.create_global::<State, GtkShell1, ()>(3, ());
}

/// Меню GTK окна с поверхностью `surface`.
pub fn menu(surface: &WlSurface) -> Option<GtkMenu> {
    with_states(surface, |s| s.data_map.get::<GtkMenuAddr>().and_then(|a| a.0.lock().unwrap().clone()))
}

fn set(state: &mut State, surface: &WlSurface, menu: Option<GtkMenu>) {
    if !surface.is_alive() {
        return;
    }
    with_states(surface, |s| {
        s.data_map.insert_if_missing_threadsafe(GtkMenuAddr::default);
        *s.data_map.get::<GtkMenuAddr>().unwrap().0.lock().unwrap() = menu;
    });
    state.core.ipc_dirty = true;
}

/// Есть ли на панелях апплет глобального меню.
fn wants_global_menu(state: &State) -> bool {
    state.core.config.panels.iter().any(|p| p.applets.iter().any(|a| a.kind == "appmenu"))
}

impl GlobalDispatch<GtkShell1, ()> for State {
    fn bind(state: &mut State, _dh: &DisplayHandle, _client: &Client, resource: New<GtkShell1>, _data: &(), di: &mut DataInit<'_, State>) {
        let shell = di.init(resource, ());
        let caps = if wants_global_menu(state) { gtk_shell1::Capability::GlobalMenuBar as u32 } else { 0 };
        shell.capabilities(caps);
    }
}

impl Dispatch<GtkShell1, ()> for State {
    fn request(
        _state: &mut State,
        _client: &Client,
        _resource: &GtkShell1,
        request: gtk_shell1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        di: &mut DataInit<'_, State>,
    ) {
        if let gtk_shell1::Request::GetGtkSurface { gtk_surface, surface } = request {
            di.init(gtk_surface, surface);
        }
    }
}

impl Dispatch<GtkSurface1, WlSurface> for State {
    fn request(
        state: &mut State,
        _client: &Client,
        _resource: &GtkSurface1,
        request: gtk_surface1::Request,
        surface: &WlSurface,
        _dh: &DisplayHandle,
        _di: &mut DataInit<'_, State>,
    ) {
        match request {
            gtk_surface1::Request::SetDbusProperties {
                application_id: _,
                app_menu_path: _,
                menubar_path,
                window_object_path,
                application_object_path,
                unique_bus_name,
            } => {
                let menu = match (menubar_path, unique_bus_name) {
                    (Some(menubar), Some(bus)) if !menubar.is_empty() && !bus.is_empty() => Some(GtkMenu {
                        bus,
                        menubar,
                        app_path: application_object_path.unwrap_or_default(),
                        window_path: window_object_path.unwrap_or_default(),
                    }),
                    _ => None,
                };
                tracing::debug!(?menu, "меню GTK");
                set(state, surface, menu);
            }
            // Показать окно (gtk_window_present): как xdg-activation без токена.
            gtk_surface1::Request::Present { .. } => {
                let allowed = state.core.config.windows.focus_stealing;
                state.activate_or_mark_urgent(surface, allowed);
            }
            gtk_surface1::Request::RequestFocus { startup_id } => {
                let allowed = startup_id.is_some() || state.core.config.windows.focus_stealing;
                state.activate_or_mark_urgent(surface, allowed);
            }
            _ => {}
        }
    }

    fn destroyed(state: &mut State, _client: ClientId, _resource: &GtkSurface1, surface: &WlSurface) {
        set(state, surface, None);
    }
}
