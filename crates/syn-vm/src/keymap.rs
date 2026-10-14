//! Клавиши для гостя: физический код evdev → номер QEMU (XT set 1, префикс E0 — бит 0x80),
//! который уходит расширенным событием клавиатуры VNC. Гость получает клавишу, а не
//! символ, — раскладку выбирает он сам.

use syngui::input::Key;

/// Код evdev → номер QEMU (qnum).
pub fn qnum(evdev: u32) -> Option<u32> {
    Some(match evdev {
        // ESC…KP. совпадают с XT set 1.
        1..=83 => evdev,
        85 => 0x76,
        86 => 0x56,
        87 => 0x57,
        88 => 0x58,
        89 => 0x73,
        92 => 0x79,
        93 => 0x70,
        94 => 0x7b,
        96 => 0x9c,
        97 => 0x9d,
        98 => 0xb5,
        99 => 0xb7,
        100 => 0xb8,
        102 => 0xc7,
        103 => 0xc8,
        104 => 0xc9,
        105 => 0xcb,
        106 => 0xcd,
        107 => 0xcf,
        108 => 0xd0,
        109 => 0xd1,
        110 => 0xd2,
        111 => 0xd3,
        113 => 0xa0,
        114 => 0xae,
        115 => 0xb0,
        116 => 0xde,
        117 => 0x59,
        119 => 0xc6,
        124 => 0x7d,
        125 => 0xdb,
        126 => 0xdc,
        127 => 0xdd,
        163 => 0x99,
        164 => 0xa2,
        165 => 0x90,
        166 => 0xa4,
        _ => return None,
    })
}

/// Код evdev по `Key` — когда окно не дало физического кода (Android, экранная клавиатура).
pub fn evdev_of(k: Key) -> Option<u32> {
    use Key::*;
    Some(match k {
        A => 30, B => 48, C => 46, D => 32, E => 18, F => 33, G => 34, H => 35, I => 23, J => 36, K => 37, L => 38, M => 50,
        N => 49, O => 24, P => 25, Q => 16, R => 19, S => 31, T => 20, U => 22, V => 47, W => 17, X => 45, Y => 21, Z => 44,
        Num1 => 2, Num2 => 3, Num3 => 4, Num4 => 5, Num5 => 6, Num6 => 7, Num7 => 8, Num8 => 9, Num9 => 10, Num0 => 11,
        F1 => 59, F2 => 60, F3 => 61, F4 => 62, F5 => 63, F6 => 64, F7 => 65, F8 => 66, F9 => 67, F10 => 68, F11 => 87, F12 => 88,
        Escape => 1, Enter => 28, Tab => 15, Backspace => 14, Delete => 111, Insert => 110, Home => 102, End => 107,
        PageUp => 104, PageDown => 109, Left => 105, Right => 106, Up => 103, Down => 108,
        Shift => 42, Ctrl => 29, Alt => 56, Meta => 125, Space => 57, ContextMenu => 127,
        MediaPlayPause => 164, MediaStop => 166, MediaNext => 163, MediaPrevious => 165,
        _ => return None,
    })
}

/// X11 keysym для обычного (не расширенного) события — запасной путь.
pub fn keysym_of(k: Key) -> u32 {
    use Key::*;
    match k {
        A | B | C | D | E | F | G | H | I | J | K | L | M | N | O | P | Q | R | S | T | U | V | W | X | Y | Z => {
            let i = [A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z].iter().position(|x| *x == k).unwrap_or(0);
            0x61 + i as u32
        }
        Num0 => 0x30,
        Num1 => 0x31,
        Num2 => 0x32,
        Num3 => 0x33,
        Num4 => 0x34,
        Num5 => 0x35,
        Num6 => 0x36,
        Num7 => 0x37,
        Num8 => 0x38,
        Num9 => 0x39,
        Space => 0x20,
        Escape => 0xff1b,
        Enter => 0xff0d,
        Tab => 0xff09,
        Backspace => 0xff08,
        Delete => 0xffff,
        Insert => 0xff63,
        Home => 0xff50,
        End => 0xff57,
        PageUp => 0xff55,
        PageDown => 0xff56,
        Left => 0xff51,
        Up => 0xff52,
        Right => 0xff53,
        Down => 0xff54,
        Shift => 0xffe1,
        Ctrl => 0xffe3,
        Alt => 0xffe9,
        Meta => 0xffeb,
        F1 => 0xffbe,
        F2 => 0xffbf,
        F3 => 0xffc0,
        F4 => 0xffc1,
        F5 => 0xffc2,
        F6 => 0xffc3,
        F7 => 0xffc4,
        F8 => 0xffc5,
        F9 => 0xffc6,
        F10 => 0xffc7,
        F11 => 0xffc8,
        F12 => 0xffc9,
        ContextMenu => 0xff67,
        _ => 0,
    }
}

/// Сочетания с панели: Ctrl+Alt+Del и прочие, которые хост перехватил бы сам.
pub const CTRL_ALT_DEL: &[u32] = &[0x1d, 0x38, 0xd3];
pub const CTRL_ALT_BACKSPACE: &[u32] = &[0x1d, 0x38, 0x0e];
pub const CTRL_ALT_F1: &[u32] = &[0x1d, 0x38, 0x3b];
pub const CTRL_ALT_F2: &[u32] = &[0x1d, 0x38, 0x3c];
pub const CTRL_ALT_F7: &[u32] = &[0x1d, 0x38, 0x41];
pub const SUPER: &[u32] = &[0xdb];
pub const PRINT: &[u32] = &[0xb7];
pub const ALT_TAB: &[u32] = &[0x38, 0x0f];
pub const ALT_F4: &[u32] = &[0x38, 0x3e];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map() {
        assert_eq!(qnum(1), Some(1));
        assert_eq!(qnum(30), Some(0x1e));
        assert_eq!(qnum(103), Some(0xc8));
        assert_eq!(qnum(97), Some(0x9d));
        assert_eq!(qnum(200), None);
        assert_eq!(keysym_of(Key::C), 0x63);
        assert_eq!(evdev_of(Key::Enter).and_then(qnum), Some(28));
    }
}
