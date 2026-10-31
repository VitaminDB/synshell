//! Перо (libinput tablet tool): клиентам с tablet-v2 — события инструмента
//! (давление, наклон, кнопки), остальным (syngui, Xwayland без планшета) —
//! эмуляция указателя: кончик — левая кнопка, нижняя кнопка пера — правая,
//! наведение — движение без курсора (на E Ink лишний кадр — обновление панели).

use std::time::{Duration, Instant};

use smithay::{
    backend::input::{
        ButtonState, Device, DeviceCapability, Event, InputBackend, ProximityState,
        TabletToolButtonEvent, TabletToolEvent, TabletToolProximityEvent, TabletToolTipEvent, TabletToolTipState,
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, SERIAL_COUNTER},
    wayland::tablet_manager::{TabletDescriptor, TabletSeatTrait},
};

use crate::{input::Under, state::State};

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;
/// Кнопки пера (evdev): нижняя и верхняя.
const BTN_STYLUS: u32 = 0x14b;
const BTN_STYLUS2: u32 = 0x14c;

/// Куда идёт ввод пера, пока кончик прижат или кнопка нажата.
#[derive(Clone, Debug)]
enum Route {
    /// Клиент с tablet-v2: поверхность и её начало.
    Native(WlSurface, Point<f64, Logical>),
    /// Эмуляция указателя.
    Pointer,
}

#[derive(Default)]
pub struct PenState {
    /// Перо у экрана (proximity in).
    pub near: bool,
    /// Маршрут, закреплённый нажатием (кончик/кнопка), — до отпускания всего.
    grab: Option<Route>,
    /// Прижато: кончик + кнопки пера.
    held: u32,
    /// Кнопки указателя, нажатые эмуляцией.
    emu_buttons: Vec<u32>,
    /// Последнее движение уходило клиенту с tablet-v2 (для журнала смены маршрута).
    native: Option<bool>,
    last_log: Option<Instant>,
}

impl State {
    /// Планшет (перо) подключён: объявить его клиентам tablet-v2.
    pub(crate) fn tablet_added<B: InputBackend>(&mut self, device: &B::Device) {
        if !device.has_capability(DeviceCapability::TabletTool) {
            return;
        }
        tracing::info!(name = %device.name(), "перо: планшет добавлен");
        let dh = self.core.display_handle.clone();
        self.core.seat.tablet_seat().add_tablet::<State>(&dh, &TabletDescriptor::from(device));
    }

    pub(crate) fn tablet_removed<B: InputBackend>(&mut self, device: &B::Device) {
        if !device.has_capability(DeviceCapability::TabletTool) {
            return;
        }
        tracing::info!(name = %device.name(), "перо: планшет убран");
        let time = self.core.clock.now().as_millis();
        self.pen_release_all(time);
        let ts = self.core.seat.tablet_seat();
        ts.remove_tablet(&TabletDescriptor::from(device));
        if ts.count_tablets() == 0 {
            ts.clear_tools();
        }
    }

    /// Маршрут для точки: закреплённый нажатием или по поверхности под пером.
    fn pen_route(&self, pos: Point<f64, Logical>, tool: &smithay::wayland::tablet_manager::TabletToolHandle) -> Route {
        if let Some(r) = &self.core.pen.grab {
            return r.clone();
        }
        if self.core.wm.overview.is_some() {
            return Route::Pointer;
        }
        match self.under(pos) {
            Under::Surface(s, loc) if tool.has_instance_for(&s.0) => Route::Native(s.0, loc),
            _ => Route::Pointer,
        }
    }

    /// Перо сдвинулось (наведение или с прижатым кончиком).
    fn pen_motion<B: InputBackend, E: TabletToolEvent<B>>(&mut self, evt: &E) -> Route {
        let pos = self.touch_location(evt);
        let ts = self.core.seat.tablet_seat();
        let tablet = ts.get_tablet(&TabletDescriptor::from(&evt.device()));
        let Some(tool) = ts.get_tool(&evt.tool()) else { return Route::Pointer };
        let route = self.pen_route(pos, &tool);
        let native = matches!(route, Route::Native(..));
        if self.core.pen.native != Some(native) {
            self.core.pen.native = Some(native);
            let window = match &route {
                Route::Native(s, _) => self.window_for_surface_tree(s),
                Route::Pointer => None,
            };
            tracing::info!(native, ?window, x = pos.x as i32, y = pos.y as i32, "перо: маршрут (native — tablet-v2, иначе указатель)");
        }
        let time = evt.time_msec();
        match &route {
            Route::Native(s, loc) => {
                if let Some(tablet) = tablet {
                    if evt.pressure_has_changed() {
                        tool.pressure(evt.pressure());
                    }
                    if evt.distance_has_changed() {
                        tool.distance(evt.distance());
                    }
                    if evt.tilt_has_changed() {
                        tool.tilt(evt.tilt());
                    }
                    if evt.rotation_has_changed() {
                        tool.rotation(evt.rotation());
                    }
                    if evt.slider_has_changed() {
                        tool.slider_position(evt.slider_position());
                    }
                    if evt.wheel_has_changed() {
                        tool.wheel(evt.wheel_delta(), evt.wheel_delta_discrete());
                    }
                    tool.motion(pos, Some((s.clone(), *loc)), &tablet, SERIAL_COUNTER.next_serial(), time);
                }
            }
            Route::Pointer => {
                // Клиент с tablet-v2 остался позади — proximity_out ему.
                if let Some(tablet) = tablet {
                    tool.motion(pos, None, &tablet, SERIAL_COUNTER.next_serial(), time);
                }
                let under = self.under(pos);
                self.pointer_moved(pos, under, SERIAL_COUNTER.next_serial(), time);
            }
        }
        if self.core.pen.last_log.is_none_or(|t| t.elapsed() >= Duration::from_secs(1)) {
            self.core.pen.last_log = Some(Instant::now());
            tracing::debug!(x = pos.x as i32, y = pos.y as i32, pressure = evt.pressure(), native, "перо: движение");
        }
        route
    }

    pub(crate) fn on_tablet_tool_axis<B: InputBackend>(&mut self, evt: B::TabletToolAxisEvent) {
        if self.core.pen.near {
            self.pen_motion::<B, _>(&evt);
        }
    }

    pub(crate) fn on_tablet_tool_proximity<B: InputBackend>(&mut self, evt: B::TabletToolProximityEvent) {
        let time = evt.time_msec();
        match evt.state() {
            ProximityState::In => {
                let dh = self.core.display_handle.clone();
                let desc = evt.tool();
                self.core.seat.tablet_seat().add_tool::<State>(self, &dh, &desc);
                self.core.pen.near = true;
                self.core.pen.native = None;
                // Курсор под пером не рисуем (E Ink): вернётся с движением мыши.
                if !self.core.cursor_hidden {
                    self.core.cursor_hidden = true;
                    self.core.queue_redraw_all();
                }
                let pos = self.touch_location(&evt);
                tracing::info!(tool = ?desc.tool_type, x = pos.x as i32, y = pos.y as i32, "перо: у экрана");
                self.pen_motion::<B, _>(&evt);
            }
            ProximityState::Out => {
                tracing::info!("перо: ушло от экрана");
                self.pen_release_all(time);
                if let Some(tool) = self.core.seat.tablet_seat().get_tool(&evt.tool()) {
                    tool.proximity_out(time);
                }
                self.core.pen.near = false;
                self.core.pen.native = None;
            }
        }
    }

    pub(crate) fn on_tablet_tool_tip<B: InputBackend>(&mut self, evt: B::TabletToolTipEvent) {
        let Some(tool) = self.core.seat.tablet_seat().get_tool(&evt.tool()) else { return };
        let time = evt.time_msec();
        let pos = self.touch_location(&evt);
        match evt.tip_state() {
            TabletToolTipState::Down => {
                // Точка касания — своим кадром до нажатия (Xwayland иначе щёлкнет в старой точке).
                let route = self.pen_motion::<B, _>(&evt);
                tracing::info!(x = pos.x as i32, y = pos.y as i32, native = matches!(route, Route::Native(..)), "перо: касание");
                self.pen_press(route, BTN_LEFT, &tool, time, true);
            }
            TabletToolTipState::Up => {
                tracing::info!(x = pos.x as i32, y = pos.y as i32, "перо: отпущено");
                self.pen_release(BTN_LEFT, &tool, time, true);
            }
        }
    }

    pub(crate) fn on_tablet_tool_button<B: InputBackend>(&mut self, evt: B::TabletToolButtonEvent) {
        let Some(tool) = self.core.seat.tablet_seat().get_tool(&evt.tool()) else { return };
        let time = evt.time_msec();
        let button = evt.button();
        tracing::info!(button = format!("{button:#x}"), state = ?evt.button_state(), "перо: кнопка");
        match evt.button_state() {
            ButtonState::Pressed => {
                let pos = self.touch_location(&evt);
                let route = self.pen_route(pos, &tool);
                self.pen_press(route, button, &tool, time, false);
            }
            ButtonState::Released => self.pen_release(button, &tool, time, false),
        }
    }

    /// Кнопка пера для эмуляции указателя: нижняя — правая, верхняя — средняя.
    fn pen_pointer_button(pen_button: u32, tip: bool) -> Option<u32> {
        match pen_button {
            _ if tip => Some(BTN_LEFT),
            BTN_STYLUS => Some(BTN_RIGHT),
            BTN_STYLUS2 => Some(BTN_MIDDLE),
            _ => None,
        }
    }

    fn pen_press(
        &mut self,
        route: Route,
        button: u32,
        tool: &smithay::wayland::tablet_manager::TabletToolHandle,
        time: u32,
        tip: bool,
    ) {
        self.core.pen.held += 1;
        self.core.pen.grab = Some(route.clone());
        match route {
            Route::Native(s, _) => {
                let serial = SERIAL_COUNTER.next_serial();
                if tip {
                    tool.tip_down(serial, time);
                    // Клавиатура — окну под пером, как при щелчке.
                    self.click_focus(&s);
                } else {
                    tool.button(button, ButtonState::Pressed, serial, time);
                }
            }
            Route::Pointer => {
                if let Some(b) = Self::pen_pointer_button(button, tip) {
                    if !self.core.pen.emu_buttons.contains(&b) {
                        self.core.pen.emu_buttons.push(b);
                        self.pointer_button(b, ButtonState::Pressed, time);
                    }
                }
            }
        }
    }

    fn pen_release(
        &mut self,
        button: u32,
        tool: &smithay::wayland::tablet_manager::TabletToolHandle,
        time: u32,
        tip: bool,
    ) {
        match self.core.pen.grab.clone() {
            Some(Route::Native(..)) => {
                if tip {
                    tool.tip_up(time);
                } else {
                    tool.button(button, ButtonState::Released, SERIAL_COUNTER.next_serial(), time);
                }
            }
            Some(Route::Pointer) => {
                if let Some(b) = Self::pen_pointer_button(button, tip) {
                    if let Some(i) = self.core.pen.emu_buttons.iter().position(|x| *x == b) {
                        self.core.pen.emu_buttons.remove(i);
                        self.pointer_button(b, ButtonState::Released, time);
                    }
                }
            }
            None => {}
        }
        self.core.pen.held = self.core.pen.held.saturating_sub(1);
        if self.core.pen.held == 0 {
            self.core.pen.grab = None;
        }
    }

    /// Отпустить всё, что эмуляция держит нажатым (перо ушло, планшет убран).
    fn pen_release_all(&mut self, time: u32) {
        for b in std::mem::take(&mut self.core.pen.emu_buttons) {
            self.pointer_button(b, ButtonState::Released, time);
        }
        self.core.pen.held = 0;
        self.core.pen.grab = None;
    }
}
