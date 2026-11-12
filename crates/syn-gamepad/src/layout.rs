//! Раскладка контроллера: элементы, их места и назначения. Файлы —
//! `~/.config/synshell/gamepads/<id>.toml`; встроенная `standard` — пока своей с этим id нет.
//!
//! ```toml
//! name = "Стандартная"
//! opacity = 0.55
//! [[element]]
//! kind = "stick"      # stick | dpad | button | trackpad
//! x = 0.14            # центр: доля ширины и высоты экрана
//! y = 0.62
//! size = 0.32         # высота (у круглых — диаметр): доля короткой стороны экрана
//! bind = "left"       # stick: left | right | pad | wasd | arrows | mouse; dpad: pad | arrows | wasd
//! [[element]]
//! kind = "button"
//! label = "A"       # текст или значок Material Icons (символ U+E000…U+F8FF)
//! bind = "pad:a"      # pad:a…  | key:space | key:ctrl+c | mouse:left | touch:left,hold,right,hover | osk:keyboard
//! ```
//!
//! `touch:<режимы>` — кнопка режима касания игры (`touch.rs`), `osk:keyboard` — перейти на экранную клавиатуру.
//! Встроенные раскладки ([`BUILTIN`]): Xbox (`standard`), PlayStation, Nintendo, Sega, сенсорная мышь, WASD,
//! ретро; свой файл с тем же id их заменяет («Сбросить» в редакторе — вернуть встроенную).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::uinput::PadButton;
use syngui::t;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Stick,
    Dpad,
    #[default]
    Button,
    Trackpad,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Element {
    pub kind: Kind,
    pub x: f32,
    pub y: f32,
    pub size: f32,
    /// Ширина к высоте: 1 — круг/квадрат, больше — «таблетка» (плечевые кнопки, тачпад).
    pub aspect: f32,
    pub label: String,
    pub bind: String,
    /// Стик: центр там, где коснулся палец (в пределах элемента).
    pub floating: bool,
    /// Тачпад и стик-мышь: скорость.
    pub sensitivity: f32,
    /// Кнопка: нажал — держится до следующего нажатия.
    pub toggle: bool,
}

impl Default for Element {
    fn default() -> Self {
        Self {
            kind: Kind::Button,
            x: 0.5,
            y: 0.5,
            size: 0.14,
            aspect: 1.0,
            label: String::new(),
            bind: String::new(),
            floating: false,
            sensitivity: 1.0,
            toggle: false,
        }
    }
}

impl Element {
    /// Прямоугольник на поверхности `w × h`: [x, y, ширина, высота].
    pub fn rect(&self, w: f32, h: f32) -> [f32; 4] {
        let side = w.min(h) * self.size;
        let (ew, eh) = (side * self.aspect.max(0.2), side);
        [self.x * w - ew / 2.0, self.y * h - eh / 2.0, ew, eh]
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub name: String,
    /// Непрозрачность элементов в покое (нажатый — ярче).
    pub opacity: f32,
    pub haptics: bool,
    #[serde(rename = "element")]
    pub elements: Vec<Element>,
}

impl Default for Layout {
    fn default() -> Self {
        standard()
    }
}

/// Что делает элемент.
#[derive(Debug, Clone, PartialEq)]
pub enum Bind {
    None,
    Pad(PadButton),
    /// Курок: левый/правый (оси Z/RZ).
    Trigger(bool),
    /// Клавиша или сочетание: держатся, пока нажата кнопка.
    Keys(Vec<u32>),
    Mouse(u16),
    WheelUp,
    WheelDown,
    /// Режим касания игры: назначение целиком (`touch:left,right,hover`) — список режимов кнопки.
    Touch(String),
    /// Перейти на экранную клавиатуру (режим ввода приложения — «Клавиатура»).
    Keyboard,
}

impl Bind {
    pub fn parse(s: &str) -> Bind {
        let s = s.trim().to_ascii_lowercase();
        let (kind, arg) = s.split_once(':').unwrap_or(("pad", s.as_str()));
        match kind {
            "pad" => match arg {
                "a" => Bind::Pad(PadButton::A),
                "b" => Bind::Pad(PadButton::B),
                "x" => Bind::Pad(PadButton::X),
                "y" => Bind::Pad(PadButton::Y),
                "lb" | "l1" => Bind::Pad(PadButton::Lb),
                "rb" | "r1" => Bind::Pad(PadButton::Rb),
                "lt" | "l2" => Bind::Trigger(false),
                "rt" | "r2" => Bind::Trigger(true),
                "back" | "select" => Bind::Pad(PadButton::Back),
                "start" => Bind::Pad(PadButton::Start),
                "guide" | "home" => Bind::Pad(PadButton::Guide),
                "l3" | "ls" => Bind::Pad(PadButton::L3),
                "r3" | "rs" => Bind::Pad(PadButton::R3),
                _ => Bind::None,
            },
            "key" => {
                let codes: Option<Vec<u32>> = arg.split('+').map(crate::keys::code).collect();
                codes.filter(|c| !c.is_empty()).map(Bind::Keys).unwrap_or(Bind::None)
            }
            "touch" => Bind::Touch(s.clone()),
            "osk" => match arg {
                "keyboard" | "kbd" => Bind::Keyboard,
                _ => Bind::None,
            },
            "mouse" => match arg {
                "left" => Bind::Mouse(crate::uinput::BTN_LEFT),
                "right" => Bind::Mouse(crate::uinput::BTN_RIGHT),
                "middle" => Bind::Mouse(crate::uinput::BTN_MIDDLE),
                "wheelup" => Bind::WheelUp,
                "wheeldown" => Bind::WheelDown,
                _ => Bind::None,
            },
            _ => Bind::None,
        }
    }
}

/// Что делает стик или крестовина.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    LeftStick,
    RightStick,
    Hat,
    Wasd,
    Arrows,
    Mouse,
}

impl Axis {
    pub fn parse(kind: Kind, s: &str) -> Axis {
        match (kind, s.trim().to_ascii_lowercase().as_str()) {
            (_, "wasd") => Axis::Wasd,
            (_, "arrows") => Axis::Arrows,
            (_, "mouse") => Axis::Mouse,
            // Стик как крестовина геймпада (D-pad, 8 направлений).
            (_, "pad" | "dpad" | "hat") => Axis::Hat,
            (Kind::Stick, "right") => Axis::RightStick,
            (Kind::Stick, _) => Axis::LeftStick,
            (_, _) => Axis::Hat,
        }
    }
}

fn el(kind: Kind, x: f32, y: f32, size: f32, label: &str, bind: &str) -> Element {
    Element { kind, x, y, size, label: label.into(), bind: bind.into(), ..Default::default() }
}

fn wide(mut e: Element, aspect: f32) -> Element {
    e.aspect = aspect;
    e
}

fn round(x: f32, y: f32, size: f32, label: &str, bind: &str) -> Element {
    el(Kind::Button, x, y, size, label, bind)
}

/// Кнопка режима касания — в правом верхнем углу.
fn touch_mode(x: f32, y: f32) -> Element {
    el(Kind::Button, x, y, 0.13, "", TOUCH_BIND)
}

/// Назначение кнопки режима касания по умолчанию.
pub const TOUCH_BIND: &str = "touch:left,hold,right,hover";

/// Встроенные раскладки: id и построитель (порядок — в списке раскладок).
pub const BUILTIN: &[(&str, fn() -> Layout)] = &[
    ("standard", standard),
    ("playstation", playstation),
    ("nintendo", nintendo),
    ("sega", sega),
    ("touch-mouse", touch_mouse),
    ("wasd", wasd),
    ("retro", retro),
];

/// Встроенная по id.
pub fn builtin(id: &str) -> Option<Layout> {
    BUILTIN.iter().find(|(i, _)| *i == id).map(|(_, f)| f())
}

/// Геймпад: два стика, крестовина, четыре кнопки справа, плечевые и курки, центральные.
/// `face` — (подпись, назначение) кнопок: низ, право, лево, верх.
fn pad_layout(name: String, face: [(&str, &str); 4], shoulders: [&str; 4], center: [(&str, &str); 2]) -> Layout {
    use Kind::*;
    let [down, right, left, up] = face;
    let [lt, lb, rb, rt] = shoulders;
    Layout {
        name,
        opacity: 0.55,
        haptics: true,
        elements: vec![
            el(Stick, 0.14, 0.60, 0.36, "", "left"),
            el(Dpad, 0.32, 0.82, 0.27, "", "pad"),
            el(Stick, 0.68, 0.82, 0.27, "", "right"),
            el(Button, 0.885, 0.78, 0.15, down.0, down.1),
            el(Button, 0.95, 0.60, 0.15, right.0, right.1),
            el(Button, 0.82, 0.60, 0.15, left.0, left.1),
            el(Button, 0.885, 0.42, 0.15, up.0, up.1),
            wide(el(Button, 0.07, 0.10, 0.11, lt, "pad:lt"), 1.8),
            wide(el(Button, 0.20, 0.10, 0.11, lb, "pad:lb"), 1.8),
            wide(el(Button, 0.70, 0.10, 0.11, rb, "pad:rb"), 1.8),
            wide(el(Button, 0.83, 0.10, 0.11, rt, "pad:rt"), 1.8),
            // центральные — рядом ниже: вверху по центру ручка контроллера с кнопками ⌨ и настройки
            wide(el(Button, 0.40, 0.22, 0.09, center[0].0, center[0].1), 1.4),
            wide(el(Button, 0.60, 0.22, 0.09, center[1].0, center[1].1), 1.4),
            touch_mode(0.95, 0.10),
        ],
    }
}

/// Встроенная раскладка в духе Xbox (под альбомную ориентацию).
pub fn standard() -> Layout {
    pad_layout(
        "Xbox".into(),
        [("A", "pad:a"), ("B", "pad:b"), ("X", "pad:x"), ("Y", "pad:y")],
        ["LT", "LB", "RB", "RT"],
        [("\u{E3E0}", "pad:back"), ("\u{E5D2}", "pad:start")],
    )
}

/// PlayStation: ✕ ○ □ △ (значки Material Icons), L1/L2/R1/R2, Share/Options — те же кнопки геймпада.
pub fn playstation() -> Layout {
    pad_layout(
        "PlayStation".into(),
        [("\u{E5CD}", "pad:a"), ("\u{E836}", "pad:b"), ("\u{E3C6}", "pad:x"), ("\u{E86B}", "pad:y")],
        ["L2", "L1", "R1", "R2"],
        [("Share", "pad:back"), ("Options", "pad:start")],
    )
}

/// Nintendo: A справа, B снизу, X сверху, Y слева (по подписи: A — кнопка A геймпада), ZL/L/R/ZR, −/+.
pub fn nintendo() -> Layout {
    pad_layout(
        "Nintendo".into(),
        [("B", "pad:b"), ("A", "pad:a"), ("Y", "pad:y"), ("X", "pad:x")],
        ["ZL", "L", "R", "ZR"],
        [("−", "pad:back"), ("+", "pad:start")],
    )
}

/// Sega Mega Drive (шесть кнопок): крестовина, A B C снизу, X Y Z сверху, Start, Mode.
pub fn sega() -> Layout {
    use Kind::*;
    Layout {
        name: "Sega".into(),
        opacity: 0.55,
        haptics: true,
        elements: vec![
            el(Dpad, 0.16, 0.66, 0.40, "", "pad"),
            round(0.66, 0.74, 0.15, "A", "pad:x"),
            round(0.79, 0.70, 0.15, "B", "pad:a"),
            round(0.92, 0.66, 0.15, "C", "pad:b"),
            round(0.66, 0.50, 0.12, "X", "pad:lb"),
            round(0.79, 0.46, 0.12, "Y", "pad:y"),
            round(0.92, 0.42, 0.12, "Z", "pad:rb"),
            wide(el(Button, 0.43, 0.88, 0.09, "Mode", "pad:back"), 1.6),
            wide(el(Button, 0.57, 0.88, 0.09, "Start", "pad:start"), 1.6),
            touch_mode(0.95, 0.10),
        ],
    }
}

/// Сенсорная мышь: палец по игре — мышь (режим касания — кнопкой), колесо, Esc, клавиатура.
pub fn touch_mouse() -> Layout {
    Layout {
        name: t!("Сенсорная мышь").into(),
        opacity: 0.5,
        haptics: true,
        elements: vec![
            touch_mode(0.95, 0.10),
            round(0.955, 0.36, 0.11, "\u{E5CE}", "mouse:wheelup"),
            round(0.955, 0.52, 0.11, "\u{E5CF}", "mouse:wheeldown"),
            round(0.955, 0.70, 0.11, "Esc", "key:esc"),
            round(0.955, 0.88, 0.11, "\u{E312}", "osk:keyboard"),
        ],
    }
}

/// Шутер (WASD): ходьба — плавающий стик WASD, обзор — тачпад-мышь, огонь, прицел, прыжок, присед…
pub fn wasd() -> Layout {
    use Kind::*;
    let mut walk = el(Stick, 0.14, 0.66, 0.36, "", "wasd");
    walk.floating = true;
    let mut look = wide(el(Trackpad, 0.66, 0.55, 0.45, "", "mouse:left"), 1.5);
    look.sensitivity = 1.2;
    Layout {
        name: "WASD".into(),
        opacity: 0.5,
        haptics: true,
        elements: vec![
            walk,
            look,
            round(0.92, 0.80, 0.17, "\u{E1B3}", "mouse:left"),
            round(0.92, 0.55, 0.12, "\u{E3B4}", "mouse:right"),
            round(0.82, 0.92, 0.12, "\u{E5D8}", "key:space"),
            round(0.33, 0.92, 0.11, "Ctrl", "key:ctrl"),
            round(0.33, 0.74, 0.11, "Shift", "key:shift"),
            round(0.45, 0.92, 0.10, "R", "key:r"),
            round(0.45, 0.76, 0.10, "E", "key:e"),
            round(0.30, 0.10, 0.09, "1", "key:1"),
            round(0.38, 0.10, 0.09, "2", "key:2"),
            round(0.46, 0.10, 0.09, "3", "key:3"),
            round(0.06, 0.10, 0.10, "Esc", "key:esc"),
            round(0.16, 0.10, 0.10, "Tab", "key:tab"),
            touch_mode(0.95, 0.10),
        ],
    }
}

/// Ретро на клавиатуре: стрелки, Z / X / пробел, Enter, Esc.
pub fn retro() -> Layout {
    use Kind::*;
    Layout {
        name: t!("Ретро (стрелки)").into(),
        opacity: 0.55,
        haptics: true,
        elements: vec![
            el(Dpad, 0.16, 0.66, 0.40, "", "arrows"),
            round(0.80, 0.72, 0.16, "Z", "key:z"),
            round(0.93, 0.60, 0.16, "X", "key:x"),
            round(0.80, 0.48, 0.12, "\u{E5D8}", "key:space"),
            wide(el(Button, 0.43, 0.88, 0.09, "Esc", "key:esc"), 1.6),
            wide(el(Button, 0.57, 0.88, 0.09, "Enter", "key:enter"), 1.6),
            touch_mode(0.95, 0.10),
        ],
    }
}

pub fn dir() -> PathBuf {
    synshell_common::paths::config_dir().join("gamepads")
}

/// Раскладка по id: файл, иначе встроенная с этим id, иначе `standard`.
pub fn load(id: &str) -> Layout {
    let fallback = || builtin(id).unwrap_or_else(standard);
    let path = dir().join(format!("{id}.toml"));
    match std::fs::read_to_string(&path) {
        Ok(s) => match toml::from_str::<Layout>(&s) {
            Ok(l) => l,
            Err(e) => {
                log::warn!("{}: {e}", path.display());
                fallback()
            }
        },
        Err(_) => fallback(),
    }
}

pub fn save(id: &str, layout: &Layout) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir())?;
    std::fs::write(dir().join(format!("{id}.toml")), toml::to_string_pretty(layout)?)?;
    Ok(())
}

/// Все раскладки: (id, имя); встроенные — первыми (свой файл с их id — под тем же местом).
pub fn list() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = BUILTIN.iter().map(|(id, _)| (id.to_string(), load(id).name)).collect();
    if let Ok(rd) = std::fs::read_dir(dir()) {
        let mut ids: Vec<String> = rd
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.strip_suffix(".toml").map(str::to_string))
            .filter(|id| builtin(id).is_none())
            .collect();
        ids.sort();
        for id in ids {
            let name = load(&id).name;
            out.push((id, name));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binds() {
        assert_eq!(Bind::parse("pad:a"), Bind::Pad(PadButton::A));
        assert_eq!(Bind::parse("rt"), Bind::Trigger(true));
        assert_eq!(Bind::parse("key:ctrl+c"), Bind::Keys(vec![29, 46]));
        assert_eq!(Bind::parse("key:nope"), Bind::None);
        assert_eq!(Bind::parse("mouse:right"), Bind::Mouse(crate::uinput::BTN_RIGHT));
        assert_eq!(Axis::parse(Kind::Stick, "right"), Axis::RightStick);
        assert_eq!(Axis::parse(Kind::Dpad, "pad"), Axis::Hat);
        assert_eq!(Axis::parse(Kind::Stick, "pad"), Axis::Hat, "стик — крестовиной");
        assert_eq!(Axis::parse(Kind::Stick, ""), Axis::LeftStick);
    }

    #[test]
    fn roundtrip() {
        for (id, f) in BUILTIN {
            let l = f();
            let s = toml::to_string_pretty(&l).unwrap();
            assert!(s.contains("[[element]]"), "{id}");
            assert_eq!(toml::from_str::<Layout>(&s).unwrap(), l, "{id}");
        }
    }

    #[test]
    fn presets() {
        for (id, f) in BUILTIN {
            let l = f();
            assert!(!l.elements.is_empty(), "{id}");
            for e in &l.elements {
                assert!((0.0..=1.0).contains(&e.x) && (0.0..=1.0).contains(&e.y), "{id}: {e:?}");
                if e.kind == Kind::Button {
                    assert_ne!(Bind::parse(&e.bind), Bind::None, "{id}: назначение «{}»", e.bind);
                }
            }
            assert!(l.elements.iter().any(|e| e.bind.starts_with("touch:")), "{id}: нет кнопки режима касания");
        }
        assert_eq!(Bind::parse(TOUCH_BIND), Bind::Touch(TOUCH_BIND.into()));
        assert_eq!(Bind::parse("osk:keyboard"), Bind::Keyboard);
    }
}
