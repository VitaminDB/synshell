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
//! bind = "left"       # stick: left | right | wasd | arrows | mouse; dpad: pad | arrows | wasd
//! [[element]]
//! kind = "button"
//! label = "A"       # текст или значок Material Icons (символ U+E000…U+F8FF)
//! bind = "pad:a"      # pad:a…  | key:space | key:ctrl+c | mouse:left
//! ```

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::uinput::PadButton;

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

/// Встроенная раскладка в духе Xbox (под альбомную ориентацию).
pub fn standard() -> Layout {
    use Kind::*;
    Layout {
        name: "Стандартная".into(),
        opacity: 0.55,
        haptics: true,
        elements: vec![
            el(Stick, 0.14, 0.60, 0.36, "", "left"),
            el(Dpad, 0.32, 0.82, 0.27, "", "pad"),
            el(Stick, 0.68, 0.82, 0.27, "", "right"),
            el(Button, 0.885, 0.78, 0.15, "A", "pad:a"),
            el(Button, 0.95, 0.60, 0.15, "B", "pad:b"),
            el(Button, 0.82, 0.60, 0.15, "X", "pad:x"),
            el(Button, 0.885, 0.42, 0.15, "Y", "pad:y"),
            wide(el(Button, 0.07, 0.10, 0.11, "LT", "pad:lt"), 1.8),
            wide(el(Button, 0.20, 0.10, 0.11, "LB", "pad:lb"), 1.8),
            wide(el(Button, 0.80, 0.10, 0.11, "RB", "pad:rb"), 1.8),
            wide(el(Button, 0.93, 0.10, 0.11, "RT", "pad:rt"), 1.8),
            wide(el(Button, 0.40, 0.10, 0.09, "\u{E3E0}", "pad:back"), 1.4),
            wide(el(Button, 0.60, 0.10, 0.09, "\u{E5D2}", "pad:start"), 1.4),
        ],
    }
}

pub fn dir() -> PathBuf {
    synshell_common::paths::config_dir().join("gamepads")
}

/// Раскладка по id: файл, иначе встроенная `standard`.
pub fn load(id: &str) -> Layout {
    let path = dir().join(format!("{id}.toml"));
    match std::fs::read_to_string(&path) {
        Ok(s) => match toml::from_str::<Layout>(&s) {
            Ok(l) => l,
            Err(e) => {
                log::warn!("{}: {e}", path.display());
                standard()
            }
        },
        Err(_) => standard(),
    }
}

pub fn save(id: &str, layout: &Layout) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir())?;
    std::fs::write(dir().join(format!("{id}.toml")), toml::to_string_pretty(layout)?)?;
    Ok(())
}

/// Все раскладки: (id, имя); встроенная — первой.
pub fn list() -> Vec<(String, String)> {
    let mut out = vec![("standard".to_string(), load("standard").name)];
    if let Ok(rd) = std::fs::read_dir(dir()) {
        let mut ids: Vec<String> = rd
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.strip_suffix(".toml").map(str::to_string))
            .filter(|id| id != "standard")
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
    }

    #[test]
    fn roundtrip() {
        let l = standard();
        let s = toml::to_string_pretty(&l).unwrap();
        assert!(s.contains("[[element]]"));
        assert_eq!(toml::from_str::<Layout>(&s).unwrap(), l);
    }
}
