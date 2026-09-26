//! Клавиши Wayland → `syngui::input::Key`.
//!
//! Буквы и цифры — по физическому коду evdev (Ctrl+C работает в любой
//! раскладке, как `PhysicalKey` у winit), остальное — по keysym xkb.

use smithay_client_toolkit::seat::keyboard::Keysym;
use syngui::input::Key;

/// `raw_code` из sctk — это код evdev (без +8).
pub fn map_key(raw_code: u32, keysym: Keysym) -> Key {
    if let Some(k) = map_evdev(raw_code) {
        return k;
    }
    map_keysym(keysym).unwrap_or(Key::Unknown(keysym.raw()))
}

fn map_evdev(code: u32) -> Option<Key> {
    use Key::*;
    Some(match code {
        2 => Num1,
        3 => Num2,
        4 => Num3,
        5 => Num4,
        6 => Num5,
        7 => Num6,
        8 => Num7,
        9 => Num8,
        10 => Num9,
        11 => Num0,
        16 => Q,
        17 => W,
        18 => E,
        19 => R,
        20 => T,
        21 => Y,
        22 => U,
        23 => I,
        24 => O,
        25 => P,
        30 => A,
        31 => S,
        32 => D,
        33 => F,
        34 => G,
        35 => H,
        36 => J,
        37 => K,
        38 => L,
        44 => Z,
        45 => X,
        46 => C,
        47 => V,
        48 => B,
        49 => N,
        50 => M,
        _ => return None,
    })
}

fn map_keysym(k: Keysym) -> Option<Key> {
    use Key::*;
    Some(match k {
        Keysym::Escape => Escape,
        Keysym::Return | Keysym::KP_Enter => Enter,
        Keysym::Tab | Keysym::ISO_Left_Tab => Tab,
        Keysym::BackSpace => Backspace,
        Keysym::Delete | Keysym::KP_Delete => Delete,
        Keysym::Insert | Keysym::KP_Insert => Insert,
        Keysym::Home | Keysym::KP_Home => Home,
        Keysym::End | Keysym::KP_End => End,
        Keysym::Page_Up | Keysym::KP_Page_Up => PageUp,
        Keysym::Page_Down | Keysym::KP_Page_Down => PageDown,
        Keysym::Left | Keysym::KP_Left => Left,
        Keysym::Right | Keysym::KP_Right => Right,
        Keysym::Up | Keysym::KP_Up => Up,
        Keysym::Down | Keysym::KP_Down => Down,
        Keysym::Shift_L | Keysym::Shift_R => Shift,
        Keysym::Control_L | Keysym::Control_R => Ctrl,
        Keysym::Alt_L | Keysym::Alt_R => Alt,
        Keysym::Super_L | Keysym::Super_R | Keysym::Meta_L | Keysym::Meta_R => Meta,
        Keysym::space => Space,
        Keysym::Menu => ContextMenu,
        Keysym::F1 => F1,
        Keysym::F2 => F2,
        Keysym::F3 => F3,
        Keysym::F4 => F4,
        Keysym::F5 => F5,
        Keysym::F6 => F6,
        Keysym::F7 => F7,
        Keysym::F8 => F8,
        Keysym::F9 => F9,
        Keysym::F10 => F10,
        Keysym::F11 => F11,
        Keysym::F12 => F12,
        Keysym::XF86_AudioPlay | Keysym::XF86_AudioPause => MediaPlayPause,
        Keysym::XF86_AudioStop => MediaStop,
        Keysym::XF86_AudioNext => MediaNext,
        Keysym::XF86_AudioPrev => MediaPrevious,
        _ => return None,
    })
}

/// Курсор syngui → курсор sctk (имена по спецификации CSS/cursor-shape).
pub fn map_cursor(c: syngui::input::CursorIcon) -> smithay_client_toolkit::seat::pointer::CursorIcon {
    use smithay_client_toolkit::seat::pointer::CursorIcon as S;
    use syngui::input::CursorIcon as C;
    match c {
        C::Default => S::Default,
        C::Pointer => S::Pointer,
        C::Text => S::Text,
        C::Grab => S::Grab,
        C::Grabbing => S::Grabbing,
        C::Move => S::Move,
        C::NotAllowed => S::NotAllowed,
        C::Crosshair => S::Crosshair,
        C::ColResize => S::ColResize,
        C::RowResize => S::RowResize,
        C::NwResize => S::NwResize,
        C::NeResize => S::NeResize,
        _ => S::Default,
    }
}
