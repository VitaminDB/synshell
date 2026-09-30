//! Ввод: клавиатура (сочетания), указатель (рамки, фокус, захваты),
//! колесо, жесты, сенсорный экран, настройки устройств libinput.

use std::time::{Duration, Instant};

use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, GestureBeginEvent, GestureEndEvent,
        GesturePinchUpdateEvent as _, GestureSwipeUpdateEvent as _, InputBackend, InputEvent, KeyState,
        KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent, TouchEvent,
    },
    desktop::{layer_map_for_output, WindowSurfaceType},
    input::{
        keyboard::{FilterResult, Keysym},
        pointer::{
            AxisFrame, ButtonEvent, CursorIcon, CursorImageStatus, GestureHoldBeginEvent, GestureHoldEndEvent,
            GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent,
            GestureSwipeEndEvent, GestureSwipeUpdateEvent, GrabStartData, MotionEvent, RelativeMotionEvent,
        },
        touch::{DownEvent, UpEvent},
    },
    output::Output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER},
    wayland::{
        compositor::with_states,
        keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitorSeat,
        pointer_constraints::{with_pointer_constraint, PointerConstraint},
        seat::WaylandFocus,
        shell::wlr_layer::{KeyboardInteractivity, Layer, LayerSurfaceCachedState},
    },
};
use synshell_common::{
    action::{Action, Direction, Mods, WorkspaceTarget},
    config::FocusMode,
};

use crate::{
    bindings::{is_modifier, mods_from_state, Trigger},
    deco::{Button, DecoHit},
    focus::FocusTarget,
    state::State,
    wm::{ResizeEdge, WindowId},
};

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;

/// Что под указателем.
#[derive(Debug, Clone)]
pub enum Under {
    /// Поверхность клиента и её начало в глобальных координатах.
    Surface(FocusTarget, Point<f64, Logical>),
    /// Рамка окна.
    Deco(WindowId, DecoHit),
    /// Невидимая кромка изменения размера.
    Resize(WindowId, ResizeEdge),
    Nothing,
}

impl Under {
    pub(crate) fn focus(&self) -> Option<(FocusTarget, Point<f64, Logical>)> {
        match self {
            Under::Surface(s, p) => Some((s.clone(), *p)),
            _ => None,
        }
    }
}

/// Состояние жеста тачпада (переключение столов тремя пальцами).
#[derive(Default)]
pub struct GestureState {
    pub fingers: u32,
    pub dx: f64,
    pub dy: f64,
    pub active: bool,
}

/// Клик по заголовку: для распознавания двойного клика.
pub struct LastTitleClick {
    pub id: WindowId,
    pub at: Instant,
}

impl State {
    pub fn process_input_event<B: InputBackend>(&mut self, event: InputEvent<B>) {
        // Любой ввод — активность: будим мониторы, сбрасываем таймер простоя.
        let is_activity = !matches!(event, InputEvent::DeviceAdded { .. } | InputEvent::DeviceRemoved { .. });
        if is_activity {
            self.note_activity();
        }
        match event {
            InputEvent::Keyboard { event } => self.on_keyboard::<B>(event),
            InputEvent::PointerMotion { event } => self.on_pointer_motion::<B>(event),
            InputEvent::PointerMotionAbsolute { event } => self.on_pointer_motion_absolute::<B>(event),
            InputEvent::PointerButton { event } => self.on_pointer_button::<B>(event),
            InputEvent::PointerAxis { event } => self.on_pointer_axis::<B>(event),
            InputEvent::GestureSwipeBegin { event } => {
                self.core.gesture = GestureState { fingers: event.fingers(), dx: 0.0, dy: 0.0, active: event.fingers() >= 3 };
                if !self.core.gesture.active {
                    let pointer = self.core.pointer.clone();
                    pointer.gesture_swipe_begin(
                        self,
                        &GestureSwipeBeginEvent { serial: SERIAL_COUNTER.next_serial(), time: event.time_msec(), fingers: event.fingers() },
                    );
                }
            }
            InputEvent::GestureSwipeUpdate { event } => {
                if self.core.gesture.active {
                    let d = event.delta();
                    self.core.gesture.dx += d.x;
                    self.core.gesture.dy += d.y;
                } else {
                    let pointer = self.core.pointer.clone();
                    pointer.gesture_swipe_update(self, &GestureSwipeUpdateEvent { time: event.time_msec(), delta: event.delta() });
                }
            }
            InputEvent::GestureSwipeEnd { event } => {
                if self.core.gesture.active {
                    let g = std::mem::take(&mut self.core.gesture);
                    if !event.cancelled() {
                        self.finish_swipe(g);
                    }
                } else {
                    let pointer = self.core.pointer.clone();
                    pointer.gesture_swipe_end(
                        self,
                        &GestureSwipeEndEvent { serial: SERIAL_COUNTER.next_serial(), time: event.time_msec(), cancelled: event.cancelled() },
                    );
                }
            }
            InputEvent::GesturePinchBegin { event } => {
                let pointer = self.core.pointer.clone();
                pointer.gesture_pinch_begin(
                    self,
                    &GesturePinchBeginEvent { serial: SERIAL_COUNTER.next_serial(), time: event.time_msec(), fingers: event.fingers() },
                );
            }
            InputEvent::GesturePinchUpdate { event } => {
                let pointer = self.core.pointer.clone();
                pointer.gesture_pinch_update(
                    self,
                    &GesturePinchUpdateEvent { time: event.time_msec(), delta: event.delta(), scale: event.scale(), rotation: event.rotation() },
                );
            }
            InputEvent::GesturePinchEnd { event } => {
                let pointer = self.core.pointer.clone();
                pointer.gesture_pinch_end(
                    self,
                    &GesturePinchEndEvent { serial: SERIAL_COUNTER.next_serial(), time: event.time_msec(), cancelled: event.cancelled() },
                );
            }
            InputEvent::GestureHoldBegin { event } => {
                let pointer = self.core.pointer.clone();
                pointer.gesture_hold_begin(
                    self,
                    &GestureHoldBeginEvent { serial: SERIAL_COUNTER.next_serial(), time: event.time_msec(), fingers: event.fingers() },
                );
            }
            InputEvent::GestureHoldEnd { event } => {
                let pointer = self.core.pointer.clone();
                pointer.gesture_hold_end(
                    self,
                    &GestureHoldEndEvent { serial: SERIAL_COUNTER.next_serial(), time: event.time_msec(), cancelled: event.cancelled() },
                );
            }
            InputEvent::TouchDown { event } => self.on_touch_down::<B>(event),
            InputEvent::TouchUp { event } => self.on_touch_up::<B>(event),
            InputEvent::TouchMotion { event } => self.on_touch_motion::<B>(event),
            InputEvent::TouchFrame { .. } => {
                if let Some(touch) = self.core.seat.get_touch() {
                    touch.frame(self);
                }
            }
            InputEvent::TouchCancel { .. } => {
                self.core.touch_gestures.pending = None;
                self.fingers_cancel();
                if let Some(touch) = self.core.seat.get_touch() {
                    touch.cancel(self);
                }
            }
            InputEvent::DeviceAdded { device } => self.device_added_generic::<B>(&device),
            _ => {}
        }
    }

    fn device_added_generic<B: InputBackend>(&mut self, device: &B::Device) {
        use smithay::backend::input::{Device, DeviceCapability};
        if device.has_capability(DeviceCapability::Touch) && self.core.seat.get_touch().is_none() {
            self.core.seat.add_touch();
        }
    }

    pub fn note_activity(&mut self) {
        self.core.last_activity = Instant::now();
        let seat = self.core.seat.clone();
        self.core.idle_notifier_state.notify_activity(&seat);
        if self.core.monitors_off {
            self.set_monitors_power(true);
        }
    }

    // ─── клавиатура ─────────────────────────────────────────────────────────

    fn on_keyboard<B: InputBackend>(&mut self, event: B::KeyboardKeyEvent) {
        let keycode = event.key_code();
        let state = event.state();
        let serial = SERIAL_COUNTER.next_serial();
        let time = Event::time_msec(&event);
        let raw = keycode.raw();

        // Сочетания подавляются, если окно попросило (виртуальные машины) —
        // кроме смены VT.
        let inhibited = self
            .core
            .keyboard
            .current_focus()
            .and_then(|f| self.core.seat.keyboard_shortcuts_inhibitor_for_surface(&f.0))
            .is_some_and(|i| i.is_active());
        let locked = self.core.is_locked();
        let overview = self.core.wm.overview.as_ref().is_some_and(|o| !o.closing);
        let keyboard = self.core.keyboard.clone();

        // Состояние xkb обновляется один раз; пересылка клиенту — отдельно,
        // чтобы изменения модификаторов и группы (grp:*_toggle) не терялись.
        let (filtered, mods_changed) = keyboard.input_intercept::<FilterResult<KeyIntent>, _>(self, keycode, state, |st, mods, handle| {
            let sym = handle.modified_sym();
            let latin = handle.raw_latin_sym_or_raw_current_sym().unwrap_or(sym);
            if state == KeyState::Released {
                if st.core.suppressed_keys.remove(&raw) {
                    // Отпустили модификатор во время Alt+Tab — фиксируем выбор.
                    return FilterResult::Intercept(if is_modifier(sym) { KeyIntent::ModifierReleased } else { KeyIntent::None });
                }
                if is_modifier(sym) {
                    return FilterResult::Intercept(KeyIntent::ModifierReleasedForward);
                }
                return FilterResult::Forward;
            }
            // Смена VT работает всегда.
            let vt = sym.raw();
            if (Keysym::XF86_Switch_VT_1.raw()..=Keysym::XF86_Switch_VT_12.raw()).contains(&vt) {
                st.core.suppressed_keys.insert(raw);
                return FilterResult::Intercept(KeyIntent::Vt((vt - Keysym::XF86_Switch_VT_1.raw() + 1) as i32));
            }
            if locked {
                return FilterResult::Forward;
            }
            if overview {
                st.core.suppressed_keys.insert(raw);
                return FilterResult::Intercept(KeyIntent::Overview(sym));
            }
            if inhibited {
                return FilterResult::Forward;
            }
            let m = mods_from_state(mods);
            let found = st
                .core
                .bindings
                .find(m, Trigger::Key(latin))
                .or_else(|| st.core.bindings.find(m, Trigger::Key(sym)))
                .map(|b| b.action.clone());
            match found {
                Some(action) => {
                    st.core.suppressed_keys.insert(raw);
                    FilterResult::Intercept(KeyIntent::Action(action))
                }
                None => FilterResult::Forward,
            }
        });
        let action = match filtered {
            FilterResult::Forward => {
                keyboard.input_forward(self, keycode, state, serial, time, mods_changed);
                None
            }
            FilterResult::Intercept(KeyIntent::ModifierReleasedForward) => {
                // Отпускание модификатора нужно и клиенту.
                keyboard.input_forward(self, keycode, state, serial, time, mods_changed);
                Some(KeyIntent::ModifierReleasedForward)
            }
            FilterResult::Intercept(intent) => {
                if mods_changed {
                    self.send_modifiers();
                }
                Some(intent)
            }
        };
        if mods_changed {
            // Группа могла смениться опцией xkb (grp:ctrl_shift_toggle и т.п.).
            self.broadcast_keyboard_layout();
        }

        match action {
            Some(KeyIntent::Action(a)) => self.do_action(a),
            Some(KeyIntent::Vt(n)) => self.backend.change_vt(n),
            Some(KeyIntent::Overview(sym)) => self.overview_key(sym),
            Some(KeyIntent::ModifierReleased) | Some(KeyIntent::ModifierReleasedForward) => {
                // Модификатор отпущен — если шёл Alt+Tab, завершаем.
                let still_held = self.core.keyboard.modifier_state();
                if !still_held.alt && !still_held.logo && !still_held.ctrl {
                    self.end_cycle();
                }
            }
            _ => {}
        }
        self.core.suppressed_keys.retain(|_| true);
        // Курсор прячется при наборе.
        if self.core.config.windows.hide_cursor_after > 0 && state == KeyState::Pressed {
            self.core.cursor_hidden = true;
            self.core.queue_redraw_all();
        }
    }

    /// Сообщить клиенту в фокусе текущие модификаторы и группу (когда само
    /// нажатие перехвачено композитором).
    fn send_modifiers(&mut self) {
        use smithay::input::keyboard::KeyboardTarget;
        let keyboard = self.core.keyboard.clone();
        let Some(focus) = keyboard.current_focus() else { return };
        let seat = self.core.seat.clone();
        let mods = keyboard.modifier_state();
        focus.modifiers(&seat, self, mods, SERIAL_COUNTER.next_serial());
    }

    /// Нажать сочетание в окне с фокусом (жест «назад», `key Alt+Left`):
    /// модификаторы, клавиша, отпускание в обратном порядке — через xkb
    /// композитора, как будто нажали на клавиатуре.
    pub fn send_key_combo(&mut self, combo: &synshell_common::action::KeyCombo) {
        use synshell_common::action::Mods;
        let Some(sym) = crate::bindings::keysym_from_name(&combo.key) else {
            tracing::warn!(key = %combo.key, "key: неизвестная клавиша");
            return;
        };
        let mut syms = Vec::new();
        for (m, s) in [(Mods::CTRL, Keysym::Control_L), (Mods::ALT, Keysym::Alt_L), (Mods::SHIFT, Keysym::Shift_L), (Mods::SUPER, Keysym::Super_L)] {
            if combo.mods.contains(m) {
                syms.push(s);
            }
        }
        syms.push(sym);
        let keyboard = self.core.keyboard.clone();
        // keysym → keycode по текущей раскладке (первый уровень любой группы).
        let codes: Vec<Option<smithay::input::keyboard::Keycode>> = keyboard.with_xkb_state(self, |ctx| {
            let xkb = ctx.xkb().lock().unwrap();
            // SAFETY: keymap только читается, состояние xkb не меняется.
            let keymap = unsafe { xkb.keymap() };
            syms.iter()
                .map(|want| {
                    let mut found = None;
                    keymap.key_for_each(|_, kc| {
                        if found.is_some() {
                            return;
                        }
                        for layout in 0..keymap.num_layouts_for_key(kc) {
                            if keymap.key_get_syms_by_level(kc, layout, 0).contains(want) {
                                found = Some(kc);
                                return;
                            }
                        }
                    });
                    found
                })
                .collect()
        });
        if codes.iter().any(Option::is_none) {
            tracing::warn!(%combo, "key: клавиши нет в раскладке");
            return;
        }
        let codes: Vec<_> = codes.into_iter().flatten().collect();
        let time = self.core.start_time.elapsed().as_millis() as u32;
        for kc in &codes {
            keyboard.input::<(), _>(self, *kc, KeyState::Pressed, SERIAL_COUNTER.next_serial(), time, |_, _, _| FilterResult::Forward);
        }
        for kc in codes.iter().rev() {
            keyboard.input::<(), _>(self, *kc, KeyState::Released, SERIAL_COUNTER.next_serial(), time, |_, _, _| FilterResult::Forward);
        }
    }

    /// «Назад» (жест от края): открыт оверлей оболочки с клавиатурой —
    /// ему (`shell back`), иначе окну — клавиша `[mobile] back_key`.
    pub fn go_back(&mut self) {
        let on_layer = self
            .core
            .keyboard
            .current_focus()
            .is_some_and(|f| self.is_layer_surface(&f.0));
        if on_layer || self.core.wm.focused.is_none() {
            self.core.ipc.broadcast(&synshell_common::ipc::Event::ShellCommand { command: "back".into() });
            return;
        }
        let key = self.core.config.mobile.back_key.trim().to_string();
        if key == "close" {
            if let Some(id) = self.core.wm.focused {
                self.close_window(id);
            }
            return;
        }
        match key.parse::<synshell_common::action::KeyCombo>() {
            Ok(combo) => self.send_key_combo(&combo),
            Err(e) => tracing::warn!(key, e, "[mobile] back_key"),
        }
    }

    pub fn set_keyboard_layout(&mut self, index: u32) {
        let keyboard = self.core.keyboard.clone();
        keyboard.with_xkb_state(self, |mut ctx| {
            ctx.set_layout(smithay::input::keyboard::Layout(index));
        });
        self.broadcast_keyboard_layout();
    }

    pub fn next_keyboard_layout(&mut self) {
        let keyboard = self.core.keyboard.clone();
        keyboard.with_xkb_state(self, |mut ctx| ctx.cycle_next_layout());
        self.broadcast_keyboard_layout();
    }

    pub fn keyboard_layouts(&mut self) -> synshell_common::ipc::KeyboardLayouts {
        let short: Vec<String> = self
            .core
            .config
            .input
            .keyboard
            .layouts
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let keyboard = self.core.keyboard.clone();
        let (names, current) = keyboard.with_xkb_state(self, |ctx| {
            let xkb = ctx.xkb().lock().unwrap();
            let names: Vec<String> = xkb.layouts().map(|l| xkb.layout_name(l).to_string()).collect();
            (names, xkb.active_layout().0)
        });
        synshell_common::ipc::KeyboardLayouts { names, short, current }
    }

    pub fn broadcast_keyboard_layout(&mut self) {
        let keyboard = self.core.keyboard.clone();
        let current = keyboard.with_xkb_state(self, |ctx| ctx.xkb().lock().unwrap().active_layout().0);
        if self.core.last_kb_layout != Some(current) {
            let kb = self.keyboard_layouts();
            self.core.last_kb_layout = Some(kb.current);
            self.core.ipc.broadcast(&synshell_common::ipc::Event::KeyboardLayoutChanged { keyboard: kb });
        }
    }

    // ─── указатель ──────────────────────────────────────────────────────────

    /// Что под точкой (глобальные логические координаты).
    pub fn under(&self, pos: Point<f64, Logical>) -> Under {
        let Some(output) = self.core.output_at(pos) else { return Under::Nothing };
        let output_geo = self.core.space.output_geometry(&output).unwrap_or_default();
        let rel = pos - output_geo.loc.to_f64();

        // Экран блокировки — только он.
        if self.core.is_locked() {
            if let Some(ls) = self.core.lock_surfaces.get(&output) {
                return Under::Surface(FocusTarget(ls.wl_surface().clone()), output_geo.loc.to_f64());
            }
            return Under::Nothing;
        }

        let layers = layer_map_for_output(&output);
        // Все поверхности слоя сверху вниз, а не только верхняя по рамке:
        // у дока область ввода — полоса значков, над ней (запас под
        // увеличение) указатель должен доходить до поверхностей и окон ниже.
        let layer_hit = |layer: Layer| -> Option<Under> {
            layers.layers_on(layer).rev().find_map(|l| {
                let lg = layers.layer_geometry(l)?;
                let (s, loc) = l.surface_under(rel - lg.loc.to_f64(), WindowSurfaceType::ALL)?;
                Some(Under::Surface(FocusTarget(s), (loc + lg.loc + output_geo.loc).to_f64()))
            })
        };
        if let Some(u) = layer_hit(Layer::Overlay) {
            return u;
        }
        let fullscreen_on_top = self.fullscreen_on_output(&output).is_some();
        if !fullscreen_on_top {
            if let Some(u) = layer_hit(Layer::Top) {
                return u;
            }
        }

        // Окна сверху вниз.
        let title_h = self.core.deco_theme.height;
        let resize_border = self.core.config.decorations.resize_border.max(0) as f64;
        let layout = self.core.wm.workspace(self.core.wm.active).layout;
        for id in self.core.wm.visible_ids().into_iter().rev() {
            let Some(m) = self.core.wm.get(id) else { continue };
            let geo = m.geometry();
            let origin = m.loc - m.window.geometry().loc;
            if let Some((s, loc)) = m.window.surface_under(pos - origin.to_f64(), WindowSurfaceType::ALL) {
                return Under::Surface(FocusTarget(s), (loc + origin).to_f64());
            }
            let top = if m.has_titlebar() { title_h } else { 0 };
            let frame = Rectangle::new((geo.loc.x, geo.loc.y - top).into(), (geo.size.w, geo.size.h + top).into());
            if top > 0 && frame.to_f64().contains(pos) && pos.y < geo.loc.y as f64 {
                let local = Point::from((pos.x - frame.loc.x as f64, pos.y - frame.loc.y as f64));
                if let Some(hit) = self.core.deco_theme.hit(frame.size.w, local) {
                    // Кромка сверху заголовка тоже тянет размер.
                    let resizable = !m.maximized && !m.fullscreen && !m.is_tiled(layout);
                    if resizable && local.y < 4.0 && matches!(hit, DecoHit::Title) {
                        return Under::Resize(id, edge_for(frame, pos, 4.0) | ResizeEdge::TOP);
                    }
                    return Under::Deco(id, hit);
                }
            }
            let resizable = !m.maximized && !m.fullscreen;
            if resizable && resize_border > 0.0 {
                let outer = Rectangle::new(
                    (frame.loc.x as f64 - resize_border, frame.loc.y as f64 - resize_border).into(),
                    (frame.size.w as f64 + 2.0 * resize_border, frame.size.h as f64 + 2.0 * resize_border).into(),
                );
                if outer.contains(pos) && !frame.to_f64().contains(pos) {
                    let edges = edge_for(frame, pos, 0.0);
                    if !edges.is_empty() {
                        return Under::Resize(id, edges);
                    }
                }
            }
        }

        if let Some(u) = layer_hit(Layer::Bottom).or_else(|| layer_hit(Layer::Background)) {
            return u;
        }
        Under::Nothing
    }

    /// Окно во весь экран, лежащее сверху на выводе.
    pub fn fullscreen_on_output(&self, output: &Output) -> Option<WindowId> {
        let name = output.name();
        let top = self
            .core
            .wm
            .visible_ids()
            .into_iter()
            .rev()
            .find(|id| self.core.wm.get(*id).is_some_and(|m| m.output.as_deref() == Some(&name)))?;
        self.core.wm.get(top).filter(|m| m.fullscreen).map(|m| m.id)
    }

    fn clamp_to_outputs(&self, pos: Point<f64, Logical>) -> Point<f64, Logical> {
        if self.core.space.output_under(pos).next().is_some() {
            return pos;
        }
        let mut best: Option<(f64, Point<f64, Logical>)> = None;
        for o in self.core.space.outputs() {
            let g = self.core.space.output_geometry(o).unwrap_or_default().to_f64();
            let x = pos.x.clamp(g.loc.x, g.loc.x + g.size.w - 1.0);
            let y = pos.y.clamp(g.loc.y, g.loc.y + g.size.h - 1.0);
            let d = (x - pos.x).powi(2) + (y - pos.y).powi(2);
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, (x, y).into()));
            }
        }
        best.map(|(_, p)| p).unwrap_or(pos)
    }

    fn on_pointer_motion<B: InputBackend>(&mut self, event: B::PointerMotionEvent) {
        let serial = SERIAL_COUNTER.next_serial();
        let pointer = self.core.pointer.clone();
        let mut pos = pointer.current_location();
        let under = self.under(pos);

        // Ограничения указателя (игры, редакторы).
        let mut locked = false;
        let mut confine_region = None;
        let mut confined = false;
        if let Some((surface, surface_loc)) = under.focus() {
            with_pointer_constraint(&surface.0, &pointer, |c| match c {
                Some(c) if c.is_active() => {
                    if !c.region().is_none_or(|r| r.contains((pos - surface_loc).to_i32_round())) {
                        return;
                    }
                    match &*c {
                        PointerConstraint::Locked(_) => locked = true,
                        PointerConstraint::Confined(cf) => {
                            confined = true;
                            confine_region = cf.region().cloned();
                        }
                    }
                }
                _ => {}
            });
        }

        pointer.relative_motion(
            self,
            under.focus(),
            &RelativeMotionEvent { delta: event.delta(), delta_unaccel: event.delta_unaccel(), utime: event.time() },
        );
        if locked {
            pointer.frame(self);
            return;
        }

        pos += event.delta();
        pos = self.clamp_to_outputs(pos);
        let new_under = self.under(pos);

        if confined {
            if let Some((surface, surface_loc)) = under.focus() {
                if new_under.focus().map(|(s, _)| s) != Some(surface.clone()) {
                    pointer.frame(self);
                    return;
                }
                if let Some(region) = confine_region {
                    if !region.contains((pos - surface_loc).to_i32_round()) {
                        pointer.frame(self);
                        return;
                    }
                }
            }
        }

        self.pointer_moved(pos, new_under, serial, event.time_msec());

        // Включить ограничение, если указатель вошёл в его область.
        if let Some((s, loc)) = self.under(pos).focus() {
            with_pointer_constraint(&s.0, &pointer, |c| {
                if let Some(c) = c {
                    if !c.is_active() && c.region().is_none_or(|r| r.contains((pos - loc).to_i32_round())) {
                        c.activate();
                    }
                }
            });
        }
    }

    fn on_pointer_motion_absolute<B: InputBackend>(&mut self, event: B::PointerMotionAbsoluteEvent) {
        // Абсолютные события — у вложенного окна и планшетов: в пределах
        // первого вывода (у вложенного режима он один).
        let Some(output) = self.core.space.outputs().next().cloned() else { return };
        let geo = self.core.space.output_geometry(&output).unwrap_or_default();
        let pos = event.position_transformed(geo.size) + geo.loc.to_f64();
        let serial = SERIAL_COUNTER.next_serial();
        let under = self.under(pos);
        self.pointer_moved(pos, under, serial, event.time_msec());
    }

    /// Общая часть движения указателя: рамки, фокус за мышью, обзор.
    fn pointer_moved(&mut self, pos: Point<f64, Logical>, under: Under, serial: smithay::utils::Serial, time: u32) {
        self.core.last_pointer_motion = Instant::now();
        if self.core.cursor_hidden {
            self.core.cursor_hidden = false;
        }
        let pointer = self.core.pointer.clone();

        if self.core.wm.overview.is_some() {
            pointer.motion(self, None, &MotionEvent { location: pos, serial, time });
            pointer.frame(self);
            self.overview_hover(pos);
            self.core.queue_redraw_all();
            return;
        }

        // Наведение на кнопки заголовка.
        let hover = match &under {
            Under::Deco(id, DecoHit::Button(b)) => Some((*id, *b)),
            _ => None,
        };
        let mut changed = false;
        for m in &mut self.core.wm.windows {
            let new = hover.filter(|(id, _)| *id == m.id).map(|(_, b)| b);
            if m.hover != new {
                m.hover = new;
                changed = true;
            }
        }
        if changed {
            self.core.queue_redraw_all();
        }

        // Курсор над рамкой — наш.
        if !pointer.is_grabbed() {
            match &under {
                Under::Resize(_, edges) => self.core.cursor_status = CursorImageStatus::Named(edges.cursor()),
                Under::Deco(..) | Under::Nothing => {
                    self.core.cursor_status = CursorImageStatus::Named(CursorIcon::Default)
                }
                Under::Surface(..) => {}
            }
        }

        pointer.motion(self, under.focus(), &MotionEvent { location: pos, serial, time });
        pointer.frame(self);

        // Фокус за мышью.
        if self.core.config.windows.focus_mode != FocusMode::Click && !pointer.is_grabbed() {
            let target = match &under {
                Under::Surface(s, _) => self.window_for_surface_tree(&s.0),
                Under::Deco(id, _) | Under::Resize(id, _) => Some(*id),
                Under::Nothing => None,
            };
            match (target, self.core.config.windows.focus_mode) {
                (Some(id), _) if Some(id) != self.core.wm.focused => {
                    let raise = self.core.config.windows.raise_on_focus;
                    // Фокус без поднятия; поднятие — с задержкой autoraise.
                    self.core.config.windows.raise_on_focus = false;
                    self.focus_window(Some(id));
                    self.core.config.windows.raise_on_focus = raise;
                    if raise {
                        self.schedule_autoraise(id);
                    }
                }
                (None, FocusMode::FollowMouse) if matches!(under, Under::Nothing) => {
                    if self.core.wm.focused.is_some() {
                        self.focus_window(None);
                    }
                }
                _ => {}
            }
        }
        // Курсор рисуется композитором — нужен кадр.
        self.core.queue_redraw_all();
    }

    fn schedule_autoraise(&mut self, id: WindowId) {
        let delay = Duration::from_millis(self.core.config.windows.autoraise_delay as u64);
        let _ = self.core.loop_handle.insert_source(
            smithay::reexports::calloop::timer::Timer::from_duration(delay),
            move |_, _, state| {
                if state.core.wm.focused == Some(id) {
                    state.core.wm.raise(id);
                    state.sync_space();
                    state.core.queue_redraw_all();
                }
                smithay::reexports::calloop::timer::TimeoutAction::Drop
            },
        );
    }

    /// Окно, которому принадлежит поверхность (включая подповерхности и меню).
    pub fn window_for_surface_tree(&self, surface: &WlSurface) -> Option<WindowId> {
        let mut root = surface.clone();
        while let Some(p) = smithay::wayland::compositor::get_parent(&root) {
            root = p;
        }
        if let Some(id) = self.core.wm.id_by_surface(&root) {
            return Some(id);
        }
        // Всплывающее меню — к окну-владельцу.
        if let Some(popup) = self.core.popups.find_popup(&root) {
            if let Ok(r) = smithay::desktop::find_popup_root_surface(&popup) {
                return self.core.wm.id_by_surface(&r);
            }
        }
        None
    }

    pub fn warp_pointer(&mut self, pos: Point<f64, Logical>) {
        let under = self.under(pos);
        let pointer = self.core.pointer.clone();
        pointer.motion(
            self,
            under.focus(),
            &MotionEvent { location: pos, serial: SERIAL_COUNTER.next_serial(), time: self.core.clock.now().as_millis() },
        );
        pointer.frame(self);
        self.core.queue_redraw_all();
    }

    fn on_pointer_button<B: InputBackend>(&mut self, event: B::PointerButtonEvent) {
        let serial = SERIAL_COUNTER.next_serial();
        let button = event.button_code();
        let state = event.state();
        let pointer = self.core.pointer.clone();
        let pos = pointer.current_location();
        let mods = mods_from_state(&self.core.keyboard.modifier_state());

        if self.core.wm.overview.is_some() && !self.core.is_locked() {
            if state == ButtonState::Pressed {
                self.overview_click(pos, button);
            }
            return;
        }

        if state == ButtonState::Pressed && !pointer.is_grabbed() && !self.core.is_locked() {
            let under = self.under(pos);
            // Модификатор + мышь: перемещение/размер из любой точки окна.
            if !self.core.bindings.mod_key.is_empty() && mods.contains(self.core.bindings.mod_key) {
                let id = match &under {
                    Under::Surface(s, _) => self.window_for_surface_tree(&s.0),
                    Under::Deco(id, _) | Under::Resize(id, _) => Some(*id),
                    Under::Nothing => None,
                };
                if let Some(id) = id {
                    self.focus_window(Some(id));
                    let start = GrabStartData { focus: None, button, location: pos };
                    match button {
                        BTN_LEFT => self.start_move(id, start, serial),
                        BTN_RIGHT => {
                            let g = self.core.wm.get(id).map(|m| m.geometry()).unwrap_or_default();
                            let edges = quadrant_edges(g, pos);
                            self.start_resize(id, edges, start, serial);
                        }
                        BTN_MIDDLE => self.toggle_maximize(id),
                        _ => {}
                    }
                    // Нажатие не уходит клиенту; отпускание обработает захват.
                    pointer.button(self, &ButtonEvent { button, state: state.into(), serial, time: event.time_msec() });
                    pointer.frame(self);
                    return;
                }
            }
            match under {
                Under::Deco(id, hit) => {
                    self.deco_press(id, hit, button, pos, serial, event.time_msec());
                    pointer.button(self, &ButtonEvent { button, state: state.into(), serial, time: event.time_msec() });
                    pointer.frame(self);
                    return;
                }
                Under::Resize(id, edges) => {
                    self.focus_window(Some(id));
                    pointer.button(self, &ButtonEvent { button, state: state.into(), serial, time: event.time_msec() });
                    if button == BTN_LEFT {
                        let start = GrabStartData { focus: None, button, location: pos };
                        self.start_resize(id, edges, start, serial);
                    }
                    pointer.frame(self);
                    return;
                }
                Under::Surface(ref s, _) => {
                    self.click_focus(&s.0);
                }
                Under::Nothing => {
                    // Клик по пустому столу/обоям: фокус остаётся, если это
                    // не слой, который хочет клавиатуру.
                }
            }
        }

        if state == ButtonState::Released {
            // Отпускание на кнопке заголовка — действие.
            if let Some((id, b)) = self.core.wm.windows.iter().find_map(|m| m.pressed.map(|b| (m.id, b))) {
                if let Some(m) = self.core.wm.get_mut(id) {
                    m.pressed = None;
                }
                let still_over = matches!(self.under(pos), Under::Deco(i, DecoHit::Button(bb)) if i == id && bb == b);
                if still_over {
                    self.deco_button_action(id, b, button);
                }
                self.core.queue_redraw_all();
            }
        }

        pointer.button(self, &ButtonEvent { button, state: state.into(), serial, time: event.time_msec() });
        pointer.frame(self);
    }

    /// Клик по поверхности: фокус окну или слою.
    pub(crate) fn click_focus(&mut self, surface: &WlSurface) {
        if let Some(id) = self.window_for_surface_tree(surface) {
            if self.core.wm.focused != Some(id) || self.core.keyboard.current_focus().map(|f| f.0) != self.core.wm.get(id).and_then(|m| m.surface()) {
                self.focus_window(Some(id));
            } else if self.core.config.windows.raise_on_focus {
                self.core.wm.raise(id);
                self.sync_space();
            }
            return;
        }
        // Слой: фокус, если хочет клавиатуру (OnDemand/Exclusive).
        let mut root = surface.clone();
        while let Some(p) = smithay::wayland::compositor::get_parent(&root) {
            root = p;
        }
        if self.is_layer_surface(&root) {
            let wants = with_states(&root, |s| {
                s.cached_state.get::<LayerSurfaceCachedState>().current().keyboard_interactivity
                    != KeyboardInteractivity::None
            });
            if wants {
                self.focus_surface(Some(root));
            }
        }
    }

    fn deco_press(&mut self, id: WindowId, hit: DecoHit, button: u32, pos: Point<f64, Logical>, serial: smithay::utils::Serial, _time: u32) {
        match hit {
            DecoHit::Button(b) => {
                if button == BTN_LEFT {
                    if let Some(m) = self.core.wm.get_mut(id) {
                        m.pressed = Some(b);
                    }
                    self.core.queue_redraw_all();
                } else if button == BTN_RIGHT && b == Button::Icon {
                    self.window_menu(id, pos);
                }
            }
            DecoHit::Title => match button {
                BTN_LEFT => {
                    self.focus_window(Some(id));
                    let double = self
                        .core
                        .last_title_click
                        .as_ref()
                        .is_some_and(|c| c.id == id && c.at.elapsed() < Duration::from_millis(400));
                    if double {
                        self.core.last_title_click = None;
                        let a = self.core.config.windows.titlebar_double_click.clone();
                        self.do_window_action(id, a);
                    } else {
                        self.core.last_title_click = Some(LastTitleClick { id, at: Instant::now() });
                        let start = GrabStartData { focus: None, button, location: pos };
                        self.start_move(id, start, serial);
                    }
                }
                BTN_MIDDLE => {
                    let a = self.core.config.windows.titlebar_middle_click.clone();
                    self.do_window_action(id, a);
                }
                BTN_RIGHT => {
                    self.focus_window(Some(id));
                    self.window_menu(id, pos);
                }
                _ => {}
            },
        }
    }

    pub(crate) fn deco_button_action(&mut self, id: WindowId, b: Button, button: u32) {
        match (b, button) {
            (Button::Close, BTN_LEFT) => self.close_window(id),
            (Button::Maximize, BTN_LEFT) => self.toggle_maximize(id),
            (Button::Maximize, BTN_MIDDLE) => self.set_snap(id, Some(crate::wm::layout::SnapZone::Top)),
            (Button::Minimize, BTN_LEFT) => self.minimize(id),
            (Button::Sticky, BTN_LEFT) => self.toggle_sticky(id),
            (Button::Above, BTN_LEFT) => self.toggle_above(id),
            (Button::Icon, BTN_LEFT) => {
                let pos = self.core.pointer.current_location();
                self.window_menu(id, pos);
            }
            _ => {}
        }
    }

    /// Меню окна показывает оболочка.
    fn window_menu(&mut self, id: WindowId, pos: Point<f64, Logical>) {
        self.core.ipc.broadcast(&synshell_common::ipc::Event::ShellCommand {
            command: format!("window-menu {id} {} {}", pos.x as i32, pos.y as i32),
        });
    }

    fn on_pointer_axis<B: InputBackend>(&mut self, event: B::PointerAxisEvent) {
        let source = event.source();
        let factor = if source == AxisSource::Finger {
            self.core.config.input.touchpad.scroll_factor
        } else {
            self.core.config.input.mouse.scroll_factor
        };
        let h = event.amount(Axis::Horizontal).unwrap_or_else(|| event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.0) * factor;
        let v = event.amount(Axis::Vertical).unwrap_or_else(|| event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.0) * factor;
        let h_discrete = event.amount_v120(Axis::Horizontal);
        let v_discrete = event.amount_v120(Axis::Vertical);

        // Сочетания с колесом (Super+колесо — столы).
        let mods = mods_from_state(&self.core.keyboard.modifier_state());
        if !mods.is_empty() && !self.core.is_locked() && source == AxisSource::Wheel {
            let trig = if v < 0.0 {
                Some(Trigger::WheelUp)
            } else if v > 0.0 {
                Some(Trigger::WheelDown)
            } else if h < 0.0 {
                Some(Trigger::WheelLeft)
            } else if h > 0.0 {
                Some(Trigger::WheelRight)
            } else {
                None
            };
            if let Some(t) = trig {
                if let Some(a) = self.core.bindings.find(mods, t).map(|b| b.action.clone()) {
                    self.do_action(a);
                    return;
                }
            }
        }
        // Колесо над заголовком: прозрачность окна.
        if self.core.config.windows.titlebar_wheel == "opacity" && source == AxisSource::Wheel {
            if let Under::Deco(id, DecoHit::Title) = self.under(self.core.pointer.current_location()) {
                if let Some(m) = self.core.wm.get_mut(id) {
                    m.opacity = (m.opacity - v.signum() as f32 * 0.05).clamp(0.1, 1.0);
                }
                self.core.queue_redraw_all();
                return;
            }
        }

        let mut frame = AxisFrame::new(event.time_msec()).source(source);
        if h != 0.0 {
            frame = frame.relative_direction(Axis::Horizontal, event.relative_direction(Axis::Horizontal));
            frame = frame.value(Axis::Horizontal, h);
            if let Some(d) = h_discrete {
                frame = frame.v120(Axis::Horizontal, (d * factor) as i32);
            }
        }
        if v != 0.0 {
            frame = frame.relative_direction(Axis::Vertical, event.relative_direction(Axis::Vertical));
            frame = frame.value(Axis::Vertical, v);
            if let Some(d) = v_discrete {
                frame = frame.v120(Axis::Vertical, (d * factor) as i32);
            }
        }
        if source == AxisSource::Finger {
            if event.amount(Axis::Horizontal) == Some(0.0) {
                frame = frame.stop(Axis::Horizontal);
            }
            if event.amount(Axis::Vertical) == Some(0.0) {
                frame = frame.stop(Axis::Vertical);
            }
        }
        let pointer = self.core.pointer.clone();
        pointer.axis(self, frame);
        pointer.frame(self);
    }

    /// Три пальца: влево/вправо — столы, вверх — обзор, вниз — закрыть обзор.
    fn finish_swipe(&mut self, g: GestureState) {
        let threshold = 60.0;
        if g.dx.abs() > g.dy.abs() && g.dx.abs() > threshold {
            let t = if g.dx < 0.0 { WorkspaceTarget::Next } else { WorkspaceTarget::Prev };
            self.workspace_action(t);
        } else if g.dy.abs() > threshold {
            if g.dy < 0.0 {
                if self.core.wm.overview.is_none() {
                    self.toggle_overview();
                }
            } else if self.core.wm.overview.is_some() {
                self.toggle_overview();
            }
        }
    }

    // ─── сенсорный экран ────────────────────────────────────────────────────

    fn touch_location<B: InputBackend, E: AbsolutePositionEvent<B>>(&self, evt: &E) -> Point<f64, Logical> {
        let output = self.core.space.outputs().next().cloned();
        let geo = output.and_then(|o| self.core.space.output_geometry(&o)).unwrap_or_default();
        evt.position_transformed(geo.size) + geo.loc.to_f64()
    }

    fn on_touch_down<B: InputBackend>(&mut self, evt: B::TouchDownEvent) {
        let Some(touch) = self.core.seat.get_touch() else { return };
        // Пальцем курсор не нужен: прячется до следующего движения мыши.
        if self.core.config.appearance.cursor_hide == "touch" && !self.core.cursor_hidden {
            self.core.cursor_hidden = true;
            self.core.queue_redraw_all();
        }
        let pos = self.touch_location(&evt);
        if self.gesture_touch_down(evt.slot(), pos, evt.time_msec()) {
            return;
        }
        if self.finger_down(evt.slot(), pos) {
            return;
        }
        let under = self.under(pos);
        if let Under::Surface(s, _) = &under {
            self.click_focus(&s.0.clone());
        }
        touch.down(
            self,
            under.focus(),
            &DownEvent { slot: evt.slot(), location: pos, serial: SERIAL_COUNTER.next_serial(), time: evt.time_msec() },
        );
    }

    fn on_touch_up<B: InputBackend>(&mut self, evt: B::TouchUpEvent) {
        if self.gesture_touch_up(evt.slot(), evt.time_msec()) {
            return;
        }
        if self.finger_up(evt.slot()) {
            return;
        }
        let Some(touch) = self.core.seat.get_touch() else { return };
        touch.up(self, &UpEvent { slot: evt.slot(), serial: SERIAL_COUNTER.next_serial(), time: evt.time_msec() });
    }

    fn on_touch_motion<B: InputBackend>(&mut self, evt: B::TouchMotionEvent) {
        let Some(touch) = self.core.seat.get_touch() else { return };
        let pos = self.touch_location(&evt);
        if self.gesture_touch_motion(evt.slot(), pos, evt.time_msec()) {
            return;
        }
        if self.finger_motion(evt.slot(), pos) {
            return;
        }
        let under = self.under(pos);
        touch.motion(
            self,
            under.focus(),
            &smithay::input::touch::MotionEvent { slot: evt.slot(), location: pos, time: evt.time_msec() },
        );
    }

    // ─── действия ───────────────────────────────────────────────────────────

    /// Действие над конкретным окном (из заголовка).
    pub fn do_window_action(&mut self, id: WindowId, action: Action) {
        match action {
            Action::ToggleMaximize => self.toggle_maximize(id),
            Action::Minimize => self.minimize(id),
            Action::Close => self.close_window(id),
            Action::ToggleFullscreen => self.toggle_fullscreen(id),
            Action::ToggleSticky => self.toggle_sticky(id),
            Action::ToggleAlwaysOnTop => self.toggle_above(id),
            Action::ToggleFloating => self.toggle_floating(id),
            Action::None => {}
            other => {
                self.focus_window(Some(id));
                self.do_action(other);
            }
        }
    }

    /// Выполнить действие (сочетание клавиш, IPC, кнопка).
    pub fn do_action(&mut self, action: Action) {
        tracing::debug!(%action, "действие");
        let action_name = action.to_string();
        let focused = self.core.wm.focused;
        match action {
            Action::None => {}
            Action::Spawn(cmd) => crate::spawn::spawn_shell(&self.core, &cmd),
            Action::Close => {
                if let Some(id) = focused {
                    self.close_window(id)
                }
            }
            Action::Kill => {
                if let Some(id) = focused {
                    self.kill_window(id)
                }
            }
            Action::ToggleFloating => {
                if let Some(id) = focused {
                    self.toggle_floating(id)
                }
            }
            Action::ToggleFullscreen => {
                if let Some(id) = focused {
                    self.toggle_fullscreen(id)
                }
            }
            Action::ToggleMaximize => {
                if let Some(id) = focused {
                    self.toggle_maximize(id)
                }
            }
            Action::Minimize => {
                if let Some(id) = focused {
                    self.minimize(id)
                }
            }
            Action::MinimizeAll => {
                for id in self.core.wm.visible_ids() {
                    self.minimize(id);
                }
                // Телефон, режим страниц: на домашнюю страницу.
                if self.core.wm.mobile.pages_mode() {
                    self.go_to_page(None, true);
                }
            }
            Action::ToggleSticky => {
                if let Some(id) = focused {
                    self.toggle_sticky(id)
                }
            }
            Action::ToggleAlwaysOnTop => {
                if let Some(id) = focused {
                    self.toggle_above(id)
                }
            }
            Action::Snap(d) => {
                if let Some(id) = focused {
                    self.snap(id, d)
                }
            }
            Action::Center => {
                if let Some(id) = focused {
                    self.center_window(id)
                }
            }
            Action::Focus(d) => self.focus_dir(d),
            Action::Move(d) => self.move_dir(d),
            Action::FocusNext => self.cycle_windows(true),
            Action::FocusPrev => self.cycle_windows(false),
            Action::Workspace(t) => self.workspace_action(t),
            Action::MoveToWorkspace(t) => {
                let ws = self.resolve_workspace(t);
                if let Some(id) = focused {
                    self.move_to_workspace(id, ws, false)
                }
            }
            Action::MoveToWorkspaceFollow(t) => {
                let ws = self.resolve_workspace(t);
                if let Some(id) = focused {
                    self.move_to_workspace(id, ws, true)
                }
            }
            Action::FocusOutput(d) => self.focus_output(d),
            Action::MoveToOutput(d) => self.move_to_output(d),
            Action::Layout(k) => self.set_layout(k),
            Action::CycleLayout => {
                let cur = self.core.wm.workspace(self.core.wm.active).layout;
                self.set_layout(cur.next());
                self.core.ipc.broadcast(&synshell_common::ipc::Event::ShellCommand {
                    command: format!("osd-layout {}", cur.next().as_str()),
                });
            }
            Action::MasterRatio(r) => self.adjust_master(r, 0),
            Action::MasterCount(c) => self.adjust_master(0.0, c),
            Action::KeyboardLayoutNext => self.next_keyboard_layout(),
            Action::KeyboardLayout(i) => self.set_keyboard_layout(i),
            Action::Screenshot => self.screenshot(false),
            Action::ScreenshotWindow => self.screenshot(true),
            Action::ScreenshotInteractive => self.screenshot_interactive(),
            Action::Overview => self.toggle_overview(),
            Action::ReloadConfig => self.reload_config(),
            Action::RestartShell => self.restart_shell(),
            Action::Restart => self.restart(),
            Action::Quit => self.quit(),
            Action::Lock => self.lock_screen(),
            Action::Suspend => {
                if self.core.config.lock.before_sleep {
                    self.lock_screen();
                }
                crate::spawn::spawn_shell(&self.core, "systemctl suspend");
            }
            // Сеанс экрана входа synlogin: пользователь без logind-сеанса,
            // polkit не пустит — просьба демону (root) и выход из сеанса.
            Action::Reboot | Action::PowerOff if std::env::var_os("SYNLOGIN_REQUEST").is_some() => {
                let path = std::env::var_os("SYNLOGIN_REQUEST").unwrap();
                let req = if matches!(action, Action::Reboot) { "!reboot" } else { "!poweroff" };
                match std::fs::write(&path, req) {
                    Ok(()) => self.quit(),
                    Err(e) => tracing::warn!(?path, "synlogin: не записать просьбу: {e}"),
                }
            }
            Action::Reboot => crate::spawn::spawn_shell(&self.core, "systemctl reboot"),
            Action::PowerOff => crate::spawn::spawn_shell(&self.core, "systemctl poweroff"),
            Action::PowerOffMonitors => self.set_monitors_power(false),
            Action::Shell(cmd) => {
                self.core.ipc.broadcast(&synshell_common::ipc::Event::ShellCommand { command: cmd });
            }
            Action::Back => self.go_back(),
            Action::Key(combo) => self.send_key_combo(&combo),
            // Режимы окон телефона и страницы — этап режимов окон; пока
            // оболочке сообщается команда, чтобы она могла ответить.
            Action::MobileMode(m) => self.set_mobile_mode(m, true),
            Action::MobileModeCycle => {
                let next = self.core.wm.mobile.mode.next();
                self.set_mobile_mode(next, true);
            }
            Action::Page(t) => self.page_action(t),
            Action::CameraHome => {
                self.core.wm.mobile.camera = smithay::utils::Point::from((0.0, 0.0));
                let _ = &action_name;
                self.relayout();
                self.broadcast_mobile();
            }
        }
    }

    pub fn lock_screen(&mut self) {
        if self.core.is_locked() {
            return;
        }
        let cmd = self.core.config.lock.command.trim().to_string();
        if cmd.is_empty() {
            // Встроенная блокировка — у оболочки (ext-session-lock).
            if self.core.ipc.subscribers() > 0 {
                self.core.ipc.broadcast(&synshell_common::ipc::Event::ShellCommand { command: "lock".into() });
                return;
            }
            for fallback in ["swaylock -f", "hyprlock"] {
                let bin = fallback.split_whitespace().next().unwrap();
                if crate::spawn::which(bin) {
                    crate::spawn::spawn_shell(&self.core, fallback);
                    return;
                }
            }
            tracing::warn!("нечем заблокировать экран: нет оболочки и swaylock");
        } else {
            crate::spawn::spawn_shell(&self.core, &cmd);
        }
    }

    /// Перезапуск композитора: цикл останавливается, после освобождения
    /// устройств `main` заменяет процесс новым бинарником (тот же PID —
    /// сеанс logind/seatd остаётся за нами).
    pub fn restart(&mut self) {
        tracing::info!("перезапуск композитора");
        self.core.restart_requested = true;
        self.quit();
    }

    pub fn quit(&mut self) {
        tracing::info!("выход из сеанса");
        self.core.ipc.broadcast(&synshell_common::ipc::Event::Exiting);
        self.core.ipc.flush_all();
        self.core.shell.stop();
        self.core.loop_signal.stop();
    }

    // ─── обзор окон ─────────────────────────────────────────────────────────

    pub fn toggle_overview(&mut self) {
        use crate::anim::{Animation, Curve};
        let dur = crate::anim::duration(&self.core.config.animations, 250).max(Duration::from_millis(1));
        if let Some(ov) = &mut self.core.wm.overview {
            if !ov.closing {
                let v = ov.anim.value();
                ov.anim = Animation::new(v, 0.0, dur, Curve::EaseOutCubic);
                ov.closing = true;
                if let Some(id) = ov.hovered.or_else(|| ov.slots.get(ov.selected).map(|s| s.0)) {
                    self.focus_window(Some(id));
                }
            }
            self.core.queue_redraw_all();
            return;
        }
        let slots = self.overview_slots();
        let selected = slots.iter().position(|(id, _)| Some(*id) == self.core.wm.focused).unwrap_or(0);
        self.core.wm.overview = Some(crate::wm::Overview {
            anim: Animation::new(0.0, 1.0, dur, Curve::EaseOutCubic),
            closing: false,
            slots,
            hovered: None,
            selected,
        });
        self.core.queue_redraw_all();
    }

    /// Разложить видимые окна сеткой на их выводах.
    fn overview_slots(&self) -> Vec<(WindowId, Rectangle<i32, Logical>)> {
        let mut out = Vec::new();
        let title_h = self.core.deco_theme.height;
        for output in self.core.space.outputs() {
            let name = output.name();
            let ids: Vec<WindowId> = self
                .core
                .wm
                .visible_ids()
                .into_iter()
                .filter(|id| self.core.wm.get(*id).is_some_and(|m| m.output.as_deref() == Some(&name)))
                .collect();
            if ids.is_empty() {
                continue;
            }
            let area = self.core.work_area(output);
            let n = ids.len();
            let cols = (n as f64).sqrt().ceil() as usize;
            let rows = n.div_ceil(cols);
            let pad = 36;
            let cell_w = (area.size.w - pad * (cols as i32 + 1)) / cols as i32;
            let cell_h = (area.size.h - pad * (rows as i32 + 1)) / rows as i32;
            for (i, id) in ids.iter().enumerate() {
                let m = self.core.wm.get(*id).unwrap();
                let top = if m.has_titlebar() { title_h } else { 0 };
                let size = m.size();
                let (fw, fh) = (size.w as f64, (size.h + top) as f64);
                let scale = (cell_w as f64 / fw).min(cell_h as f64 / fh).min(1.0);
                let (w, h) = ((fw * scale) as i32, (fh * scale) as i32);
                let (c, r) = ((i % cols) as i32, (i / cols) as i32);
                let cx = area.loc.x + pad + c * (cell_w + pad) + (cell_w - w) / 2;
                let cy = area.loc.y + pad + r * (cell_h + pad) + (cell_h - h) / 2;
                out.push((*id, Rectangle::new((cx, cy).into(), (w.max(1), h.max(1)).into())));
            }
        }
        out
    }

    fn overview_hover(&mut self, pos: Point<f64, Logical>) {
        if let Some(ov) = &mut self.core.wm.overview {
            ov.hovered = ov.slots.iter().find(|(_, r)| r.to_f64().contains(pos)).map(|(id, _)| *id);
            if let Some(h) = ov.hovered {
                if let Some(i) = ov.slots.iter().position(|(id, _)| *id == h) {
                    ov.selected = i;
                }
            }
        }
    }

    fn overview_click(&mut self, pos: Point<f64, Logical>, button: u32) {
        self.overview_hover(pos);
        let hovered = self.core.wm.overview.as_ref().and_then(|o| o.hovered);
        match (hovered, button) {
            (Some(id), BTN_MIDDLE) => {
                self.close_window(id);
            }
            (Some(_), _) | (None, _) => {
                if hovered.is_none() {
                    if let Some(ov) = &mut self.core.wm.overview {
                        ov.selected = usize::MAX;
                    }
                }
                self.toggle_overview();
            }
        }
    }

    fn overview_key(&mut self, sym: Keysym) {
        let Some(ov) = &mut self.core.wm.overview else { return };
        let n = ov.slots.len();
        match sym {
            Keysym::Escape => {
                ov.hovered = None;
                ov.selected = usize::MAX;
                let focused = self.core.wm.focused;
                self.toggle_overview();
                self.focus_window(focused);
            }
            Keysym::Return | Keysym::KP_Enter | Keysym::space => self.toggle_overview(),
            Keysym::Right | Keysym::Tab | Keysym::Down if n > 0 => {
                ov.hovered = None;
                ov.selected = (ov.selected.min(n - 1) + 1) % n;
                self.core.queue_redraw_all();
            }
            Keysym::Left | Keysym::ISO_Left_Tab | Keysym::Up if n > 0 => {
                ov.hovered = None;
                ov.selected = (ov.selected.min(n - 1) + n - 1) % n;
                self.core.queue_redraw_all();
            }
            _ => {}
        }
    }
}

enum KeyIntent {
    None,
    Action(Action),
    Vt(i32),
    Overview(Keysym),
    ModifierReleased,
    ModifierReleasedForward,
}

/// Край(я) рамки у точки `p` вне/на границе прямоугольника.
fn edge_for(frame: Rectangle<i32, Logical>, p: Point<f64, Logical>, inset: f64) -> ResizeEdge {
    let mut e = ResizeEdge::empty();
    let corner = 16.0;
    let (l, t) = (frame.loc.x as f64, frame.loc.y as f64);
    let (r, b) = (l + frame.size.w as f64, t + frame.size.h as f64);
    if p.x < l + inset || (p.x < l + corner && (p.y < t + inset || p.y >= b - inset)) {
        e |= ResizeEdge::LEFT;
    }
    if p.x >= r - inset || (p.x >= r - corner && (p.y < t + inset || p.y >= b - inset)) {
        e |= ResizeEdge::RIGHT;
    }
    if p.y < t + inset || (p.y < t + corner && (p.x < l + inset || p.x >= r - inset)) {
        e |= ResizeEdge::TOP;
    }
    if p.y >= b - inset || (p.y >= b - corner && (p.x < l + inset || p.x >= r - inset)) {
        e |= ResizeEdge::BOTTOM;
    }
    e
}

/// Для Super+ПКМ: ближайший угол окна.
fn quadrant_edges(g: Rectangle<i32, Logical>, p: Point<f64, Logical>) -> ResizeEdge {
    let cx = g.loc.x as f64 + g.size.w as f64 / 2.0;
    let cy = g.loc.y as f64 + g.size.h as f64 / 2.0;
    let mut e = ResizeEdge::empty();
    e |= if p.x < cx { ResizeEdge::LEFT } else { ResizeEdge::RIGHT };
    e |= if p.y < cy { ResizeEdge::TOP } else { ResizeEdge::BOTTOM };
    e
}

#[allow(dead_code)]
fn dir_name(d: Direction) -> &'static str {
    d.as_str()
}

#[allow(dead_code)]
fn mods_name(m: Mods) -> String {
    format!("{m:?}")
}
