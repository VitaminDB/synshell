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
    /// Буквы — раскладка как у экранной клавиатуры Steam (5 рядов: цифры, три ряда букв, нижний).
    Letters,
    /// Смайлы и знаки вне US-раскладки (•, √, €, °, ©…) — клавиша со смайликом.
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
    /// Caps Lock: буквы заглавными до повторного нажатия (цифры и знаки — без Shift).
    Caps,
    Page(Page),
    /// Следующая раскладка (EN → RU → …).
    Layout,
    /// Показать/спрятать ряды Esc/Tab/стрелки/F1–F12.
    Fn,
    /// Вставить из буфера обмена (Ctrl+V; при захвате — текстом).
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
    /// Подпись при активном Shift/Caps (для букв — заглавная, для цифр и знаков — символ с Shift).
    pub shifted: Option<String>,
    /// Мелкая подпись над основной — символ с Shift у цифр и знаков (как у клавиатуры Steam).
    /// Такие клавиши Caps Lock не трогает.
    pub hint: Option<String>,
    pub action: Action,
    /// Ширина в долях обычной клавиши.
    pub width: f32,
    pub class: &'static str,
    /// Варианты по долгому нажатию (полоска над клавишей, выбор скольжением):
    /// цифры и символы у букв, ё/ъ, знаки препинания у точки — как в Gboard.
    pub alts: Vec<String>,
    /// Вторая клавиша под этой в той же ячейке (половина высоты ряда): блок стрелок.
    pub below: Option<Box<Key>>,
}

impl Key {
    fn new(label: &str, action: Action, width: f32, class: &'static str) -> Self {
        Self { label: label.into(), shifted: None, hint: None, action, width, class, alts: Vec::new(), below: None }
    }
    fn letter(label: &str, upper: &str, code: u32) -> Self {
        Self { shifted: Some(upper.into()), ..Self::new(label, Action::Key { code, shift: false, repeat: false, latin: false }, 1.0, "key") }
    }
    /// Знак с символом Shift над ним: `latin` — из US-раскладки при любом языке.
    fn dual(label: &str, upper: &str, code: u32, latin: bool) -> Self {
        Self {
            shifted: Some(upper.into()),
            hint: Some(upper.into()),
            ..Self::new(label, Action::Key { code, shift: false, repeat: false, latin }, 1.0, "key")
        }
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
    fn over(mut self, below: Key) -> Self {
        self.below = Some(Box::new(below));
        self
    }
    /// Печатный символ: цифры, буквы и знаки (подпись нажатой — над пальцем).
    pub fn is_char(&self) -> bool {
        self.class == "key" && matches!(self.action, Action::Key { .. } | Action::Char(_))
    }
}

/// Варианты буквы по долгому нажатию, как в Gboard: цифры на верхнем ряду,
/// символы на среднем и нижнем; у «е» — «ё», у «ь» — «ъ».
fn letter_alts(row: usize, i: usize, ch: char) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    match ch {
        'е' => out.push("ё".into()),
        'ь' => out.push("ъ".into()),
        _ => {}
    }
    let sym = match row {
        0 => "1234567890".chars().nth(i),
        1 => "@#$_&-+()".chars().nth(i),
        _ => "*\"':;!?".chars().nth(i),
    };
    if let Some(s) = sym {
        out.push(s.to_string());
    }
    out
}

/// Язык: имя, группа в keymap `us,ru` и клавиши основного блока по физическим местам
/// (как на обычной клавиатуре: в RU знаки US заняты буквами — ё, х ъ, ж э, б ю).
pub struct Lang {
    pub name: &'static str,
    /// Имя раскладки xkb (`us`, `ru`).
    pub xkb: &'static str,
    pub group: u32,
    /// Ряды букв (по три): буквы ряда без знаков по краям.
    pub rows: [&'static str; 3],
    /// Буквы на местах знаков US (`` ` [ ] ; ' , . ``): пусто — там знаки US.
    pub extra: &'static str,
}

pub const LANGS: &[Lang] = &[
    Lang { name: "EN", xkb: "us", group: 0, rows: ["qwertyuiop", "asdfghjkl", "zxcvbnm"], extra: "" },
    Lang { name: "RU", xkb: "ru", group: 1, rows: ["йцукенгшщз", "фывапролд", "ячсмить"], extra: "ёхъжэбю" },
];

/// evdev-коды физических клавиш трёх буквенных рядов (QWERTY): q-p-[ , a-l-;-' , z-m-,-.
const ROW_CODES: [&[u32]; 3] = [
    &[16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26],
    &[30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40],
    &[44, 45, 46, 47, 48, 49, 50, 51, 52],
];

/// Подпись клавиши «плавающая ↔ внизу» — значок Material Icons (open_with), не текст.
pub const FLOAT_ICON: &str = "\u{E89F}";
/// Значки Material Icons нижнего ряда и стрелок: смайлик, глобус (язык), спрятать клавиатуру,
/// клавиатура (доп. ряды), стрелки, стереть.
pub const EMOJI_ICON: &str = "\u{E24E}";
pub const LANG_ICON: &str = "\u{E894}";
pub const HIDE_ICON: &str = "\u{E31A}";
pub const FN_ICON: &str = "\u{E312}";
pub const BACKSPACE_ICON: &str = "\u{E14A}";
/// «Вставить» на узкой клавиатуре (content_paste).
pub const PASTE_ICON: &str = "\u{E14F}";
const UP_ICON: &str = "\u{E316}";
const DOWN_ICON: &str = "\u{E313}";
const LEFT_ICON: &str = "\u{E314}";
const RIGHT_ICON: &str = "\u{E315}";

/// Подпись «Вставить» — переводится (`t!`) в ui; здесь — ключ каталога.
pub const PASTE_LABEL: &str = "Вставить";

/// Смайлы (три ряда) и знаки вне US-раскладки (четвёртый) страницы [`Page::Emoji`] — по 15 в ряд,
/// как ширина буквенной страницы.
const EMOJI_ROWS: [&str; 4] = [
    "😀😃😄😁😆😅😂🤣😊😇🙂😉😍😘😋",
    "😎🤔😐😴😢😭😡😱🥳🤗🙄😬🤩😏🥺",
    "👍👎👌🙏👏💪👋🤝👀💯🔥🎉🎁💖⭐",
    "•√π÷×¶∆£€¢°©®™§",
];

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

/// Знак US с его символом Shift: `[` → `{`.
fn us_dual(c: char) -> Key {
    let (code, _) = us_char(c).expect("знак US-раскладки");
    let upper = "`1234567890-=[]\\;',./".find(c).map(|i| "~!@#$%^&*()_+{}|:\"<>?".chars().nth(i).unwrap()).unwrap_or(c);
    Key::dual(&c.to_string(), &upper.to_string(), code, true)
}

/// Буква языка на физической клавише `code`.
fn lang_letter(ch: char, code: u32) -> Key {
    Key::letter(&ch.to_string(), &ch.to_uppercase().to_string(), code)
}

/// Ширина рядов (в обычных клавишах): все по 15, как у клавиатуры Steam.
#[cfg(test)]
const WIDTH: f32 = 15.0;

/// Ряды страницы для языка `lang`: 5 рядов одной высоты.
pub fn rows(page: Page, lang: &Lang) -> Vec<Vec<Key>> {
    let mut out: Vec<Vec<Key>> = Vec::new();
    let extra: Vec<char> = lang.extra.chars().collect();
    // Буква языка на месте знака US (ё вместо `, х ъ вместо [ ] …) или сам знак.
    let place = |i: usize, us: char| -> Key {
        match extra.get(i) {
            Some(&ch) => lang_letter(ch, us_char(us).unwrap().0),
            None => us_dual(us),
        }
    };
    match page {
        Page::Letters => {
            // 1: ` 1 … 0 - = и широкий ⌫.
            let mut r1 = vec![place(0, '`')];
            r1.extend("1234567890-=".chars().map(us_dual));
            r1.push(Key::repeat(BACKSPACE_ICON, KEY_BACKSPACE, 2.0));
            out.push(r1);
            // 2–4: буквы с клавишами по краям.
            let letters = |r: usize| -> Vec<Key> {
                lang.rows[r]
                    .chars()
                    .zip(ROW_CODES[r].iter())
                    .enumerate()
                    .map(|(i, (ch, code))| lang_letter(ch, *code).with_alts(letter_alts(r, i, ch)))
                    .collect()
            };
            let mut r2 = vec![Key::special("Tab", Action::Key { code: KEY_TAB, shift: false, repeat: false, latin: false }, 1.5)];
            r2.extend(letters(0));
            r2.extend([place(1, '['), place(2, ']')]);
            // «\» — у RU тоже знак (там Shift даёт «/»): из своей группы, как на клавиатуре.
            r2.push(if extra.is_empty() { Key { width: 1.5, ..us_dual('\\') } } else { Key { width: 1.5, ..Key::dual("\\", "/", 43, false) } });
            out.push(r2);
            let mut r3 = vec![Key::special("Caps", Action::Caps, 1.75)];
            r3.extend(letters(1));
            r3.extend([place(3, ';'), place(4, '\'')]);
            r3.push(Key::special("Enter", Action::Key { code: KEY_ENTER, shift: false, repeat: false, latin: false }, 2.25));
            out.push(r3);
            let mut r4 = vec![Key::special("Shift", Action::Modifier(Modifier::Shift), 2.5)];
            r4.extend(letters(2));
            r4.extend([place(5, ','), place(6, '.')]);
            // «/» в RU — «.» (Shift — «,») на той же клавише своей группы.
            r4.push(if extra.is_empty() { us_dual('/') } else { Key::dual(".", ",", 53, false).alts(&["!", "?", ",", ";", ":", "-", "\"", "'"]) });
            if extra.is_empty() {
                // Знаки препинания у точки, как раньше в нижнем ряду.
                if let Some(dot) = r4.iter_mut().find(|k| k.label == ".") {
                    dot.alts = ["!", "?", ",", ";", ":", "-", "\"", "'"].iter().map(|s| s.to_string()).collect();
                }
            }
            r4.push(Key::special("Shift", Action::Modifier(Modifier::Shift), 2.5));
            out.push(r4);
        }
        Page::Emoji => {
            for (i, chars) in EMOJI_ROWS.iter().enumerate() {
                let mut row: Vec<Key> = chars.chars().map(|c| Key::new(&c.to_string(), Action::Char(c), 1.0, "key")).collect();
                if i == 0 {
                    // ⌫ на своём месте — справа в первом ряду; смайлов в нём на один меньше.
                    row.truncate(13);
                    row.push(Key::repeat(BACKSPACE_ICON, KEY_BACKSPACE, 2.0));
                }
                out.push(row);
            }
        }
    }
    // Нижний ряд: смайлы, язык, свои — доп. ряды ⌨ и плавающая; пробел, стрелки, «Вставить», «Скрыть».
    let emoji = match page {
        Page::Letters => Key::special(EMOJI_ICON, Action::Page(Page::Emoji), 1.0),
        Page::Emoji => Key::special(EMOJI_ICON, Action::Page(Page::Letters), 1.0),
    };
    let arrow = |icon: &str, code: u32| Key::repeat(icon, code, 1.0);
    out.push(vec![
        emoji,
        Key::special(LANG_ICON, Action::Layout, 1.0),
        Key::special(FN_ICON, Action::Fn, 1.0),
        Key::special(FLOAT_ICON, Action::Float, 1.0),
        Key::special(lang.name, Action::Key { code: KEY_SPACE, shift: false, repeat: false, latin: false }, 5.5),
        // Блок стрелок как у Steam: два столбца по две половинки.
        arrow(UP_ICON, KEY_UP).over(arrow(LEFT_ICON, KEY_LEFT)),
        arrow(DOWN_ICON, KEY_DOWN).over(arrow(RIGHT_ICON, KEY_RIGHT)),
        Key::special(PASTE_LABEL, Action::Paste, 2.0),
        Key::special(HIDE_ICON, Action::Hide, 1.5),
    ]);
    out
}

/// Функциональные ряды: Esc/Tab/модификаторы/навигация, затем F1–F6 и
/// F7–F12 со стрелками перевёрнутой «T» справа, как на клавиатуре.
pub fn fn_rows() -> Vec<Vec<Key>> {
    fn k(label: &str, code: u32, width: f32, repeat: bool) -> Key {
        Key::new(label, Action::Key { code, shift: false, repeat, latin: false }, width, "key key-fn")
    }
    fn f(n: u32) -> Key {
        let code = match n {
            1..=10 => KEY_F1 + n - 1,
            11 => KEY_F11,
            _ => KEY_F12,
        };
        Key::new(&format!("F{n}"), Action::Key { code, shift: false, repeat: false, latin: false }, 1.0, "key key-fn key-f")
    }
    fn m(label: &str, m: Modifier, width: f32) -> Key {
        Key::new(label, Action::Modifier(m), width, "key key-fn")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(row: &[Key]) -> Vec<String> {
        row.iter().map(|k| k.label.clone()).collect()
    }

    fn row_width(row: &[Key]) -> f32 {
        row.iter().map(|k| k.width).sum()
    }

    #[test]
    fn steam_layout_en() {
        let rows = rows(Page::Letters, &LANGS[0]);
        assert_eq!(rows.len(), 5);
        for r in &rows {
            assert!((row_width(r) - WIDTH).abs() < 0.01, "ширина ряда {:?}", labels(r));
        }
        assert_eq!(labels(&rows[0])[..13].join(""), "`1234567890-=");
        assert_eq!(rows[0][13].action, Action::Key { code: KEY_BACKSPACE, shift: false, repeat: true, latin: false });
        let hints: String = rows[0][..13].iter().map(|k| k.hint.clone().unwrap()).collect();
        assert_eq!(hints, "~!@#$%^&*()_+");
        assert_eq!(labels(&rows[1]).join(" "), "Tab q w e r t y u i o p [ ] \\");
        assert_eq!(labels(&rows[2]).join(" "), "Caps a s d f g h j k l ; ' Enter");
        assert_eq!(labels(&rows[3]).join(" "), "Shift z x c v b n m , . / Shift");
        assert_eq!(rows[2][0].action, Action::Caps);
        // Знаки — из US-раскладки при любом языке, буквы — нет; у букв нет подписи Shift.
        assert!(matches!(rows[1][11].action, Action::Key { code: 26, latin: true, .. }));
        assert!(rows[1][1].hint.is_none() && rows[1][1].shifted.as_deref() == Some("Q"));
        // Нижний ряд: смайлы, язык, доп. ряды, плавающая, пробел, стрелки парами, «Вставить», «Скрыть».
        let bottom: Vec<Action> = rows[4].iter().map(|k| k.action).collect();
        assert_eq!(bottom[0], Action::Page(Page::Emoji));
        assert_eq!(bottom[1], Action::Layout);
        assert_eq!(bottom[2], Action::Fn);
        assert_eq!(bottom[3], Action::Float);
        assert!(matches!(bottom[4], Action::Key { code: KEY_SPACE, .. }));
        let arrows: Vec<u32> = rows[4][5..7]
            .iter()
            .flat_map(|k| [k, k.below.as_deref().unwrap()])
            .map(|k| match k.action {
                Action::Key { code, repeat: true, .. } => code,
                a => panic!("{a:?}"),
            })
            .collect();
        assert_eq!(arrows, [KEY_UP, KEY_LEFT, KEY_DOWN, KEY_RIGHT]);
        assert_eq!(bottom[7], Action::Paste);
        assert_eq!(bottom[8], Action::Hide);
        // Варианты по долгому нажатию у букв и точки сохранились.
        assert_eq!(rows[1][1].alts, ["1"]);
        assert!(rows[3].iter().find(|k| k.label == ".").unwrap().alts.contains(&"?".to_string()));
    }

    #[test]
    fn steam_layout_ru() {
        let rows = rows(Page::Letters, &LANGS[1]);
        for r in &rows {
            assert!((row_width(r) - WIDTH).abs() < 0.01, "ширина ряда {:?}", labels(r));
        }
        assert_eq!(labels(&rows[0])[..13].join(""), "ё1234567890-=");
        assert_eq!(labels(&rows[1]).join(" "), "Tab й ц у к е н г ш щ з х ъ \\");
        assert_eq!(labels(&rows[2]).join(" "), "Caps ф ы в а п р о л д ж э Enter");
        assert_eq!(labels(&rows[3]).join(" "), "Shift я ч с м и т ь б ю . Shift");
        // ё, х, ъ, ж, э, б, ю — буквы на физических местах знаков US, в своей группе.
        assert!(matches!(rows[0][0].action, Action::Key { code: 41, latin: false, .. }));
        assert!(matches!(rows[1][11].action, Action::Key { code: 26, latin: false, .. }));
        assert!(matches!(rows[3][9].action, Action::Key { code: 52, latin: false, .. }));
        assert_eq!(rows[0][0].shifted.as_deref(), Some("Ё"));
        assert!(rows[0][0].hint.is_none());
        // Точка RU: Shift — запятая, в своей группе (код 53).
        let dot = &rows[3][10];
        assert_eq!((dot.label.as_str(), dot.hint.as_deref()), (".", Some(",")));
        assert!(matches!(dot.action, Action::Key { code: 53, latin: false, .. }));
        // Цифры с символами US над ними.
        assert_eq!(rows[0][2].hint.as_deref(), Some("@"));
        assert_eq!(rows[1][5].alts, ["ё", "5"]);
    }

    #[test]
    fn emoji_page() {
        let rows = rows(Page::Emoji, &LANGS[0]);
        assert_eq!(rows.len(), 5);
        for r in &rows {
            assert!((row_width(r) - WIDTH).abs() < 0.01, "ширина ряда {:?}", labels(r));
        }
        assert_eq!(rows[0][0].action, Action::Char('😀'));
        assert!(rows[3].iter().any(|k| k.action == Action::Char('€')));
        assert_eq!(rows[4][0].action, Action::Page(Page::Letters));
    }

    #[test]
    fn terminals() {
        assert!(is_terminal("syn-terminal"));
        assert!(is_terminal("org.wezfurlong.wezterm"));
        assert!(is_terminal("foot"));
        assert!(!is_terminal("syn-notes"));
        assert!(!is_terminal("firefox"));
    }
}
