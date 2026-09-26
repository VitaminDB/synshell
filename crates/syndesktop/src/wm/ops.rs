//! Операции оконного менеджера. Живут на `State`, потому что смена фокуса
//! клавиатуры в smithay требует `&mut State`.

use std::time::Duration;

use smithay::{
    desktop::{layer_map_for_output, Window, WindowSurface},
    output::Output,
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{protocol::wl_surface::WlSurface, Resource},
    },
    utils::{Logical, Point, Rectangle, Size, SERIAL_COUNTER},
    wayland::{
        compositor::with_states,
        seat::WaylandFocus,
        shell::xdg::{SurfaceCachedState, XdgToplevelSurfaceData},
    },
};
use syndesktop_common::{
    action::{Direction, LayoutKind, WorkspaceTarget},
    config::Placement,
    ipc::{Event, WindowInfo},
};

use super::{
    layout::{self, SnapZone},
    output_for_rect, Managed, WindowId, WorkspaceSwitch,
};
use crate::{
    anim::{self, Animation, Curve},
    focus::FocusTarget,
    state::State,
};

type Rect = Rectangle<i32, Logical>;

impl State {
    // ─── появление и исчезновение ───────────────────────────────────────────

    /// Новое окно (xdg toplevel или X11): регистрируем, но не показываем до
    /// первого буфера.
    pub fn add_window(&mut self, window: Window) -> WindowId {
        let id = self.core.wm.alloc_id();
        let ws = self.core.wm.active;
        let mut m = Managed::new(id, window, ws);
        m.ssd = self.core.default_decoration_mode()
            == smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode::ServerSide;
        if m.window.is_x11() {
            // X11-окна рисуют рамки сами (или не рисуют вовсе, как у меню).
            m.ssd = false;
        }
        self.core.wm.windows.push(m);
        id
    }

    /// Первичный configure xdg-окна: применить правила и выбрать размер.
    pub fn initial_configure(&mut self, surface: &WlSurface) {
        let Some(id) = self.core.wm.id_by_surface(surface) else { return };
        self.apply_rules(id);
        let output = self.target_output_for(id);
        let area = output.as_ref().map(|o| self.core.work_area(o)).unwrap_or_else(|| Rectangle::from_size((1280, 800).into()));
        let tiled = {
            let m = self.core.wm.get(id).unwrap();
            let layout = self.core.wm.workspace(m.workspace).layout;
            layout != LayoutKind::Floating && !m.floating
        };
        let (fullscreen, maximized, rule_size, ssd) = {
            let m = self.core.wm.get(id).unwrap();
            (m.fullscreen, m.maximized, m.rule_size, m.ssd)
        };
        let title_h = if ssd { self.core.deco_theme.height } else { 0 };
        let Some(toplevel) = self.core.wm.get(id).and_then(|m| m.window.toplevel().cloned()) else { return };
        toplevel.with_pending_state(|s| {
            s.bounds = Some((area.size.w, (area.size.h - title_h).max(1)).into());
            if fullscreen {
                if let Some(o) = &output {
                    let geo = self.core.space.output_geometry(o).unwrap_or_default();
                    s.states.set(xdg_toplevel::State::Fullscreen);
                    s.size = Some(geo.size);
                }
            } else if maximized {
                s.states.set(xdg_toplevel::State::Maximized);
                s.size = Some((area.size.w, (area.size.h - title_h).max(1)).into());
            } else if tiled {
                // Размер уточнит раскладка после появления; пока — пропорция области.
                s.states.set(xdg_toplevel::State::TiledLeft);
                s.states.set(xdg_toplevel::State::TiledRight);
                s.states.set(xdg_toplevel::State::TiledTop);
                s.states.set(xdg_toplevel::State::TiledBottom);
            } else if let Some(sz) = rule_size {
                s.size = Some(sz);
            }
            s.decoration_mode = Some(if ssd {
                smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode::ServerSide
            } else {
                smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode::ClientSide
            });
        });
        toplevel.send_configure();
    }

    fn apply_rules(&mut self, id: WindowId) {
        let Some(m) = self.core.wm.get(id) else { return };
        if m.rules_applied {
            return;
        }
        let r = super::rules::resolve(&self.core.rules, &m.app_id(), &m.title());
        let count = self.core.wm.count();
        let m = self.core.wm.get_mut(id).unwrap();
        m.rules_applied = true;
        if let Some(ws) = r.workspace {
            m.workspace = ws.saturating_sub(1).min(count - 1);
        }
        if let Some(f) = r.floating {
            m.floating = f;
        }
        m.output = r.output.clone();
        m.maximized |= r.maximized;
        m.fullscreen |= r.fullscreen;
        m.sticky |= r.sticky;
        m.above |= r.always_on_top;
        m.no_focus |= r.no_focus;
        m.skip_taskbar |= r.skip_taskbar;
        if let Some(o) = r.opacity {
            m.opacity = o.clamp(0.05, 1.0);
        }
        if let Some(d) = r.decorations {
            if !m.window.is_x11() {
                m.ssd = d;
            }
        }
        m.rule_size = r.size.map(|[w, h]| (w.max(1), h.max(1)).into());
        m.rule_position = r.position.map(|[x, y]| (x, y).into());
        m.rule_center = r.center;
        if let (Some([w, h]), Some(t)) = (r.min_size, m.window.toplevel()) {
            with_states(t.wl_surface(), |s| {
                s.cached_state.get::<SurfaceCachedState>().pending().min_size = (w, h).into();
            });
        }
    }

    /// Вывод для окна: по правилу, по привязке, иначе под указателем.
    fn target_output_for(&self, id: WindowId) -> Option<smithay::output::Output> {
        let m = self.core.wm.get(id)?;
        if let Some(name) = &m.output {
            if let Some(o) = self.core.space.outputs().find(|o| crate::backend::output_matches(o, name)) {
                return Some(o.clone());
            }
        }
        self.core.output_under_pointer()
    }

    /// Первый буфер: разместить, показать, дать фокус.
    pub fn map_window(&mut self, id: WindowId) {
        let Some(output) = self.target_output_for(id) else { return };
        let area = self.core.work_area(&output);
        let placement = self.core.config.windows.placement;
        let pointer = self.core.pointer.current_location();
        let title_h = self.core.deco_theme.height;
        let others: Vec<Rect> = self
            .core
            .wm
            .visible_ids()
            .iter()
            .filter_map(|x| self.core.wm.get(*x))
            .map(|w| w.geometry())
            .collect();
        let cascade_n = others.len() as i32;
        let anim_dur = anim::duration(&self.core.config.animations, 220);
        let open_style = self.core.config.animations.window_open.clone();
        {
            let m = self.core.wm.get_mut(id).unwrap();
            m.mapped = true;
            m.output = Some(output.name());
            let top = if m.has_titlebar() { title_h } else { 0 };
            let size = m.window.geometry().size;
            let size = Size::from((size.w.max(1), size.h.max(1)));
            // Рамка = содержимое + заголовок.
            let frame = Size::from((size.w, size.h + top));
            let loc = if let Some(p) = m.rule_position {
                p
            } else if m.rule_center || placement == Placement::Center {
                center_in(area, frame)
            } else {
                match placement {
                    Placement::Cascade => {
                        let step = 32 * (cascade_n % 8);
                        (area.loc.x + 40 + step, area.loc.y + 40 + step).into()
                    }
                    Placement::UnderMouse => {
                        let x = pointer.x as i32 - frame.w / 2;
                        let y = pointer.y as i32 - top / 2;
                        clamp_into(area, Rectangle::new((x, y).into(), frame)).loc
                    }
                    Placement::Smart => smart_place(area, frame, &others),
                    Placement::Center => center_in(area, frame),
                }
            };
            // Не класть окно ровно поверх другого: сдвиг каскадом.
            let mut loc = loc;
            let mut guard = 0;
            while others.iter().any(|o| (o.loc.x - loc.x).abs() < 8 && (o.loc.y - top - loc.y).abs() < 8) && guard < 16 {
                loc.x += 32;
                loc.y += 32;
                guard += 1;
            }
            m.loc = Point::from((loc.x, loc.y + top));
            m.float_geo = Some(Rectangle::new(m.loc, size));
            if open_style != "none" && !anim_dur.is_zero() {
                m.open_anim = Some(Animation::new(0.0, 1.0, anim_dur, Curve::EaseOutCubic));
            }
        }
        let (above, maximized, fullscreen) = {
            let m = self.core.wm.get(id).unwrap();
            (m.above, m.maximized, m.fullscreen)
        };
        self.core.wm.stack.push(id);
        if above {
            self.core.wm.raise(id);
        }
        if maximized {
            self.apply_maximized(id, true);
        }
        if fullscreen {
            self.apply_fullscreen(id, true, Some(output.clone()));
        }
        self.create_foreign_handle(id);
        self.relayout();
        let (no_focus, ws) = {
            let m = self.core.wm.get(id).unwrap();
            (m.no_focus, m.workspace)
        };
        if !no_focus && ws == self.core.wm.active && !self.core.is_locked() {
            self.focus_window(Some(id));
        }
        self.core.ipc_dirty = true;
        self.core.queue_redraw_all();
    }

    /// Окно убрало буфер или уничтожено.
    pub fn unmap_window(&mut self, id: WindowId, destroyed: bool) {
        let was_focused = self.core.wm.focused == Some(id);
        if let Some(m) = self.core.wm.get(id) {
            let w = m.window.clone();
            self.core.space.unmap_elem(&w);
        }
        self.core.wm.stack.retain(|x| *x != id);
        self.core.wm.focus_history.retain(|x| *x != id);
        for ws in &mut self.core.wm.workspaces {
            if ws.last_focus == Some(id) {
                ws.last_focus = None;
            }
        }
        if let Some(sw) = &mut self.core.wm.switcher {
            sw.order.retain(|x| *x != id);
            sw.index = sw.index.min(sw.order.len().saturating_sub(1));
        }
        if destroyed {
            if let Some(m) = self.core.wm.get(id) {
                if let Some(h) = &m.foreign_handle {
                    h.send_closed();
                }
            }
            self.core.wm.windows.retain(|w| w.id != id);
            self.core.ipc.broadcast(&Event::WindowClosed { id });
        } else if let Some(m) = self.core.wm.get_mut(id) {
            m.mapped = false;
            if let Some(h) = m.foreign_handle.take() {
                h.send_closed();
            }
            m.last_info = None;
            self.core.ipc.broadcast(&Event::WindowClosed { id });
        }
        if was_focused {
            self.core.wm.focused = None;
            self.focus_after_close();
        }
        self.relayout();
        self.core.ipc_dirty = true;
        self.core.queue_redraw_all();
    }

    fn create_foreign_handle(&mut self, id: WindowId) {
        let (title, app_id) = {
            let m = self.core.wm.get(id).unwrap();
            (m.title(), m.app_id())
        };
        let handle = self
            .core
            .foreign_toplevel_list_state
            .new_toplevel::<State>(&title, &app_id);
        if let Some(m) = self.core.wm.get_mut(id) {
            m.foreign_handle = Some(handle);
        }
    }

    /// Заголовок или app_id изменились.
    pub fn window_title_changed(&mut self, id: WindowId) {
        if let Some(m) = self.core.wm.get(id) {
            if let Some(h) = &m.foreign_handle {
                h.send_title(&m.title());
                h.send_app_id(&m.app_id());
                h.send_done();
            }
        }
        self.core.ipc_dirty = true;
        self.core.queue_redraw_all();
    }

    // ─── фокус ───────────────────────────────────────────────────────────────

    /// Дать окну фокус клавиатуры (или снять фокус с окон при `None`).
    pub fn focus_window(&mut self, id: Option<WindowId>) {
        if self.core.is_locked() {
            return;
        }
        let prev = self.core.wm.focused;
        if let Some(id) = id {
            let Some(m) = self.core.wm.get(id) else { return };
            if m.minimized {
                self.unminimize(id);
            }
            let ws = self.core.wm.get(id).unwrap().workspace;
            let sticky = self.core.wm.get(id).unwrap().sticky;
            if ws != self.core.wm.active && !sticky {
                self.switch_workspace(ws, false);
            }
            if self.core.config.windows.raise_on_focus {
                self.core.wm.raise(id);
            }
            self.core.wm.note_focus(id);
            let active = self.core.wm.active;
            self.core.wm.workspace_mut(active).last_focus = Some(id);
            if let Some(m) = self.core.wm.get_mut(id) {
                m.urgent = false;
            }
        }
        self.core.wm.focused = id;
        // Состояние «активно» у xdg-окон.
        for m in &self.core.wm.windows {
            let active = Some(m.id) == id;
            if m.window.set_activated(active) {
                if let Some(t) = m.window.toplevel() {
                    if t.is_initial_configure_sent() {
                        t.send_pending_configure();
                    }
                }
            }
            if let Some(x) = m.window.x11_surface() {
                let _ = x.set_activated(active);
            }
        }
        // Раскладка клавиатуры окна.
        if self.core.config.input.keyboard.per_window_layout && prev != id {
            self.save_and_restore_layout(prev, id);
        }
        let target = id
            .and_then(|i| self.core.wm.get(i))
            .and_then(|m| m.window.wl_surface().map(|s| FocusTarget(s.into_owned())));
        if let (Some(i), Some(x11)) = (id, id.and_then(|i| self.core.wm.get(i)).and_then(|m| m.window.x11_surface().cloned())) {
            let _ = i;
            if let Some(xw) = self.core.xwayland.as_mut().and_then(|x| x.wm.as_mut()) {
                let _ = xw.raise_window(&x11);
            }
        }
        let keyboard = self.core.keyboard.clone();
        keyboard.set_focus(self, target, SERIAL_COUNTER.next_serial());
        self.sync_space();
        if prev != id {
            self.core.ipc.broadcast(&Event::WindowFocused { id });
        }
        self.core.ipc_dirty = true;
        self.core.queue_redraw_all();
    }

    fn save_and_restore_layout(&mut self, prev: Option<WindowId>, next: Option<WindowId>) {
        let keyboard = self.core.keyboard.clone();
        let current = keyboard.with_xkb_state(self, |ctx| ctx.xkb().lock().unwrap().active_layout().0);
        if let Some(p) = prev.and_then(|p| self.core.wm.get_mut(p)) {
            p.kb_layout = Some(current);
        }
        if let Some(target) = next.and_then(|n| self.core.wm.get(n)).and_then(|m| m.kb_layout) {
            if target != current {
                self.set_keyboard_layout(target);
            }
        }
    }

    /// После закрытия/сворачивания окна в фокусе — следующее по истории на
    /// этом столе.
    pub fn focus_after_close(&mut self) {
        let active = self.core.wm.active;
        let next = self
            .core
            .wm
            .focus_history
            .iter()
            .rev()
            .copied()
            .find(|id| {
                self.core
                    .wm
                    .get(*id)
                    .is_some_and(|w| w.mapped && !w.minimized && w.on_workspace(active))
            })
            .or_else(|| self.core.wm.visible_ids().last().copied());
        self.focus_window(next);
    }

    /// Фокус на поверхность, не являющуюся окном (панель, меню оболочки).
    pub fn focus_surface(&mut self, surface: Option<WlSurface>) {
        let keyboard = self.core.keyboard.clone();
        keyboard.set_focus(self, surface.map(FocusTarget), SERIAL_COUNTER.next_serial());
    }

    /// Окно в фокусе.
    pub fn focused(&self) -> Option<WindowId> {
        self.core.wm.focused
    }

    // ─── синхронизация Space ────────────────────────────────────────────────

    /// Привести `Space` к модели: видимые окна в порядке стопки, остальные — вон.
    pub fn sync_space(&mut self) {
        let visible = self.core.wm.visible_ids();
        let mapped: Vec<Window> = self.core.space.elements().cloned().collect();
        for w in mapped {
            let keep = self
                .core
                .wm
                .by_window(&w)
                .is_some_and(|m| visible.contains(&m.id));
            if !keep {
                self.core.space.unmap_elem(&w);
            }
        }
        for id in visible {
            if let Some(m) = self.core.wm.get(id) {
                self.core.space.map_element(m.window.clone(), m.loc, false);
            }
        }
    }

    // ─── раскладка ──────────────────────────────────────────────────────────

    /// Пересчитать геометрию окон активного стола (плитка, развёрнутые,
    /// во весь экран, прилепленные) по всем выводам.
    pub fn relayout(&mut self) {
        let outputs: Vec<(Output, Rect)> = self
            .core
            .space
            .outputs()
            .map(|o| (o.clone(), self.core.space.output_geometry(o).unwrap_or_default()))
            .collect();
        if outputs.is_empty() {
            return;
        }
        let active = self.core.wm.active;
        let ws = self.core.wm.workspace(active);
        let layout = ws.layout;
        let params = layout::Params {
            gaps_inner: self.core.config.windows.gaps_inner.max(0),
            gaps_outer: self.core.config.windows.gaps_outer.max(0),
            master_ratio: ws.master_ratio,
            master_count: ws.master_count,
        };
        let title_h = self.core.deco_theme.height;
        let animate = self.core.config.animations.layout_changes;
        let move_dur = anim::duration(&self.core.config.animations, 180);

        // Привязка окон к выводам (исчезнувший вывод — на основной).
        let primary = self.core.primary_output().map(|o| o.name());
        for m in &mut self.core.wm.windows {
            let valid = m.output.as_ref().is_some_and(|n| outputs.iter().any(|(o, _)| &o.name() == n));
            if !valid {
                m.output = output_for_rect(&outputs, m.geometry()).map(|o| o.name()).or_else(|| primary.clone());
            }
        }

        for (output, _geo) in &outputs {
            let area = self.core.work_area(output);
            let name = output.name();
            // Плитка: окна этого вывода на активном столе, в порядке стопки
            // по времени появления (стабильный порядок: по id).
            let mut tiled: Vec<WindowId> = self
                .core
                .wm
                .windows
                .iter()
                .filter(|m| m.on_workspace(active) && m.output.as_deref() == Some(&name) && m.is_tiled(layout))
                .filter(|m| !m.maximized)
                .map(|m| m.id)
                .collect();
            tiled.sort_by_key(|id| self.core.wm.tile_order_key(*id));
            let rects = layout::arrange(layout, area, tiled.len(), &params);
            for (id, frame) in tiled.iter().zip(rects) {
                self.place_frame(*id, frame, title_h, animate, move_dur, true);
            }
            // Развёрнутые, прилепленные, во весь экран.
            let others: Vec<WindowId> = self
                .core
                .wm
                .windows
                .iter()
                .filter(|m| m.mapped && m.on_workspace(active) && m.output.as_deref() == Some(&name))
                .map(|m| m.id)
                .collect();
            for id in others {
                let (fullscreen, maximized, snap, tiled_now) = {
                    let m = self.core.wm.get(id).unwrap();
                    (m.fullscreen, m.maximized, m.snap, m.is_tiled(layout))
                };
                if fullscreen {
                    let geo = self.core.space.output_geometry(output).unwrap_or_default();
                    self.configure_content(id, geo, false);
                } else if maximized {
                    self.place_frame(id, area, title_h, false, move_dur, false);
                } else if let (Some(z), false) = (snap, tiled_now) {
                    let r = z.rect(area, params.gaps_outer.min(8));
                    self.place_frame(id, r, title_h, animate, move_dur, false);
                }
            }
        }
        self.sync_space();
        self.core.queue_redraw_all();
    }

    /// Поставить окно в прямоугольник рамки (с заголовком, если он есть).
    fn place_frame(&mut self, id: WindowId, frame: Rect, title_h: i32, animate: bool, dur: Duration, tiled: bool) {
        let Some(m) = self.core.wm.get(id) else { return };
        let top = if m.has_titlebar() { title_h } else { 0 };
        let content = Rectangle::new(
            (frame.loc.x, frame.loc.y + top).into(),
            (frame.size.w.max(1), (frame.size.h - top).max(1)).into(),
        );
        let old = m.loc;
        if animate && !dur.is_zero() && m.mapped && old != content.loc && m.open_anim.is_none() {
            let m = self.core.wm.get_mut(id).unwrap();
            m.move_anim = Some((Animation::new(0.0, 1.0, dur, Curve::EaseOutCubic), old));
        }
        self.configure_content(id, content, tiled);
    }

    /// Задать окну положение и размер содержимого.
    pub fn configure_content(&mut self, id: WindowId, content: Rect, tiled: bool) {
        let Some(m) = self.core.wm.get_mut(id) else { return };
        m.loc = content.loc;
        let (maximized, fullscreen) = (m.maximized, m.fullscreen);
        let snapped = m.snap.is_some();
        match m.window.underlying_surface() {
            WindowSurface::Wayland(t) => {
                t.with_pending_state(|s| {
                    s.size = Some(content.size);
                    set_state(&mut s.states, xdg_toplevel::State::Maximized, maximized);
                    set_state(&mut s.states, xdg_toplevel::State::Fullscreen, fullscreen);
                    let edge_tiled = tiled || maximized || snapped;
                    for st in [
                        xdg_toplevel::State::TiledLeft,
                        xdg_toplevel::State::TiledRight,
                        xdg_toplevel::State::TiledTop,
                        xdg_toplevel::State::TiledBottom,
                    ] {
                        set_state(&mut s.states, st, edge_tiled);
                    }
                });
                if t.is_initial_configure_sent() {
                    t.send_pending_configure();
                }
            }
            WindowSurface::X11(x) => {
                let _ = x.configure(content);
            }
        }
        let w = m.window.clone();
        let loc = m.loc;
        if self.core.space.elements().any(|e| e == &w) {
            self.core.space.map_element(w, loc, false);
            self.sync_space();
        }
    }

    /// Вернуть плавающему окну его запомненную геометрию.
    fn restore_float(&mut self, id: WindowId) {
        let Some(m) = self.core.wm.get(id) else { return };
        let geo = m.float_geo.unwrap_or_else(|| m.geometry());
        let m = self.core.wm.get_mut(id).unwrap();
        m.loc = geo.loc;
        match m.window.underlying_surface() {
            WindowSurface::Wayland(t) => {
                t.with_pending_state(|s| {
                    s.size = Some(geo.size);
                    for st in [
                        xdg_toplevel::State::Maximized,
                        xdg_toplevel::State::Fullscreen,
                        xdg_toplevel::State::TiledLeft,
                        xdg_toplevel::State::TiledRight,
                        xdg_toplevel::State::TiledTop,
                        xdg_toplevel::State::TiledBottom,
                    ] {
                        s.states.unset(st);
                    }
                });
                if t.is_initial_configure_sent() {
                    t.send_pending_configure();
                }
            }
            WindowSurface::X11(x) => {
                let _ = x.configure(geo);
            }
        }
    }

    /// Запомнить текущую геометрию как «плавающую» (перед развёртыванием и т.п.).
    fn remember_float(&mut self, id: WindowId) {
        let layout = self.core.wm.workspace(self.core.wm.active).layout;
        if let Some(m) = self.core.wm.get_mut(id) {
            if !m.maximized && !m.fullscreen && m.snap.is_none() && !m.is_tiled(layout) {
                m.float_geo = Some(m.geometry());
            }
        }
    }

    // ─── состояния окна ─────────────────────────────────────────────────────

    pub fn toggle_maximize(&mut self, id: WindowId) {
        let Some(m) = self.core.wm.get(id) else { return };
        let v = !m.maximized;
        self.apply_maximized(id, v);
    }

    pub fn apply_maximized(&mut self, id: WindowId, on: bool) {
        if on {
            self.remember_float(id);
        }
        let Some(m) = self.core.wm.get_mut(id) else { return };
        m.maximized = on;
        m.snap = None;
        if let Some(x) = m.window.x11_surface() {
            let _ = x.set_maximized(on);
        }
        if !on {
            let layout = self.core.wm.workspace(self.core.wm.active).layout;
            let tiled = self.core.wm.get(id).unwrap().is_tiled(layout);
            if !tiled {
                self.restore_float(id);
            }
        }
        self.relayout();
        self.core.ipc_dirty = true;
    }

    pub fn toggle_fullscreen(&mut self, id: WindowId) {
        let Some(m) = self.core.wm.get(id) else { return };
        let v = !m.fullscreen;
        self.apply_fullscreen(id, v, None);
    }

    pub fn apply_fullscreen(&mut self, id: WindowId, on: bool, output: Option<Output>) {
        if on {
            self.remember_float(id);
        }
        let Some(m) = self.core.wm.get_mut(id) else { return };
        m.fullscreen = on;
        if let Some(o) = output {
            m.output = Some(o.name());
        }
        if let Some(x) = m.window.x11_surface() {
            let _ = x.set_fullscreen(on);
        }
        if !on {
            let (maximized, snap) = (m.maximized, m.snap);
            let layout = self.core.wm.workspace(self.core.wm.active).layout;
            let tiled = self.core.wm.get(id).unwrap().is_tiled(layout);
            if !maximized && snap.is_none() && !tiled {
                self.restore_float(id);
            }
        } else {
            self.core.wm.raise(id);
        }
        self.relayout();
        self.core.ipc_dirty = true;
    }

    pub fn minimize(&mut self, id: WindowId) {
        let Some(m) = self.core.wm.get(id) else { return };
        if m.minimized || !m.mapped {
            return;
        }
        let dur = anim::duration(&self.core.config.animations, 200);
        let style = self.core.config.animations.minimize.clone();
        let m = self.core.wm.get_mut(id).unwrap();
        m.minimized = true;
        if style != "none" && !dur.is_zero() {
            let target = m.minimize_rect.unwrap_or_else(|| {
                let g = m.geometry();
                Rectangle::new((g.loc.x + g.size.w / 2, g.loc.y + g.size.h).into(), (1, 1).into())
            });
            m.minimize_anim = Some((Animation::new(0.0, 1.0, dur, Curve::EaseInCubic), target));
        }
        if self.core.wm.focused == Some(id) {
            self.core.wm.focused = None;
            self.focus_after_close();
        }
        self.relayout();
        self.core.ipc_dirty = true;
    }

    pub fn unminimize(&mut self, id: WindowId) {
        let Some(m) = self.core.wm.get_mut(id) else { return };
        if !m.minimized {
            return;
        }
        m.minimized = false;
        let dur = anim::duration(&self.core.config.animations, 200);
        let style = self.core.config.animations.minimize.clone();
        let m = self.core.wm.get_mut(id).unwrap();
        if style != "none" && !dur.is_zero() {
            let target = m.minimize_rect.unwrap_or_else(|| {
                let g = m.geometry();
                Rectangle::new((g.loc.x + g.size.w / 2, g.loc.y + g.size.h).into(), (1, 1).into())
            });
            m.minimize_anim = Some((Animation::new(1.0, 0.0, dur, Curve::EaseOutCubic), target));
        }
        self.core.wm.raise(id);
        self.relayout();
        self.core.ipc_dirty = true;
    }

    pub fn toggle_floating(&mut self, id: WindowId) {
        let layout = self.core.wm.workspace(self.core.wm.active).layout;
        let Some(m) = self.core.wm.get(id) else { return };
        let was_tiled = m.is_tiled(layout);
        if !was_tiled {
            self.remember_float(id);
        }
        let m = self.core.wm.get_mut(id).unwrap();
        m.floating = !m.floating;
        let now_tiled = m.is_tiled(layout);
        if was_tiled && !now_tiled {
            // Выпустить из плитки: прежняя плавающая геометрия или по центру.
            let output = self.target_output_for(id);
            let area = output.map(|o| self.core.work_area(&o)).unwrap_or_default();
            let m = self.core.wm.get_mut(id).unwrap();
            if m.float_geo.is_none() {
                let size = layout::default_float_size(area);
                let frame = center_in(area, size);
                m.float_geo = Some(Rectangle::new(frame, size));
            }
            self.restore_float(id);
            self.core.wm.raise(id);
        }
        self.relayout();
        self.core.ipc_dirty = true;
    }

    pub fn toggle_sticky(&mut self, id: WindowId) {
        let active = self.core.wm.active;
        if let Some(m) = self.core.wm.get_mut(id) {
            m.sticky = !m.sticky;
            if !m.sticky {
                m.workspace = active;
            }
        }
        self.core.ipc_dirty = true;
        self.core.queue_redraw_all();
    }

    pub fn toggle_above(&mut self, id: WindowId) {
        if let Some(m) = self.core.wm.get_mut(id) {
            m.above = !m.above;
        }
        self.core.wm.raise(id);
        self.sync_space();
        self.core.ipc_dirty = true;
        self.core.queue_redraw_all();
    }

    /// Super+стрелка: прилепить к половине/четверти (или вернуть).
    pub fn snap(&mut self, id: WindowId, dir: Direction) {
        let layout = self.core.wm.workspace(self.core.wm.active).layout;
        let Some(m) = self.core.wm.get(id) else { return };
        if m.is_tiled(layout) {
            // В плитке Super+стрелки двигают окно.
            self.move_dir(dir);
            return;
        }
        if m.maximized && dir == Direction::Down {
            self.apply_maximized(id, false);
            return;
        }
        let next = SnapZone::step(m.snap, dir);
        self.set_snap(id, next);
    }

    pub fn set_snap(&mut self, id: WindowId, zone: Option<SnapZone>) {
        if zone.is_some() {
            self.remember_float(id);
        }
        let Some(m) = self.core.wm.get_mut(id) else { return };
        m.snap = zone;
        m.maximized = false;
        if zone.is_none() {
            self.restore_float(id);
        }
        self.relayout();
        self.core.ipc_dirty = true;
    }

    pub fn center_window(&mut self, id: WindowId) {
        let Some(output) = self.target_output_for(id) else { return };
        let area = self.core.work_area(&output);
        let title_h = self.core.deco_theme.height;
        let Some(m) = self.core.wm.get_mut(id) else { return };
        if m.maximized || m.fullscreen {
            return;
        }
        let top = if m.has_titlebar() { title_h } else { 0 };
        let size = m.size();
        let frame_loc = center_in(area, (size.w, size.h + top).into());
        m.loc = (frame_loc.x, frame_loc.y + top).into();
        m.snap = None;
        m.float_geo = Some(m.geometry());
        self.sync_space();
        self.core.queue_redraw_all();
    }

    pub fn close_window(&mut self, id: WindowId) {
        let Some(m) = self.core.wm.get(id) else { return };
        match m.window.underlying_surface() {
            WindowSurface::Wayland(t) => t.send_close(),
            WindowSurface::X11(x) => {
                let _ = x.close();
            }
        }
    }

    pub fn kill_window(&mut self, id: WindowId) {
        let Some(m) = self.core.wm.get(id) else { return };
        let pid = m.window.wl_surface().and_then(|s| {
            let client = s.client()?;
            client.get_credentials(&self.core.display_handle).ok().map(|c| c.pid)
        });
        if let Some(pid) = pid {
            if pid > 1 && pid != std::process::id() as i32 {
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
                return;
            }
        }
        self.close_window(id);
    }

    // ─── рабочие столы ──────────────────────────────────────────────────────

    pub fn resolve_workspace(&self, t: WorkspaceTarget) -> u32 {
        let n = self.core.wm.count();
        let cur = self.core.wm.active;
        let wrap = self.core.config.workspaces.wrap;
        match t {
            WorkspaceTarget::Index(i) => (i - 1).min(n - 1),
            WorkspaceTarget::Next => {
                if cur + 1 < n {
                    cur + 1
                } else if wrap {
                    0
                } else {
                    cur
                }
            }
            WorkspaceTarget::Prev => {
                if cur > 0 {
                    cur - 1
                } else if wrap {
                    n - 1
                } else {
                    cur
                }
            }
            WorkspaceTarget::Last => self.core.wm.previous.min(n - 1),
        }
    }

    /// Перейти на стол `ws`. `focus` — восстановить фокус на столе.
    pub fn switch_workspace(&mut self, ws: u32, focus: bool) {
        let ws = ws.min(self.core.wm.count() - 1);
        let cur = self.core.wm.active;
        if ws == cur {
            return;
        }
        let dur = anim::duration(&self.core.config.animations, 260);
        let style = self.core.config.animations.workspace_switch.clone();
        if style != "none" && !dur.is_zero() {
            let dir = if ws > cur { 1 } else { -1 };
            self.core.wm.switch = Some(WorkspaceSwitch {
                from: cur,
                to: ws,
                dir,
                anim: Animation::new(0.0, 1.0, dur, Curve::EaseOutCubic),
            });
        }
        self.core.wm.previous = cur;
        self.core.wm.active = ws;
        // Окно, которое тащат мышью, едет с нами.
        if let Some(dragged) = self.dragged_window() {
            if let Some(m) = self.core.wm.get_mut(dragged) {
                m.workspace = ws;
            }
        }
        self.relayout();
        if focus {
            let last = self.core.wm.workspace(ws).last_focus.filter(|id| {
                self.core
                    .wm
                    .get(*id)
                    .is_some_and(|w| w.mapped && !w.minimized && w.on_workspace(ws))
            });
            let target = last.or_else(|| self.core.wm.visible_ids().last().copied());
            self.focus_window(target);
        }
        self.broadcast_workspaces();
        self.core.ipc_dirty = true;
        self.core.queue_redraw_all();
    }

    pub fn workspace_action(&mut self, t: WorkspaceTarget) {
        let target = self.resolve_workspace(t);
        if target == self.core.wm.active
            && self.core.config.workspaces.back_and_forth
            && matches!(t, WorkspaceTarget::Index(_))
        {
            let prev = self.core.wm.previous;
            self.switch_workspace(prev, true);
        } else {
            self.switch_workspace(target, true);
        }
    }

    pub fn move_to_workspace(&mut self, id: WindowId, ws: u32, follow: bool) {
        let ws = ws.min(self.core.wm.count() - 1);
        let Some(m) = self.core.wm.get_mut(id) else { return };
        if m.workspace == ws && !m.sticky {
            return;
        }
        m.workspace = ws;
        m.sticky = false;
        if follow {
            self.switch_workspace(ws, false);
            self.focus_window(Some(id));
        } else {
            if self.core.wm.focused == Some(id) {
                self.core.wm.focused = None;
                self.focus_after_close();
            }
            self.relayout();
        }
        self.broadcast_workspaces();
        self.core.ipc_dirty = true;
    }

    // ─── направленные действия ──────────────────────────────────────────────

    /// Геометрии видимых окон (рамки) — для навигации по направлению.
    fn visible_frames(&self) -> Vec<(WindowId, Rect)> {
        let title_h = self.core.deco_theme.height;
        self.core
            .wm
            .visible_ids()
            .into_iter()
            .filter_map(|id| {
                let m = self.core.wm.get(id)?;
                let g = m.geometry();
                let top = if m.has_titlebar() { title_h } else { 0 };
                Some((id, Rectangle::new((g.loc.x, g.loc.y - top).into(), (g.size.w, g.size.h + top).into())))
            })
            .collect()
    }

    pub fn focus_dir(&mut self, dir: Direction) {
        let frames = self.visible_frames();
        let Some(cur) = self.core.wm.focused.and_then(|f| frames.iter().position(|(id, _)| *id == f)) else {
            if let Some((id, _)) = frames.last() {
                self.focus_window(Some(*id));
            }
            return;
        };
        let rects: Vec<Rect> = frames.iter().map(|(_, r)| *r).collect();
        if let Some(n) = layout::neighbor(&rects, cur, dir) {
            self.focus_window(Some(frames[n].0));
        } else {
            // Нет соседа — на соседний вывод.
            self.focus_output(dir);
        }
    }

    pub fn move_dir(&mut self, dir: Direction) {
        let Some(id) = self.core.wm.focused else { return };
        let layout = self.core.wm.workspace(self.core.wm.active).layout;
        let Some(m) = self.core.wm.get(id) else { return };
        if m.is_tiled(layout) {
            let frames = self.visible_frames();
            let tiled: Vec<(WindowId, Rect)> = frames
                .into_iter()
                .filter(|(i, _)| self.core.wm.get(*i).is_some_and(|w| w.is_tiled(layout)))
                .collect();
            let Some(cur) = tiled.iter().position(|(i, _)| *i == id) else { return };
            let rects: Vec<Rect> = tiled.iter().map(|(_, r)| *r).collect();
            if let Some(n) = layout::neighbor(&rects, cur, dir) {
                self.core.wm.swap_tile_order(id, tiled[n].0);
                self.relayout();
            } else {
                self.move_to_output_dir(id, dir);
            }
            return;
        }
        // Плавающее — сдвиг на 5% рабочей области.
        let output = self.target_output_for(id);
        let area = output.map(|o| self.core.work_area(&o)).unwrap_or_default();
        let step_x = (area.size.w / 20).max(10);
        let step_y = (area.size.h / 20).max(10);
        let m = self.core.wm.get_mut(id).unwrap();
        match dir {
            Direction::Left => m.loc.x -= step_x,
            Direction::Right => m.loc.x += step_x,
            Direction::Up => m.loc.y -= step_y,
            Direction::Down => m.loc.y += step_y,
        }
        m.snap = None;
        m.float_geo = Some(m.geometry());
        self.sync_space();
        self.core.queue_redraw_all();
    }

    fn output_in_dir(&self, from: &Output, dir: Direction) -> Option<Output> {
        let g = self.core.space.output_geometry(from)?;
        let rects: Vec<(Output, Rect)> = self
            .core
            .space
            .outputs()
            .map(|o| (o.clone(), self.core.space.output_geometry(o).unwrap_or_default()))
            .collect();
        let idx = rects.iter().position(|(o, _)| o == from)?;
        let only: Vec<Rect> = rects.iter().map(|(_, r)| *r).collect();
        let _ = g;
        layout::neighbor(&only, idx, dir).map(|i| rects[i].0.clone())
    }

    pub fn focus_output(&mut self, dir: Direction) {
        let Some(cur) = self.core.output_under_pointer() else { return };
        let Some(target) = self.output_in_dir(&cur, dir) else { return };
        let area = self.core.work_area(&target);
        let center = Point::<f64, Logical>::from((
            area.loc.x as f64 + area.size.w as f64 / 2.0,
            area.loc.y as f64 + area.size.h as f64 / 2.0,
        ));
        self.warp_pointer(center);
        let name = target.name();
        let win = self
            .core
            .wm
            .focus_history
            .iter()
            .rev()
            .copied()
            .find(|id| {
                self.core.wm.get(*id).is_some_and(|w| {
                    w.mapped && !w.minimized && w.on_workspace(self.core.wm.active) && w.output.as_deref() == Some(&name)
                })
            });
        if win.is_some() {
            self.focus_window(win);
        }
    }

    fn move_to_output_dir(&mut self, id: WindowId, dir: Direction) {
        let Some(m) = self.core.wm.get(id) else { return };
        let Some(cur) = m.output.as_ref().and_then(|n| self.core.output_by_name(n)) else { return };
        let Some(target) = self.output_in_dir(&cur, dir) else { return };
        self.move_window_to_output(id, &target);
    }

    pub fn move_to_output(&mut self, dir: Direction) {
        if let Some(id) = self.core.wm.focused {
            self.move_to_output_dir(id, dir);
        }
    }

    pub fn move_window_to_output(&mut self, id: WindowId, target: &Output) {
        let from_area = self
            .core
            .wm
            .get(id)
            .and_then(|m| m.output.as_ref())
            .and_then(|n| self.core.output_by_name(n))
            .map(|o| self.core.work_area(&o));
        let to_area = self.core.work_area(target);
        let Some(m) = self.core.wm.get_mut(id) else { return };
        m.output = Some(target.name());
        // Плавающее окно переносим с сохранением относительного положения.
        if let Some(from) = from_area {
            let rel_x = (m.loc.x - from.loc.x) as f64 / from.size.w.max(1) as f64;
            let rel_y = (m.loc.y - from.loc.y) as f64 / from.size.h.max(1) as f64;
            m.loc = (
                to_area.loc.x + (rel_x * to_area.size.w as f64) as i32,
                to_area.loc.y + (rel_y * to_area.size.h as f64) as i32,
            )
                .into();
            if let Some(fg) = &mut m.float_geo {
                fg.loc = m.loc;
            }
        }
        self.relayout();
        self.core.ipc_dirty = true;
    }

    pub fn set_layout(&mut self, kind: LayoutKind) {
        let active = self.core.wm.active;
        let prev = self.core.wm.workspace(active).layout;
        if prev == kind {
            return;
        }
        // Уходя из плавающей раскладки — запомнить геометрию окон.
        if prev == LayoutKind::Floating {
            let ids = self.core.wm.visible_ids();
            for id in ids {
                self.remember_float(id);
            }
        }
        self.core.wm.workspace_mut(active).layout = kind;
        if kind == LayoutKind::Floating {
            let ids: Vec<WindowId> = self.core.wm.visible_ids();
            for id in ids {
                let (max, fs, snap) = {
                    let m = self.core.wm.get(id).unwrap();
                    (m.maximized, m.fullscreen, m.snap)
                };
                if !max && !fs && snap.is_none() {
                    self.restore_float(id);
                }
            }
        }
        self.relayout();
        self.broadcast_workspaces();
    }

    pub fn adjust_master(&mut self, ratio_delta: f32, count_delta: i32) {
        let active = self.core.wm.active;
        let ws = self.core.wm.workspace_mut(active);
        ws.master_ratio = (ws.master_ratio + ratio_delta).clamp(0.1, 0.9);
        ws.master_count = (ws.master_count as i32 + count_delta).clamp(1, 16) as u32;
        self.relayout();
    }

    // ─── Alt+Tab ────────────────────────────────────────────────────────────

    /// Следующее (или предыдущее) окно по истории фокуса. Пока держат
    /// модификатор — выбор движется, отпустили — зафиксирован.
    pub fn cycle_windows(&mut self, forward: bool) {
        if self.core.wm.switcher.is_none() {
            let mut order: Vec<WindowId> = self
                .core
                .wm
                .focus_history
                .iter()
                .rev()
                .copied()
                .filter(|id| self.core.wm.get(*id).is_some_and(|w| w.mapped && !w.skip_taskbar))
                .collect();
            // Окна без истории фокуса — в конец.
            for m in &self.core.wm.windows {
                if m.mapped && !m.skip_taskbar && !order.contains(&m.id) {
                    order.push(m.id);
                }
            }
            if order.is_empty() {
                return;
            }
            self.core.wm.switcher = Some(super::Switcher { order, index: 0 });
        }
        let sw = self.core.wm.switcher.as_mut().unwrap();
        let n = sw.order.len();
        if n == 0 {
            self.core.wm.switcher = None;
            return;
        }
        sw.index = if forward { (sw.index + 1) % n } else { (sw.index + n - 1) % n };
        let target = sw.order[sw.index];
        let ids: Vec<String> = sw.order.iter().map(|x| x.to_string()).collect();
        let cmd = format!("window-switcher {} {}", target, ids.join(","));
        self.core.ipc.broadcast(&Event::ShellCommand { command: cmd });
        // Показываем выбранное окно сразу (как Plasma с «показать выбранное»).
        self.focus_window(Some(target));
    }

    /// Отпущен модификатор — завершить Alt+Tab.
    pub fn end_cycle(&mut self) {
        if self.core.wm.switcher.take().is_some() {
            self.core.ipc.broadcast(&Event::ShellCommand { command: "window-switcher-end".into() });
            if let Some(f) = self.core.wm.focused {
                self.core.wm.note_focus(f);
            }
        }
    }

    // ─── IPC ────────────────────────────────────────────────────────────────

    pub fn window_info(&self, m: &Managed) -> WindowInfo {
        let g = m.geometry();
        let pid = m.window.wl_surface().and_then(|s| {
            let client = s.client()?;
            client.get_credentials(&self.core.display_handle).ok().map(|c| c.pid)
        });
        let layout = self.core.wm.workspace(m.workspace).layout;
        WindowInfo {
            id: m.id,
            title: m.title(),
            app_id: m.app_id(),
            pid,
            workspace: m.workspace,
            output: m.output.clone(),
            focused: self.core.wm.focused == Some(m.id),
            minimized: m.minimized,
            maximized: m.maximized,
            fullscreen: m.fullscreen,
            floating: !m.is_tiled(layout),
            sticky: m.sticky,
            always_on_top: m.above,
            urgent: m.urgent,
            skip_taskbar: m.skip_taskbar,
            geometry: [g.loc.x, g.loc.y, g.size.w, g.size.h],
        }
    }

    pub fn all_window_infos(&self) -> Vec<WindowInfo> {
        self.core
            .wm
            .windows
            .iter()
            .filter(|m| m.mapped)
            .map(|m| self.window_info(m))
            .collect()
    }

    pub fn workspace_infos(&self) -> Vec<syndesktop_common::ipc::WorkspaceInfo> {
        (0..self.core.wm.count())
            .map(|i| syndesktop_common::ipc::WorkspaceInfo {
                index: i,
                name: self.core.config.workspaces.name(i),
                active: i == self.core.wm.active,
                windows: self.core.wm.windows.iter().filter(|w| w.mapped && w.workspace == i).count() as u32,
                urgent: self.core.wm.windows.iter().any(|w| w.urgent && w.workspace == i),
                layout: self.core.wm.workspace(i).layout,
            })
            .collect()
    }

    pub fn broadcast_workspaces(&mut self) {
        let workspaces = self.workspace_infos();
        self.core.ipc.broadcast(&Event::WorkspacesChanged { workspaces });
    }

    /// Разослать изменившиеся окна (вызывается раз за цикл событий).
    pub fn flush_ipc(&mut self) {
        if !self.core.ipc_dirty {
            return;
        }
        self.core.ipc_dirty = false;
        let mut changed = Vec::new();
        for m in &self.core.wm.windows {
            if !m.mapped {
                continue;
            }
            let info = self.window_info(m);
            if m.last_info.as_ref() != Some(&info) {
                changed.push(info);
            }
        }
        let ws_counts_changed = !changed.is_empty();
        for info in changed {
            if let Some(m) = self.core.wm.get_mut(info.id) {
                m.last_info = Some(info.clone());
            }
            self.core.ipc.broadcast(&Event::WindowChanged { window: info });
        }
        if ws_counts_changed {
            self.broadcast_workspaces();
        }
    }

    /// Разрешить поверхности (окну) внимание: активировать или пометить.
    pub fn activate_or_mark_urgent(&mut self, surface: &WlSurface, allowed: bool) {
        let Some(id) = self.core.wm.id_by_surface(surface) else { return };
        if allowed {
            self.focus_window(Some(id));
        } else if self.core.wm.focused != Some(id) {
            if let Some(m) = self.core.wm.get_mut(id) {
                m.urgent = true;
            }
            self.core.ipc_dirty = true;
        }
    }

    /// Подогнать геометрию xdg-окна в процессе изменения размера за левый/верхний край.
    pub fn fixup_resize_location(&mut self, surface: &WlSurface) {
        let Some(m) = self.core.wm.by_surface_mut(surface) else { return };
        let Some(r) = m.resizing else { return };
        let size = m.window.geometry().size;
        if r.edges.contains(super::ResizeEdge::LEFT) {
            m.loc.x = r.initial_loc.x + (r.initial_size.w - size.w);
        }
        if r.edges.contains(super::ResizeEdge::TOP) {
            m.loc.y = r.initial_loc.y + (r.initial_size.h - size.h);
        }
        if m.resize_finishing {
            m.resizing = None;
            m.resize_finishing = false;
            m.float_geo = Some(m.geometry());
        }
        let (w, loc) = (m.window.clone(), m.loc);
        if self.core.space.elements().any(|e| e == &w) {
            self.core.space.map_element(w, loc, false);
            self.sync_space();
        }
    }

    /// Выводы изменились (добавлен/убран/режим/масштаб): переложить окна.
    pub fn outputs_changed(&mut self) {
        let outputs: Vec<Output> = self.core.space.outputs().cloned().collect();
        for o in &outputs {
            layer_map_for_output(o).arrange();
        }
        // Окна за пределами всех выводов — вернуть на основной.
        let rects: Vec<Rect> = outputs.iter().filter_map(|o| self.core.space.output_geometry(o)).collect();
        let primary = self.core.primary_output();
        let ids: Vec<WindowId> = self.core.wm.windows.iter().map(|w| w.id).collect();
        for id in ids {
            let g = self.core.wm.get(id).unwrap().geometry();
            let visible = rects.iter().any(|r| r.overlaps(g));
            if !visible {
                if let Some(p) = &primary {
                    let area = self.core.work_area(p);
                    let m = self.core.wm.get_mut(id).unwrap();
                    m.loc = center_in(area, g.size);
                    m.output = Some(p.name());
                    if let Some(fg) = &mut m.float_geo {
                        fg.loc = m.loc;
                    }
                }
            }
        }
        self.relayout();
        let outputs = crate::ipc::output_infos(self);
        self.core.ipc.broadcast(&Event::OutputsChanged { outputs });
    }
}

impl super::Wm {
    /// Порядковый ключ окна в плитке (порядок перестраивается `swap_tile_order`).
    pub fn tile_order_key(&self, id: WindowId) -> i64 {
        self.get(id)
            .map(|m| m.window.user_data().get::<TileOrder>().map(|o| o.0.get()).unwrap_or(m.id as i64 * 1000))
            .unwrap_or(i64::MAX)
    }

    pub fn swap_tile_order(&mut self, a: WindowId, b: WindowId) {
        let (ka, kb) = (self.tile_order_key(a), self.tile_order_key(b));
        for (id, k) in [(a, kb), (b, ka)] {
            if let Some(m) = self.get(id) {
                m.window.user_data().insert_if_missing(|| TileOrder(std::cell::Cell::new(k)));
                m.window.user_data().get::<TileOrder>().unwrap().0.set(k);
            }
        }
    }

    /// Сделать окно мастером (первым в плитке).
    pub fn make_master(&mut self, id: WindowId) {
        let min = self.windows.iter().map(|m| self.tile_order_key(m.id)).min().unwrap_or(0);
        if let Some(m) = self.get(id) {
            m.window.user_data().insert_if_missing(|| TileOrder(std::cell::Cell::new(min - 1)));
            m.window.user_data().get::<TileOrder>().unwrap().0.set(min - 1);
        }
    }
}

/// Порядок окна в плитке (по умолчанию — по времени появления).
struct TileOrder(std::cell::Cell<i64>);

fn set_state(states: &mut smithay::wayland::shell::xdg::ToplevelStateSet, st: xdg_toplevel::State, on: bool) {
    if on {
        states.set(st);
    } else {
        states.unset(st);
    }
}

pub fn center_in(area: Rect, size: Size<i32, Logical>) -> Point<i32, Logical> {
    let x = area.loc.x + (area.size.w - size.w) / 2;
    let y = area.loc.y + (area.size.h - size.h) / 2;
    (x.max(area.loc.x), y.max(area.loc.y)).into()
}

fn clamp_into(area: Rect, r: Rect) -> Rect {
    let x = r.loc.x.clamp(area.loc.x, (area.loc.x + area.size.w - r.size.w).max(area.loc.x));
    let y = r.loc.y.clamp(area.loc.y, (area.loc.y + area.size.h - r.size.h).max(area.loc.y));
    Rectangle::new((x, y).into(), r.size)
}

/// «Умное» размещение: перебор позиций сеткой, минимум перекрытия.
fn smart_place(area: Rect, size: Size<i32, Logical>, others: &[Rect]) -> Point<i32, Logical> {
    let step = 24;
    let mut best = (i64::MAX, center_in(area, size));
    let max_x = (area.loc.x + area.size.w - size.w).max(area.loc.x);
    let max_y = (area.loc.y + area.size.h - size.h).max(area.loc.y);
    let mut y = area.loc.y;
    while y <= max_y {
        let mut x = area.loc.x;
        while x <= max_x {
            let r = Rectangle::new((x, y).into(), size);
            let overlap: i64 = others
                .iter()
                .filter_map(|o| o.intersection(r))
                .map(|i| i.size.w as i64 * i.size.h as i64)
                .sum();
            if overlap < best.0 {
                best = (overlap, r.loc);
                if overlap == 0 {
                    return best.1;
                }
            }
            x += step;
        }
        y += step;
    }
    best.1
}

/// Текущий заголовок xdg-окна (для title_changed без Managed).
#[allow(dead_code)]
pub fn toplevel_title(surface: &WlSurface) -> String {
    with_states(surface, |s| {
        s.data_map
            .get::<XdgToplevelSurfaceData>()
            .and_then(|d| d.lock().unwrap().title.clone())
            .unwrap_or_default()
    })
}
