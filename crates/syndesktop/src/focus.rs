//! Цель фокуса ввода — обёртка над `WlSurface`.
//!
//! Рамки окон композитор обрабатывает сам (до передачи событий клиенту),
//! поэтому фокус указателя и клавиатуры — всегда поверхность клиента.
//! Обёртка нужна ради `From<PopupKind>`: захват всплывающих меню smithay
//! требует его от типа фокуса, а для чужого `WlSurface` его не объявить.

use std::borrow::Cow;

use smithay::{
    backend::input::KeyState,
    desktop::{LayerSurface, PopupKind, Window},
    input::{
        keyboard::{KeyboardTarget, KeysymHandle, ModifiersState},
        pointer::{
            AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
            GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent,
            GestureSwipeUpdateEvent, MotionEvent, PointerTarget, RelativeMotionEvent,
        },
        touch::{DownEvent, OrientationEvent, ShapeEvent, TouchTarget, UpEvent},
        Seat,
    },
    reexports::wayland_server::{backend::ObjectId, protocol::wl_surface::WlSurface},
    utils::{IsAlive, Serial},
    wayland::{seat::WaylandFocus, session_lock::LockSurface},
};

use crate::state::State;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FocusTarget(pub WlSurface);

impl FocusTarget {
    pub fn surface(&self) -> &WlSurface {
        &self.0
    }
}

impl From<WlSurface> for FocusTarget {
    fn from(s: WlSurface) -> Self {
        Self(s)
    }
}

impl From<&WlSurface> for FocusTarget {
    fn from(s: &WlSurface) -> Self {
        Self(s.clone())
    }
}

impl From<PopupKind> for FocusTarget {
    fn from(p: PopupKind) -> Self {
        Self(p.wl_surface().clone())
    }
}

impl From<LayerSurface> for FocusTarget {
    fn from(l: LayerSurface) -> Self {
        Self(l.wl_surface().clone())
    }
}

impl From<LockSurface> for FocusTarget {
    fn from(l: LockSurface) -> Self {
        Self(l.wl_surface().clone())
    }
}

impl TryFrom<&Window> for FocusTarget {
    type Error = ();
    fn try_from(w: &Window) -> Result<Self, ()> {
        w.wl_surface().map(|s| Self(s.into_owned())).ok_or(())
    }
}

impl IsAlive for FocusTarget {
    fn alive(&self) -> bool {
        self.0.alive()
    }
}

impl WaylandFocus for FocusTarget {
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        Some(Cow::Borrowed(&self.0))
    }

    fn same_client_as(&self, object_id: &ObjectId) -> bool {
        self.0.same_client_as(object_id)
    }
}

impl KeyboardTarget<State> for FocusTarget {
    fn enter(&self, seat: &Seat<State>, data: &mut State, keys: Vec<KeysymHandle<'_>>, serial: Serial) {
        KeyboardTarget::enter(&self.0, seat, data, keys, serial)
    }

    fn leave(&self, seat: &Seat<State>, data: &mut State, serial: Serial) {
        KeyboardTarget::leave(&self.0, seat, data, serial)
    }

    fn key(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        key: KeysymHandle<'_>,
        state: KeyState,
        serial: Serial,
        time: u32,
    ) {
        KeyboardTarget::key(&self.0, seat, data, key, state, serial, time)
    }

    fn modifiers(&self, seat: &Seat<State>, data: &mut State, modifiers: ModifiersState, serial: Serial) {
        KeyboardTarget::modifiers(&self.0, seat, data, modifiers, serial)
    }
}

impl PointerTarget<State> for FocusTarget {
    fn enter(&self, seat: &Seat<State>, data: &mut State, event: &MotionEvent) {
        PointerTarget::enter(&self.0, seat, data, event)
    }
    fn motion(&self, seat: &Seat<State>, data: &mut State, event: &MotionEvent) {
        PointerTarget::motion(&self.0, seat, data, event)
    }
    fn relative_motion(&self, seat: &Seat<State>, data: &mut State, event: &RelativeMotionEvent) {
        PointerTarget::relative_motion(&self.0, seat, data, event)
    }
    fn button(&self, seat: &Seat<State>, data: &mut State, event: &ButtonEvent) {
        PointerTarget::button(&self.0, seat, data, event)
    }
    fn axis(&self, seat: &Seat<State>, data: &mut State, frame: AxisFrame) {
        PointerTarget::axis(&self.0, seat, data, frame)
    }
    fn frame(&self, seat: &Seat<State>, data: &mut State) {
        PointerTarget::frame(&self.0, seat, data)
    }
    fn gesture_swipe_begin(&self, seat: &Seat<State>, data: &mut State, event: &GestureSwipeBeginEvent) {
        PointerTarget::gesture_swipe_begin(&self.0, seat, data, event)
    }
    fn gesture_swipe_update(&self, seat: &Seat<State>, data: &mut State, event: &GestureSwipeUpdateEvent) {
        PointerTarget::gesture_swipe_update(&self.0, seat, data, event)
    }
    fn gesture_swipe_end(&self, seat: &Seat<State>, data: &mut State, event: &GestureSwipeEndEvent) {
        PointerTarget::gesture_swipe_end(&self.0, seat, data, event)
    }
    fn gesture_pinch_begin(&self, seat: &Seat<State>, data: &mut State, event: &GesturePinchBeginEvent) {
        PointerTarget::gesture_pinch_begin(&self.0, seat, data, event)
    }
    fn gesture_pinch_update(&self, seat: &Seat<State>, data: &mut State, event: &GesturePinchUpdateEvent) {
        PointerTarget::gesture_pinch_update(&self.0, seat, data, event)
    }
    fn gesture_pinch_end(&self, seat: &Seat<State>, data: &mut State, event: &GesturePinchEndEvent) {
        PointerTarget::gesture_pinch_end(&self.0, seat, data, event)
    }
    fn gesture_hold_begin(&self, seat: &Seat<State>, data: &mut State, event: &GestureHoldBeginEvent) {
        PointerTarget::gesture_hold_begin(&self.0, seat, data, event)
    }
    fn gesture_hold_end(&self, seat: &Seat<State>, data: &mut State, event: &GestureHoldEndEvent) {
        PointerTarget::gesture_hold_end(&self.0, seat, data, event)
    }
    fn leave(&self, seat: &Seat<State>, data: &mut State, serial: Serial, time: u32) {
        PointerTarget::leave(&self.0, seat, data, serial, time)
    }
}

impl TouchTarget<State> for FocusTarget {
    fn down(&self, seat: &Seat<State>, data: &mut State, event: &DownEvent, seq: Serial) {
        TouchTarget::down(&self.0, seat, data, event, seq)
    }
    fn up(&self, seat: &Seat<State>, data: &mut State, event: &UpEvent, seq: Serial) {
        TouchTarget::up(&self.0, seat, data, event, seq)
    }
    fn motion(&self, seat: &Seat<State>, data: &mut State, event: &smithay::input::touch::MotionEvent, seq: Serial) {
        TouchTarget::motion(&self.0, seat, data, event, seq)
    }
    fn frame(&self, seat: &Seat<State>, data: &mut State, seq: Serial) {
        TouchTarget::frame(&self.0, seat, data, seq)
    }
    fn cancel(&self, seat: &Seat<State>, data: &mut State, seq: Serial) {
        TouchTarget::cancel(&self.0, seat, data, seq)
    }
    fn shape(&self, seat: &Seat<State>, data: &mut State, event: &ShapeEvent, seq: Serial) {
        TouchTarget::shape(&self.0, seat, data, event, seq)
    }
    fn orientation(&self, seat: &Seat<State>, data: &mut State, event: &OrientationEvent, seq: Serial) {
        TouchTarget::orientation(&self.0, seat, data, event, seq)
    }
}
