//! Оформление «Цикла»: готовые стили поверх светлой/тёмной темы — палитра, фон-рисунок
//! (SVG из data/decor) и форма отметок (капли месячных, цветок овуляции, сердечки близости).
//! «Классика» — прежний вид: круги и обводка «ручкой», без рисунка.

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

    /// Фон-рисунок (SVG из data/decor, см. gen.py): светлый и тёмный варианты.
    pub fn background(self, dark: bool) -> Option<(&'static str, &'static [u8])> {
        Some(match (self, dark) {
            (Decor::Classic, _) => return None,
            (Decor::Sakura, false) => ("decor-sakura-light", include_bytes!("../data/decor/sakura-light.svg")),
            (Decor::Sakura, true) => ("decor-sakura-dark", include_bytes!("../data/decor/sakura-dark.svg")),
            (Decor::Berry, false) => ("decor-berry-light", include_bytes!("../data/decor/berry-light.svg")),
            (Decor::Berry, true) => ("decor-berry-dark", include_bytes!("../data/decor/berry-dark.svg")),
            (Decor::Lavender, false) => ("decor-lavender-light", include_bytes!("../data/decor/lavender-light.svg")),
            (Decor::Lavender, true) => ("decor-lavender-dark", include_bytes!("../data/decor/lavender-dark.svg")),
            (Decor::Daisy, false) => ("decor-daisy-light", include_bytes!("../data/decor/daisy-light.svg")),
            (Decor::Daisy, true) => ("decor-daisy-dark", include_bytes!("../data/decor/daisy-dark.svg")),
            (Decor::Night, _) => ("decor-night", include_bytes!("../data/decor/night.svg")),
        })
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
                ("rose", "#e0457b"),
                ("rose-2", "#ea6b95"),
                ("rose-soft", "#fbdbe6"),
                ("rose-line", "#e0457b80"),
                ("plum", "#b05f9c"),
                ("plum-soft", "#b05f9c22"),
                ("ovul", "#f9cbdc"),
                ("ovul-line", "#e0457b"),
                ("fert-text", "#9c4a62"),
                ("today", "#2fbfa5"),
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
                ("rose", "#ff6f9a"),
                ("rose-2", "#ff8fb0"),
                ("rose-soft", "#4a2333"),
                ("rose-line", "#ff6f9a99"),
                ("plum", "#d58ac0"),
                ("plum-soft", "#d58ac02a"),
                ("ovul", "#5a2c3e"),
                ("ovul-line", "#ff8fb0"),
                ("fert-text", "#ffc0d3"),
                ("today", "#3dd6bb"),
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
                ("rose", "#e5384f"),
                ("rose-2", "#f05a6e"),
                ("rose-soft", "#fde0e2"),
                ("rose-line", "#e5384f80"),
                ("plum", "#c2416b"),
                ("plum-soft", "#c2416b22"),
                ("ovul", "#ffd36e"),
                ("ovul-line", "#e59a00"),
                ("today", "#33b6a0"),
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
                ("rose", "#ff5c70"),
                ("rose-2", "#ff7a8b"),
                ("rose-soft", "#4a1f26"),
                ("rose-line", "#ff5c7099"),
                ("plum", "#ff7aa0"),
                ("plum-soft", "#ff7aa02a"),
                ("today", "#3fd1b9"),
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
                ("rose", "#c2408a"),
                ("rose-2", "#d1609d"),
                ("rose-soft", "#f6dbea"),
                ("rose-line", "#c2408a80"),
                ("plum", "#8a63d2"),
                ("plum-soft", "#8a63d222"),
                ("ovul", "#e4d6fb"),
                ("ovul-line", "#8a63d2"),
                ("fert-text", "#6d4fae"),
                ("today", "#35a9d6"),
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
                ("rose", "#ff6fae"),
                ("rose-2", "#ff8cc0"),
                ("rose-soft", "#4a2440"),
                ("rose-line", "#ff6fae99"),
                ("plum", "#b597ff"),
                ("plum-soft", "#b597ff2a"),
                ("ovul", "#3d3160"),
                ("ovul-line", "#b597ff"),
                ("fert-text", "#d9c9ff"),
                ("today", "#4fc6e8"),
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
                ("rose", "#ec4f55"),
                ("rose-2", "#f4736e"),
                ("rose-soft", "#fde1dc"),
                ("rose-line", "#ec4f5580"),
                ("plum", "#e5844f"),
                ("plum-soft", "#e5844f22"),
                ("ovul", "#ffe08a"),
                ("ovul-line", "#e0a800"),
                ("today", "#3a9fe0"),
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
                ("rose", "#ff7a6e"),
                ("rose-2", "#ff958b"),
                ("rose-soft", "#46241f"),
                ("rose-line", "#ff7a6e99"),
                ("plum", "#ffad7a"),
                ("plum-soft", "#ffad7a2a"),
                ("today", "#4cb3ee"),
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
                ("rose", "#ff6fa8"),
                ("rose-2", "#ff8cbc"),
                ("rose-soft", "#4a2148"),
                ("rose-line", "#ff6fa899"),
                ("plum", "#a58bff"),
                ("plum-soft", "#a58bff2a"),
                ("fert", "#3a2f52"),
                ("ovul", "#4b3d70"),
                ("ovul-line", "#ffd36b"),
                ("fert-text", "#ffe2a0"),
                ("today", "#5ad1e6"),
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
            (Decor::Classic, false) => Ink { blood: c("#e0466f"), blood_soft: c("#f7dfe5"), heart: c("#e0466f"), petal: c("#f6d370"), center: c("#e6a800") },
            (Decor::Classic, true) => Ink { blood: c("#f0577f"), blood_soft: c("#43202b"), heart: c("#f0577f"), petal: c("#7e5e0a"), center: c("#f2b300") },
            (Decor::Sakura, false) => Ink { blood: c("#d81b47"), blood_soft: c("#f7c6d4"), heart: c("#ef4f86"), petal: c("#f7a9c2"), center: c("#f4c96b") },
            (Decor::Sakura, true) => Ink { blood: c("#ff4d6d"), blood_soft: c("#5b2a39"), heart: c("#ff7aa6"), petal: c("#c75d86"), center: c("#ffd27a") },
            (Decor::Berry, false) => Ink { blood: c("#d61f3b"), blood_soft: c("#f9c9cf"), heart: c("#e5384f"), petal: c("#ffd36e"), center: c("#e59a00") },
            (Decor::Berry, true) => Ink { blood: c("#ff3d57"), blood_soft: c("#5a222a"), heart: c("#ff5c70"), petal: c("#f2b300"), center: c("#ffd36e") },
            (Decor::Lavender, false) => Ink { blood: c("#c81d5e"), blood_soft: c("#f3c9dc"), heart: c("#c2408a"), petal: c("#c7b0f2"), center: c("#f3c65a") },
            (Decor::Lavender, true) => Ink { blood: c("#ff4f8b"), blood_soft: c("#55264a"), heart: c("#ff6fae"), petal: c("#7c62c4"), center: c("#ffd27a") },
            (Decor::Daisy, false) => Ink { blood: c("#e23b3b"), blood_soft: c("#fbcfca"), heart: c("#ec4f55"), petal: c("#ffffff"), center: c("#ffc94a") },
            (Decor::Daisy, true) => Ink { blood: c("#ff5a4f"), blood_soft: c("#55291f"), heart: c("#ff7a6e"), petal: c("#e8f3ec"), center: c("#ffc94a") },
            (Decor::Night, _) => Ink { blood: c("#ff3f7a"), blood_soft: c("#4f2353"), heart: c("#ff6fa8"), petal: c("#ffd36b"), center: c("#fff3c4") },
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
            let x = a * 0.78 * t.sin() * (t / 2.0).sin().powf(1.4);
            let y = -a * t.cos();
            (cx + x, cy + y + a * 0.06)
        })
        .collect()
}

pub fn paint_drop(c: &mut CanvasContext, cx: f32, cy: f32, h: f32, fill: Color, gloss: bool) {
    c.set_color(fill);
    c.fill_polygon_concave(&drop_points(cx, cy, h));
    if gloss {
        // блик на округлой части
        c.set_color(Color::white().with_alpha(0.38));
        c.fill_polygon(&ellipse(cx - h * 0.14, cy + h * 0.1, h * 0.06, h * 0.12, -0.35, 14));
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
