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
    /// Текстура нейтральная — красится цветом фигуры.
    pub tint: bool,
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
    Finish { id: 1, en: "Gold", ru: "золото", tint: false, base: "#e9b83a", accent: "#fff1a8", shade: Shade::Metal, anim: Anim::Sheen, burst: Burst::Coins, sfx: Sfx::Gold },
    Finish { id: 2, en: "Silver", ru: "серебро", tint: false, base: "#c9ced6", accent: "#ffffff", shade: Shade::Metal, anim: Anim::Sheen, burst: Burst::Coins, sfx: Sfx::Metal },
    Finish { id: 3, en: "Copper", ru: "медь", tint: false, base: "#c8703c", accent: "#ffc79a", shade: Shade::Metal, anim: Anim::Sheen, burst: Burst::Coins, sfx: Sfx::Metal },
    Finish { id: 4, en: "Bronze", ru: "бронза", tint: false, base: "#9a6a34", accent: "#e2b878", shade: Shade::Metal, anim: Anim::None, burst: Burst::Coins, sfx: Sfx::Metal },
    Finish { id: 5, en: "Glass", ru: "стекло", tint: true, base: "#9fd3ff", accent: "#ffffff", shade: Shade::Glass, anim: Anim::None, burst: Burst::Shards, sfx: Sfx::Glass },
    Finish { id: 6, en: "Crystal", ru: "хрусталь", tint: true, base: "#bfeaff", accent: "#ffffff", shade: Shade::Glass, anim: Anim::Sparkle, burst: Burst::Shards, sfx: Sfx::Crystal },
    Finish { id: 7, en: "Velvet", ru: "велюр", tint: true, base: "#b0407a", accent: "#ffd0ea", shade: Shade::Soft, anim: Anim::None, burst: Burst::Fibres, sfx: Sfx::Velvet },
    Finish { id: 8, en: "Oak", ru: "дуб", tint: false, base: "#c48b4c", accent: "#f0c88c", shade: Shade::Matte, anim: Anim::None, burst: Burst::Chips, sfx: Sfx::Wood },
    Finish { id: 9, en: "Walnut", ru: "орех", tint: false, base: "#5f3a22", accent: "#b07a4c", shade: Shade::Matte, anim: Anim::None, burst: Burst::Chips, sfx: Sfx::Wood },
    Finish { id: 10, en: "Marble", ru: "мрамор", tint: false, base: "#e9e6e1", accent: "#ffffff", shade: Shade::Glass, anim: Anim::None, burst: Burst::Dust, sfx: Sfx::Stone },
    Finish { id: 11, en: "Granite", ru: "гранит", tint: false, base: "#7d7f86", accent: "#d0d2d8", shade: Shade::Matte, anim: Anim::None, burst: Burst::Dust, sfx: Sfx::Stone },
    Finish { id: 12, en: "Obsidian", ru: "обсидиан", tint: false, base: "#1b1a22", accent: "#8f8fb0", shade: Shade::Glass, anim: Anim::Sheen, burst: Burst::Shards, sfx: Sfx::Glass },
    Finish { id: 13, en: "Lava", ru: "лава", tint: false, base: "#3a2426", accent: "#ff7a2f", shade: Shade::Glow, anim: Anim::Pulse, burst: Burst::Embers, sfx: Sfx::Lava },
    Finish { id: 14, en: "Ice", ru: "лёд", tint: false, base: "#bfe6ff", accent: "#ffffff", shade: Shade::Glass, anim: Anim::Sparkle, burst: Burst::Shards, sfx: Sfx::Ice },
    Finish { id: 15, en: "Jade", ru: "нефрит", tint: false, base: "#4f9a6a", accent: "#bfe9c8", shade: Shade::Glass, anim: Anim::None, burst: Burst::Dust, sfx: Sfx::Stone },
    Finish { id: 16, en: "Gem", ru: "самоцвет", tint: true, base: "#e04a6a", accent: "#ffffff", shade: Shade::Glass, anim: Anim::Sparkle, burst: Burst::Shards, sfx: Sfx::Crystal },
    Finish { id: 17, en: "Pearl", ru: "перламутр", tint: false, base: "#efe3ea", accent: "#ffffff", shade: Shade::Glass, anim: Anim::Iridescent, burst: Burst::Dust, sfx: Sfx::Glass },
    Finish { id: 18, en: "Neon", ru: "неон", tint: true, base: "#2ad4ff", accent: "#ffffff", shade: Shade::Glow, anim: Anim::Pulse, burst: Burst::Sparks, sfx: Sfx::Neon },
    Finish { id: 19, en: "Leather", ru: "кожа", tint: false, base: "#7a4a2b", accent: "#c8905e", shade: Shade::Soft, anim: Anim::None, burst: Burst::Chips, sfx: Sfx::Velvet },
    Finish { id: 20, en: "Denim", ru: "джинса", tint: false, base: "#3f5f96", accent: "#a9c2ee", shade: Shade::Soft, anim: Anim::None, burst: Burst::Fibres, sfx: Sfx::Velvet },
    Finish { id: 21, en: "Porcelain", ru: "фарфор", tint: true, base: "#f2f4f7", accent: "#ffffff", shade: Shade::Glass, anim: Anim::None, burst: Burst::Shards, sfx: Sfx::Glass },
    Finish { id: 22, en: "Candy", ru: "леденец", tint: true, base: "#ff5fa2", accent: "#ffffff", shade: Shade::Glass, anim: Anim::Sheen, burst: Burst::Sprinkles, sfx: Sfx::Candy },
    Finish { id: 23, en: "Chocolate", ru: "шоколад", tint: false, base: "#4a2a1c", accent: "#9a6a4c", shade: Shade::Soft, anim: Anim::None, burst: Burst::Chips, sfx: Sfx::Candy },
    Finish { id: 24, en: "Hologram", ru: "голограмма", tint: false, base: "#b8b8d8", accent: "#ffffff", shade: Shade::Metal, anim: Anim::Iridescent, burst: Burst::Sparks, sfx: Sfx::Neon },
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

// ── атлас ──────────────────────────────────────────────────────────────────

pub const ATLAS_COLS: u32 = 8;
pub const ATLAS_ROWS: u32 = 4;
/// Тайл 256 px, из них поле 16 px с каждой стороны — продолжение краёв
/// (защита от протечек соседей на mip-уровнях), содержимое — 224 px.
pub const ATLAS_TILE: f32 = 256.0;
pub const ATLAS_GUTTER: f32 = 16.0;
/// Доля содержимого тайла, попадающая в один кубик.
pub const WINDOW: f32 = 0.72;

/// UV-окно тайла отделки `f` для кубика с зерном `seed`: часть тайла со
/// смещением по зерну — соседние кубики одной отделки выглядят по-разному.
pub fn uv(f: &Finish, seed: u32) -> Rect {
    let slot = f.id as u32 - 1;
    let (col, row) = (slot % ATLAS_COLS, slot / ATLAS_COLS);
    let content = (ATLAS_TILE - ATLAS_GUTTER * 2.0) / ATLAS_TILE;
    let win = content * WINDOW;
    let free = content - win;
    let u0 = (col as f32 * ATLAS_TILE + ATLAS_GUTTER) / (ATLAS_TILE * ATLAS_COLS as f32) + free * hash01(seed, 1) / ATLAS_COLS as f32;
    let v0 = (row as f32 * ATLAS_TILE + ATLAS_GUTTER) / (ATLAS_TILE * ATLAS_ROWS as f32) + free * hash01(seed, 2) / ATLAS_ROWS as f32;
    Rect::new(Point::new(u0, v0), Size::new(win / ATLAS_COLS as f32, win / ATLAS_ROWS as f32))
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
            let uv = uv(f, 7);
            assert!(uv.origin.x >= 0.0 && uv.origin.x + uv.width() <= 1.0 + 1e-5);
            assert!(uv.origin.y >= 0.0 && uv.origin.y + uv.height() <= 1.0 + 1e-5);
        }
        assert_eq!(FINISHES.len() as u32, ATLAS_COLS * ATLAS_ROWS - 8);
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
