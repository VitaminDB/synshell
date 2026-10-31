//! Значки панели (Material Icons, шрифт подключает приложение).

#![allow(dead_code)]

pub const PEN: &str = "\u{E3C9}"; // edit
pub const BALLPOINT: &str = "\u{E150}"; // create
pub const PENCIL: &str = "\u{E254}"; // mode_edit
pub const HIGHLIGHTER: &str = "\u{E25F}"; // highlight
pub const BRUSH: &str = "\u{E3AE}"; // brush
pub const CALLIGRAPHY: &str = "\u{E155}"; // gesture
pub const ERASER: &str = "\u{E664}"; // auto_fix_normal
pub const LASSO: &str = "\u{EF52}"; // highlight_alt
pub const SHAPES: &str = "\u{E574}"; // category
pub const TEXT: &str = "\u{E264}"; // title
pub const FILL: &str = "\u{E23A}"; // format_color_fill
pub const EYEDROPPER: &str = "\u{E3B8}"; // colorize
pub const HAND: &str = "\u{E925}"; // pan_tool
pub const UNDO: &str = "\u{E166}";
pub const REDO: &str = "\u{E15A}";
pub const LAYERS: &str = "\u{E53B}";
pub const MORE: &str = "\u{E5D4}";
pub const PALETTE: &str = "\u{E40A}";
pub const WIDTH: &str = "\u{E91A}"; // line_weight
pub const ADD: &str = "\u{E145}";
pub const DELETE: &str = "\u{E872}";
pub const COPY: &str = "\u{E14D}";
pub const CUT: &str = "\u{E14E}";
pub const PASTE: &str = "\u{E14F}";
pub const VISIBLE: &str = "\u{E8F4}";
pub const HIDDEN: &str = "\u{E8F5}";
pub const LOCK: &str = "\u{E897}";
pub const UNLOCK: &str = "\u{E898}";
pub const UP: &str = "\u{E5D8}";
pub const DOWN: &str = "\u{E5DB}";
pub const FIT: &str = "\u{EA10}";
pub const EXPORT: &str = "\u{E171}"; // save_alt
pub const MAGIC: &str = "\u{E663}"; // auto_fix_high
pub const LINE: &str = "\u{E15B}"; // remove
pub const ARROW: &str = "\u{E5C8}"; // arrow_forward
pub const RECT: &str = "\u{E3C6}"; // crop_square
pub const ELLIPSE: &str = "\u{E40C}"; // panorama_fish_eye
pub const TRIANGLE: &str = "\u{E86B}"; // change_history
pub const TOUCH: &str = "\u{E913}"; // touch_app
pub const NO_TOUCH: &str = "\u{F1B0}"; // do_not_touch
pub const CHECK: &str = "\u{E5CA}";
pub const TEMPLATE: &str = "\u{E3EC}"; // grid_on
pub const IMAGE: &str = "\u{E3F4}";
pub const CLEAR_PAGE: &str = "\u{E16C}"; // delete_sweep
pub const PREV: &str = "\u{E5CB}"; // chevron_left
pub const NEXT: &str = "\u{E5CC}"; // chevron_right
pub const PAGES: &str = "\u{E8F0}"; // view_module
pub const CLOSE: &str = "\u{E5CD}";

use super::model::{Brush, ShapeKind};
use super::Tool;

pub fn brush(b: Brush) -> &'static str {
    match b {
        Brush::Fountain => PEN,
        Brush::Ballpoint => BALLPOINT,
        Brush::Pencil => PENCIL,
        Brush::Highlighter => HIGHLIGHTER,
        Brush::Brush => BRUSH,
        Brush::Calligraphy => CALLIGRAPHY,
    }
}

pub fn shape(k: ShapeKind) -> &'static str {
    match k {
        ShapeKind::Line | ShapeKind::Polyline => LINE,
        ShapeKind::Arrow => ARROW,
        ShapeKind::Rect | ShapeKind::Polygon => RECT,
        ShapeKind::Ellipse => ELLIPSE,
        ShapeKind::Triangle => TRIANGLE,
    }
}

pub fn tool(t: Tool) -> &'static str {
    match t {
        Tool::Pen => PEN,
        Tool::Eraser => ERASER,
        Tool::Lasso => LASSO,
        Tool::Shape => SHAPES,
        Tool::Text => TEXT,
        Tool::Fill => FILL,
        Tool::Eyedropper => EYEDROPPER,
        Tool::Hand => HAND,
    }
}
