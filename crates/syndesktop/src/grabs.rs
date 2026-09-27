//! Захваты указателя: перемещение и изменение размера окна мышью.

use smithay::{
    desktop::WindowSurface,
    input::pointer::{
        AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
        GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent,
        GestureSwipeUpdateEvent, GrabStartData, MotionEvent, PointerGrab, PointerInnerHandle, RelativeMotionEvent,
    },
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel,
    utils::{Logical, Point, Rectangle, Serial, Size},
    wayland::{compositor::with_states, shell::xdg::SurfaceCachedState},
};
use syndesktop_common::action::LayoutKind;

use crate::{
    anim::{Animation, Curve},
    focus::FocusTarget,
    state::State,
    wm::{layout::SnapZone, ResizeData, ResizeEdge, WindowId},
};

/// Проброс жестов без изменений — одинаков для всех захватов.
macro_rules! forward_gestures {
    () => {
        fn gesture_swipe_begin(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>, event: &GestureSwipeBeginEvent) {
            handle.gesture_swipe_begin(data, event)
        }
        fn gesture_swipe_update(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>, event: &GestureSwipeUpdateEvent) {
            handle.gesture_swipe_update(data, event)
        }
        fn gesture_swipe_end(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>, event: &GestureSwipeEndEvent) {
            handle.gesture_swipe_end(data, event)
        }
        fn gesture_pinch_begin(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>, event: &GesturePinchBeginEvent) {
            handle.gesture_pinch_begin(data, event)
        }
        fn gesture_pinch_update(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>, event: &GesturePinchUpdateEvent) {
            handle.gesture_pinch_update(data, event)
        }
        fn gesture_pinch_end(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>, event: &GesturePinchEndEvent) {
            handle.gesture_pinch_end(data, event)
        }
        fn gesture_hold_begin(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>, event: &GestureHoldBeginEvent) {
            handle.gesture_hold_begin(data, event)
        }
        fn gesture_hold_end(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>, event: &GestureHoldEndEvent) {
            handle.gesture_hold_end(data, event)
        }
    };
}

// ─── перемещение ───────────────────────────────────────────────────────────

pub struct MoveGrab {
    pub start: GrabStartData<State>,
    pub id: WindowId,
    pub initial_loc: Point<i32, Logical>,
    /// Окно было развёрнуто/прилеплено — «отлипнет» при первом движении.
    pub detach_pending: bool,
    /// Окно в плитке: по отпусканию меняется местами с окном под курсором.
    pub tiled: bool,
    pub snap: Option<SnapZone>,
}

impl PointerGrab<State> for MoveGrab {
    fn motion(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>, _focus: Option<(FocusTarget, Point<f64, Logical>)>, event: &MotionEvent) {
        handle.motion(data, None, event);
        if data.core.wm.get(self.id).is_none() {
            handle.unset_grab(self, data, event.serial, event.time, true);
            return;
        }
        let delta = event.location - self.start.location;
        if self.detach_pending {
            if delta.x.abs() < 8.0 && delta.y.abs() < 8.0 {
                return;
            }
            self.detach_pending = false;
            self.detach(data, event.location);
        }
        let mut new_loc = (self.initial_loc.to_f64() + delta).to_i32_round();
        let title_h = data.core.deco_theme.height;
        let (size, top) = {
            let m = data.core.wm.get(self.id).unwrap();
            (m.size(), if m.has_titlebar() { title_h } else { 0 })
        };
        let output = data.core.output_at(event.location);
        let area = output.as_ref().map(|o| data.core.work_area(o));

        // Прилипание к краям рабочей области и соседним окнам.
        if data.core.config.windows.snap && !self.tiled {
            if let Some(area) = area {
                let d = data.core.config.windows.snap_distance.max(0);
                let frame: Rectangle<i32, Logical> = Rectangle::new((new_loc.x, new_loc.y - top).into(), (size.w, size.h + top).into());
                let mut edges_x = vec![area.loc.x, area.loc.x + area.size.w];
                let mut edges_y = vec![area.loc.y, area.loc.y + area.size.h];
                for m in &data.core.wm.windows {
                    if m.id == self.id || !m.mapped || m.minimized || !m.on_workspace(data.core.wm.active) {
                        continue;
                    }
                    let g = m.geometry();
                    let t = if m.has_titlebar() { title_h } else { 0 };
                    edges_x.extend([g.loc.x, g.loc.x + g.size.w]);
                    edges_y.extend([g.loc.y - t, g.loc.y + g.size.h]);
                }
                let snap_axis = |pos: i32, len: i32, edges: &[i32]| -> i32 {
                    let mut best = (d + 1, pos);
                    for &e in edges {
                        for (cand, dist) in [(e, (pos - e).abs()), (e - len, (pos + len - e).abs())] {
                            if dist < best.0 {
                                best = (dist, cand);
                            }
                        }
                    }
                    best.1
                };
                let x = snap_axis(frame.loc.x, frame.size.w, &edges_x);
                let y = snap_axis(frame.loc.y, frame.size.h, &edges_y);
                new_loc = (x, y + top).into();
            }
        }

        // Подсветка зоны прилипания к половинам экрана.
        let zone = if data.core.config.windows.edge_tiling && !self.tiled {
            area.and_then(|a| {
                let geo = output.as_ref().and_then(|o| data.core.space.output_geometry(o)).unwrap_or(a);
                SnapZone::from_pointer(geo, event.location, 3.0)
            })
        } else {
            None
        };
        if zone != self.snap {
            self.snap = zone;
            data.core.wm.snap_preview = match (zone, area) {
                (Some(SnapZone::Top), Some(a)) => Some((a, Animation::new(0.0, 1.0, std::time::Duration::from_millis(140), Curve::EaseOutCubic))),
                (Some(z), Some(a)) => Some((
                    z.rect(a, data.core.config.windows.gaps_outer.min(8)),
                    Animation::new(0.0, 1.0, std::time::Duration::from_millis(140), Curve::EaseOutCubic),
                )),
                _ => None,
            };
        }

        let m = data.core.wm.get_mut(self.id).unwrap();
        m.loc = new_loc;
        if let Some(o) = &output {
            m.output = Some(o.name());
        }
        let (w, loc) = (m.window.clone(), m.loc);
        data.core.space.map_element(w, loc, false);
        data.sync_space();
        data.core.queue_redraw_all();
    }

    fn relative_motion(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>, focus: Option<(FocusTarget, Point<f64, Logical>)>, event: &RelativeMotionEvent) {
        handle.relative_motion(data, focus, event);
    }

    fn button(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>, event: &ButtonEvent) {
        handle.button(data, event);
        if handle.current_pressed().is_empty() {
            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }

    fn axis(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>, details: AxisFrame) {
        handle.axis(data, details)
    }

    fn frame(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>) {
        handle.frame(data)
    }

    forward_gestures!();

    fn start_data(&self) -> &GrabStartData<State> {
        &self.start
    }

    fn unset(&mut self, data: &mut State) {
        data.core.wm.snap_preview = None;
        data.core.wm.dragging = None;
        if data.core.wm.get(self.id).is_none() {
            return;
        }
        if self.tiled {
            // Поменяться местами с плиточным окном под курсором.
            let pos = data.core.pointer.current_location();
            let layout = data.core.wm.workspace(data.core.wm.active).layout;
            let target = data
                .core
                .wm
                .visible_ids()
                .into_iter()
                .rev()
                .filter(|id| *id != self.id)
                .find(|id| {
                    data.core.wm.get(*id).is_some_and(|m| m.is_tiled(layout) && m.geometry().to_f64().contains(pos))
                });
            if let Some(t) = target {
                data.core.wm.swap_tile_order(self.id, t);
            }
            // Окно могло переехать на другой вывод.
            if let Some(o) = data.core.output_at(pos) {
                if let Some(m) = data.core.wm.get_mut(self.id) {
                    m.output = Some(o.name());
                }
            }
            data.relayout();
            return;
        }
        match self.snap.take() {
            Some(SnapZone::Top) => data.apply_maximized(self.id, true),
            Some(z) => data.set_snap(self.id, Some(z)),
            None => {
                if let Some(m) = data.core.wm.get_mut(self.id) {
                    m.float_geo = Some(m.geometry());
                }
            }
        }
        data.core.ipc_dirty = true;
        data.core.queue_redraw_all();
    }
}

impl MoveGrab {
    /// Отлепить развёрнутое/прилепленное окно: вернуть плавающий размер так,
    /// чтобы курсор остался на той же доле ширины заголовка.
    fn detach(&mut self, data: &mut State, pointer: Point<f64, Logical>) {
        let Some(m) = data.core.wm.get(self.id) else { return };
        let old = m.geometry();
        let float = m.float_geo.unwrap_or(old);
        let rel = ((pointer.x - old.loc.x as f64) / old.size.w.max(1) as f64).clamp(0.0, 1.0);
        let was_max = m.maximized;
        let title_h = data.core.deco_theme.height;
        let frame_top = old.loc.y - if m.has_titlebar() { title_h } else { 0 };
        let new_y = {
            let m = data.core.wm.get_mut(self.id).unwrap();
            m.maximized = false;
            m.snap = None;
            // Окно тащат с панели (заголовок окна на ней): курсор выше рамки —
            // заголовок встаёт под курсор.
            let top = if m.has_titlebar() { title_h } else { 0 };
            if top > 0 && pointer.y < frame_top as f64 {
                pointer.y as i32 + top / 2
            } else {
                old.loc.y
            }
        };
        let new_x = pointer.x as i32 - (rel * float.size.w as f64) as i32;
        let new_loc: Point<i32, Logical> = (new_x, new_y).into();
        let m = data.core.wm.get_mut(self.id).unwrap();
        m.float_geo = Some(Rectangle::new(new_loc, float.size));
        m.loc = new_loc;
        if let WindowSurface::Wayland(t) = m.window.underlying_surface() {
            t.with_pending_state(|s| {
                s.size = Some(float.size);
                for st in [
                    xdg_toplevel::State::Maximized,
                    xdg_toplevel::State::TiledLeft,
                    xdg_toplevel::State::TiledRight,
                    xdg_toplevel::State::TiledTop,
                    xdg_toplevel::State::TiledBottom,
                ] {
                    s.states.unset(st);
                }
            });
            t.send_pending_configure();
        } else if let Some(x) = m.window.x11_surface() {
            let _ = x.set_maximized(false);
            let _ = x.configure(Rectangle::new(new_loc, float.size));
        }
        let _ = was_max;
        self.initial_loc = new_loc - (pointer - self.start.location).to_i32_round();
        data.core.ipc_dirty = true;
    }
}

// ─── изменение размера ─────────────────────────────────────────────────────

pub struct ResizeGrab {
    pub start: GrabStartData<State>,
    pub id: WindowId,
    pub edges: ResizeEdge,
    pub initial_loc: Point<i32, Logical>,
    pub initial_size: Size<i32, Logical>,
    pub last_size: Size<i32, Logical>,
    /// Плиточное окно: тянем долю мастер-области.
    pub tiled_ratio: Option<(f32, i32)>,
}

impl PointerGrab<State> for ResizeGrab {
    fn motion(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>, _focus: Option<(FocusTarget, Point<f64, Logical>)>, event: &MotionEvent) {
        handle.motion(data, None, event);
        let Some(m) = data.core.wm.get(self.id) else {
            handle.unset_grab(self, data, event.serial, event.time, true);
            return;
        };
        let (mut dx, mut dy) = (event.location - self.start.location).into();

        if let Some((ratio0, area_w)) = self.tiled_ratio {
            // Доля мастера: левый край мастера не двигается.
            if self.edges.contains(ResizeEdge::LEFT) {
                dx = -dx;
            }
            let active = data.core.wm.active;
            let ws = data.core.wm.workspace_mut(active);
            ws.master_ratio = (ratio0 + dx as f32 / area_w.max(1) as f32).clamp(0.1, 0.9);
            data.relayout();
            return;
        }

        let mut w = self.initial_size.w;
        let mut h = self.initial_size.h;
        if self.edges.intersects(ResizeEdge::LEFT | ResizeEdge::RIGHT) {
            if self.edges.contains(ResizeEdge::LEFT) {
                dx = -dx;
            }
            w = (self.initial_size.w as f64 + dx) as i32;
        }
        if self.edges.intersects(ResizeEdge::TOP | ResizeEdge::BOTTOM) {
            if self.edges.contains(ResizeEdge::TOP) {
                dy = -dy;
            }
            h = (self.initial_size.h as f64 + dy) as i32;
        }
        let (min, max) = size_limits(m);
        w = w.max(min.w.max(40)).min(if max.w > 0 { max.w } else { i32::MAX });
        h = h.max(min.h.max(30)).min(if max.h > 0 { max.h } else { i32::MAX });
        self.last_size = (w, h).into();
        match m.window.underlying_surface() {
            WindowSurface::Wayland(t) => {
                t.with_pending_state(|s| {
                    s.states.set(xdg_toplevel::State::Resizing);
                    s.size = Some(self.last_size);
                });
                t.send_pending_configure();
            }
            WindowSurface::X11(x) => {
                let mut loc = self.initial_loc;
                if self.edges.contains(ResizeEdge::LEFT) {
                    loc.x += self.initial_size.w - w;
                }
                if self.edges.contains(ResizeEdge::TOP) {
                    loc.y += self.initial_size.h - h;
                }
                let _ = x.configure(Rectangle::new(loc, self.last_size));
            }
        }
    }

    fn relative_motion(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>, focus: Option<(FocusTarget, Point<f64, Logical>)>, event: &RelativeMotionEvent) {
        handle.relative_motion(data, focus, event);
    }

    fn button(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>, event: &ButtonEvent) {
        handle.button(data, event);
        if handle.current_pressed().is_empty() {
            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }

    fn axis(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>, details: AxisFrame) {
        handle.axis(data, details)
    }

    fn frame(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>) {
        handle.frame(data)
    }

    forward_gestures!();

    fn start_data(&self) -> &GrabStartData<State> {
        &self.start
    }

    fn unset(&mut self, data: &mut State) {
        if self.tiled_ratio.is_some() {
            return;
        }
        let Some(m) = data.core.wm.get_mut(self.id) else { return };
        match m.window.underlying_surface() {
            WindowSurface::Wayland(t) => {
                t.with_pending_state(|s| {
                    s.states.unset(xdg_toplevel::State::Resizing);
                    s.size = Some(self.last_size);
                });
                t.send_pending_configure();
                // Положение поправит последний коммит (fixup_resize_location).
                m.resize_finishing = true;
            }
            WindowSurface::X11(_) => {
                m.resizing = None;
                m.float_geo = Some(m.geometry());
            }
        }
        data.core.ipc_dirty = true;
    }
}

fn size_limits(m: &crate::wm::Managed) -> (Size<i32, Logical>, Size<i32, Logical>) {
    if let Some(t) = m.window.toplevel() {
        with_states(t.wl_surface(), |states| {
            let mut guard = states.cached_state.get::<SurfaceCachedState>();
            let d = guard.current();
            (d.min_size, d.max_size)
        })
    } else if let Some(x) = m.window.x11_surface() {
        (x.min_size().unwrap_or_default(), x.max_size().unwrap_or_default())
    } else {
        (Size::default(), Size::default())
    }
}

impl State {
    /// Начать перемещение окна (из запроса клиента, заголовка или Super+ЛКМ).
    pub fn start_move(&mut self, id: WindowId, start: GrabStartData<State>, serial: Serial) {
        let Some(m) = self.core.wm.get(id) else { return };
        if m.fullscreen {
            return;
        }
        let layout = self.core.wm.workspace(self.core.wm.active).layout;
        let grab = MoveGrab {
            start,
            id,
            initial_loc: m.loc,
            detach_pending: m.maximized || m.snap.is_some(),
            tiled: m.is_tiled(layout),
            snap: None,
        };
        self.core.wm.raise(id);
        self.core.wm.dragging = Some(id);
        self.sync_space();
        let pointer = self.core.pointer.clone();
        pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Clear);
        self.core.cursor_status = smithay::input::pointer::CursorImageStatus::Named(smithay::input::pointer::CursorIcon::Grabbing);
    }

    /// Начать изменение размера.
    pub fn start_resize(&mut self, id: WindowId, edges: ResizeEdge, start: GrabStartData<State>, serial: Serial) {
        let Some(m) = self.core.wm.get(id) else { return };
        if m.fullscreen || m.maximized || edges.is_empty() {
            return;
        }
        let layout = self.core.wm.workspace(self.core.wm.active).layout;
        let tiled_ratio = if m.is_tiled(layout) {
            if !matches!(layout, LayoutKind::Tile) || !edges.intersects(ResizeEdge::LEFT | ResizeEdge::RIGHT) {
                return;
            }
            let area_w = m
                .output
                .as_ref()
                .and_then(|n| self.core.output_by_name(n))
                .map(|o| self.core.work_area(&o).size.w)
                .unwrap_or(1000);
            Some((self.core.wm.workspace(self.core.wm.active).master_ratio, area_w))
        } else {
            None
        };
        let loc = m.loc;
        let size = m.size();
        if tiled_ratio.is_none() {
            let m = self.core.wm.get_mut(id).unwrap();
            m.resizing = Some(ResizeData { edges, initial_loc: loc, initial_size: size });
            m.resize_finishing = false;
            m.snap = None;
        }
        let grab = ResizeGrab {
            start,
            id,
            edges,
            initial_loc: loc,
            initial_size: size,
            last_size: size,
            tiled_ratio,
        };
        let pointer = self.core.pointer.clone();
        pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Clear);
        self.core.cursor_status = smithay::input::pointer::CursorImageStatus::Named(edges.cursor());
    }

    /// Окно, которое сейчас тащат мышью.
    pub fn dragged_window(&self) -> Option<WindowId> {
        self.core.wm.dragging
    }
}
