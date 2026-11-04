//! Оформление «Цикла»: готовые стили поверх светлой/тёмной темы — палитра и форма отметок
//! (капли месячных, цветок овуляции, сердечки близости). «Классика» — основной вид: круги.

use std::f32::consts::{PI, TAU};

use serde::{Deserialize, Serialize};
use syngui::prelude::*;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Decor {
    #[default]
    Classic,
    Sakura,
    Berry,
    Lavender,
    Daisy,
    Night,
}

/// Знак овуляции.
#[derive(Clone, Copy, PartialEq)]
pub enum OvulMark {
    Circle,
    Flower,
    Star,
    Daisy,
}

impl Decor {
    pub const ALL: [Decor; 6] = [Decor::Classic, Decor::Sakura, Decor::Berry, Decor::Lavender, Decor::Daisy, Decor::Night];

    pub fn name(self) -> String {
        match self {
            Decor::Classic => t!("Классика"),
            Decor::Sakura => t!("Сакура"),
            Decor::Berry => t!("Ягодка"),
            Decor::Lavender => t!("Лаванда"),
            Decor::Daisy => t!("Ромашки"),
            Decor::Night => t!("Звёздная ночь"),
        }
    }

    /// Капли, цветы и сердечки вместо кругов и обводки.
    pub fn shapes(self) -> bool {
        self != Decor::Classic
    }

    /// Стиль только тёмный (светлая/тёмная тема на него не влияют).
    pub fn always_dark(self) -> bool {
        self == Decor::Night
    }

    pub fn ovul_mark(self) -> OvulMark {
        match self {
            Decor::Classic => OvulMark::Circle,
            Decor::Sakura | Decor::Lavender => OvulMark::Flower,
            Decor::Berry | Decor::Night => OvulMark::Star,
            Decor::Daisy => OvulMark::Daisy,
        }
    }

    /// Переменные темы, которые стиль меняет (поверх базовой светлой/тёмной палитры).
    pub fn vars(self, dark: bool) -> &'static [(&'static str, &'static str)] {
        match (self, dark) {
            (Decor::Classic, _) => &[],
            (Decor::Sakura, false) => &[
                ("bg", "#fff6f8"),
                ("bg-2", "#fff6f8"),
                ("surface-alt", "#fdeef2"),
                ("border", "#f6dfe6"),
                ("rose", "#ff2f7a"),
                ("rose-2", "#ff5a96"),
                ("rose-soft", "#ffcfe0"),
                ("rose-line", "#ff2f7a99"),
                ("plum", "#c43ab4"),
                ("plum-soft", "#c43ab426"),
                ("ovul", "#ffb3d1"),
                ("ovul-line", "#ff2f7a"),
                ("fert-text", "#b0124f"),
                ("today", "#00c2a8"),
                ("titlebar", "#fdf0f3"),
                ("sidebar", "#fdf0f3"),
                ("nav", "#fff6f8f2"),
            ],
            (Decor::Sakura, true) => &[
                ("bg", "#1d1418"),
                ("bg-2", "#1d1418"),
                ("surface", "#2a1d23"),
                ("surface-alt", "#33232b"),
                ("border", "#3d2a33"),
                ("rose", "#ff4d8d"),
                ("rose-2", "#ff75aa"),
                ("rose-soft", "#5e1937"),
                ("rose-line", "#ff4d8daa"),
                ("plum", "#f065d8"),
                ("plum-soft", "#f065d833"),
                ("ovul", "#74204a"),
                ("ovul-line", "#ff8cc0"),
                ("fert-text", "#ffc2da"),
                ("today", "#1fe0c6"),
                ("menu-bg", "#2a1d23"),
                ("titlebar", "#1a1215"),
                ("sidebar", "#1a1215"),
                ("nav", "#1f161af5"),
            ],
            (Decor::Berry, false) => &[
                ("bg", "#fffaf5"),
                ("bg-2", "#fffaf5"),
                ("surface-alt", "#fff0ea"),
                ("border", "#f8e1d8"),
                ("rose", "#ff1f45"),
                ("rose-2", "#ff4a66"),
                ("rose-soft", "#ffd0d7"),
                ("rose-line", "#ff1f4599"),
                ("plum", "#e0245e"),
                ("plum-soft", "#e0245e26"),
                ("ovul", "#ffd23f"),
                ("ovul-line", "#ff9500"),
                ("today", "#00c2a8"),
                ("titlebar", "#fff3ec"),
                ("sidebar", "#fff3ec"),
                ("nav", "#fffaf5f2"),
            ],
            (Decor::Berry, true) => &[
                ("bg", "#1c1213"),
                ("bg-2", "#1c1213"),
                ("surface", "#291a1b"),
                ("surface-alt", "#321f21"),
                ("border", "#3d2729"),
                ("rose", "#ff3d5e"),
                ("rose-2", "#ff6680"),
                ("rose-soft", "#5e1520"),
                ("rose-line", "#ff3d5eaa"),
                ("plum", "#ff5c95"),
                ("plum-soft", "#ff5c9533"),
                ("today", "#1fe0c6"),
                ("menu-bg", "#291a1b"),
                ("titlebar", "#180f10"),
                ("sidebar", "#180f10"),
                ("nav", "#1e1314f5"),
            ],
            (Decor::Lavender, false) => &[
                ("bg", "#f8f5ff"),
                ("bg-2", "#f8f5ff"),
                ("surface-alt", "#f0eafc"),
                ("border", "#e6ddf7"),
                ("rose", "#e8288f"),
                ("rose-2", "#f052a6"),
                ("rose-soft", "#ffd0ea"),
                ("rose-line", "#e8288f99"),
                ("plum", "#7c3dff"),
                ("plum-soft", "#7c3dff26"),
                ("ovul", "#dccaff"),
                ("ovul-line", "#7c3dff"),
                ("fert-text", "#5a1fd6"),
                ("today", "#14a8ff"),
                ("titlebar", "#f3eefd"),
                ("sidebar", "#f3eefd"),
                ("nav", "#f8f5fff2"),
            ],
            (Decor::Lavender, true) => &[
                ("bg", "#16131f"),
                ("bg-2", "#16131f"),
                ("surface", "#211c2e"),
                ("surface-alt", "#2a2439"),
                ("border", "#352d47"),
                ("rose", "#ff4fa8"),
                ("rose-2", "#ff78bd"),
                ("rose-soft", "#5c1a4a"),
                ("rose-line", "#ff4fa8aa"),
                ("plum", "#a974ff"),
                ("plum-soft", "#a974ff33"),
                ("ovul", "#4b2d85"),
                ("ovul-line", "#b892ff"),
                ("fert-text", "#dcc9ff"),
                ("today", "#3cc8ff"),
                ("menu-bg", "#211c2e"),
                ("titlebar", "#13101b"),
                ("sidebar", "#13101b"),
                ("nav", "#18141ff5"),
            ],
            (Decor::Daisy, false) => &[
                ("bg", "#f5fbf6"),
                ("bg-2", "#f5fbf6"),
                ("surface-alt", "#eaf5ee"),
                ("border", "#dcefe3"),
                ("rose", "#ff3b3b"),
                ("rose-2", "#ff6363"),
                ("rose-soft", "#ffd4ce"),
                ("rose-line", "#ff3b3b99"),
                ("plum", "#ff7a1a"),
                ("plum-soft", "#ff7a1a26"),
                ("ovul", "#ffd94a"),
                ("ovul-line", "#f5a300"),
                ("today", "#1a9dff"),
                ("titlebar", "#eef7f1"),
                ("sidebar", "#eef7f1"),
                ("nav", "#f5fbf6f2"),
            ],
            (Decor::Daisy, true) => &[
                ("bg", "#121a16"),
                ("bg-2", "#121a16"),
                ("surface", "#1a241f"),
                ("surface-alt", "#212d27"),
                ("border", "#2b3a32"),
                ("rose", "#ff5a4f"),
                ("rose-2", "#ff7d73"),
                ("rose-soft", "#5e1d16"),
                ("rose-line", "#ff5a4faa"),
                ("plum", "#ff9a4d"),
                ("plum-soft", "#ff9a4d33"),
                ("today", "#2eb0ff"),
                ("menu-bg", "#1a241f"),
                ("titlebar", "#0f1612"),
                ("sidebar", "#0f1612"),
                ("nav", "#141c18f5"),
            ],
            (Decor::Night, _) => &[
                ("bg", "#15122b"),
                ("bg-2", "#15122b"),
                ("surface", "#1f1a3a"),
                ("surface-alt", "#282249"),
                ("fg", "#f1edff"),
                ("muted", "#a49fc6"),
                ("faint", "#5d5786"),
                ("border", "#332c5a"),
                ("hover", "#ffffff12"),
                ("pressed", "#ffffff1e"),
                ("rose", "#ff4f9a"),
                ("rose-2", "#ff78b4"),
                ("rose-soft", "#5c1652"),
                ("rose-line", "#ff4f9aaa"),
                ("plum", "#9a6bff"),
                ("plum-soft", "#9a6bff33"),
                ("fert", "#3d2a63"),
                ("ovul", "#5a3d99"),
                ("ovul-line", "#ffd23f"),
                ("fert-text", "#ffe28a"),
                ("today", "#3fe0ff"),
                ("menu-bg", "#211c3d"),
                ("titlebar", "#120f25"),
                ("sidebar", "#120f25"),
                ("nav", "#17142ef5"),
            ],
        }
    }

    /// Цвета рисунков: капли, сердечки, цветы.
    pub fn ink(self, dark: bool) -> Ink {
        let c = Color::from_hex;
        match (self, dark) {
            (Decor::Classic, false) => Ink { blood: c("#e8294f"), blood_soft: c("#ffa3b5"), heart: c("#f0357a"), petal: c("#f2a516"), center: c("#b36f00") },
            (Decor::Classic, true) => Ink { blood: c("#ff4767"), blood_soft: c("#6b1a2c"), heart: c("#ff5a96"), petal: c("#f5ae2e"), center: c("#ffd27a") },
            (Decor::Sakura, false) => Ink { blood: c("#e8003d"), blood_soft: c("#ff9dbb"), heart: c("#ff2f7a"), petal: c("#ff9ec4"), center: c("#ffc93c") },
            (Decor::Sakura, true) => Ink { blood: c("#ff2e5b"), blood_soft: c("#6b1f36"), heart: c("#ff5c99"), petal: c("#e05595"), center: c("#ffd23f") },
            (Decor::Berry, false) => Ink { blood: c("#e8002f"), blood_soft: c("#ffa3b0"), heart: c("#ff1f45"), petal: c("#ffd23f"), center: c("#ff9500") },
            (Decor::Berry, true) => Ink { blood: c("#ff2e4f"), blood_soft: c("#6b1a24"), heart: c("#ff3d5e"), petal: c("#ffb800"), center: c("#ffe066") },
            (Decor::Lavender, false) => Ink { blood: c("#e0005a"), blood_soft: c("#ffa0cb"), heart: c("#e8288f"), petal: c("#b896ff"), center: c("#ffc93c") },
            (Decor::Lavender, true) => Ink { blood: c("#ff3d8a"), blood_soft: c("#65204f"), heart: c("#ff4fa8"), petal: c("#8a5cff"), center: c("#ffd23f") },
            (Decor::Daisy, false) => Ink { blood: c("#f01e1e"), blood_soft: c("#ffaaa1"), heart: c("#ff3b3b"), petal: c("#ffffff"), center: c("#ffc000") },
            (Decor::Daisy, true) => Ink { blood: c("#ff4438"), blood_soft: c("#66221a"), heart: c("#ff5a4f"), petal: c("#eef8f1"), center: c("#ffc000") },
            (Decor::Night, _) => Ink { blood: c("#ff2e6e"), blood_soft: c("#5c1f5e"), heart: c("#ff4f9a"), petal: c("#ffd23f"), center: c("#fff3c4") },
        }
    }
}

#[derive(Clone, Copy)]
pub struct Ink {
    /// Капля месячных и её бледный вариант для прогноза.
    pub blood: Color,
    pub blood_soft: Color,
    pub heart: Color,
    /// Лепестки и серединка цветка (овуляция).
    pub petal: Color,
    pub center: Color,
}

// ── Фигуры ───────────────────────────────────────────────────────────────────

/// Капля остриём вверх: середина (cx, cy), высота `h`.
pub fn drop_points(cx: f32, cy: f32, h: f32) -> Vec<(f32, f32)> {
    let a = h / 2.0;
    let n = 48;
    (0..n)
        .map(|i| {
            let t = i as f32 / n as f32 * TAU;
            // кривая «слезы»: острие при t = 0, округлое дно при t = π
            let x = a * 1.02 * t.sin() * (t / 2.0).sin().powf(1.05);
            let y = -a * t.cos();
            (cx + x, cy + y + a * 0.06)
        })
        .collect()
}

pub fn paint_drop(c: &mut CanvasContext, cx: f32, cy: f32, h: f32, fill: Color, gloss: bool) {
    c.set_color(fill);
    c.fill_polygon_concave(&drop_points(cx, cy, h));
    if gloss {
        c.set_color(Color::white().with_alpha(0.3));
        c.fill_polygon(&ellipse(cx - h * 0.18, cy + h * 0.1, h * 0.06, h * 0.12, -0.4, 14));
    }
}

/// Сердечко: середина (cx, cy), ширина `w`.
pub fn heart_points(cx: f32, cy: f32, w: f32) -> Vec<(f32, f32)> {
    let s = w / 34.0;
    let n = 48;
    (0..n)
        .map(|i| {
            let t = i as f32 / n as f32 * TAU;
            let x = 16.0 * t.sin().powi(3);
            let y = 13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos();
            (cx + x * s, cy - y * s - s * 1.5)
        })
        .collect()
}

pub fn paint_heart(c: &mut CanvasContext, cx: f32, cy: f32, w: f32, fill: Color, outline: Option<Color>) {
    if let Some(o) = outline {
        c.set_color(o);
        c.fill_polygon_concave(&heart_points(cx, cy, w + 3.0));
    }
    c.set_color(fill);
    c.fill_polygon_concave(&heart_points(cx, cy, w));
}

fn ellipse(cx: f32, cy: f32, rx: f32, ry: f32, rot: f32, n: usize) -> Vec<(f32, f32)> {
    let (s, co) = rot.sin_cos();
    (0..n)
        .map(|i| {
            let t = i as f32 / n as f32 * TAU;
            let (x, y) = (rx * t.cos(), ry * t.sin());
            (cx + x * co - y * s, cy + x * s + y * co)
        })
        .collect()
}

/// Цветок из `n` круглых лепестков вокруг серединки.
pub fn paint_flower(c: &mut CanvasContext, cx: f32, cy: f32, r: f32, petals: usize, rot: f32, petal: Color, center: Color) {
    c.set_color(petal);
    for i in 0..petals {
        let a = rot + i as f32 / petals as f32 * TAU;
        c.fill_circle(cx + a.cos() * r * 0.52, cy + a.sin() * r * 0.52, r * 0.48);
    }
    c.set_color(center);
    c.fill_circle(cx, cy, r * 0.3);
}

/// Ромашка: узкие лепестки и жёлтая серединка.
pub fn paint_daisy(c: &mut CanvasContext, cx: f32, cy: f32, r: f32, rot: f32, petal: Color, edge: Option<Color>, center: Color) {
    let n = 10;
    for pass in 0..2 {
        let (col, grow) = match (pass, edge) {
            (0, Some(e)) => (e, 1.12),
            (0, None) => continue,
            _ => (petal, 1.0),
        };
        c.set_color(col);
        for i in 0..n {
            let a = rot + i as f32 / n as f32 * TAU;
            let (s, co) = a.sin_cos();
            c.fill_polygon(&ellipse(cx + co * r * 0.55, cy + s * r * 0.55, r * 0.42 * grow, r * 0.14 * grow, a, 12));
        }
    }
    c.set_color(center);
    c.fill_circle(cx, cy, r * 0.26);
}

/// Звезда с `n` лучами.
pub fn star_points(cx: f32, cy: f32, r: f32, inner: f32, n: usize, rot: f32) -> Vec<(f32, f32)> {
    (0..n * 2)
        .map(|i| {
            let a = rot - PI / 2.0 + i as f32 / (n * 2) as f32 * TAU;
            let rr = if i % 2 == 0 { r } else { r * inner };
            (cx + a.cos() * rr, cy + a.sin() * rr)
        })
        .collect()
}

/// Знак овуляции на дате размером `size`.
pub fn paint_ovul(c: &mut CanvasContext, mark: OvulMark, size: f32, ink: &Ink) {
    let (cx, cy) = (size / 2.0, size / 2.0);
    match mark {
        OvulMark::Circle => {}
        OvulMark::Flower => {
            // серединка под цифрой: светлее светлых лепестков (тёмная цифра), темнее тёмных (белая)
            let light = ink.petal.relative_luminance() > 0.4;
            let center = if light { ink.petal.lerp(&Color::white(), 0.55) } else { ink.petal.lerp(&Color::black(), 0.3) };
            paint_flower(c, cx, cy, size * 0.5, 5, -PI / 2.0, ink.petal, center);
        }
        OvulMark::Star => {
            c.set_color(ink.petal);
            c.fill_polygon_concave(&star_points(cx, cy + size * 0.03, size * 0.54, 0.6, 5, 0.0));
        }
        OvulMark::Daisy => {
            paint_daisy(c, cx, cy, size * 0.5, 0.0, ink.petal, Some(ink.center.with_alpha(0.6)), ink.center.lerp(&Color::white(), 0.35));
        }
    }
}
