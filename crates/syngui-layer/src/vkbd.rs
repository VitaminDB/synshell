//! Виртуальная клавиатура (`zwp_virtual_keyboard_v1`) и активность полей
//! ввода (`zwp_input_method_v2`) — для экранной клавиатуры (synkeyboard).
//!
//! Клавиши шлются композитору как evdev-коды; модификаторы — обычные клавиши,
//! состояние xkb считает композитор. Раскладка задаётся текстом keymap
//! (`xkb_keymap("us,ru", …)`), группа переключается `virtual_keyboard_group`.

use std::io::Write;
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::time::Instant;

use smithay_client_toolkit::reexports::client::{
    globals::GlobalList, protocol::wl_seat, Connection, Dispatch, QueueHandle,
};
use wayland_protocols_misc::{
    zwp_input_method_v2::client::{zwp_input_method_manager_v2, zwp_input_method_v2},
    zwp_virtual_keyboard_v1::client::{zwp_virtual_keyboard_manager_v1, zwp_virtual_keyboard_v1},
};

use crate::state::State;

#[derive(Default)]
pub(crate) struct Vkbd {
    manager: Option<zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1>,
    keyboard: Option<zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1>,
    /// Раскладка, которую просили до появления seat.
    pending_keymap: Option<String>,
    im_manager: Option<zwp_input_method_manager_v2::ZwpInputMethodManagerV2>,
    im: Option<zwp_input_method_v2::ZwpInputMethodV2>,
    im_wanted: bool,
    /// Состояние между `activate/deactivate` и `done`.
    im_pending: bool,
    start: Option<Instant>,
    /// Текущая группа (раскладка) keymap.
    group: u32,
}

impl Vkbd {
    pub(crate) fn bind(&mut self, globals: &GlobalList, qh: &QueueHandle<State>) {
        self.manager = globals.bind(qh, 1..=1, ()).ok();
        self.im_manager = globals.bind(qh, 1..=1, ()).ok();
        self.start = Some(Instant::now());
        log::debug!(
            "виртуальная клавиатура: {}, input-method: {}",
            self.manager.is_some(),
            self.im_manager.is_some()
        );
    }

    fn time_ms(&self) -> u32 {
        self.start.map(|s| s.elapsed().as_millis() as u32).unwrap_or(0)
    }
}

/// Seat появился — можно создать объекты, которых ждали.
pub(crate) fn seat_ready(st: &mut State, qh: &QueueHandle<State>) {
    if let Some(keymap) = st.vk.pending_keymap.take() {
        set_keymap(st, keymap);
    }
    if st.vk.im_wanted {
        im_enable(st);
    }
    let _ = qh;
}

fn ensure_keyboard(st: &mut State) -> bool {
    if st.vk.keyboard.is_some() {
        return true;
    }
    let (Some(manager), Some(seat), Some(qh)) = (&st.vk.manager, &st.seat, &st.qh) else { return false };
    st.vk.keyboard = Some(manager.create_virtual_keyboard(seat, qh, ()));
    true
}

pub(crate) fn set_keymap(st: &mut State, keymap: String) {
    if st.seat.is_none() {
        st.vk.pending_keymap = Some(keymap);
        return;
    }
    if !ensure_keyboard(st) {
        log::warn!("композитор не поддерживает zwp_virtual_keyboard_v1");
        return;
    }
    // keymap уходит через memfd; протокол требует завершающий NUL.
    let mut data = keymap.into_bytes();
    data.push(0);
    let fd = unsafe { libc::memfd_create(c"xkb-keymap".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        log::error!("memfd_create: {}", std::io::Error::last_os_error());
        return;
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut file = std::fs::File::from(fd);
    if let Err(e) = file.write_all(&data) {
        log::error!("keymap: {e}");
        return;
    }
    let fd: OwnedFd = file.into();
    if let Some(kb) = &st.vk.keyboard {
        kb.keymap(1 /* WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1 */, fd.as_fd(), data.len() as u32);
    }
}

pub(crate) fn key(st: &mut State, code: u32, pressed: bool) {
    if !ensure_keyboard(st) {
        return;
    }
    let t = st.vk.time_ms();
    if let Some(kb) = &st.vk.keyboard {
        kb.key(t, code, if pressed { 1 } else { 0 });
    }
}

pub(crate) fn group(st: &mut State, group: u32) {
    st.vk.group = group;
    modifiers(st, 0, 0, 0);
}

pub(crate) fn modifiers(st: &mut State, depressed: u32, latched: u32, locked: u32) {
    if !ensure_keyboard(st) {
        return;
    }
    let group = st.vk.group;
    if let Some(kb) = &st.vk.keyboard {
        kb.modifiers(depressed, latched, locked, group);
    }
}

pub(crate) fn im_enable(st: &mut State) {
    st.vk.im_wanted = true;
    if st.vk.im.is_some() {
        return;
    }
    let (Some(manager), Some(seat), Some(qh)) = (&st.vk.im_manager, &st.seat, &st.qh) else {
        if st.vk.im_manager.is_none() {
            log::warn!("композитор не поддерживает zwp_input_method_v2 — автопоказ клавиатуры недоступен");
        }
        return;
    };
    st.vk.im = Some(manager.get_input_method(seat, qh, ()));
}

smithay_client_toolkit::reexports::client::delegate_noop!(State: ignore zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1);
smithay_client_toolkit::reexports::client::delegate_noop!(State: ignore zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1);
smithay_client_toolkit::reexports::client::delegate_noop!(State: ignore zwp_input_method_manager_v2::ZwpInputMethodManagerV2);

impl Dispatch<zwp_input_method_v2::ZwpInputMethodV2, ()> for State {
    fn event(
        st: &mut Self,
        _: &zwp_input_method_v2::ZwpInputMethodV2,
        event: zwp_input_method_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwp_input_method_v2::Event;
        match event {
            Event::Activate => st.vk.im_pending = true,
            Event::Deactivate => st.vk.im_pending = false,
            Event::Done => crate::set_input_method_active(st.vk.im_pending),
            Event::Unavailable => {
                log::warn!("input-method уже занят другим клиентом");
                st.vk.im = None;
            }
            _ => {}
        }
    }
}

#[allow(dead_code)]
fn _seat_type_check(_: &wl_seat::WlSeat) {}
