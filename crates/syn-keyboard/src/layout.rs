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
pub const KEY_V: u32 = 47;

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
    /// Буквы: 3 ряда букв (по ~10 широких клавиш) и нижний ряд.
    Letters,
    /// Цифры и основные знаки (`?123`).
    Symbols,
    /// Остальные знаки (`=\<`): `~ | • √ € © …`.
    Symbols2,
    /// Смайлы (долгое нажатие запятой или клавиша на странице знаков).
    Emoji,
}

/// Что делает клавиша.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    /// Обычная клавиша (evdev-код); `shift` — послать с Shift (символы вроде `!`);
    /// `latin` — символ US-раскладки: нажимается в группе 0 при любом языке,
    /// иначе в RU код `.` дал бы «ю», а `/` — «.».
    Key { code: u32, shift: bool, repeat: bool, latin: bool },
    /// Символ вне раскладки (смайлик, `€`, `√`): печатается через временную клавишу keymap.
    Char(char),
    Modifier(Modifier),
    Page(Page),
    /// Следующая раскладка (EN → RU → …).
    Layout,
    /// Показать/спрятать ряды F1–F12, Ins/Del/Home/End/PgUp/PgDn.
    Fn,
    /// Вставить из буфера обмена (Ctrl+V; терминалу — Ctrl+Shift+V; при захвате — текстом).
    Paste,
    Hide,
    /// Плавающая ↔ внизу.
    Float,
    /// Пустое место (выравнивание коротких рядов).
    Spacer,
}

#[derive(Debug, Clone)]
pub struct Key {
    pub label: String,
    /// Подпись при активном Shift/Caps (у букв — заглавная).
    pub shifted: Option<String>,
    /// Мелкая подпись в углу — что даёт долгое нажатие (цифра над q…p, смайлик у запятой).
    pub hint: Option<String>,
    pub action: Action,
    /// Ширина в долях обычной клавиши.
    pub width: f32,
    pub class: &'static str,
    /// Варианты по долгому нажатию (полоска над клавишей, выбор скольжением):
    /// цифры и символы у букв, ё/ъ, знаки препинания у точки.
    pub alts: Vec<String>,
    /// Действие по долгому нажатию вместо полоски вариантов (запятая → смайлы).
    pub long: Option<Action>,
}

impl Key {
    fn new(label: &str, action: Action, width: f32, class: &'static str) -> Self {
        Self { label: label.into(), shifted: None, hint: None, action, width, class, alts: Vec::new(), long: None }
    }
    fn letter(label: &str, upper: &str, code: u32) -> Self {
        Self { shifted: Some(upper.into()), ..Self::new(label, Action::Key { code, shift: false, repeat: false, latin: false }, 1.0, "key") }
    }
    fn special(label: &str, action: Action, width: f32) -> Self {
        Self::new(label, action, width, "key key-special")
    }
    fn repeat(label: &str, code: u32, width: f32) -> Self {
        Self::new(label, Action::Key { code, shift: false, repeat: true, latin: false }, width, "key key-special")
    }
    fn alts(mut self, list: &[&str]) -> Self {
        self.alts = list.iter().map(|s| s.to_string()).collect();
        self
    }
    fn with_alts(mut self, list: Vec<String>) -> Self {
        self.alts = list;
        self
    }
    fn hint(mut self, h: &str) -> Self {
        self.hint = Some(h.into());
        self
    }
    fn wide(mut self, width: f32) -> Self {
        self.width = width;
        self
    }
    /// Печатный символ: цифры, буквы и знаки (подпись нажатой — над пальцем).
    pub fn is_char(&self) -> bool {
        self.class == "key" && matches!(self.action, Action::Key { .. } | Action::Char(_))
    }
    /// Буква: Caps Lock делает её заглавной (цифры и знаки он не трогает).
    pub fn is_letter(&self) -> bool {
        matches!(self.action, Action::Key { latin: false, .. }) && self.shifted.is_some()
    }
}

/// Варианты буквы по долгому нажатию: цифра у верхнего ряда, символы у среднего
/// и нижнего — первым (его и даёт удержание без сдвига пальца); у «е» — «ё», у «ь» — «ъ».
fn letter_alts(row: usize, i: usize, ch: char) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let sym = match row {
        0 => "1234567890".chars().nth(i),
        1 => "@#$_&-+()".chars().nth(i),
        _ => "*\"':;!?".chars().nth(i),
    };
    match ch {
        'е' => out.push("ё".into()),
        'ь' => out.push("ъ".into()),
        _ => {}
    }
    if let Some(s) = sym {
        out.push(s.to_string());
    }
    out
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

/// Значки Material Icons (частная область Юникода — рисуются значком, не текстом).
pub const FLOAT_ICON: &str = "\u{E89F}";
pub const LANG_ICON: &str = "\u{E894}";
pub const HIDE_ICON: &str = "\u{E31A}";
pub const FN_ICON: &str = "\u{E312}";
pub const BACKSPACE_ICON: &str = "\u{E14A}";
pub const ENTER_ICON: &str = "\u{E31B}";
pub const SHIFT_ICON: &str = "\u{E5D8}";
/// «Вставить» (content_paste).
pub const PASTE_ICON: &str = "\u{E14F}";
const UP_ICON: &str = "\u{E316}";
const DOWN_ICON: &str = "\u{E313}";
const LEFT_ICON: &str = "\u{E314}";
const RIGHT_ICON: &str = "\u{E315}";

/// Смайлы страницы [`Page::Emoji`]: три ряда по 10 (в последнем — 8 и ⌫).
const EMOJI_ROWS: [&str; 3] = ["😀😂🤣😊😍😘😎🤔😉🙂", "😅😭😢😡😱🥳🤗🙄👍👎", "👌🙏👏💪🔥🎉💯💖"];

/// Имя клавиши для программы, принимающей клавиши вместо виртуальной клавиатуры (`show … for <сокет>`):
/// клавиши без символа. `None` — клавиша такой программе не нужна.
pub fn key_name(code: u32) -> Option<&'static str> {
    Some(match code {
        KEY_BACKSPACE => "backspace",
        KEY_ENTER => "enter",
        KEY_SPACE => "space",
        KEY_TAB => "tab",
        KEY_ESC => "escape",
        KEY_LEFT => "left",
        KEY_RIGHT => "right",
        KEY_UP => "up",
        KEY_DOWN => "down",
        KEY_HOME => "home",
        KEY_END => "end",
        KEY_DELETE => "delete",
        _ => return None,
    })
}

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

/// Терминал ли это по app_id: вставка в нём — Ctrl+Shift+V, а не Ctrl+V.
pub fn is_terminal(app_id: &str) -> bool {
    let id = app_id.to_ascii_lowercase();
    ["term", "foot", "kitty", "alacritty", "konsole", "ghostty", "tilix", "terminator", "st-256color"].iter().any(|t| id.contains(t))
}

/// Клавиша знака: US-раскладка (группа 0) или, вне её, — символ временной клавишей.
fn sym(c: char) -> Key {
    match us_char(c) {
        Some((code, shift)) => Key::new(&c.to_string(), Action::Key { code, shift, repeat: false, latin: true }, 1.0, "key"),
        None => Key::new(&c.to_string(), Action::Char(c), 1.0, "key"),
    }
}

fn syms(s: &str) -> Vec<Key> {
    s.chars().map(sym).collect()
}

/// Число колонок языка (самый длинный ряд букв, не меньше 10): ряды всех страниц этой ширины.
pub fn columns(lang: &Lang) -> f32 {
    lang.rows.iter().map(|r| r.chars().count() as f32).fold(10.0, f32::max)
}

fn units(row: &[Key]) -> f32 {
    row.iter().map(|k| k.width).sum::<f32>()
}

/// Ряды одинаковой ширины `max`: короткие ряды получают пустые поля по краям
/// (средний ряд букв — сдвиг на полклавиши), клавиши во всех рядах одной ширины.
fn pad_rows(rows: &mut [Vec<Key>], max: f32) {
    for row in rows.iter_mut() {
        let extra = max - units(row);
        if extra > 0.05 {
            let spacer = || Key::new("", Action::Spacer, extra / 2.0, "key-spacer");
            row.insert(0, spacer());
            row.push(spacer());
        }
    }
}

/// Ряд с ⌫ справа и клавишей `left` слева: края добирают ширину до `cols`.
fn edged(left: Key, mid: Vec<Key>, cols: f32) -> Vec<Key> {
    let side = ((cols - units(&mid)) / 2.0).max(1.0);
    let mut row = vec![left.wide(side)];
    row.extend(mid);
    row.push(Key::repeat(BACKSPACE_ICON, KEY_BACKSPACE, side));
    row
}

/// Ряды страницы `page` для языка `lang` (с нижним); `number_row` — ряд цифр над буквами.
pub fn rows(page: Page, lang: &Lang, number_row: bool) -> Vec<Vec<Key>> {
    let cols = columns(lang);
    let mut out: Vec<Vec<Key>> = Vec::new();
    match page {
        Page::Letters => {
            if number_row {
                out.push(syms("1234567890"));
            }
            for (r, letters) in lang.rows.iter().enumerate() {
                let keys: Vec<Key> = letters
                    .chars()
                    .zip(ROW_CODES[r].iter())
                    .enumerate()
                    .map(|(i, (ch, code))| {
                        let mut k = Key::letter(&ch.to_string(), &ch.to_uppercase().to_string(), *code).with_alts(letter_alts(r, i, ch));
                        // Без ряда цифр цифра — мелко в углу верхнего ряда (долгое нажатие).
                        if r == 0 && !number_row {
                            if let Some(d) = "1234567890".chars().nth(i) {
                                k.hint = Some(d.to_string());
                                k.alts.retain(|a| a != &d.to_string());
                                k.alts.insert(0, d.to_string());
                            }
                        }
                        k
                    })
                    .collect();
                out.push(if r == 2 { edged(Key::special(SHIFT_ICON, Action::Modifier(Modifier::Shift), 1.0), keys, cols) } else { keys });
            }
        }
        Page::Symbols => {
            out.push(syms("1234567890"));
            out.push(syms("@#$_&-+()/"));
            out.push(edged(Key::special("=\\<", Action::Page(Page::Symbols2), 1.0), syms("*\"':;!?"), cols));
        }
        Page::Symbols2 => {
            out.push(syms("~`|•√π÷×¶∆"));
            out.push(syms("£€¢^°={}<>"));
            out.push(edged(Key::special("?123", Action::Page(Page::Symbols), 1.0), syms("\\©®™%[]"), cols));
        }
        Page::Emoji => {
            for (i, s) in EMOJI_ROWS.iter().enumerate() {
                let mut row: Vec<Key> = s.chars().map(|c| Key::new(&c.to_string(), Action::Char(c), 1.0, "key")).collect();
                if i == 2 {
                    row.push(Key::repeat(BACKSPACE_ICON, KEY_BACKSPACE, 2.0));
                }
                out.push(row);
            }
        }
    }
    let n = out.len();
    pad_rows(&mut out[..n], cols);
    // Нижний ряд: слой (?123 / ABC), запятая (долгое — смайлы), язык, пробел с именем языка, точка, Enter.
    let layer = match page {
        Page::Letters => Key::special("?123", Action::Page(Page::Symbols), 1.5),
        _ => Key::special("ABC", Action::Page(Page::Letters), 1.5),
    };
    let comma = Key { long: Some(Action::Page(Page::Emoji)), ..sym(',') }.hint("☺");
    let space = Key::special(lang.name, Action::Key { code: KEY_SPACE, shift: false, repeat: false, latin: false }, cols - 6.0);
    out.push(vec![
        layer,
        comma,
        Key::special(LANG_ICON, Action::Layout, 1.0),
        space,
        sym('.').alts(&["!", "?", ",", ";", ":", "-", "\"", "'"]),
        Key::special(ENTER_ICON, Action::Key { code: KEY_ENTER, shift: false, repeat: false, latin: false }, 1.5),
    ]);
    out
}

/// Дополнительная полоса над клавишами (для терминала и программ): Esc, Tab, Ctrl, Alt, стрелки,
/// «Вставить», ещё ряды (F1–F12…), плавающая ↔ внизу, «Скрыть».
pub fn toolbar() -> Vec<Key> {
    let k = |label: &str, code: u32, repeat: bool| Key::new(label, Action::Key { code, shift: false, repeat, latin: false }, 1.0, "key key-fn");
    let s = |label: &str, action: Action| Key::new(label, action, 1.0, "key key-fn");
    vec![
        k("Esc", KEY_ESC, false),
        k("Tab", KEY_TAB, false),
        s("Ctrl", Action::Modifier(Modifier::Ctrl)),
        s("Alt", Action::Modifier(Modifier::Alt)),
        k(LEFT_ICON, KEY_LEFT, true),
        k(UP_ICON, KEY_UP, true),
        k(DOWN_ICON, KEY_DOWN, true),
        k(RIGHT_ICON, KEY_RIGHT, true),
        s(PASTE_ICON, Action::Paste),
        s(FN_ICON, Action::Fn),
        s(FLOAT_ICON, Action::Float),
        s(HIDE_ICON, Action::Hide),
    ]
}

/// Ещё ряды (кнопка ⌨ полосы): ⌘ Ins Del Home End PgUp PgDn, F1–F6, F7–F12.
pub fn fn_rows() -> Vec<Vec<Key>> {
    fn k(label: &str, code: u32, repeat: bool) -> Key {
        Key::new(label, Action::Key { code, shift: false, repeat, latin: false }, 1.0, "key key-fn")
    }
    fn f(n: u32) -> Key {
        let code = match n {
            1..=10 => KEY_F1 + n - 1,
            11 => KEY_F11,
            _ => KEY_F12,
        };
        Key::new(&format!("F{n}"), Action::Key { code, shift: false, repeat: false, latin: false }, 1.0, "key key-fn key-f")
    }
    let row1 = vec![
        Key::new("⌘", Action::Modifier(Modifier::Super), 1.0, "key key-fn"),
        k("Ins", KEY_INSERT, false),
        k("Del", KEY_DELETE, true),
        k("Home", KEY_HOME, false),
        k("End", KEY_END, false),
        k("PgUp", KEY_PAGEUP, true),
        k("PgDn", KEY_PAGEDOWN, true),
    ];
    vec![row1, (1..=6).map(f).collect(), (7..=12).map(f).collect()]
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


#[cfg(test)]
mod tests {
    use super::*;

    fn labels(row: &[Key]) -> Vec<String> {
        row.iter().filter(|k| k.action != Action::Spacer).map(|k| k.label.clone()).collect()
    }

    fn check_widths(rows: &[Vec<Key>], cols: f32) {
        for r in rows {
            assert!((units(r) - cols).abs() < 0.01, "ширина ряда {:?}: {}", labels(r), units(r));
        }
    }

    #[test]
    fn letters_en() {
        let rows = rows(Page::Letters, &LANGS[0], false);
        assert_eq!(rows.len(), 4);
        check_widths(&rows, 10.0);
        assert_eq!(labels(&rows[0]).concat(), "qwertyuiop");
        // Цифры — мелко над верхним рядом и первыми по долгому нажатию.
        assert_eq!(rows[0][0].hint.as_deref(), Some("1"));
        assert_eq!(rows[0][9].alts[0], "0");
        // Средний ряд — со сдвигом на полклавиши.
        assert_eq!(rows[1][0].action, Action::Spacer);
        assert!((rows[1][0].width - 0.5).abs() < 0.01);
        assert_eq!(rows[2][0].action, Action::Modifier(Modifier::Shift));
        assert_eq!(labels(&rows[2])[1..8].concat(), "zxcvbnm");
        assert!(matches!(rows[2][8].action, Action::Key { code: KEY_BACKSPACE, repeat: true, .. }));
        let bottom: Vec<Action> = rows[3].iter().map(|k| k.action).collect();
        assert_eq!(bottom[0], Action::Page(Page::Symbols));
        assert_eq!(rows[3][1].long, Some(Action::Page(Page::Emoji)));
        assert_eq!(bottom[2], Action::Layout);
        assert!(matches!(bottom[3], Action::Key { code: KEY_SPACE, .. }));
        assert_eq!(rows[3][3].label, "EN");
        assert!(rows[3][4].alts.contains(&"?".to_string()));
        assert!(matches!(bottom[5], Action::Key { code: KEY_ENTER, .. }));
        // Служебные — не в основных рядах.
        let all: Vec<Action> = rows.iter().flatten().map(|k| k.action).collect();
        assert!(!all.iter().any(|a| matches!(a, Action::Paste | Action::Hide | Action::Fn | Action::Key { code: KEY_TAB, .. })));
    }

    #[test]
    fn number_row() {
        let rows = rows(Page::Letters, &LANGS[0], true);
        assert_eq!(rows.len(), 5);
        check_widths(&rows, 10.0);
        assert_eq!(labels(&rows[0]).concat(), "1234567890");
        assert!(rows[1][0].hint.is_none());
    }

    #[test]
    fn letters_ru() {
        let rows = rows(Page::Letters, &LANGS[1], false);
        check_widths(&rows, 11.0);
        assert_eq!(labels(&rows[0]).concat(), "йцукенгшщзх");
        assert_eq!(labels(&rows[1]).concat(), "фывапролджэ");
        assert_eq!(labels(&rows[2])[1..10].concat(), "ячсмитьбю");
        assert_eq!(rows[0][4].alts, ["5", "ё"]);
        assert!(rows[2][7].alts.contains(&"ъ".to_string()));
        assert!(matches!(rows[0][10].action, Action::Key { code: 26, latin: false, .. }));
        assert_eq!(rows[3][3].label, "RU");
    }

    #[test]
    fn symbol_pages() {
        for lang in LANGS {
            for page in [Page::Symbols, Page::Symbols2, Page::Emoji] {
                let rows = rows(page, lang, false);
                assert_eq!(rows.len(), 4);
                check_widths(&rows, columns(lang));
                assert_eq!(rows[3][0].action, Action::Page(Page::Letters));
            }
        }
        let s2 = rows(Page::Symbols2, &LANGS[0], false);
        // Знаки вне US — временной клавишей, US — кодом.
        assert!(s2.iter().flatten().any(|k| k.action == Action::Char('€')));
        assert!(s2.iter().flatten().any(|k| matches!(k.action, Action::Key { code: 41, shift: true, latin: true, .. })));
        let e = rows(Page::Emoji, &LANGS[0], false);
        assert_eq!(e[0][0].action, Action::Char('😀'));
    }

    #[test]
    fn toolbar_has_terminal_keys() {
        let t: Vec<Action> = toolbar().iter().map(|k| k.action).collect();
        for want in [Action::Paste, Action::Hide, Action::Fn, Action::Float, Action::Modifier(Modifier::Ctrl), Action::Modifier(Modifier::Alt)] {
            assert!(t.contains(&want), "{want:?}");
        }
        for code in [KEY_ESC, KEY_TAB, KEY_LEFT, KEY_RIGHT, KEY_UP, KEY_DOWN] {
            assert!(t.iter().any(|a| matches!(a, Action::Key { code: c, .. } if *c == code)));
        }
    }

    #[test]
    fn terminals() {
        assert!(is_terminal("syn-terminal"));
        assert!(is_terminal("foot"));
        assert!(!is_terminal("syn-notes"));
    }
}
