//! Имена клавиш → коды evdev (`KEY_*` из linux/input-event-codes.h). Клавиши идут композитору
//! через zwp_virtual_keyboard_v1 с раскладкой `us`: код — физическая клавиша, не символ.

/// Код клавиши по имени: `a`…`z`, `0`…`9`, `f1`…`f12`, `space`, `enter`, `esc`, `tab`,
/// `backspace`, `shift`, `ctrl`, `alt`, `super`, стрелки `up`/`down`/`left`/`right` и прочие ниже.
pub fn code(name: &str) -> Option<u32> {
    let n = name.trim().to_ascii_lowercase();
    let c = match n.as_str() {
        "esc" | "escape" => 1,
        "minus" | "-" => 12,
        "equal" | "=" => 13,
        "backspace" | "bs" => 14,
        "tab" => 15,
        "leftbrace" | "[" => 26,
        "rightbrace" | "]" => 27,
        "enter" | "return" => 28,
        "ctrl" | "control" | "leftctrl" => 29,
        "semicolon" | ";" => 39,
        "apostrophe" | "'" => 40,
        "grave" | "`" | "~" => 41,
        "shift" | "leftshift" => 42,
        "backslash" | "\\" => 43,
        "comma" | "," => 51,
        "dot" | "." => 52,
        "slash" | "/" => 53,
        "rightshift" => 54,
        "alt" | "leftalt" => 56,
        "space" => 57,
        "capslock" | "caps" => 58,
        "rightctrl" => 97,
        "rightalt" | "altgr" => 100,
        "home" => 102,
        "up" => 103,
        "pageup" | "pgup" => 104,
        "left" => 105,
        "right" => 106,
        "end" => 107,
        "down" => 108,
        "pagedown" | "pgdn" => 109,
        "insert" | "ins" => 110,
        "delete" | "del" => 111,
        "super" | "meta" | "win" => 125,
        "f11" => 87,
        "f12" => 88,
        _ => {
            if let Some(k) = n.strip_prefix('f').and_then(|k| k.parse::<u32>().ok()) {
                return (1..=10).contains(&k).then_some(58 + k);
            }
            let mut it = n.chars();
            let (Some(ch), None) = (it.next(), it.next()) else { return None };
            return letter_or_digit(ch);
        }
    };
    Some(c)
}

fn letter_or_digit(c: char) -> Option<u32> {
    const ROW1: &str = "qwertyuiop"; // 16..25
    const ROW2: &str = "asdfghjkl"; // 30..38
    const ROW3: &str = "zxcvbnm"; // 44..50
    if let Some(i) = ROW1.find(c) {
        return Some(16 + i as u32);
    }
    if let Some(i) = ROW2.find(c) {
        return Some(30 + i as u32);
    }
    if let Some(i) = ROW3.find(c) {
        return Some(44 + i as u32);
    }
    match c {
        '1'..='9' => Some(2 + (c as u32 - '1' as u32)),
        '0' => Some(11),
        _ => None,
    }
}

/// Бит модификатора xkb (раскладка `us`) для клавиши-модификатора.
pub fn modifier_mask(code: u32) -> u32 {
    match code {
        42 | 54 => 1,  // Shift
        29 | 97 => 4,  // Control
        56 | 100 => 8, // Mod1 (Alt)
        125 => 64,     // Mod4 (Super)
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(code("w"), Some(17));
        assert_eq!(code("A"), Some(30));
        assert_eq!(code("1"), Some(2));
        assert_eq!(code("0"), Some(11));
        assert_eq!(code("f1"), Some(59));
        assert_eq!(code("f10"), Some(68));
        assert_eq!(code("f12"), Some(88));
        assert_eq!(code("space"), Some(57));
        assert_eq!(code("m"), Some(50));
        assert_eq!(code("nope"), None);
    }
}
