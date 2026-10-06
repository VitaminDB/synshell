//! Xwayland: X11-программы как обычные окна (smithay X11Wm).

use std::os::fd::OwnedFd;
use std::process::Stdio;

use smithay::{
    delegate_xwayland_keyboard_grab, delegate_xwayland_shell,
    desktop::{Space, Window},
    reexports::wayland_server::{protocol::wl_surface::WlSurface, Client},
    output::Output,
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
        X11Surface, X11Wm, XWayland, XWaylandClientData, XWaylandEvent, XwmHandler,
    },
};

use crate::{focus::FocusTarget, state::State, wm::ResizeEdge};

pub struct XwaylandState {
    pub wm: Option<X11Wm>,
    pub display: Option<u32>,
    pub shell_state: XWaylandShellState,
    client: Client,
    /// Масштаб, уже сообщённый X11-клиентам (0 — ещё никакой).
    scale: f64,
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
            // smithay запускает Xwayland с чистым окружением. Выбор графического
            // драйвера ему нужен тот же, что композитору: без него на телефоне
            // (zink поверх turnip) glamor не поднимается — нет DRI3, X11-программы
            // остаются без GPU.
            std::env::vars().filter(|(k, _)| ["MESA_", "VK_", "TU_", "LIBGL_", "GBM_", "EGL_", "__GLX_", "__EGL_"].iter().any(|p| k.starts_with(p))),
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
        // Номер дисплея известен сразу (сокет уже слушается): оболочка и
        // окружение активации запускаются раньше Ready и должны получить DISPLAY,
        // иначе X11-программы, запущенные из оболочки, не находят Xwayland.
        let display_number = xwayland.display_number();
        std::env::set_var("DISPLAY", format!(":{display_number}"));
        self.core.xwayland = Some(XwaylandState { wm: None, display: Some(display_number), shell_state, client: client.clone(), scale: 0.0 });
        // До первого wl_output: Xwayland сразу получит экран в физических пикселях.
        self.update_xwayland_scale();
        let res = self.core.loop_handle.insert_source(xwayland, move |event, _, state| match event {
            XWaylandEvent::Ready { x11_socket, display_number } => {
                match X11Wm::start_wm(state.core.loop_handle.clone(), x11_socket, client.clone()) {
                    Ok(wm) => {
                        if let Some(x) = &mut state.core.xwayland {
                            x.wm = Some(wm);
                            x.scale = 0.0;
                        }
                        state.update_xwayland_scale();
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

    /// Масштаб X11-клиентов — наибольший масштаб мониторов. X11 не знает о
    /// масштабе: Xwayland получает экран в физических пикселях (окна чёткие, а
    /// не растянутые), размер интерфейса программы берут из DPI (XSETTINGS и
    /// Xft.dpi).
    pub fn update_xwayland_scale(&mut self) {
        let scale = self
            .core
            .space
            .outputs()
            .map(|o| o.current_scale().fractional_scale())
            .fold(1.0, f64::max);
        let Some(x) = self.core.xwayland.as_mut() else { return };
        if x.scale == scale {
            return;
        }
        x.scale = scale;
        if let Some(data) = x.client.get_data::<XWaylandClientData>() {
            data.compositor_state.set_client_scale(scale);
        }
        let display = x.display;
        let ready = if let Some(wm) = x.wm.as_mut() {
            let dpi = scale * 96.0 * 1024.0;
            let int = scale.round().max(1.0);
            let settings = [
                ("Xft/DPI".to_string(), (dpi.round() as i32).into()),
                ("Gdk/UnscaledDPI".to_string(), ((dpi / int).round() as i32).into()),
                ("Gdk/WindowScalingFactor".to_string(), (int as i32).into()),
            ];
            if let Err(e) = wm.set_xsettings(settings.into_iter()) {
                tracing::warn!(?e, "XSETTINGS");
            }
            if let Some((px, w, h, xh, yh)) = self.core.cursor.default_image(scale) {
                let _ = wm.set_cursor(&px, (w as u16, h as u16).into(), (xh as u16, yh as u16).into());
            }
            true
        } else {
            false
        };
        // Новый размер экрана — Xwayland узнаёт его из wl_output/xdg_output.
        for output in self.core.space.outputs() {
            output.change_current_state(None, None, None, None);
        }
        if !ready {
            return;
        }
        // Xft.dpi читают Qt, Tk, Chromium и всё, что не слушает XSETTINGS.
        if let (Some(display), true) = (display, crate::spawn::which("xrdb")) {
            let dpi = (scale * 96.0).round() as i32;
            crate::spawn::spawn_shell(&self.core, &format!("echo 'Xft.dpi: {dpi}' | DISPLAY=:{display} xrdb -merge"));
        }
        tracing::info!(scale, "масштаб Xwayland");
        self.relayout();
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

/// Задать X11-окну геометрию. При дробном масштабе логический размер не
/// выражает размер монитора в пикселях X11 точно (2712 px при 2.35 — это
/// 1154.04, окно 1155 получило бы 2714 px и вылезло за экран X11): край
/// окна, совпавший с краем монитора, ставится ровно на его пиксельный край.
pub fn x11_configure(space: &Space<Window>, xwayland: &Option<XwaylandState>, x: &X11Surface, rect: Rectangle<i32, Logical>) {
    let scale = xwayland.as_ref().map(|x| x.scale).filter(|s| *s > 0.0).unwrap_or(1.0);
    let round = |v: i32| (v as f64 * scale).round() as i32;
    let (mut w, mut h) = (round(rect.size.w), round(rect.size.h));
    // Монитор под окном; окно может быть и за экраном (страница в листании) —
    // тогда любой монитор с этим масштабом: важен размер.
    let same_scale = |o: &&Output| o.current_scale().fractional_scale() == scale;
    let output = space
        .outputs()
        .filter(same_scale)
        .find(|o| space.output_geometry(o).is_some_and(|g| g.contains(rect.loc)))
        .or_else(|| space.outputs().find(same_scale));
    if let Some((o, g)) = output.and_then(|o| Some((o, space.output_geometry(o)?))) {
        if let Some(mode) = o.current_mode() {
            let px = o.current_transform().transform_size(mode.size);
            // Поправка — только на ошибку округления (пара пикселей), иначе
            // геометрия вывода не та, что мы думаем (другой вывод, поворот на лету).
            let fix = |v: &mut i32, exact: i32| {
                if (exact - *v).abs() <= 2 && exact > 0 {
                    *v = exact;
                }
            };
            if rect.size.w == g.size.w {
                fix(&mut w, px.w);
            } else if (rect.loc.x + rect.size.w - g.loc.x - g.size.w).abs() <= 1 {
                fix(&mut w, px.w - round(rect.loc.x - g.loc.x));
            }
            if rect.size.h == g.size.h {
                fix(&mut h, px.h);
            } else if (rect.loc.y + rect.size.h - g.loc.y - g.size.h).abs() <= 1 {
                fix(&mut h, px.h - round(rect.loc.y - g.loc.y));
            }
        }
    }
    tracing::debug!(id = x.window_id(), ?rect, w, h, "X11 configure");
    let _ = x.configure_with_client_size(rect, (w, h));
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
        let ssd = self.core.default_decoration_mode()
            == smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode::ServerSide;
        let id = self.add_window(Window::new_x11_window(window.clone()));
        if let Some(m) = self.core.wm.get_mut(id) {
            // Окна без своих рамок получают наши (если рамки вообще рисуем мы).
            m.ssd = ssd && !window.is_decorated();
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
        tracing::debug!(id = window.window_id(), title = window.title(), "X11 unmap");
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
            x11_configure(&self.core.space, &self.core.xwayland, &window, geo);
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
        x11_configure(&self.core.space, &self.core.xwayland, &window, geo);
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
                x11_configure(&self.core.space, &self.core.xwayland, x, Rectangle::new(m.loc, m.window.geometry().size));
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
