//! Миры и режимы блоков.
//!
//! * **Уровни** считаются из очков партии; каждый новый уровень открывает
//!   следующий «мир» — свою палитру фона/поля и свои фоновые частицы
//!   (пузыри, снег, светлячки…). Смена мира — плавный кроссфейд.
//! * **Режимы блоков**: на время (несколько ходов) все обычные кубики
//!   рисуются одним материалом — деревом, витражным стеклом, металлом или
//!   магмой, — сохраняя оттенок своего цвета. Смена катится волной по полю.
//!   Особые кубики (золото из раздачи и т. п.) остаются собой — по ним
//!   по-прежнему видно бонус.

use super::finish::{self, Finish};
use super::theme::{self, hex};
use crate::anim::{clamp01, smoothstep};
use crate::game::Material;
use crate::persist::Variant;
use syngui::prelude::*;

// ── миры ───────────────────────────────────────────────────────────────────

/// Фоновые частицы мира.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ambient {
    Bokeh,
    Bubbles,
    Petals,
    Fireflies,
    Stars,
    Snow,
    Embers,
    Aurora,
}

pub struct World {
    pub name: &'static str,
    pub top: &'static str,
    pub bottom: &'static str,
    pub glow: &'static str,
    pub panel: &'static str,
    pub edge: &'static str,
    pub empty: &'static str,
    /// Цвет фейерверков, кольца уровня и частиц мира.
    pub accent: &'static str,
    pub ambient: Ambient,
}

/// Мир «Бархат» — стартовый мир вида игры «Люкс».
pub const VELVET: usize = 8;

pub const WORLDS: [World; 9] = [
    World { name: "Сумерки", top: theme::BG_TOP, bottom: theme::BG_BOTTOM, glow: theme::GLOW, panel: theme::PANEL, edge: theme::PANEL_EDGE, empty: theme::CELL_EMPTY, accent: "#c9d2ff", ambient: Ambient::Bokeh },
    World { name: "Океан", top: "#27b4d8", bottom: "#0b3474", glow: "#7fe7ff", panel: "#0a2243", edge: "#2a70a8", empty: "#12325a", accent: "#8ff0ff", ambient: Ambient::Bubbles },
    World { name: "Закат", top: "#ff9466", bottom: "#6e2a86", glow: "#ffc08a", panel: "#34143c", edge: "#a0467a", empty: "#47204f", accent: "#ffd0a0", ambient: Ambient::Petals },
    World { name: "Лес", top: "#48b876", bottom: "#123f30", glow: "#b6ff9a", panel: "#0e2a20", edge: "#2f8a5a", empty: "#173b2c", accent: "#e6ff8a", ambient: Ambient::Fireflies },
    World { name: "Космос", top: "#2c2166", bottom: "#06071a", glow: "#9d7bff", panel: "#110e2c", edge: "#4b3aa8", empty: "#1b1840", accent: "#d6c8ff", ambient: Ambient::Stars },
    World { name: "Зима", top: "#94cffa", bottom: "#3559ad", glow: "#e6f6ff", panel: "#1a2c5a", edge: "#6d9ad8", empty: "#253f78", accent: "#ffffff", ambient: Ambient::Snow },
    World { name: "Вулкан", top: "#b3362a", bottom: "#24080f", glow: "#ff8a4a", panel: "#260d12", edge: "#9a3a2a", empty: "#381418", accent: "#ffb347", ambient: Ambient::Embers },
    World { name: "Сияние", top: "#1c5f6c", bottom: "#081430", glow: "#7affc8", panel: "#0b1d2c", edge: "#2a8a7a", empty: "#122c3c", accent: "#9affd8", ambient: Ambient::Aurora },
    World { name: "Бархат", top: "#5a1f4c", bottom: "#1a0a1e", glow: "#ffd27a", panel: "#241028", edge: "#a8813a", empty: "#33193a", accent: "#ffd27a", ambient: Ambient::Bokeh },
];

/// Длительность кроссфейда между мирами.
pub const WORLD_FADE: f32 = 1.8;

/// Очки, с которых начинается уровень `level` (1 — с нуля):
/// 0, 1000, 2500, 4500, 7000, 10 000, 13 500… — шаг растёт на 500.
pub fn threshold(level: u32) -> i32 {
    let n = level.saturating_sub(1) as i32;
    n * 1000 + 250 * n * (n - 1)
}

pub fn level_for(score: i32) -> u32 {
    let mut l = 1;
    while threshold(l + 1) <= score {
        l += 1;
    }
    l
}

/// Доля пути к следующему уровню (0..1).
pub fn progress(score: i32) -> f32 {
    let l = level_for(score);
    let (a, b) = (threshold(l), threshold(l + 1));
    clamp01((score - a) as f32 / (b - a) as f32)
}

pub fn world_index(level: u32) -> usize {
    world_index_for(level, Variant::Standard)
}

/// Мир уровня для вида игры: «Люкс» начинает с «Бархата» и дальше идёт
/// по кругу тех же миров.
pub fn world_index_for(level: u32, v: Variant) -> usize {
    let start = match v {
        Variant::Standard => 0,
        Variant::Deluxe => VELVET,
    };
    (start + level.max(1) as usize - 1) % WORLDS.len()
}

/// Цвета текущего кадра (во время смены мира — смесь двух).
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub top: Color,
    pub bottom: Color,
    pub glow: Color,
    pub panel: Color,
    pub edge: Color,
    pub empty: Color,
    pub accent: Color,
}

impl Palette {
    pub fn of(w: &World) -> Self {
        Palette { top: hex(w.top), bottom: hex(w.bottom), glow: hex(w.glow), panel: hex(w.panel), edge: hex(w.edge), empty: hex(w.empty), accent: hex(w.accent) }
    }

    pub fn mix(a: &Palette, b: &Palette, t: f32) -> Self {
        Palette {
            top: a.top.lerp(&b.top, t),
            bottom: a.bottom.lerp(&b.bottom, t),
            glow: a.glow.lerp(&b.glow, t),
            panel: a.panel.lerp(&b.panel, t),
            edge: a.edge.lerp(&b.edge, t),
            empty: a.empty.lerp(&b.empty, t),
            accent: a.accent.lerp(&b.accent, t),
        }
    }
}

/// Переход между мирами: `from` → `to`, начат в `born`.
#[derive(Clone, Copy, Debug)]
pub struct WorldFade {
    pub from: usize,
    pub to: usize,
    pub born: f32,
}

impl WorldFade {
    pub fn t(&self, now: f32) -> f32 {
        smoothstep((now - self.born) / WORLD_FADE)
    }

    pub fn palette(&self, now: f32) -> Palette {
        Palette::mix(&Palette::of(&WORLDS[self.from]), &Palette::of(&WORLDS[self.to]), self.t(now))
    }
}

// ── фоновые частицы ────────────────────────────────────────────────────────

/// Псевдослучайное 0..1 из номера частицы и «соли» (стабильно между кадрами).
fn rnd(i: usize, salt: u32) -> f32 {
    let s = ((i as f32 + 1.0) * (12.9898 + salt as f32 * 7.13)).sin() * 43758.547;
    s - s.floor()
}

fn dot(list: &mut DisplayList, x: f32, y: f32, r: f32, color: Color) {
    list.push_rect(Rect::new(Point::new(x - r, y - r), Size::new(r * 2.0, r * 2.0)), color, [r; 4]);
}

/// Нарисовать фоновые частицы мира `kind` с общей прозрачностью `alpha`.
pub fn draw_ambient(list: &mut DisplayList, kind: Ambient, b: Rect, t: f32, alpha: f32, accent: Color) {
    if alpha <= 0.01 {
        return;
    }
    let (x0, y0, w, h) = (b.origin.x, b.origin.y, b.width(), b.height());
    match kind {
        Ambient::Bokeh => {
            // медленно всплывающие «боке» + мерцающие искры
            for i in 0..11 {
                let f = rnd(i, 0);
                let r = 18.0 + f * 34.0;
                let speed = 7.0 + f * 11.0;
                let x = x0 + (i as f32 * 0.618 + f * 0.3) % 1.0 * w + (t * (0.25 + f * 0.2) + i as f32).sin() * 22.0;
                let period = h + r * 2.0;
                let y = y0 + h + r - ((t * speed + f * period + i as f32 * 97.0) % period);
                dot(list, x, y, r, Color::WHITE.with_alpha((0.03 + f * 0.035) * alpha));
            }
            twinkles(list, b, t, 18, 0.25 * alpha, Color::WHITE);
        }
        Ambient::Bubbles => {
            // пузыри поднимаются, покачиваясь; блик в левом верхнем углу
            for i in 0..22 {
                let f = rnd(i, 1);
                let r = 3.0 + f * 11.0;
                let speed = 22.0 + (1.0 - f) * 38.0;
                let period = h + r * 2.0;
                let y = y0 + h + r - ((t * speed + rnd(i, 2) * period) % period);
                let x = x0 + rnd(i, 3) * w + (t * (1.2 + f) + i as f32).sin() * (6.0 + r);
                let a = (0.10 + 0.12 * (1.0 - f)) * alpha;
                list.push_outline(Rect::new(Point::new(x - r, y - r), Size::new(r * 2.0, r * 2.0)), accent.with_alpha(a * 1.6), 1.2, 0.0, [r; 4]);
                dot(list, x, y, r, accent.with_alpha(a * 0.35));
                dot(list, x - r * 0.35, y - r * 0.35, r * 0.25, Color::WHITE.with_alpha(a * 2.2));
            }
            // лучи света сверху
            for i in 0..3 {
                let sway = (t * 0.2 + i as f32 * 2.0).sin() * 0.08;
                let cx = x0 + w * (0.2 + 0.3 * i as f32 + sway);
                let bw = 70.0 + 30.0 * i as f32;
                let pulse = (t * 0.5 + i as f32 * 1.7).sin() * 0.5 + 0.5;
                list.push_gradient_rect(
                    Rect::new(Point::new(cx - bw * 0.5, y0), Size::new(bw, h * 0.55)),
                    Gradient::Linear { angle_deg: 180.0, stops: vec![ColorStop::new(Color::WHITE.with_alpha((0.025 + 0.025 * pulse) * alpha), 0.0), ColorStop::new(Color::WHITE.with_alpha(0.0), 1.0)] },
                    [0.0; 4],
                );
            }
        }
        Ambient::Petals => {
            // лепестки кружатся и падают по диагонали; «поворот» — сжатие ширины
            for i in 0..20 {
                let f = rnd(i, 4);
                let s = 4.0 + f * 5.0;
                let speed = 18.0 + f * 22.0;
                let period = h + 40.0;
                let p = (t * speed + rnd(i, 5) * period) % period;
                let y = y0 - 20.0 + p;
                let x = x0 + (rnd(i, 6) * w + p * 0.35 + (t * 0.9 + i as f32).sin() * 18.0) % (w + 20.0) - 10.0;
                let spin = (t * (1.5 + f * 2.0) + i as f32).sin();
                let pw = s * (0.35 + 0.65 * spin.abs());
                let col = hex("#ffb3c6").lerp(&accent, rnd(i, 7) * 0.6);
                list.push_rect(Rect::new(Point::new(x - pw, y - s * 0.6), Size::new(pw * 2.0, s * 1.2)), col.with_alpha(0.55 * alpha), [s * 0.6; 4]);
            }
            twinkles(list, b, t, 8, 0.2 * alpha, accent);
        }
        Ambient::Fireflies => {
            // светлячки блуждают по фигурам Лиссажу и мерцают
            for i in 0..24 {
                let f = rnd(i, 8);
                let x = x0 + w * (0.5 + 0.46 * (t * (0.11 + f * 0.09) + rnd(i, 9) * 6.28).sin());
                let y = y0 + h * (0.5 + 0.46 * (t * (0.07 + rnd(i, 10) * 0.08) + rnd(i, 11) * 6.28).cos());
                let pulse = ((t * (1.4 + f * 1.8) + i as f32 * 1.7).sin() * 0.5 + 0.5).powi(2);
                let a = (0.15 + 0.85 * pulse) * alpha;
                dot(list, x, y, 9.0, accent.with_alpha(0.10 * a));
                dot(list, x, y, 4.5, accent.with_alpha(0.25 * a));
                dot(list, x, y, 1.8, Color::WHITE.with_alpha(0.9 * a));
            }
        }
        Ambient::Stars => {
            // звёздное небо + редкие падающие звёзды
            twinkles(list, b, t, 60, 0.7 * alpha, Color::WHITE);
            for i in 0..3 {
                let period = 5.0 + i as f32 * 2.3;
                let k = ((t + i as f32 * 1.9) % period) / 0.9;
                if k < 1.0 {
                    let n = ((t + i as f32 * 1.9) / period).floor() as usize + i * 17;
                    let sx = x0 + w * (0.2 + 0.7 * rnd(n, 12));
                    let sy = y0 + h * (0.05 + 0.35 * rnd(n, 13));
                    let len = 90.0;
                    let hx = sx - k * 260.0;
                    let hy = sy + k * 130.0;
                    let a = (1.0 - k) * alpha;
                    list.push_line_strip(vec![[hx + len * 0.9, hy - len * 0.45], [hx, hy]], Color::WHITE.with_alpha(0.25 * a), 2.0);
                    list.push_line_strip(vec![[hx + len * 0.3, hy - len * 0.15], [hx, hy]], Color::WHITE.with_alpha(0.7 * a), 2.2);
                    dot(list, hx, hy, 2.2, Color::WHITE.with_alpha(a));
                }
            }
            // туманность
            for i in 0..3 {
                let r = w * (0.35 + 0.1 * i as f32);
                let cx = x0 + w * (0.2 + 0.35 * i as f32) + (t * 0.05 + i as f32).sin() * 20.0;
                let cy = y0 + h * (0.25 + 0.3 * rnd(i, 14));
                dot(list, cx, cy, r, accent.with_alpha(0.035 * alpha));
            }
        }
        Ambient::Snow => {
            // три слоя снега: дальний мелкий и медленный, ближний крупный
            for i in 0..54 {
                let layer = i % 3;
                let s = [1.4, 2.4, 3.6][layer];
                let speed = [22.0, 38.0, 60.0][layer];
                let period = h + 20.0;
                let y = y0 - 10.0 + (t * speed + rnd(i, 15) * period) % period;
                let x = x0 + rnd(i, 16) * w + (t * (0.6 + rnd(i, 17)) + i as f32).sin() * (8.0 + 6.0 * layer as f32);
                dot(list, x, y, s, Color::WHITE.with_alpha([0.35, 0.55, 0.75][layer] * alpha));
            }
        }
        Ambient::Embers => {
            // пепел и угли поднимаются снизу, мерцая
            for i in 0..30 {
                let f = rnd(i, 18);
                let s = 1.5 + f * 2.8;
                let speed = 30.0 + f * 55.0;
                let period = h + 20.0;
                let y = y0 + h + 10.0 - (t * speed + rnd(i, 19) * period) % period;
                let x = x0 + rnd(i, 20) * w + (t * (1.0 + f) + i as f32).sin() * 14.0;
                let flick = ((t * 9.0 + i as f32 * 3.1).sin() * 0.5 + 0.5) * 0.5 + 0.5;
                let life = clamp01((y - y0) / (h * 0.7));
                let col = hex("#ff5a1f").lerp(&accent, f);
                dot(list, x, y, s * 2.4, col.with_alpha(0.10 * flick * life * alpha));
                dot(list, x, y, s, col.with_alpha(0.85 * flick * life * alpha));
            }
            // жар снизу
            list.push_gradient_rect(
                Rect::new(Point::new(x0, y0 + h * 0.6), Size::new(w, h * 0.4)),
                Gradient::Linear { angle_deg: 180.0, stops: vec![ColorStop::new(hex("#ff5a1f").with_alpha(0.0), 0.0), ColorStop::new(hex("#ff5a1f").with_alpha(0.14 * alpha), 1.0)] },
                [0.0; 4],
            );
        }
        Ambient::Aurora => {
            twinkles(list, b, t, 30, 0.45 * alpha, Color::WHITE);
            // волнистые ленты сияния: толстые полупрозрачные линии в несколько слоёв
            let cols = [accent, hex("#6ad3ff"), hex("#c38bff")];
            for (k, col) in cols.iter().enumerate() {
                let base_y = y0 + h * (0.14 + 0.1 * k as f32);
                let pts: Vec<[f32; 2]> = (0..=24)
                    .map(|j| {
                        let u = j as f32 / 24.0;
                        let y = base_y
                            + (u * 5.0 + t * (0.35 + 0.1 * k as f32) + k as f32 * 2.0).sin() * h * 0.045
                            + (u * 11.0 - t * 0.5).sin() * h * 0.015;
                        [x0 - 20.0 + u * (w + 40.0), y]
                    })
                    .collect();
                let breathe = (t * 0.6 + k as f32 * 1.3).sin() * 0.5 + 0.5;
                for (width, a) in [(64.0, 0.035), (34.0, 0.06), (12.0, 0.10)] {
                    list.push_line_strip(pts.clone(), col.with_alpha(a * (0.6 + 0.4 * breathe) * alpha), width);
                }
            }
        }
    }
}

/// Мерцающие точки-звёзды в случайных местах.
fn twinkles(list: &mut DisplayList, b: Rect, t: f32, n: usize, alpha: f32, color: Color) {
    for i in 0..n {
        let f1 = rnd(i, 21);
        let x = b.origin.x + f1 * b.width();
        let y = b.origin.y + rnd(i, 22) * b.height();
        let tw = ((t * (1.2 + f1 * 1.5) + i as f32 * 2.1).sin() * 0.5 + 0.5).powi(3);
        let s = 0.8 + tw * 1.4 + rnd(i, 23);
        dot(list, x, y, s * 0.5, color.with_alpha(alpha * (0.25 + 0.75 * tw)));
    }
}

// ── режимы блоков ──────────────────────────────────────────────────────────

/// Сколько ходов длится режим.
pub const SKIN_MOVES: u32 = 12;
/// Задержка волны смены на одну диагональ поля.
pub const SKIN_STEP: f32 = 0.045;
/// Вспышка кубика в момент смены.
pub const SKIN_FLASH: f32 = 0.35;
/// Трей меняется после того, как волна прошла всё поле.
pub const TRAY_DELAY: f32 = 15.0 * SKIN_STEP;
/// Материалы, которые бывают режимами.
pub const SKINS: [Material; 4] = [Material::Wood, Material::Crystal, Material::Gold, Material::Lava];

pub fn cell_delay(r: usize, c: usize) -> f32 {
    (r + c) as f32 * SKIN_STEP
}

#[derive(Clone, Copy, Debug)]
pub struct SkinMode {
    pub material: Material,
    pub born: f32,
    /// Момент начала обратной волны (режим закончился).
    pub end_at: Option<f32>,
    pub moves_left: u32,
}

impl SkinMode {
    pub fn new(material: Material, born: f32) -> Self {
        SkinMode { material, born, end_at: None, moves_left: SKIN_MOVES }
    }

    /// Действует ли режим для кубика с задержкой волны `delay`.
    pub fn on(&self, now: f32, delay: f32) -> bool {
        now >= self.born + delay && self.end_at.map(|e| now < e + delay).unwrap_or(true)
    }

    /// Вспышка смены для кубика с задержкой `delay` (0..1).
    pub fn flash(&self, now: f32, delay: f32) -> f32 {
        let one = |t0: f32| {
            let t = now - t0 - delay;
            if (0.0..SKIN_FLASH).contains(&t) {
                (1.0 - t / SKIN_FLASH).powi(2)
            } else {
                0.0
            }
        };
        one(self.born).max(self.end_at.map(one).unwrap_or(0.0))
    }

    /// Обратная волна прошла целиком — режим можно забыть.
    pub fn finished(&self, now: f32) -> bool {
        self.end_at.map(|e| now > e + TRAY_DELAY + SKIN_FLASH + 0.3).unwrap_or(false)
    }

    /// Видимость подписи режима (0..1).
    pub fn chip_alpha(&self, now: f32) -> f32 {
        let a = clamp01((now - self.born) / 0.4);
        match self.end_at {
            Some(e) => a * (1.0 - clamp01((now - e) / 0.4)),
            None => a,
        }
    }
}

pub fn skin_name(m: Material) -> &'static str {
    match m {
        Material::Wood => "ДЕРЕВО",
        Material::Crystal => "ВИТРАЖ",
        Material::Gold => "МЕТАЛЛ",
        Material::Lava => "МАГМА",
        Material::Plain => "",
    }
}

/// Как нарисовать кубик: материал узора, базовый цвет, акцент (свечение
/// трещин); в «Люксе» — отделка (текстура) и признак бонусной фигуры.
#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub material: Material,
    pub base: Color,
    pub accent: Option<Color>,
    pub finish: Option<&'static Finish>,
    /// Фигура с бонусным материалом (в «Люксе» помечается искрой).
    pub bonus: bool,
}

/// Внешний вид кубика цвета `color_idx` из материала `own`; `skin` — режим,
/// действующий для этого кубика. Особые материалы режим не перекрывает.
pub fn look(own: Material, color_idx: u8, skin: Option<Material>) -> Look {
    match (own, skin) {
        (Material::Plain, Some(m)) if m != Material::Plain => skin_look(m, color_idx),
        _ => Look { material: own, base: theme::material_color(own, color_idx), accent: None, finish: None, bonus: false },
    }
}

/// Внешний вид кубика в «Люксе»: отделка `finish` (0 — по материалу/цвету);
/// режим блоков временно даёт обычным кубикам отделку своего материала.
pub fn look_deluxe(own: Material, color_idx: u8, finish: u8, skin: Option<Material>) -> Look {
    let id = match (own, skin) {
        (Material::Plain, Some(m)) if m != Material::Plain => m.finish(),
        _ => finish::resolve(finish, own, color_idx),
    };
    let f = finish::get(id);
    Look {
        material: own,
        base: finish::base_color(f, theme::cell_color(color_idx)),
        accent: Some(hex(f.accent)),
        finish: Some(f),
        bonus: own != Material::Plain,
    }
}

/// Вид кубика по виду игры.
pub fn look_for(v: Variant, own: Material, color_idx: u8, finish: u8, skin: Option<Material>) -> Look {
    match v {
        Variant::Standard => look(own, color_idx, skin),
        Variant::Deluxe => look_deluxe(own, color_idx, finish, skin),
    }
}

fn skin_look(m: Material, idx: u8) -> Look {
    let c = theme::cell_color(idx);
    match m {
        Material::Wood => {
            // разные породы: от светлой берёзы до тёмного ореха, с оттенком цвета
            const WOOD: [&str; 8] = ["#b97a3e", "#7c5234", "#a8713d", "#c98c4e", "#6e4424", "#d9ae72", "#9c6b3f", "#8a4a2e"];
            Look { material: m, base: hex(WOOD[idx as usize % 8]).lerp(&c, 0.24), accent: None, finish: None, bonus: false }
        }
        // витраж: цвет остаётся ярким, чуть светлее — стекло
        Material::Crystal => Look { material: m, base: c.lerp(&Color::WHITE, 0.28), accent: None, finish: None, bonus: false },
        // металлик: цвет, приглушённый сталью
        Material::Gold => Look { material: m, base: c.lerp(&hex("#8e97a8"), 0.45).darken(0.1), accent: Some(c), finish: None, bonus: false },
        // магма: тёмный камень, трещины светятся цветом фигуры
        Material::Lava => Look { material: m, base: c.lerp(&hex("#1a141b"), 0.88), accent: Some(c.lighten(0.15)), finish: None, bonus: false },
        Material::Plain => Look { material: m, base: c, accent: None, finish: None, bonus: false },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_grow_with_score() {
        assert_eq!(level_for(0), 1);
        assert_eq!(level_for(999), 1);
        assert_eq!(level_for(1000), 2);
        assert_eq!(level_for(2500), 3);
        assert_eq!(level_for(4500), 4);
        assert!(progress(500) > 0.49 && progress(500) < 0.51);
        for l in 1..40 {
            assert!(threshold(l + 1) > threshold(l));
            assert_eq!(level_for(threshold(l)), l);
        }
        assert_eq!(world_index(1), 0);
        assert_eq!(world_index(10), 0);
        assert_eq!(world_index_for(1, Variant::Deluxe), VELVET);
        assert_eq!(world_index_for(2, Variant::Deluxe), 0);
    }

    #[test]
    fn deluxe_look_uses_finish_and_skin() {
        let l = look_deluxe(Material::Plain, 3, 7, None);
        assert_eq!(l.finish.unwrap().id, 7);
        assert!(!l.bonus);
        // режим блоков «дерево» даёт отделку дуба обычному кубику
        assert_eq!(look_deluxe(Material::Plain, 3, 7, Some(Material::Wood)).finish.unwrap().en, "Oak");
        // бонусное золото остаётся золотом и помечено
        let g = look_deluxe(Material::Gold, 3, 7, Some(Material::Wood));
        assert_eq!(g.finish.unwrap().en, "Gold");
        assert!(g.bonus);
        // старое сохранение без отделки — красящаяся отделка по цвету
        assert!(look_deluxe(Material::Plain, 2, 0, None).finish.unwrap().tint);
    }

    #[test]
    fn skin_wave_switches_and_returns() {
        let mut s = SkinMode::new(Material::Wood, 10.0);
        assert!(!s.on(9.9, 0.0));
        assert!(s.on(10.0, 0.0));
        assert!(!s.on(10.1, cell_delay(7, 7)));
        s.end_at = Some(20.0);
        assert!(!s.on(20.0, 0.0));
        assert!(s.on(20.0, cell_delay(7, 7)));
        assert!(!s.finished(20.5));
        assert!(s.finished(22.0));
    }

    #[test]
    fn skin_keeps_special_materials() {
        assert_eq!(look(Material::Gold, 3, Some(Material::Wood)).material, Material::Gold);
        assert_eq!(look(Material::Plain, 3, Some(Material::Wood)).material, Material::Wood);
        assert_eq!(look(Material::Plain, 3, None).material, Material::Plain);
    }
}
