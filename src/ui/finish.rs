//! Отделки — визуальные материалы кубиков в виде игры «Эльвира».
//!
//! У каждой фигуры своя отделка (`Piece.finish`, 1..=`FINISH_COUNT`): золото,
//! стекло, велюр, дуб… Таблица описывает, как отделку рисовать (тайл атласа,
//! красится ли цветом фигуры, характер объёма и анимации), какие частицы и
//! звук дать при очистке и как она называется по-английски и по-русски —
//! основа игрового словаря.

use super::theme::hex;
use crate::audio::Sfx;
use crate::game::{Material, FINISH_COUNT};
use syngui::prelude::*;

/// Характер объёма и бликов.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shade {
    /// Резкий контраст, зеркальная полоса.
    Metal,
    /// Лёгкий объём, светлая кромка, косой блик.
    Glass,
    /// Мягкое потемнение к краям (ворс, ткань).
    Soft,
    /// Слабый объём (камень, дерево).
    Matte,
    /// Тёмная сердцевина, светящаяся кромка.
    Glow,
}

/// Анимация поверх текстуры.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anim {
    None,
    /// Бегущая полоса блеска.
    Sheen,
    /// Мерцающие искры.
    Sparkle,
    /// Пульсирующее свечение.
    Pulse,
    /// Радужный перелив.
    Iridescent,
}

/// Частицы при очистке.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Burst {
    Coins,
    Shards,
    Chips,
    Dust,
    Embers,
    Sparks,
    Fibres,
    Sprinkles,
}

#[derive(Clone, Copy, Debug)]
pub struct Finish {
    pub id: u8,
    pub en: &'static str,
    pub ru: &'static str,
    /// Спрайт нейтральный (серый) — красится цветом фигуры; блики лежат
    /// отдельным белым слоем атласа.
    pub tint: bool,
    /// Радиус скругления силуэта (доля половины стороны) — как в спрайте.
    pub radius: f32,
    /// Цвет без текстуры (fallback и частицы).
    pub base: &'static str,
    /// Акцент бликов/частиц.
    pub accent: &'static str,
    pub shade: Shade,
    pub anim: Anim,
    pub burst: Burst,
    pub sfx: Sfx,
}

/// Порядок = номер тайла в атласе (8×4, слева направо, сверху вниз).
pub const FINISHES: [Finish; FINISH_COUNT as usize] = [
    Finish { id: 1, en: "Gold", ru: "золото", tint: false, radius: 0.22, base: "#e9b83a", accent: "#fff1a8", shade: Shade::Metal, anim: Anim::Sheen, burst: Burst::Coins, sfx: Sfx::Gold },
    Finish { id: 2, en: "Silver", ru: "серебро", tint: false, radius: 0.22, base: "#c9ced6", accent: "#ffffff", shade: Shade::Metal, anim: Anim::Sheen, burst: Burst::Coins, sfx: Sfx::Metal },
    Finish { id: 3, en: "Copper", ru: "медь", tint: false, radius: 0.22, base: "#c8703c", accent: "#ffc79a", shade: Shade::Metal, anim: Anim::Sheen, burst: Burst::Coins, sfx: Sfx::Metal },
    Finish { id: 4, en: "Bronze", ru: "бронза", tint: false, radius: 0.20, base: "#9a6a34", accent: "#e2b878", shade: Shade::Metal, anim: Anim::None, burst: Burst::Coins, sfx: Sfx::Metal },
    Finish { id: 5, en: "Glass", ru: "стекло", tint: true, radius: 0.40, base: "#9fd3ff", accent: "#ffffff", shade: Shade::Glass, anim: Anim::None, burst: Burst::Shards, sfx: Sfx::Glass },
    Finish { id: 6, en: "Crystal", ru: "хрусталь", tint: true, radius: 0.30, base: "#bfeaff", accent: "#ffffff", shade: Shade::Glass, anim: Anim::Sparkle, burst: Burst::Shards, sfx: Sfx::Crystal },
    Finish { id: 7, en: "Velvet", ru: "велюр", tint: true, radius: 0.38, base: "#b0407a", accent: "#ffd0ea", shade: Shade::Soft, anim: Anim::None, burst: Burst::Fibres, sfx: Sfx::Velvet },
    Finish { id: 8, en: "Oak", ru: "дуб", tint: false, radius: 0.16, base: "#c48b4c", accent: "#f0c88c", shade: Shade::Matte, anim: Anim::None, burst: Burst::Chips, sfx: Sfx::Wood },
    Finish { id: 9, en: "Walnut", ru: "орех", tint: false, radius: 0.16, base: "#5f3a22", accent: "#b07a4c", shade: Shade::Matte, anim: Anim::None, burst: Burst::Chips, sfx: Sfx::Wood },
    Finish { id: 10, en: "Marble", ru: "мрамор", tint: false, radius: 0.18, base: "#e9e6e1", accent: "#ffffff", shade: Shade::Glass, anim: Anim::None, burst: Burst::Dust, sfx: Sfx::Stone },
    Finish { id: 11, en: "Granite", ru: "гранит", tint: false, radius: 0.18, base: "#7d7f86", accent: "#d0d2d8", shade: Shade::Matte, anim: Anim::None, burst: Burst::Dust, sfx: Sfx::Stone },
    Finish { id: 12, en: "Obsidian", ru: "обсидиан", tint: false, radius: 0.30, base: "#1b1a22", accent: "#8f8fb0", shade: Shade::Glass, anim: Anim::Sheen, burst: Burst::Shards, sfx: Sfx::Glass },
    Finish { id: 13, en: "Lava", ru: "лава", tint: false, radius: 0.22, base: "#3a2426", accent: "#ff7a2f", shade: Shade::Glow, anim: Anim::Pulse, burst: Burst::Embers, sfx: Sfx::Lava },
    Finish { id: 14, en: "Ice", ru: "лёд", tint: false, radius: 0.34, base: "#bfe6ff", accent: "#ffffff", shade: Shade::Glass, anim: Anim::Sparkle, burst: Burst::Shards, sfx: Sfx::Ice },
    Finish { id: 15, en: "Jade", ru: "нефрит", tint: false, radius: 0.36, base: "#4f9a6a", accent: "#bfe9c8", shade: Shade::Glass, anim: Anim::None, burst: Burst::Dust, sfx: Sfx::Stone },
    Finish { id: 16, en: "Gem", ru: "самоцвет", tint: true, radius: 0.30, base: "#e04a6a", accent: "#ffffff", shade: Shade::Glass, anim: Anim::Sparkle, burst: Burst::Shards, sfx: Sfx::Crystal },
    Finish { id: 17, en: "Pearl", ru: "перламутр", tint: false, radius: 0.45, base: "#efe3ea", accent: "#ffffff", shade: Shade::Glass, anim: Anim::Iridescent, burst: Burst::Dust, sfx: Sfx::Glass },
    Finish { id: 18, en: "Neon", ru: "неон", tint: true, radius: 0.18, base: "#2ad4ff", accent: "#ffffff", shade: Shade::Glow, anim: Anim::Pulse, burst: Burst::Sparks, sfx: Sfx::Neon },
    Finish { id: 19, en: "Leather", ru: "кожа", tint: false, radius: 0.30, base: "#7a4a2b", accent: "#c8905e", shade: Shade::Soft, anim: Anim::None, burst: Burst::Chips, sfx: Sfx::Velvet },
    Finish { id: 20, en: "Denim", ru: "джинса", tint: false, radius: 0.30, base: "#3f5f96", accent: "#a9c2ee", shade: Shade::Soft, anim: Anim::None, burst: Burst::Fibres, sfx: Sfx::Velvet },
    Finish { id: 21, en: "Porcelain", ru: "фарфор", tint: true, radius: 0.32, base: "#f2f4f7", accent: "#ffffff", shade: Shade::Glass, anim: Anim::None, burst: Burst::Shards, sfx: Sfx::Glass },
    Finish { id: 22, en: "Candy", ru: "леденец", tint: true, radius: 0.48, base: "#ff5fa2", accent: "#ffffff", shade: Shade::Glass, anim: Anim::Sheen, burst: Burst::Sprinkles, sfx: Sfx::Candy },
    Finish { id: 23, en: "Chocolate", ru: "шоколад", tint: false, radius: 0.14, base: "#4a2a1c", accent: "#9a6a4c", shade: Shade::Soft, anim: Anim::None, burst: Burst::Chips, sfx: Sfx::Candy },
    Finish { id: 24, en: "Hologram", ru: "голограмма", tint: false, radius: 0.12, base: "#b8b8d8", accent: "#ffffff", shade: Shade::Metal, anim: Anim::Iridescent, burst: Burst::Sparks, sfx: Sfx::Neon },
];

/// Отделка по номеру (1..=FINISH_COUNT); 0 и мусор → отделка по цвету.
pub fn get(id: u8) -> &'static Finish {
    match id {
        1..=FINISH_COUNT => &FINISHES[id as usize - 1],
        _ => &FINISHES[for_color(id) as usize - 1],
    }
}

/// Отделка для фигуры без отделки (старые сохранения): по цвету — из
/// красящихся, чтобы цвет фигуры сохранился.
pub fn for_color(color_idx: u8) -> u8 {
    const BY_COLOR: [u8; 8] = [5, 5, 16, 22, 7, 21, 18, 6];
    BY_COLOR[(color_idx as usize).min(7)]
}

/// Отделка кубика: бонусный материал всегда выглядит собой (золото —
/// золотом), иначе своя отделка, иначе по цвету.
pub fn resolve(finish: u8, material: Material, color_idx: u8) -> u8 {
    if material != Material::Plain {
        material.finish()
    } else if (1..=FINISH_COUNT).contains(&finish) {
        finish
    } else {
        for_color(color_idx)
    }
}

/// Базовый цвет кубика отделки `f` при цвете фигуры `color` (для fallback,
/// tint текстуры и частиц).
pub fn base_color(f: &Finish, color: Color) -> Color {
    if f.tint {
        color.lerp(&hex(f.base), 0.15)
    } else {
        hex(f.base)
    }
}

// ── атлас спрайтов (tools/build_atlas.py) ──────────────────────────────────

pub const ATLAS_COLS: u32 = 8;
pub const ATLAS_ROWS: u32 = 8;
/// Тайл 192 px: блок 150 px в центре, поле 21 px под тень и свечение.
pub const ATLAS_TILE: f32 = 192.0;
pub const ATLAS_BLOCK: f32 = 150.0;
pub const ATLAS_MARGIN: f32 = 21.0;
/// Вариантов спрайта на отделку (разные вырезы текстуры).
pub const VARIANTS: u32 = 2;

/// Слоты отделки в атласе: базовые спрайты по вариантам и, для красящихся,
/// слои бликов. Порядок тот же, что в `build_atlas.py`.
pub fn slots(f: &Finish) -> ([u32; 2], Option<[u32; 2]>) {
    let mut slot = 0u32;
    for g in FINISHES.iter() {
        let mut base = [0u32; 2];
        let mut gloss = [0u32; 2];
        for v in 0..VARIANTS as usize {
            base[v] = slot;
            slot += 1;
            if g.tint {
                gloss[v] = slot;
                slot += 1;
            }
        }
        if g.id == f.id {
            return (base, if g.tint { Some(gloss) } else { None });
        }
    }
    ([0, 0], None)
}

/// UV целого тайла `slot`.
pub fn uv_slot(slot: u32) -> Rect {
    let (col, row) = (slot % ATLAS_COLS, slot / ATLAS_COLS);
    Rect::new(
        Point::new(col as f32 / ATLAS_COLS as f32, row as f32 / ATLAS_ROWS as f32),
        Size::new(1.0 / ATLAS_COLS as f32, 1.0 / ATLAS_ROWS as f32),
    )
}

/// UV базового спрайта и слоя бликов для кубика с зерном `seed`.
pub fn uv(f: &Finish, seed: u32) -> (Rect, Option<Rect>) {
    let (base, gloss) = slots(f);
    let v = (seed % VARIANTS) as usize;
    (uv_slot(base[v]), gloss.map(|g| uv_slot(g[v])))
}

/// Прямоугольник квада спрайта: блок занимает `rect`, поле тайла выходит за него.
pub fn sprite_rect(rect: Rect) -> Rect {
    let k = ATLAS_MARGIN / ATLAS_BLOCK;
    let dx = rect.width() * k;
    let dy = rect.height() * k;
    Rect::new(Point::new(rect.origin.x - dx, rect.origin.y - dy), Size::new(rect.width() + dx * 2.0, rect.height() + dy * 2.0))
}

/// Радиусы скругления силуэта для `rect`.
pub fn corner(f: &Finish, rect: Rect) -> [f32; 4] {
    [rect.width().min(rect.height()) * 0.5 * f.radius; 4]
}

pub fn hash01(seed: u32, salt: u32) -> f32 {
    let mut x = seed.wrapping_mul(0x9E37_79B9) ^ salt.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    x = x.wrapping_mul(0xC2B2_AE35);
    x ^= x >> 16;
    (x & 0xFFFF) as f32 / 65535.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_consistent() {
        for (i, f) in FINISHES.iter().enumerate() {
            assert_eq!(f.id as usize, i + 1);
            assert!(!f.en.is_empty() && !f.ru.is_empty());
            assert_eq!(get(f.id).id, f.id);
            assert!(f.radius > 0.05 && f.radius <= 0.5);
            let (base, gloss) = uv(f, 7);
            for r in [Some(base), gloss].into_iter().flatten() {
                assert!(r.origin.x >= 0.0 && r.origin.x + r.width() <= 1.0 + 1e-5);
                assert!(r.origin.y >= 0.0 && r.origin.y + r.height() <= 1.0 + 1e-5);
            }
            assert_eq!(gloss.is_some(), f.tint);
        }
        // слоты: 24 отделки × 2 варианта + 7 красящихся × 2 слоя бликов = 62 ≤ 64
        let (last, g) = slots(&FINISHES[FINISHES.len() - 1]);
        assert_eq!(last, [60, 61]);
        assert!(g.is_none());
        assert_eq!(slots(&FINISHES[4]).0, [8, 10]);
        assert_eq!(slots(&FINISHES[4]).1, Some([9, 11]));
        let sr = sprite_rect(Rect::new(Point::new(10.0, 10.0), Size::new(100.0, 100.0)));
        assert!((sr.origin.x - (10.0 - 14.0)).abs() < 1e-3 && (sr.width() - 128.0).abs() < 1e-3);
        for c in 0..8 {
            assert!((1..=FINISH_COUNT).contains(&for_color(c)));
            assert!(get(for_color(c)).tint, "отделка по цвету должна краситься");
        }
        assert_eq!(resolve(0, Material::Gold, 3), Material::Gold.finish());
        assert_eq!(resolve(0, Material::Plain, 3), for_color(3));
        assert_eq!(resolve(9, Material::Gold, 3), Material::Gold.finish());
        assert_eq!(resolve(9, Material::Plain, 3), 9);
        assert_eq!(get(Material::Gold.finish()).en, "Gold");
        assert_eq!(get(Material::Crystal.finish()).en, "Crystal");
        assert_eq!(get(Material::Wood.finish()).en, "Oak");
        assert_eq!(get(Material::Lava.finish()).en, "Lava");
    }
}
