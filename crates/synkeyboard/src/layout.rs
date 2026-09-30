//! Раскладки и клавиши. Коды — evdev (`KEY_*` из linux/input-event-codes.h):
//! композитор получает их через zwp_virtual_keyboard_v1 и сам считает xkb.

pub const KEY_ESC: u32 = 1;
pub const KEY_BACKSPACE: u32 = 14;
pub const KEY_TAB: u32 = 15;
pub const KEY_ENTER: u32 = 28;
pub const KEY_LEFTCTRL: u32 = 29;
pub const KEY_LEFTSHIFT: u32 = 42;
pub const KEY_LEFTALT: u32 = 56;
pub const KEY_SPACE: u32 = 57;
pub const KEY_F1: u32 = 59; // F1..F10 = 59..68, F11 = 87, F12 = 88
pub const KEY_F11: u32 = 87;
pub const KEY_F12: u32 = 88;
pub const KEY_HOME: u32 = 102;
pub const KEY_UP: u32 = 103;
pub const KEY_PAGEUP: u32 = 104;
pub const KEY_LEFT: u32 = 105;
pub const KEY_RIGHT: u32 = 106;
pub const KEY_END: u32 = 107;
pub const KEY_DOWN: u32 = 108;
pub const KEY_PAGEDOWN: u32 = 109;
pub const KEY_INSERT: u32 = 110;
pub const KEY_DELETE: u32 = 111;
pub const KEY_LEFTMETA: u32 = 125;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modifier {
    Shift,
    Ctrl,
    Alt,
    Super,
}

impl Modifier {
    /// Маска модификатора в keymap xkb (индексы Shift/Control/Mod1/Mod4).
    pub fn mask(self) -> u32 {
        match self {
            Modifier::Shift => 1 << 0,
            Modifier::Ctrl => 1 << 2,
            Modifier::Alt => 1 << 3,
            Modifier::Super => 1 << 6,
        }
    }
    pub fn code(self) -> u32 {
        match self {
            Modifier::Shift => KEY_LEFTSHIFT,
            Modifier::Ctrl => KEY_LEFTCTRL,
            Modifier::Alt => KEY_LEFTALT,
            Modifier::Super => KEY_LEFTMETA,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Letters,
    Symbols,
    Symbols2,
}

/// Что делает клавиша.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    /// Обычная клавиша (evdev-код); `shift` — послать с Shift (символы вроде `!`);
    /// `latin` — символ US-раскладки: нажимается в группе 0 при любом языке,
    /// иначе в RU код `.` дал бы «ю», а `/` — «.».
    Key { code: u32, shift: bool, repeat: bool, latin: bool },
    Modifier(Modifier),
    Page(Page),
    /// Следующая раскладка (EN → RU → …).
    Layout,
    /// Показать/спрятать ряды Esc/Tab/стрелки/F1–F12.
    Fn,
    Hide,
    /// Пустое место (выравнивание коротких рядов).
    Spacer,
}

#[derive(Debug, Clone)]
pub struct Key {
    pub label: String,
    /// Подпись при активном Shift/Caps (для букв — заглавная).
    pub shifted: Option<String>,
    pub action: Action,
    /// Ширина в долях обычной клавиши.
    pub width: f32,
    pub class: &'static str,
}

impl Key {
    fn plain(label: &str, code: u32) -> Self {
        Self { label: label.into(), shifted: None, action: Action::Key { code, shift: false, repeat: false, latin: true }, width: 1.0, class: "key" }
    }
    fn letter(label: &str, upper: &str, code: u32) -> Self {
        Self { label: label.into(), shifted: Some(upper.into()), action: Action::Key { code, shift: false, repeat: false, latin: false }, width: 1.0, class: "key" }
    }
    fn shifted(label: &str, code: u32) -> Self {
        Self { label: label.into(), shifted: None, action: Action::Key { code, shift: true, repeat: false, latin: true }, width: 1.0, class: "key" }
    }
    fn special(label: &str, action: Action, width: f32) -> Self {
        Self { label: label.into(), shifted: None, action, width, class: "key key-special" }
    }
    fn repeat(label: &str, code: u32, width: f32) -> Self {
        Self { label: label.into(), shifted: None, action: Action::Key { code, shift: false, repeat: true, latin: false }, width, class: "key key-special" }
    }
}

/// Язык: имя, группа в keymap `us,ru` и три ряда букв по физическим клавишам.
pub struct Lang {
    pub name: &'static str,
    /// Имя раскладки xkb (`us`, `ru`).
    pub xkb: &'static str,
    pub group: u32,
    pub rows: [&'static str; 3],
}

pub const LANGS: &[Lang] = &[
    Lang { name: "EN", xkb: "us", group: 0, rows: ["qwertyuiop", "asdfghjkl", "zxcvbnm"] },
    Lang { name: "RU", xkb: "ru", group: 1, rows: ["йцукенгшщзх", "фывапролджэ", "ячсмитьбю"] },
];

/// evdev-коды физических клавиш трёх буквенных рядов (QWERTY): q-p-[ , a-l-;-' , z-m-,-.
const ROW_CODES: [&[u32]; 3] = [
    &[16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26],
    &[30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40],
    &[44, 45, 46, 47, 48, 49, 50, 51, 52],
];

/// Символ US-раскладки → (код, нужен ли Shift).
pub fn us_char(c: char) -> Option<(u32, bool)> {
    let s = |code| Some((code, true));
    let p = |code| Some((code, false));
    match c {
        '1'..='9' => p(c as u32 - '1' as u32 + 2),
        '0' => p(11),
        '!' => s(2), '@' => s(3), '#' => s(4), '$' => s(5), '%' => s(6), '^' => s(7), '&' => s(8), '*' => s(9), '(' => s(10), ')' => s(11),
        '-' => p(12), '_' => s(12), '=' => p(13), '+' => s(13),
        '[' => p(26), '{' => s(26), ']' => p(27), '}' => s(27),
        ';' => p(39), ':' => s(39), '\'' => p(40), '"' => s(40), '`' => p(41), '~' => s(41),
        '\\' => p(43), '|' => s(43), ',' => p(51), '<' => s(51), '.' => p(52), '>' => s(52), '/' => p(53), '?' => s(53),
        ' ' => p(KEY_SPACE), '\n' => p(KEY_ENTER), '\t' => p(KEY_TAB),
        'a'..='z' => {
            let i = "qwertyuiopasdfghjklzxcvbnm".find(c)?;
            let code = if i < 10 { ROW_CODES[0][i] } else if i < 19 { ROW_CODES[1][i - 10] } else { ROW_CODES[2][i - 19] };
            p(code)
        }
        'A'..='Z' => us_char(c.to_ascii_lowercase()).map(|(code, _)| (code, true)),
        _ => None,
    }
}

fn sym(label: &str) -> Key {
    let c = label.chars().next().unwrap();
    match us_char(c) {
        Some((code, true)) => Key::shifted(label, code),
        Some((code, false)) => Key::plain(label, code),
        None => Key::plain(label, KEY_SPACE),
    }
}

/// Число колонок языка (самый широкий ряд букв): каждая раскладка заполняет
/// всю ширину, ряды внутри неё выровнены по самому длинному.
fn columns(lang: &Lang) -> f32 {
    lang.rows.iter().map(|r| r.chars().count() as f32).fold(10.0, f32::max)
}

fn units(row: &[Key]) -> f32 {
    row.iter().map(|k| k.width).sum::<f32>()
}

/// Ряды одинаковой ширины `max`: короткие ряды получают пустые поля по краям,
/// как у Gboard, чтобы клавиши во всех рядах были одной ширины.
fn pad_rows(rows: &mut [Vec<Key>], max: f32) {
    for row in rows.iter_mut() {
        let extra = max - units(row);
        if extra > 0.05 {
            let spacer = || Key { label: String::new(), shifted: None, action: Action::Spacer, width: extra / 2.0, class: "key-spacer" };
            row.insert(0, spacer());
            row.push(spacer());
        }
    }
}

/// Ряды страницы для языка `lang`, с учётом Shift/Caps для подписей.
pub fn rows(page: Page, lang: &Lang) -> Vec<Vec<Key>> {
    let mut out = page_rows(page, lang);
    pad_rows(&mut out[..3], columns(lang));
    out
}

fn page_rows(page: Page, lang: &Lang) -> Vec<Vec<Key>> {
    let mut out: Vec<Vec<Key>> = Vec::new();
    match page {
        Page::Letters => {
            for (r, letters) in lang.rows.iter().enumerate() {
                let mut row: Vec<Key> = letters
                    .chars()
                    .zip(ROW_CODES[r].iter())
                    .map(|(ch, code)| Key::letter(&ch.to_string(), &ch.to_uppercase().to_string(), *code))
                    .collect();
                if r == 2 {
                    // ⇧ и ⌫ добирают ширину ряда до верхних, но не шире их.
                    let top = out.iter().map(|r| units(r)).fold(0.0, f32::max);
                    let side = ((top - units(&row)) / 2.0).max(1.0);
                    row.insert(0, Key::special("⇧", Action::Modifier(Modifier::Shift), side));
                    row.push(Key::repeat("⌫", KEY_BACKSPACE, side));
                }
                out.push(row);
            }
        }
        Page::Symbols => {
            out.push("1234567890".chars().map(|c| sym(&c.to_string())).collect());
            out.push("@#$_&-+()/".chars().map(|c| sym(&c.to_string())).collect());
            let mut row: Vec<Key> = vec![Key::special("=\\<", Action::Page(Page::Symbols2), 1.5)];
            row.extend("*\"':;!?".chars().map(|c| sym(&c.to_string())));
            row.push(Key::repeat("⌫", KEY_BACKSPACE, 1.5));
            out.push(row);
        }
        Page::Symbols2 => {
            out.push("~`|•√π÷×¶∆".chars().map(|c| sym(&c.to_string())).collect());
            out.push("£€$¢^°={}".chars().map(|c| sym(&c.to_string())).collect());
            let mut row: Vec<Key> = vec![Key::special("?123", Action::Page(Page::Symbols), 1.5)];
            row.extend("\\©®™%[]".chars().map(|c| sym(&c.to_string())));
            row.push(Key::repeat("⌫", KEY_BACKSPACE, 1.5));
            out.push(row);
        }
    }
    // Нижний ряд.
    let page_key = match page {
        Page::Letters => Key::special("?123", Action::Page(Page::Symbols), 1.5),
        _ => Key::special("ABC", Action::Page(Page::Letters), 1.5),
    };
    out.push(vec![
        Key::special("⌄", Action::Hide, 1.0),
        Key { width: 1.3, ..page_key },
        Key::special("⌨", Action::Fn, 1.0),
        Key::special(lang.name, Action::Layout, 1.0),
        Key::special("", Action::Key { code: KEY_SPACE, shift: false, repeat: false, latin: false }, 3.2),
        sym("."),
        Key::special("⏎", Action::Key { code: KEY_ENTER, shift: false, repeat: false, latin: false }, 1.5),
    ]);
    out
}

/// Функциональные ряды: Esc/Tab/модификаторы/навигация, затем F1–F6 и
/// F7–F12 со стрелками перевёрнутой «T» справа, как на клавиатуре.
pub fn fn_rows() -> Vec<Vec<Key>> {
    fn k(label: &str, code: u32, width: f32, repeat: bool) -> Key {
        Key { label: label.into(), shifted: None, action: Action::Key { code, shift: false, repeat, latin: false }, width, class: "key key-fn" }
    }
    fn f(n: u32) -> Key {
        let code = match n {
            1..=10 => KEY_F1 + n - 1,
            11 => KEY_F11,
            _ => KEY_F12,
        };
        Key { label: format!("F{n}"), shifted: None, action: Action::Key { code, shift: false, repeat: false, latin: false }, width: 1.0, class: "key key-fn key-f" }
    }
    fn m(label: &str, m: Modifier, width: f32) -> Key {
        Key { label: label.into(), shifted: None, action: Action::Modifier(m), width, class: "key key-fn" }
    }
    let row1 = vec![
        k("Esc", KEY_ESC, 1.0, false),
        k("Tab", KEY_TAB, 1.0, false),
        m("Ctrl", Modifier::Ctrl, 1.1),
        m("Alt", Modifier::Alt, 1.0),
        m("⌘", Modifier::Super, 0.8),
        k("Ins", KEY_INSERT, 0.9, false),
        k("Del", KEY_DELETE, 0.9, true),
        k("PgUp", KEY_PAGEUP, 1.5, true),
        k("PgDn", KEY_PAGEDOWN, 1.5, true),
    ];
    let mut row2: Vec<Key> = (1..=6).map(f).collect();
    row2.extend([k("Home", KEY_HOME, 1.0, false), k("↑", KEY_UP, 1.0, true), k("End", KEY_END, 1.0, false)]);
    let mut row3: Vec<Key> = (7..=12).map(f).collect();
    row3.extend([k("←", KEY_LEFT, 1.0, true), k("↓", KEY_DOWN, 1.0, true), k("→", KEY_RIGHT, 1.0, true)]);
    vec![row1, row2, row3]
}

/// Клавиша по имени для CLI (`synkeyboard key ctrl+c`): `a`…`z`, `0`…`9`,
/// `f1`…`f12`, `enter`, `tab`, `esc`, `space`, `backspace`, `del`, стрелки…
pub fn key_by_name(name: &str) -> Option<u32> {
    let code = match name {
        "enter" | "return" => KEY_ENTER,
        "tab" => KEY_TAB,
        "esc" | "escape" => KEY_ESC,
        "space" => KEY_SPACE,
        "backspace" | "bs" => KEY_BACKSPACE,
        "delete" | "del" => KEY_DELETE,
        "insert" | "ins" => KEY_INSERT,
        "home" => KEY_HOME,
        "end" => KEY_END,
        "pageup" | "pgup" => KEY_PAGEUP,
        "pagedown" | "pgdn" => KEY_PAGEDOWN,
        "left" => KEY_LEFT,
        "right" => KEY_RIGHT,
        "up" => KEY_UP,
        "down" => KEY_DOWN,
        "f11" => KEY_F11,
        "f12" => KEY_F12,
        _ => {
            if let Some(n) = name.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()) {
                if (1..=10).contains(&n) {
                    return Some(KEY_F1 + n - 1);
                }
                return None;
            }
            let mut chars = name.chars();
            let (c, None) = (chars.next()?, chars.next()) else { return None };
            return us_char(c).map(|(code, _)| code);
        }
    };
    Some(code)
}
