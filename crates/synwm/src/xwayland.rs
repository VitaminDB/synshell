//! Xwayland: X11-программы как обычные окна (smithay X11Wm).

use std::os::fd::OwnedFd;
use std::process::Stdio;

use smithay::{
    delegate_xwayland_keyboard_grab, delegate_xwayland_shell,
    desktop::Window,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Rectangle, SERIAL_COUNTER},
    wayland::{
        compositor::CompositorHandler,
        selection::{
            data_device::{clear_data_device_selection, current_data_device_selection_userdata, request_data_device_client_selection, set_data_device_selection},
            primary_selection::{clear_primary_selection, current_primary_selection_userdata, request_primary_client_selection, set_primary_selection},
            SelectionTarget,
        },
        xwayland_keyboard_grab::XWaylandKeyboardGrabHandler,
        xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    },
    xwayland::{
        xwm::{Reorder, ResizeEdge as X11ResizeEdge, XwmId},
        X11Surface, X11Wm, XWayland, XWaylandEvent, XwmHandler,
    },
};

use crate::{focus::FocusTarget, state::State, wm::ResizeEdge};

pub struct XwaylandState {
    pub wm: Option<X11Wm>,
    pub display: Option<u32>,
    pub shell_state: XWaylandShellState,
}

impl State {
    pub fn start_xwayland(&mut self) {
        if !self.core.config.general.xwayland {
            return;
        }
        let shell_state = XWaylandShellState::new::<State>(&self.core.display_handle);
        smithay::wayland::xwayland_keyboard_grab::XWaylandKeyboardGrabState::new::<State>(&self.core.display_handle);
        let (xwayland, client) = match XWayland::spawn(
            &self.core.display_handle,
            None,
            std::iter::empty::<(String, String)>(),
            true,
            Stdio::null(),
            Stdio::null(),
            |_| (),
        ) {
            Ok(x) => x,
            Err(e) => {
                tracing::warn!(?e, "Xwayland не запущен (нет Xwayland?)");
                return;
            }
        };
        self.core.xwayland = Some(XwaylandState { wm: None, display: None, shell_state });
        let res = self.core.loop_handle.insert_source(xwayland, move |event, _, state| match event {
            XWaylandEvent::Ready { x11_socket, display_number } => {
                match X11Wm::start_wm(state.core.loop_handle.clone(), x11_socket, client.clone()) {
                    Ok(mut wm) => {
                        if let Some((px, w, h, xh, yh)) = state.core.cursor.default_image() {
                            let _ = wm.set_cursor(&px, (w as u16, h as u16).into(), (xh as u16, yh as u16).into());
                        }
                        if let Some(x) = &mut state.core.xwayland {
                            x.wm = Some(wm);
                            x.display = Some(display_number);
                        }
                        std::env::set_var("DISPLAY", format!(":{display_number}"));
                        tracing::info!(display_number, "Xwayland готов");
                    }
                    Err(e) => tracing::warn!(?e, "X11Wm не запущен"),
                }
            }
            XWaylandEvent::Error => tracing::warn!("Xwayland упал при запуске"),
        });
        if let Err(e) = res {
            tracing::warn!(?e, "источник Xwayland");
        }
    }

    fn x11_id(&self, window: &X11Surface) -> Option<crate::wm::WindowId> {
        self.core
            .wm
            .windows
            .iter()
            .find(|m| m.window.x11_surface() == Some(window))
            .map(|m| m.id)
    }
}

impl XwmHandler for State {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.core.xwayland.as_mut().and_then(|x| x.wm.as_mut()).expect("xwm")
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}
    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Err(e) = window.set_mapped(true) {
            tracing::warn!(?e, "X11 map");
            return;
        }
        let id = self.add_window(Window::new_x11_window(window.clone()));
        if let Some(m) = self.core.wm.get_mut(id) {
            // Окна без своих рамок получают наши.
            m.ssd = !window.is_decorated();
            m.rules_applied = false;
        }
        self.map_x11(id, false);
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        let id = self.add_window(Window::new_x11_window(window.clone()));
        if let Some(m) = self.core.wm.get_mut(id) {
            m.ssd = false;
            m.override_redirect = true;
            m.skip_taskbar = true;
            m.no_focus = true;
            m.floating = true;
            m.above = true;
        }
        self.map_x11(id, true);
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(id) = self.x11_id(&window) {
            self.unmap_window(id, true);
        }
        if !window.is_override_redirect() {
            let _ = window.set_mapped(false);
        }
    }

    fn destroyed_window(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(id) = self.x11_id(&window) {
            self.unmap_window(id, true);
        }
    }

    fn configure_request(&mut self, _xwm: XwmId, window: X11Surface, x: Option<i32>, y: Option<i32>, w: Option<u32>, h: Option<u32>, _reorder: Option<Reorder>) {
        let mut geo = window.geometry();
        let managed = self.x11_id(&window).and_then(|id| self.core.wm.get(id));
        let layout = self.core.wm.workspace(self.core.wm.active).layout;
        if managed.is_some_and(|m| m.is_tiled(layout) || m.maximized || m.fullscreen) {
            // Плитка/развёрнутое — размер задаём мы.
            let _ = window.configure(geo);
            return;
        }
        if let Some(w) = w {
            geo.size.w = w as i32;
        }
        if let Some(h) = h {
            geo.size.h = h as i32;
        }
        if managed.is_none() {
            if let Some(x) = x {
                geo.loc.x = x;
            }
            if let Some(y) = y {
                geo.loc.y = y;
            }
        }
        let _ = window.configure(geo);
    }

    fn configure_notify(&mut self, _xwm: XwmId, window: X11Surface, geometry: Rectangle<i32, Logical>, _above: Option<u32>) {
        let Some(id) = self.x11_id(&window) else { return };
        let Some(m) = self.core.wm.get_mut(id) else { return };
        if m.override_redirect {
            m.loc = geometry.loc;
            self.sync_space();
            self.core.queue_redraw_all();
        }
    }

    fn maximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(id) = self.x11_id(&window) {
            self.apply_maximized(id, true);
        }
    }

    fn unmaximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(id) = self.x11_id(&window) {
            self.apply_maximized(id, false);
        }
    }

    fn fullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(id) = self.x11_id(&window) {
            self.apply_fullscreen(id, true, None);
        }
    }

    fn unfullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(id) = self.x11_id(&window) {
            self.apply_fullscreen(id, false, None);
        }
    }

    fn minimize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(id) = self.x11_id(&window) {
            self.minimize(id);
        }
    }

    fn unminimize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(id) = self.x11_id(&window) {
            self.unminimize(id);
        }
    }

    fn resize_request(&mut self, _xwm: XwmId, window: X11Surface, button: u32, edge: X11ResizeEdge) {
        let Some(id) = self.x11_id(&window) else { return };
        let edges = match edge {
            X11ResizeEdge::Top => ResizeEdge::TOP,
            X11ResizeEdge::Bottom => ResizeEdge::BOTTOM,
            X11ResizeEdge::Left => ResizeEdge::LEFT,
            X11ResizeEdge::Right => ResizeEdge::RIGHT,
            X11ResizeEdge::TopLeft => ResizeEdge::TOP | ResizeEdge::LEFT,
            X11ResizeEdge::TopRight => ResizeEdge::TOP | ResizeEdge::RIGHT,
            X11ResizeEdge::BottomLeft => ResizeEdge::BOTTOM | ResizeEdge::LEFT,
            X11ResizeEdge::BottomRight => ResizeEdge::BOTTOM | ResizeEdge::RIGHT,
        };
        let pos = self.core.pointer.current_location();
        let start = smithay::input::pointer::GrabStartData { focus: None, button, location: pos };
        self.start_resize(id, edges, start, SERIAL_COUNTER.next_serial());
    }

    fn move_request(&mut self, _xwm: XwmId, window: X11Surface, button: u32) {
        let Some(id) = self.x11_id(&window) else { return };
        let pos = self.core.pointer.current_location();
        let start = smithay::input::pointer::GrabStartData { focus: None, button, location: pos };
        self.start_move(id, start, SERIAL_COUNTER.next_serial());
    }

    fn allow_selection_access(&mut self, xwm: XwmId, _selection: SelectionTarget) -> bool {
        // Доступ к буферу — у X11-окна в фокусе.
        let Some(focus) = self.core.keyboard.current_focus() else { return false };
        self.core
            .wm
            .windows
            .iter()
            .filter_map(|m| m.window.x11_surface())
            .any(|x| x.xwm_id() == Some(xwm) && x.wl_surface().as_ref() == Some(&focus.0))
    }

    fn send_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_type: String, fd: OwnedFd) {
        match selection {
            SelectionTarget::Clipboard => {
                if let Err(e) = request_data_device_client_selection(&self.core.seat, mime_type, fd) {
                    tracing::warn!(?e, "буфер обмена → X11");
                }
            }
            SelectionTarget::Primary => {
                if let Err(e) = request_primary_client_selection(&self.core.seat, mime_type, fd) {
                    tracing::warn!(?e, "primary → X11");
                }
            }
        }
    }

    fn new_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_types: Vec<String>) {
        let dh = self.core.display_handle.clone();
        let seat = self.core.seat.clone();
        match selection {
            SelectionTarget::Clipboard => set_data_device_selection(&dh, &seat, mime_types, ()),
            SelectionTarget::Primary => set_primary_selection(&dh, &seat, mime_types, ()),
        }
    }

    fn cleared_selection(&mut self, _xwm: XwmId, selection: SelectionTarget) {
        let seat = self.core.seat.clone();
        match selection {
            SelectionTarget::Clipboard => {
                if current_data_device_selection_userdata(&seat).is_some() {
                    clear_data_device_selection(&self.core.display_handle, &seat);
                }
            }
            SelectionTarget::Primary => {
                if current_primary_selection_userdata(&seat).is_some() {
                    clear_primary_selection(&self.core.display_handle, &seat);
                }
            }
        }
    }
}

impl State {
    fn map_x11(&mut self, id: crate::wm::WindowId, override_redirect: bool) {
        if override_redirect {
            let Some(geo) = self.core.wm.get(id).and_then(|m| m.window.x11_surface().map(|x| x.geometry())) else {
                return;
            };
            let output = self.core.output_at(geo.loc.to_f64()).map(|o| o.name());
            let Some(m) = self.core.wm.get_mut(id) else { return };
            m.loc = geo.loc;
            m.mapped = true;
            m.output = output;
            self.core.wm.stack.push(id);
            self.sync_space();
            self.core.queue_redraw_all();
            return;
        }
        self.map_window(id);
        // Сообщить X11-окну его геометрию.
        if let Some(m) = self.core.wm.get(id) {
            if let Some(x) = m.window.x11_surface() {
                let _ = x.configure(Rectangle::new(m.loc, m.window.geometry().size));
            }
        }
    }
}

impl XWaylandShellHandler for State {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.core.xwayland.as_mut().expect("xwayland").shell_state
    }

    fn surface_associated(&mut self, _xwm: XwmId, _wl_surface: WlSurface, _surface: X11Surface) {
        self.core.queue_redraw_all();
    }
}

impl XWaylandKeyboardGrabHandler for State {
    fn keyboard_focus_for_xsurface(&self, surface: &WlSurface) -> Option<FocusTarget> {
        self.core.wm.by_surface(surface).map(|_| FocusTarget(surface.clone()))
    }
}

delegate_xwayland_shell!(State);
delegate_xwayland_keyboard_grab!(State);

#[allow(dead_code)]
fn _assert_compositor_handler<T: CompositorHandler>() {}
