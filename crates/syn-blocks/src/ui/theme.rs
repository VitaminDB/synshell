//! Палитра, шрифты, глифы иконок и размеры темы.

use syngui::prelude::*;

/// Семейство шрифта (системное; при отсутствии — fallback фреймворка).
pub const FONT: &str = "Inter";

// Фон
pub const BG_TOP: &str = "#5d70f2";
pub const BG_BOTTOM: &str = "#2f3db4";
pub const GLOW: &str = "#8fa0ff";

// Панель поля
pub const PANEL: &str = "#1b2150";
pub const PANEL_EDGE: &str = "#3a47a8";
pub const CELL_EMPTY: &str = "#262d63";
pub const CELL_EMPTY_HI: &str = "#2e3775";

// Текст
pub const TEXT: &str = "#ffffff";
pub const TEXT_DIM: &str = "#c6ccf8";
pub const TEXT_SHADOW: &str = "#1a2166";
pub const GOLD: &str = "#ffc93c";
pub const GOLD_DEEP: &str = "#e39a0f";

// Карточки/кнопки оверлеев
pub const CARD: &str = "#232a5e";
pub const CARD_EDGE: &str = "#4a58c2";
pub const BTN: &str = "#4f63ee";
pub const BTN_HOVER: &str = "#6a7cff";
pub const BTN_SECONDARY: &str = "#2f3a86";
pub const BTN_SECONDARY_HOVER: &str = "#3d4aa6";
pub const DANGER: &str = "#f5484b";

/// Цвета кубиков по индексу 1..=7.
pub const PALETTE: [&str; 8] = [
    "#000000", // 0 — пусто
    "#3f7ff5", // 1 blue
    "#3ccf4c", // 2 green
    "#ff7a2f", // 3 orange
    "#b35cf0", // 4 purple
    "#ffc93c", // 5 yellow
    "#33cfee", // 6 cyan
    "#f5484b", // 7 red
];

pub fn cell_color(idx: u8) -> Color {
    let i = (idx as usize).min(PALETTE.len() - 1);
    Color::from_hex(PALETTE[i])
}

pub fn hex(s: &str) -> Color {
    Color::from_hex(s)
}

/// Базовый цвет кубика с учётом материала: у особых материалов свой вид.
pub fn material_color(m: crate::game::Material, color_idx: u8) -> Color {
    use crate::game::Material::*;
    match m {
        Plain => cell_color(color_idx),
        Wood => hex("#b97a3e"),
        Crystal => hex("#8fe3ff"),
        Gold => hex("#f4c445"),
        Lava => hex("#4b3a44"),
    }
}

/// Акцентный цвет материала (частицы, подписи бонуса).
pub fn material_accent(m: crate::game::Material) -> Color {
    use crate::game::Material::*;
    match m {
        Plain => Color::WHITE,
        Wood => hex("#d9a066"),
        Crystal => hex("#bff3ff"),
        Gold => hex("#ffe27a"),
        Lava => hex("#ff7a2f"),
    }
}

// Material Icons (шрифт подключён через `with_icon_font`).
pub const ICON_TROPHY: &str = "\u{EA23}";
pub const ICON_SETTINGS: &str = "\u{E8B8}";
pub const ICON_REPLAY: &str = "\u{E5D5}"; // refresh (E0xx заняты PUA-глифами Inter)
pub const ICON_PLAY: &str = "\u{E5C8}"; // arrow_forward
pub const ICON_CLOSE: &str = "\u{E5CD}";
pub const ICON_VIEW: &str = "\u{E8F4}"; // visibility
pub const ICON_BOLT: &str = "\u{EA0B}";
pub const ICON_FIRE: &str = "\u{E80E}"; // whatshot
pub const ICON_PREV: &str = "\u{E5CB}"; // chevron_left
pub const ICON_LOOP: &str = "\u{EB3D}"; // all_inclusive — треки по кругу
pub const ICON_NEXT: &str = "\u{E5CC}"; // chevron_right
pub const ICON_REROLL: &str = "\u{E86A}"; // cached
pub const ICON_HAMMER: &str = "\u{E90E}"; // gavel
pub const ICON_BOMB: &str = "\u{E837}"; // radio_button_checked
pub const ICON_STAR: &str = "\u{E838}";
pub const ICON_CHECK_ON: &str = "\u{E834}"; // check_box
pub const ICON_CHECK_OFF: &str = "\u{E835}"; // check_box_outline_blank
pub const ICON_KEYBOARD: &str = "\u{E312}"; // не использовать с Inter (PUA)
pub const ICON_HOME: &str = "\u{E88A}"; // home
pub const ICON_FULLSCREEN: &str = "\u{E5D0}"; // fullscreen
pub const ICON_FULLSCREEN_EXIT: &str = "\u{E5D1}"; // fullscreen_exit

/// Слова похвалы по числу линий за ход.
pub fn praise(lines: usize) -> Option<&'static str> {
    match lines {
        0 | 1 => None,
        2 => Some("КЛАСС!"),
        3 => Some("ОТЛИЧНО!"),
        4 => Some("ВЕЛИКОЛЕПНО!"),
        _ => Some("НЕВЕРОЯТНО!"),
    }
}
