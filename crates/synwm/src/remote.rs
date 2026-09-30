//! Удалённый ввод (synlink, IPC `input`): синтетический `InputBackend`.
//! События идут через `process_input_event`, как от libinput, — сочетания
//! клавиш, жесты от края, пальцы и фокус работают так же, как на самом
//! устройстве. Абсолютные координаты — доли 0..1 выбранного вывода.

use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisRelativeDirection, AxisSource, ButtonState, Device, DeviceCapability, Event,
    InputBackend, InputEvent, KeyState, KeyboardKeyEvent, Keycode, PointerAxisEvent, PointerButtonEvent,
    PointerMotionAbsoluteEvent, PointerMotionEvent, TouchCancelEvent, TouchDownEvent, TouchEvent, TouchFrameEvent,
    TouchMotionEvent, TouchSlot, TouchUpEvent, UnusedEvent,
};
use smithay::input::keyboard::{FilterResult, Keysym};
use smithay::output::Output;
use smithay::utils::SERIAL_COUNTER;
use synshell_common::ipc::InputEvent as Remote;

use crate::state::State;

#[derive(Debug)]
pub struct RemoteInput;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RemoteDevice;

impl Device for RemoteDevice {
    fn id(&self) -> String {
        "synlink".into()
    }
    fn name(&self) -> String {
        "synlink remote input".into()
    }
    fn has_capability(&self, c: DeviceCapability) -> bool {
        matches!(c, DeviceCapability::Keyboard | DeviceCapability::Pointer | DeviceCapability::Touch)
    }
    fn usb_id(&self) -> Option<(u32, u32)> {
        None
    }
    fn syspath(&self) -> Option<std::path::PathBuf> {
        None
    }
}

/// Одно синтетическое событие: общие поля всех видов.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ev {
    time_us: u64,
    x: f64,
    y: f64,
    dx: f64,
    dy: f64,
    code: u32,
    pressed: bool,
    slot: Option<u32>,
    discrete: bool,
}

impl Event<RemoteInput> for Ev {
    fn time(&self) -> u64 {
        self.time_us
    }
    fn device(&self) -> RemoteDevice {
        RemoteDevice
    }
}

impl KeyboardKeyEvent<RemoteInput> for Ev {
    fn key_code(&self) -> Keycode {
        // evdev → xkb: смещение 8.
        Keycode::new(self.code + 8)
    }
    fn state(&self) -> KeyState {
        if self.pressed {
            KeyState::Pressed
        } else {
            KeyState::Released
        }
    }
    fn count(&self) -> u32 {
        u32::from(self.pressed)
    }
}

impl PointerButtonEvent<RemoteInput> for Ev {
    fn button_code(&self) -> u32 {
        self.code
    }
    fn state(&self) -> ButtonState {
        if self.pressed {
            ButtonState::Pressed
        } else {
            ButtonState::Released
        }
    }
}

impl PointerAxisEvent<RemoteInput> for Ev {
    fn amount(&self, axis: Axis) -> Option<f64> {
        if self.discrete {
            return None;
        }
        Some(match axis {
            Axis::Horizontal => self.dx,
            Axis::Vertical => self.dy,
        })
    }
    fn amount_v120(&self, axis: Axis) -> Option<f64> {
        if !self.discrete {
            return None;
        }
        // Шаг колеса — 15 px = 120 единиц.
        Some(match axis {
            Axis::Horizontal => self.dx,
            Axis::Vertical => self.dy,
        } * 120.0
            / 15.0)
    }
    fn source(&self) -> AxisSource {
        if self.discrete {
            AxisSource::Wheel
        } else {
            AxisSource::Finger
        }
    }
    fn relative_direction(&self, _axis: Axis) -> AxisRelativeDirection {
        AxisRelativeDirection::Identical
    }
}

impl PointerMotionEvent<RemoteInput> for Ev {
    fn delta_x(&self) -> f64 {
        self.dx
    }
    fn delta_y(&self) -> f64 {
        self.dy
    }
    fn delta_x_unaccel(&self) -> f64 {
        self.dx
    }
    fn delta_y_unaccel(&self) -> f64 {
        self.dy
    }
}

impl AbsolutePositionEvent<RemoteInput> for Ev {
    fn x(&self) -> f64 {
        self.x
    }
    fn y(&self) -> f64 {
        self.y
    }
    fn x_transformed(&self, width: i32) -> f64 {
        self.x.clamp(0.0, 1.0) * width as f64
    }
    fn y_transformed(&self, height: i32) -> f64 {
        self.y.clamp(0.0, 1.0) * height as f64
    }
}

impl PointerMotionAbsoluteEvent<RemoteInput> for Ev {}

impl TouchEvent<RemoteInput> for Ev {
    fn slot(&self) -> TouchSlot {
        // Слоты удалённых пальцев не пересекаются с настоящими.
        self.slot.map(|s| 1000 + s).into()
    }
}
impl TouchDownEvent<RemoteInput> for Ev {}
impl TouchMotionEvent<RemoteInput> for Ev {}
impl TouchUpEvent<RemoteInput> for Ev {}
impl TouchCancelEvent<RemoteInput> for Ev {}
impl TouchFrameEvent<RemoteInput> for Ev {}

impl InputBackend for RemoteInput {
    type Device = RemoteDevice;
    type KeyboardKeyEvent = Ev;
    type PointerAxisEvent = Ev;
    type PointerButtonEvent = Ev;
    type PointerMotionEvent = Ev;
    type PointerMotionAbsoluteEvent = Ev;
    type GestureSwipeBeginEvent = UnusedEvent;
    type GestureSwipeUpdateEvent = UnusedEvent;
    type GestureSwipeEndEvent = UnusedEvent;
    type GesturePinchBeginEvent = UnusedEvent;
    type GesturePinchUpdateEvent = UnusedEvent;
    type GesturePinchEndEvent = UnusedEvent;
    type GestureHoldBeginEvent = UnusedEvent;
    type GestureHoldEndEvent = UnusedEvent;
    type TouchDownEvent = Ev;
    type TouchUpEvent = Ev;
    type TouchMotionEvent = Ev;
    type TouchCancelEvent = Ev;
    type TouchFrameEvent = Ev;
    type TabletToolAxisEvent = UnusedEvent;
    type TabletToolProximityEvent = UnusedEvent;
    type TabletToolTipEvent = UnusedEvent;
    type TabletToolButtonEvent = UnusedEvent;
    type SwitchToggleEvent = UnusedEvent;
    type SpecialEvent = UnusedEvent;
}

impl State {
    /// Вывод для абсолютных координат ввода: удалённый ввод выбирает свой,
    /// остальные — первый (у вложенного окна и планшета он один).
    pub fn absolute_input_output(&self) -> Option<Output> {
        self.core
            .remote_output
            .clone()
            .filter(|o| self.core.space.outputs().any(|x| x == o))
            .or_else(|| self.core.space.outputs().next().cloned())
    }

    /// IPC `input`: выполнить пачку событий удалённого ввода.
    pub fn remote_input(&mut self, output: Option<&str>, events: Vec<Remote>) -> Result<(), String> {
        let out = match output {
            Some(name) => Some(self.core.output_by_name(name).ok_or_else(|| format!("нет вывода {name}"))?),
            None => None,
        };
        self.core.remote_output = out;
        let time_us = self.core.start_time.elapsed().as_micros() as u64;
        for e in events {
            let ev = Ev { time_us, ..Default::default() };
            let event: InputEvent<RemoteInput> = match e {
                Remote::Motion { x, y } => InputEvent::PointerMotionAbsolute { event: Ev { x, y, ..ev } },
                Remote::MotionRelative { dx, dy } => InputEvent::PointerMotion { event: Ev { dx, dy, ..ev } },
                Remote::Button { button, pressed } => InputEvent::PointerButton { event: Ev { code: button, pressed, ..ev } },
                Remote::Axis { dx, dy, discrete } => InputEvent::PointerAxis { event: Ev { dx, dy, discrete, ..ev } },
                Remote::Key { code, pressed } => InputEvent::Keyboard { event: Ev { code, pressed, ..ev } },
                Remote::TouchDown { id, x, y } => {
                    self.ensure_touch();
                    InputEvent::TouchDown { event: Ev { x, y, slot: Some(id), ..ev } }
                }
                Remote::TouchMotion { id, x, y } => InputEvent::TouchMotion { event: Ev { x, y, slot: Some(id), ..ev } },
                Remote::TouchUp { id } => InputEvent::TouchUp { event: Ev { slot: Some(id), ..ev } },
                Remote::TouchFrame => InputEvent::TouchFrame { event: ev },
                Remote::TouchCancel => InputEvent::TouchCancel { event: ev },
                Remote::Combo { combo } => {
                    match combo.parse::<synshell_common::action::KeyCombo>() {
                        Ok(c) => {
                            self.note_activity();
                            self.send_key_combo(&c);
                        }
                        Err(e) => return Err(format!("сочетание {combo}: {e}")),
                    }
                    continue;
                }
                Remote::Text { text } => {
                    self.note_activity();
                    self.type_text(&text);
                    continue;
                }
            };
            self.process_input_event(event);
        }
        self.core.remote_output = None;
        Ok(())
    }

    /// Без сенсорного экрана (десктоп) касания некому принять — добавить
    /// сенсорную способность сиденью по первому удалённому касанию.
    fn ensure_touch(&mut self) {
        if self.core.seat.get_touch().is_none() {
            self.core.seat.add_touch();
        }
    }

    /// Набрать текст клавишами раскладок: символ → keysym → код клавиши
    /// (уровень 0 или 1 с Shift), предпочтительно в активной группе; символ
    /// из другой группы (кириллица при активной латинице) набирается с
    /// временным переключением группы, затем прежняя возвращается.
    pub fn type_text(&mut self, text: &str) {
        use smithay::input::keyboard::Layout;
        let keyboard = self.core.keyboard.clone();
        let (active, plan): (u32, Vec<Option<(Keycode, u32, bool)>>) = keyboard.with_xkb_state(self, |ctx| {
            let xkb = ctx.xkb().lock().unwrap();
            let active = xkb.active_layout().0;
            // SAFETY: keymap только читается, состояние xkb не меняется.
            let keymap = unsafe { xkb.keymap() };
            let plan = text
                .chars()
                .map(|ch| {
                    let want = match ch {
                        '\n' => Keysym::Return,
                        '\t' => Keysym::Tab,
                        c => Keysym::from(smithay::input::keyboard::xkb::utf32_to_keysym(c as u32)),
                    };
                    let mut found: Option<(Keycode, u32, bool)> = None;
                    keymap.key_for_each(|_, kc| {
                        if found.is_some_and(|f| f.1 == active) {
                            return;
                        }
                        for layout in 0..keymap.num_layouts_for_key(kc) {
                            for level in 0..2u32 {
                                if keymap.key_get_syms_by_level(kc, layout, level).contains(&want)
                                    && (found.is_none() || layout == active)
                                {
                                    found = Some((kc, layout, level == 1));
                                }
                            }
                        }
                    });
                    found
                })
                .collect();
            (active, plan)
        });
        let shift_code = Keycode::new(42 + 8);
        let time = self.core.start_time.elapsed().as_millis() as u32;
        let mut group = active;
        for (kc, layout, shift) in plan.into_iter().flatten() {
            if layout != group {
                keyboard.with_xkb_state(self, |mut ctx| ctx.set_layout(Layout(layout)));
                group = layout;
            }
            let mut press = |st: &mut State, code, state| {
                keyboard.input::<(), _>(st, code, state, SERIAL_COUNTER.next_serial(), time, |_, _, _| FilterResult::Forward);
            };
            if shift {
                press(self, shift_code, KeyState::Pressed);
            }
            press(self, kc, KeyState::Pressed);
            press(self, kc, KeyState::Released);
            if shift {
                press(self, shift_code, KeyState::Released);
            }
        }
        if group != active {
            keyboard.with_xkb_state(self, |mut ctx| ctx.set_layout(Layout(active)));
        }
    }
}
