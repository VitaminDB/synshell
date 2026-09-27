//! `org_kde_kwin_appmenu`: окно сообщает, где на D-Bus его меню
//! (`com.canonical.dbusmenu`) — так делают Qt/KDE-программы. Адрес уходит
//! оболочке в `WindowInfo::appmenu`, глобальное меню рисует апплет `appmenu`.

use std::sync::Mutex;

use smithay::reexports::wayland_server::{
    backend::ClientId, protocol::wl_surface::WlSurface, Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New,
    Resource,
};
use smithay::wayland::compositor::with_states;
use wayland_protocols_plasma::appmenu::server::{
    org_kde_kwin_appmenu::{self, OrgKdeKwinAppmenu},
    org_kde_kwin_appmenu_manager::{self, OrgKdeKwinAppmenuManager},
};

use crate::state::State;

/// Адрес меню поверхности: (служба, путь).
#[derive(Default)]
struct AppMenuAddr(Mutex<Option<(String, String)>>);

pub fn init(dh: &DisplayHandle) {
    dh.create_global::<State, OrgKdeKwinAppmenuManager, ()>(2, ());
}

/// Меню окна с поверхностью `surface`, если оно сообщило адрес.
pub fn address(surface: &WlSurface) -> Option<(String, String)> {
    with_states(surface, |s| s.data_map.get::<AppMenuAddr>().and_then(|a| a.0.lock().unwrap().clone()))
}

fn set(state: &mut State, surface: &WlSurface, addr: Option<(String, String)>) {
    if !surface.is_alive() {
        return;
    }
    with_states(surface, |s| {
        s.data_map.insert_if_missing_threadsafe(AppMenuAddr::default);
        *s.data_map.get::<AppMenuAddr>().unwrap().0.lock().unwrap() = addr;
    });
    state.core.ipc_dirty = true;
}

impl GlobalDispatch<OrgKdeKwinAppmenuManager, ()> for State {
    fn bind(
        _state: &mut State,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<OrgKdeKwinAppmenuManager>,
        _data: &(),
        di: &mut DataInit<'_, State>,
    ) {
        di.init(resource, ());
    }
}

impl Dispatch<OrgKdeKwinAppmenuManager, ()> for State {
    fn request(
        _state: &mut State,
        _client: &Client,
        _resource: &OrgKdeKwinAppmenuManager,
        request: org_kde_kwin_appmenu_manager::Request,
        _data: &(),
        _dh: &DisplayHandle,
        di: &mut DataInit<'_, State>,
    ) {
        if let org_kde_kwin_appmenu_manager::Request::Create { id, surface } = request {
            di.init(id, surface);
        }
    }
}

impl Dispatch<OrgKdeKwinAppmenu, WlSurface> for State {
    fn request(
        state: &mut State,
        _client: &Client,
        _resource: &OrgKdeKwinAppmenu,
        request: org_kde_kwin_appmenu::Request,
        surface: &WlSurface,
        _dh: &DisplayHandle,
        _di: &mut DataInit<'_, State>,
    ) {
        match request {
            org_kde_kwin_appmenu::Request::SetAddress { service_name, object_path } => {
                tracing::debug!(service_name, object_path, "меню приложения");
                set(state, surface, Some((service_name, object_path)));
            }
            org_kde_kwin_appmenu::Request::Release => set(state, surface, None),
            _ => {}
        }
    }

    fn destroyed(state: &mut State, _client: ClientId, _resource: &OrgKdeKwinAppmenu, surface: &WlSurface) {
        set(state, surface, None);
    }
}
