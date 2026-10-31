//! Перо (libinput tablet tool): клиентам с tablet-v2 — события инструмента
//! (давление, наклон, кнопки), остальным (syngui, Xwayland без планшета) —
//! эмуляция указателя: кончик — левая кнопка, нижняя кнопка пера — правая,
//! наведение — движение без курсора (на E Ink лишний кадр — обновление панели).
//!
//! На E Ink с режимом пера в драйвере (`pen` в sysfs, [`Eink::set_pen`]): перо у экрана
//! или касание — режим включается сразу, ушло — выключается через [`PEN_MODE_HOLD`]
//! (вернулось раньше — остаётся). В режиме пера кадры клиентов уходят без ожидания
//! расчётного vblank (`Core::queue_redraw_for_surface`), задержка «событие пера →
//! `queue_frame`» — в журнал (цель `synwm::pen`, уровень debug; итог — info).

use std::sync::mpsc;
use std::time::{Duration, Instant};

use synshell_common::display::Eink;

use smithay::{
    backend::input::{
        ButtonState, Device, DeviceCapability, Event, InputBackend, ProximityState,
        TabletToolButtonEvent, TabletToolEvent, TabletToolProximityEvent, TabletToolTipEvent, TabletToolTipState,
    },
    reexports::{
        calloop::{
            timer::{TimeoutAction, Timer},
            RegistrationToken,
        },
        wayland_server::protocol::wl_surface::WlSurface,
    },
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

/// Сколько держать режим пера E Ink после ухода пера от экрана.
pub const PEN_MODE_HOLD: Duration = Duration::from_millis(700);

/// Защёлка режима пера: включается сразу, выключается по таймеру после ухода пера.
#[derive(Default, Debug)]
struct PenLatch {
    on: bool,
    timer: bool,
}

/// Что сделать после события защёлки.
#[derive(Default, Debug, PartialEq, Eq)]
struct LatchStep {
    /// Записать в драйвер (только при смене).
    write: Option<bool>,
    cancel_timer: bool,
    arm_timer: bool,
}

impl PenLatch {
    /// Перо у экрана или коснулось: включить, таймер выхода отменить.
    fn arrive(&mut self) -> LatchStep {
        let cancel_timer = std::mem::take(&mut self.timer);
        let write = (!self.on).then_some(true);
        self.on = true;
        LatchStep { write, cancel_timer, arm_timer: false }
    }

    /// Перо ушло от экрана: выключить через [`PEN_MODE_HOLD`].
    fn leave(&mut self) -> LatchStep {
        if !self.on || self.timer {
            return LatchStep::default();
        }
        self.timer = true;
        LatchStep { arm_timer: true, ..Default::default() }
    }

    /// Таймер выхода сработал.
    fn expire(&mut self) -> LatchStep {
        if !std::mem::take(&mut self.timer) || !self.on {
            return LatchStep::default();
        }
        self.on = false;
        LatchStep { write: Some(false), ..Default::default() }
    }
}

/// Запись `pen` в sysfs своим потоком: `pen_store` берёт замок драйвера, а тот может
/// быть занят обменом с FPGA — главный поток (ввод, кадры) ждать не должен.
struct PenWriter(mpsc::Sender<bool>);

impl PenWriter {
    fn spawn(e: Eink) -> Option<PenWriter> {
        if !e.has_pen() {
            return None;
        }
        let (tx, rx) = mpsc::channel::<bool>();
        std::thread::Builder::new()
            .name("synwm-pen".into())
            .spawn(move || {
                // Остался включённым от прошлого запуска (synwm упал с пером у экрана).
                if e.pen() == Some(true) {
                    let _ = e.set_pen(false);
                }
                while let Ok(mut on) = rx.recv() {
                    // Накопилось несколько — важно последнее (драйвер сам сверяет с текущим).
                    while let Ok(next) = rx.try_recv() {
                        on = next;
                    }
                    let t = Instant::now();
                    match e.set_pen(on) {
                        Ok(()) => tracing::debug!(target: "synwm::pen", on, us = t.elapsed().as_micros() as u64, "перо: записан режим пера"),
                        Err(err) => tracing::warn!(on, %err, "перо: не записан режим пера"),
                    }
                }
            })
            .ok()?;
        Some(PenWriter(tx))
    }
}

/// Задержка «событие пера → `queue_frame`» за время режима пера.
#[derive(Default, Debug)]
struct Latency {
    frames: u32,
    sum: u64,
    min: u32,
    max: u32,
}

impl Latency {
    fn add(&mut self, ms: u32) {
        if self.frames == 0 || ms < self.min {
            self.min = ms;
        }
        self.max = self.max.max(ms);
        self.sum += ms as u64;
        self.frames += 1;
    }
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
    /// Режим пера E Ink (только при `core.eink`).
    latch: PenLatch,
    exit_timer: Option<RegistrationToken>,
    /// Нет атрибута `pen` у драйвера — `None`: режим только у композитора (быстрые кадры).
    writer: Option<PenWriter>,
    /// libinput-время (мс, CLOCK_MONOTONIC) первого и последнего события пера,
    /// ещё не показанных кадром.
    ev_first: Option<u32>,
    ev_last: Option<u32>,
    latency: Latency,
}

impl PenState {
    /// Режим пера включён: кадры клиентов — сразу, без ожидания расчётного vblank.
    pub fn fast(&self) -> bool {
        self.latch.on
    }

    fn note_event(&mut self, time: u32) {
        if self.latch.on {
            self.ev_first.get_or_insert(time);
            self.ev_last = Some(time);
        }
    }

    /// Кадр с изменениями отдан драйверу (`queue_frame`) в момент `now` (мс, CLOCK_MONOTONIC).
    pub fn frame_queued(&mut self, now: u32, render_us: u64) {
        let Some(last) = self.ev_last.take() else { return };
        let first = self.ev_first.take().unwrap_or(last);
        let ms = now.wrapping_sub(last);
        let from_first = now.wrapping_sub(first);
        // Часы разошлись (не тот clock у libinput) — не считать мусор.
        if ms > 10_000 {
            return;
        }
        self.latency.add(ms);
        tracing::debug!(
            target: "synwm::pen",
            ms,
            from_first,
            render_us,
            "перо: от события до queue_frame {ms} мс (от первого непоказанного {from_first} мс, рендер {render_us} мкс)"
        );
    }
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
        self.pen_mode_leave();
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
                    // Наклон и поворот пера — в осях повёрнутого изображения, как и точка.
                    let t = self.input_transform();
                    if evt.tilt_has_changed() {
                        tool.tilt(rotate_vec(t, evt.tilt()));
                    }
                    if evt.rotation_has_changed() {
                        tool.rotation(rotate_angle(t, evt.rotation()));
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

    /// Экран E Ink: включить режим пера, если драйвер его умеет (после `core.eink`).
    pub(crate) fn init_pen_mode(&mut self) {
        if !self.core.eink {
            return;
        }
        let writer = Eink::find().and_then(PenWriter::spawn);
        tracing::info!(driver = writer.is_some(), "перо: режим пера E Ink (driver — атрибут pen у драйвера)");
        self.core.pen.writer = writer;
    }

    /// Перо у экрана или коснулось — режим пера сразу.
    fn pen_mode_arrive(&mut self) {
        if self.core.eink {
            let step = self.core.pen.latch.arrive();
            self.pen_mode_apply(step);
        }
    }

    /// Перо ушло — режим пера выключится через [`PEN_MODE_HOLD`].
    fn pen_mode_leave(&mut self) {
        let step = self.core.pen.latch.leave();
        self.pen_mode_apply(step);
    }

    fn pen_mode_apply(&mut self, step: LatchStep) {
        if step.cancel_timer {
            if let Some(t) = self.core.pen.exit_timer.take() {
                self.core.loop_handle.remove(t);
            }
        }
        if step.arm_timer {
            self.core.pen.exit_timer = self
                .core
                .loop_handle
                .insert_source(Timer::from_duration(PEN_MODE_HOLD), |_, _, state| {
                    state.core.pen.exit_timer = None;
                    let step = state.core.pen.latch.expire();
                    state.pen_mode_apply(step);
                    TimeoutAction::Drop
                })
                .ok();
        }
        let Some(on) = step.write else { return };
        let pen = &mut self.core.pen;
        if let Some(w) = &pen.writer {
            let _ = w.0.send(on);
        }
        if on {
            pen.latency = Latency::default();
            tracing::info!("перо: режим пера включён");
        } else {
            pen.ev_first = None;
            pen.ev_last = None;
            let l = std::mem::take(&mut pen.latency);
            let avg = if l.frames > 0 { l.sum / l.frames as u64 } else { 0 };
            tracing::info!(
                frames = l.frames,
                min = l.min,
                avg,
                max = l.max,
                "перо: режим пера выключен; событие → queue_frame: кадров {}, мин {} / сред {avg} / макс {} мс",
                l.frames,
                l.min,
                l.max
            );
        }
    }

    pub(crate) fn on_tablet_tool_axis<B: InputBackend>(&mut self, evt: B::TabletToolAxisEvent) {
        self.core.pen.note_event(evt.time_msec());
        if self.core.pen.near {
            self.pen_motion::<B, _>(&evt);
        }
    }

    pub(crate) fn on_tablet_tool_proximity<B: InputBackend>(&mut self, evt: B::TabletToolProximityEvent) {
        let time = evt.time_msec();
        match evt.state() {
            ProximityState::In => {
                // Режим пера — до первого кадра под пером.
                self.pen_mode_arrive();
                self.core.pen.note_event(time);
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
                self.pen_mode_leave();
            }
        }
    }

    pub(crate) fn on_tablet_tool_tip<B: InputBackend>(&mut self, evt: B::TabletToolTipEvent) {
        let Some(tool) = self.core.seat.tablet_seat().get_tool(&evt.tool()) else { return };
        let time = evt.time_msec();
        let pos = self.touch_location(&evt);
        match evt.tip_state() {
            TabletToolTipState::Down => {
                self.pen_mode_arrive();
                self.core.pen.note_event(time);
                // Точка касания — своим кадром до нажатия (Xwayland иначе щёлкнет в старой точке).
                let route = self.pen_motion::<B, _>(&evt);
                tracing::info!(x = pos.x as i32, y = pos.y as i32, native = matches!(route, Route::Native(..)), "перо: касание");
                self.pen_press(route, BTN_LEFT, &tool, time, true);
            }
            TabletToolTipState::Up => {
                self.core.pen.note_event(time);
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

/// Вектор в осях панели (x вправо, y вниз) — в оси изображения под поворотом `t`.
fn rotate_vec(t: smithay::utils::Transform, (x, y): (f64, f64)) -> (f64, f64) {
    let zero = smithay::utils::Size::<f64, Logical>::from((0.0, 0.0));
    let p = t.transform_point_in(Point::<f64, Logical>::from((x, y)), &zero);
    (p.x, p.y)
}

/// Угол поворота пера (градусы по часовой) — в оси изображения под поворотом `t`.
fn rotate_angle(t: smithay::utils::Transform, deg: f64) -> f64 {
    if t == smithay::utils::Transform::Normal {
        return deg;
    }
    let a = deg.to_radians();
    let (x, y) = rotate_vec(t, (a.cos(), a.sin()));
    y.atan2(x).to_degrees().rem_euclid(360.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::utils::Transform as T;

    #[test]
    fn pen_latch_enter_exit() {
        let mut l = PenLatch::default();
        // подлёт: включить, без таймера
        assert_eq!(l.arrive(), LatchStep { write: Some(true), ..Default::default() });
        // касание при включённом — повторно не писать
        assert_eq!(l.arrive(), LatchStep::default());
        // ушло — таймер; ещё раз ушло — второго таймера нет
        assert_eq!(l.leave(), LatchStep { arm_timer: true, ..Default::default() });
        assert_eq!(l.leave(), LatchStep::default());
        // вернулось до срока: таймер отменить, писать нечего
        assert_eq!(l.arrive(), LatchStep { cancel_timer: true, ..Default::default() });
        assert!(l.on);
        // отменённый таймер всё же сработал (гонка) — ничего
        assert_eq!(l.expire(), LatchStep::default());
        assert!(l.on);
        // ушло и срок вышел — выключить один раз
        assert_eq!(l.leave(), LatchStep { arm_timer: true, ..Default::default() });
        assert_eq!(l.expire(), LatchStep { write: Some(false), ..Default::default() });
        assert!(!l.on);
        assert_eq!(l.expire(), LatchStep::default());
        // уход без подлёта (планшет убран) — ничего
        assert_eq!(l.leave(), LatchStep::default());
        // снова подлёт — снова включить
        assert_eq!(l.arrive(), LatchStep { write: Some(true), ..Default::default() });
    }

    #[test]
    fn pen_latency_only_in_pen_mode() {
        let mut p = PenState::default();
        p.note_event(100);
        p.frame_queued(150, 0);
        assert_eq!(p.latency.frames, 0);
        p.latch.arrive();
        p.note_event(1000);
        p.note_event(1010);
        p.frame_queued(1072, 0);
        assert_eq!((p.latency.frames, p.latency.min, p.latency.max), (1, 62, 62));
        // кадр без новых событий пера — не считается
        p.frame_queued(1100, 0);
        assert_eq!(p.latency.frames, 1);
        p.note_event(u32::MAX - 5);
        p.frame_queued(4, 0);
        assert_eq!((p.latency.frames, p.latency.max), (2, 62));
        assert_eq!(p.latency.min, 10);
    }

    #[test]
    fn tilt_follows_output_rotation() {
        assert_eq!(rotate_vec(T::Normal, (10.0, -5.0)), (10.0, -5.0));
        // Вывод повёрнут на 90°: x панели смотрит вниз изображения (как и точки касания).
        let p = T::_90.transform_point_in(Point::<f64, Logical>::from((1.0, 0.0)), &smithay::utils::Size::from((0.0, 0.0)));
        assert_eq!(rotate_vec(T::_90, (1.0, 0.0)), (p.x, p.y));
        let (x, y) = rotate_vec(T::_180, (3.0, 4.0));
        assert_eq!((x, y), (-3.0, -4.0));
        assert_eq!(rotate_angle(T::Normal, 30.0), 30.0);
        assert!((rotate_angle(T::_180, 30.0) - 210.0).abs() < 1e-9);
        let q = rotate_angle(T::_90, 30.0);
        assert!((q - 120.0).abs() < 1e-9 || (q - 300.0).abs() < 1e-9, "{q}");
    }
}
