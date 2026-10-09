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

use synshell_common::config::X11Resolution;

use crate::{focus::FocusTarget, state::State, wm::ResizeEdge};
use synshell_tr::t;

/// Один Xwayland. Первый в [`crate::state::Core::xwayland`] — общий
/// (разрешение `[x11] resolution`, его DISPLAY получают все программы
/// сеанса), остальные запускаются для программ со своим разрешением
/// (`[[x11_app]]`, [`State::x11_display_for`]).
pub struct XwaylandState {
    pub wm: Option<X11Wm>,
    pub display: Option<u32>,
    client: Client,
    /// Масштаб, уже сообщённый X11-клиентам (0 — ещё никакой).
    scale: f64,
    /// Разрешение экрана этого Xwayland (строка `[x11]`/`[[x11_app]]`).
    pub resolution: String,
    /// Файл режимов экрана для патченого Xwayland (`XWAYLAND_MODES_FILE`):
    /// строки «ширина высота» — `[x11] modes` в пикселях этого Xwayland.
    modes_file: std::path::PathBuf,
    modes_written: String,
    dpi: u32,
}

impl XwaylandState {
    fn xwm_id(&self) -> Option<XwmId> {
        self.wm.as_ref().map(|w| w.id())
    }
}

/// Xwayland, которому принадлежит X11-окно.
pub fn instance_of<'a>(list: &'a [XwaylandState], x: &X11Surface) -> Option<&'a XwaylandState> {
    let id = x.xwm_id()?;
    list.iter().find(|i| i.xwm_id() == Some(id))
}

/// Владелец буфера обмена X11 и его типы — передать остальным Xwayland.
#[derive(Default)]
pub struct X11Selection {
    pub clipboard: Option<(XwmId, Vec<String>)>,
    pub primary: Option<(XwmId, Vec<String>)>,
}

impl X11Selection {
    pub fn get(&self, t: SelectionTarget) -> &Option<(XwmId, Vec<String>)> {
        match t {
            SelectionTarget::Clipboard => &self.clipboard,
            SelectionTarget::Primary => &self.primary,
        }
    }
    pub fn get_mut(&mut self, t: SelectionTarget) -> &mut Option<(XwmId, Vec<String>)> {
        match t {
            SelectionTarget::Clipboard => &mut self.clipboard,
            SelectionTarget::Primary => &mut self.primary,
        }
    }
}

impl State {
    pub fn start_xwayland(&mut self) {
        if !self.core.config.general.xwayland {
            return;
        }
        self.core.xwayland_shell = Some(XWaylandShellState::new::<State>(&self.core.display_handle));
        smithay::wayland::xwayland_keyboard_grab::XWaylandKeyboardGrabState::new::<State>(&self.core.display_handle);
        let resolution = self.core.config.x11.resolution.clone();
        let Some(display_number) = self.spawn_xwayland(resolution) else { return };
        // Номер дисплея известен сразу (сокет уже слушается): оболочка и
        // окружение активации запускаются раньше Ready и должны получить DISPLAY,
        // иначе X11-программы, запущенные из оболочки, не находят Xwayland.
        std::env::set_var("DISPLAY", format!(":{display_number}"));
    }

    /// Запустить Xwayland с разрешением `resolution`; номер дисплея — сразу.
    fn spawn_xwayland(&mut self, resolution: String) -> Option<u32> {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let modes_file = runtime.join(format!("synwm-x11-modes-{}-{}", std::process::id(), self.core.xwayland.len()));
        let _ = std::fs::write(&modes_file, "");
        let mut env: Vec<(String, String)> = std::env::vars()
            // smithay запускает Xwayland с чистым окружением. Выбор графического
            // драйвера ему нужен тот же, что композитору: без него на телефоне
            // (zink поверх turnip) glamor не поднимается — нет DRI3, X11-программы
            // остаются без GPU.
            .filter(|(k, _)| ["MESA_", "VK_", "TU_", "LIBGL_", "GBM_", "EGL_", "__GLX_", "__EGL_"].iter().any(|p| k.starts_with(p)))
            .collect();
        env.push(("XWAYLAND_MODES_FILE".into(), modes_file.display().to_string()));
        let (xwayland, client) = match XWayland::spawn(
            &self.core.display_handle,
            None,
            env,
            true,
            Stdio::null(),
            Stdio::null(),
            |_| (),
        ) {
            Ok(x) => x,
            Err(e) => {
                tracing::warn!(?e, "Xwayland не запущен (нет Xwayland?)");
                return None;
            }
        };
        let display_number = xwayland.display_number();
        self.core.xwayland.push(XwaylandState {
            wm: None,
            display: Some(display_number),
            client: client.clone(),
            scale: 0.0,
            resolution,
            modes_file,
            modes_written: String::new(),
            dpi: 0,
        });
        // До первого wl_output: Xwayland сразу получит экран нужного размера.
        self.update_xwayland_scale();
        let res = self.core.loop_handle.insert_source(xwayland, move |event, _, state| match event {
            XWaylandEvent::Ready { x11_socket, display_number } => {
                match X11Wm::start_wm(state.core.loop_handle.clone(), x11_socket, client.clone()) {
                    Ok(wm) => {
                        if let Some(x) = state.core.xwayland.iter_mut().find(|x| x.display == Some(display_number)) {
                            x.wm = Some(wm);
                            x.scale = 0.0;
                            x.modes_written.clear();
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
        Some(display_number)
    }

    /// DISPLAY для программы с именами `keys`: Xwayland с её разрешением
    /// (`[[x11_app]]`, иначе общий). Программы с одинаковым разрешением делят
    /// один Xwayland; нового нет — запускается.
    pub fn x11_display_for(&mut self, keys: &[String]) -> Result<(u32, String), String> {
        if self.core.xwayland.is_empty() {
            return Err(t!("Xwayland выключен ([general] xwayland)").into());
        }
        let wanted = self.core.config.x11_resolution_for(keys).to_string();
        let parsed = X11Resolution::parse(&wanted).ok_or_else(|| t!("непонятное разрешение «{wanted}»", wanted = wanted))?;
        let same = |r: &str| X11Resolution::parse(r) == Some(parsed);
        // Общий подходит, если разрешение совпадает.
        if let Some(i) = self.core.xwayland.iter().position(|x| same(&x.resolution)) {
            let x = &self.core.xwayland[i];
            return x.display.map(|d| (d, x.resolution.clone())).ok_or_else(|| t!("Xwayland без дисплея").into());
        }
        let d = self.spawn_xwayland(wanted.clone()).ok_or(t!("Xwayland не запущен"))?;
        tracing::info!(display = d, resolution = %wanted, ?keys, "отдельный Xwayland");
        Ok((d, wanted))
    }

    /// Масштаб X11-клиентов (пикселей X11 на логический пиксель) — по
    /// разрешению каждого Xwayland относительно монитора с наибольшим
    /// масштабом. X11 о масштабе не знает: `native` — экран в физических
    /// пикселях (окна чёткие), меньше — композитор растягивает окна, а
    /// интерфейс программы крупнее. Размер интерфейса программы берут из DPI
    /// (XSETTINGS и Xft.dpi).
    pub fn update_xwayland_scale(&mut self) {
        // общий Xwayland следует за [x11] resolution
        if let Some(first) = self.core.xwayland.first_mut() {
            first.resolution = self.core.config.x11.resolution.clone();
        }
        let reference = self
            .core
            .space
            .outputs()
            .max_by(|a, b| a.current_scale().fractional_scale().total_cmp(&b.current_scale().fractional_scale()))
            .cloned();
        // Логический размер — дробный (пиксели режима / масштаб): так же Xwayland
        // считает свой экран (xdg_output), и `720p` даёт ровно 720 пикселей.
        let (output_scale, logical) = reference
            .as_ref()
            .and_then(|o| {
                let s = o.current_scale().fractional_scale();
                let px = o.current_transform().transform_size(o.current_mode()?.size);
                Some((s, (px.w as f64 / s, px.h as f64 / s)))
            })
            .unwrap_or((1.0, (1280.0, 800.0)));
        let long = logical.0.max(logical.1);
        let short = logical.0.min(logical.1);
        let dpi_setting = self.core.config.x11.dpi;
        let modes = self.core.config.x11.modes.clone();
        let mut changed = false;
        for i in 0..self.core.xwayland.len() {
            let res = X11Resolution::parse(&self.core.xwayland[i].resolution).unwrap_or_else(|| {
                tracing::warn!(resolution = %self.core.xwayland[i].resolution, "непонятное разрешение X11, беру native");
                X11Resolution::Native
            });
            let scale = res.client_scale(output_scale, logical);
            // Режимы экрана для игр — в пикселях этого Xwayland, длинная сторона первой.
            let screen = (long * scale, short * scale);
            let mut list = String::new();
            for m in &modes {
                match X11Resolution::parse(m) {
                    Some(r) => {
                        let (w, h) = r.mode_size(screen, scale, output_scale);
                        list.push_str(&format!("{w} {h}\n"));
                    }
                    None => tracing::warn!(mode = %m, "непонятный режим X11"),
                }
            }
            let dpi = if dpi_setting > 0 { dpi_setting as f64 } else { scale * 96.0 };
            let x = &mut self.core.xwayland[i];
            let modes_changed = x.modes_written != list;
            if modes_changed {
                if let Err(e) = std::fs::write(&x.modes_file, &list) {
                    tracing::warn!(?e, file = %x.modes_file.display(), "режимы X11");
                }
                x.modes_written = list;
                changed = true;
            }
            let dpi_changed = x.dpi != dpi.round() as u32;
            if x.scale == scale && !dpi_changed {
                continue;
            }
            changed = true;
            x.scale = scale;
            if let Some(data) = x.client.get_data::<XWaylandClientData>() {
                data.compositor_state.set_client_scale(scale);
            }
            let dnum = x.display;
            let Some(wm) = x.wm.as_mut() else { continue };
            x.dpi = dpi.round() as u32;
            let xdpi = dpi * 1024.0;
            let int = scale.round().max(1.0);
            let settings = [
                ("Xft/DPI".to_string(), (xdpi.round() as i32).into()),
                ("Gdk/UnscaledDPI".to_string(), ((xdpi / int).round() as i32).into()),
                ("Gdk/WindowScalingFactor".to_string(), (int as i32).into()),
            ];
            if let Err(e) = wm.set_xsettings(settings.into_iter()) {
                tracing::warn!(?e, "XSETTINGS");
            }
            if let Some((px, w, h, xh, yh)) = self.core.cursor.default_image(scale) {
                let _ = wm.set_cursor(&px, (w as u16, h as u16).into(), (xh as u16, yh as u16).into());
            }
            // Xft.dpi читают Qt, Tk, Chromium и всё, что не слушает XSETTINGS.
            if let (Some(display), true) = (dnum, crate::spawn::which("xrdb")) {
                crate::spawn::spawn_shell(&self.core, &format!("echo 'Xft.dpi: {}' | DISPLAY=:{display} xrdb -merge", dpi.round() as i32));
            }
            tracing::info!(display = ?dnum, scale, dpi, resolution = %self.core.xwayland[i].resolution, "масштаб Xwayland");
        }
        if !changed {
            return;
        }
        // Новый размер экрана или список режимов — Xwayland перечитывает их при
        // событиях wl_output/xdg_output.
        for output in self.core.space.outputs() {
            output.change_current_state(None, None, None, None);
        }
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
/// Экран X11 в пикселях Xwayland с масштабом `scale` — монитор под точкой `at`.
pub fn x11_screen_px(space: &Space<Window>, at: smithay::utils::Point<i32, Logical>, scale: f64) -> (i32, i32) {
    let output = space
        .outputs()
        .find(|o| space.output_geometry(o).is_some_and(|g| g.contains(at)))
        .or_else(|| space.outputs().next());
    output
        .and_then(|o| {
            let px = o.current_transform().transform_size(o.current_mode()?.size);
            let os = o.current_scale().fractional_scale();
            Some(((px.w as f64 / os * scale).round() as i32, (px.h as f64 / os * scale).round() as i32))
        })
        .unwrap_or((1280, 800))
}

pub fn x11_configure(space: &Space<Window>, xwayland: &[XwaylandState], x: &X11Surface, rect: Rectangle<i32, Logical>) {
    let scale = instance_of(xwayland, x).map(|i| i.scale).filter(|s| *s > 0.0).unwrap_or(1.0);
    let round = |v: i32| (v as f64 * scale).round() as i32;
    let (mut w, mut h) = (round(rect.size.w), round(rect.size.h));
    // Монитор под окном; окно может быть и за экраном (страница в листании) —
    // тогда первый: важен размер.
    let output = space
        .outputs()
        .find(|o| space.output_geometry(o).is_some_and(|g| g.contains(rect.loc)))
        .or_else(|| space.outputs().next());
    if let Some((o, g)) = output.and_then(|o| Some((o, space.output_geometry(o)?))) {
        if let Some(mode) = o.current_mode() {
            // Экран X11 — пиксели монитора в масштабе этого Xwayland (так его
            // размер считает xdg_output).
            let px = o.current_transform().transform_size(mode.size);
            let os = o.current_scale().fractional_scale();
            let ex = |p: i32| (p as f64 / os * scale).round() as i32;
            let (pw, ph) = (ex(px.w), ex(px.h));
            // Поправка — только на ошибку округления (пара пикселей), иначе
            // геометрия вывода не та, что мы думаем (другой вывод, поворот на лету).
            let fix = |v: &mut i32, exact: i32| {
                if (exact - *v).abs() <= 2 && exact > 0 {
                    *v = exact;
                }
            };
            if rect.size.w == g.size.w {
                fix(&mut w, pw);
            } else if (rect.loc.x + rect.size.w - g.loc.x - g.size.w).abs() <= 1 {
                fix(&mut w, pw - round(rect.loc.x - g.loc.x));
            }
            if rect.size.h == g.size.h {
                fix(&mut h, ph);
            } else if (rect.loc.y + rect.size.h - g.loc.y - g.size.h).abs() <= 1 {
                fix(&mut h, ph - round(rect.loc.y - g.loc.y));
            }
        }
    }
    tracing::debug!(id = x.window_id(), ?rect, w, h, "X11 configure");
    let _ = x.configure_with_client_size(rect, (w, h));
}

impl XwmHandler for State {
    fn xwm_state(&mut self, xwm: XwmId) -> &mut X11Wm {
        self.core
            .xwayland
            .iter_mut()
            .filter_map(|x| x.wm.as_mut())
            .find(|w| w.id() == xwm)
            .expect("xwm")
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
        let id = self.x11_id(&window);
        // Полноэкранное окно игры меняет размер под выбранный в игре режим экрана
        // (Xwayland эмулирует смену режима и растягивает окно на экран через
        // wp_viewporter): меньше экрана — принимаем, иначе — размер экрана.
        if let Some(m) = id.and_then(|id| self.core.wm.get(id)).filter(|m| m.fullscreen) {
            let scale = instance_of(&self.core.xwayland, &window).map(|i| i.scale).filter(|s| *s > 0.0).unwrap_or(1.0);
            let full = x11_screen_px(&self.core.space, m.loc, scale);
            let (rw, rh) = (
                w.map(|v| (v as f64 * scale).round() as i32).unwrap_or(full.0),
                h.map(|v| (v as f64 * scale).round() as i32).unwrap_or(full.1),
            );
            let mode = (rw < full.0 - 2 || rh < full.1 - 2).then_some((rw.max(1), rh.max(1)));
            let rect = Rectangle::new(m.loc, m.window.geometry().size);
            if let Some(m) = id.and_then(|id| self.core.wm.get_mut(id)) {
                m.x11_mode = mode;
            }
            match mode {
                Some(size) => {
                    tracing::info!(id = window.window_id(), ?size, "X11: режим экрана игры");
                    let _ = window.configure_with_client_size(rect, size);
                }
                None => x11_configure(&self.core.space, &self.core.xwayland, &window, rect),
            }
            return;
        }
        let managed = id.and_then(|id| self.core.wm.get(id));
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

    fn send_selection(&mut self, xwm: XwmId, selection: SelectionTarget, mime_type: String, fd: OwnedFd) {
        // Буфер принадлежит программе другого Xwayland — данные берём у него.
        if let Some((owner, _)) = self.core.x11_selection.get(selection).clone() {
            if owner != xwm {
                let handle = self.core.loop_handle.clone();
                if let Some(wm) = self.core.xwayland.iter_mut().filter_map(|x| x.wm.as_mut()).find(|w| w.id() == owner) {
                    if let Err(e) = wm.send_selection(selection, mime_type, fd, handle) {
                        tracing::warn!(?e, "буфер обмена между Xwayland");
                    }
                }
                return;
            }
        }
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

    fn new_selection(&mut self, xwm: XwmId, selection: SelectionTarget, mime_types: Vec<String>) {
        *self.core.x11_selection.get_mut(selection) = Some((xwm, mime_types.clone()));
        let dh = self.core.display_handle.clone();
        let seat = self.core.seat.clone();
        match selection {
            SelectionTarget::Clipboard => set_data_device_selection(&dh, &seat, mime_types, ()),
            SelectionTarget::Primary => set_primary_selection(&dh, &seat, mime_types, ()),
        }
    }

    fn cleared_selection(&mut self, xwm: XwmId, selection: SelectionTarget) {
        if self.core.x11_selection.get(selection).as_ref().is_some_and(|(o, _)| *o == xwm) {
            *self.core.x11_selection.get_mut(selection) = None;
        }
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
        self.core.xwayland_shell.as_mut().expect("xwayland")
    }

    fn surface_associated(&mut self, _xwm: XwmId, _wl_surface: WlSurface, surface: X11Surface) {
        // X11-окно получает фокус при появлении, но его wl_surface связывается
        // позже — тогда фокус клавиатуры ушёл в никуда, и клавиши (в том числе
        // экранной клавиатуры) окну не доходили, пока по нему не коснуться.
        if let Some(id) = self.x11_id(&surface) {
            if self.core.wm.focused == Some(id) && self.core.keyboard.current_focus().is_none() {
                self.focus_window(Some(id));
            }
        }
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
