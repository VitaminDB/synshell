//! Адаптивная геометрия экрана: колонка контента, HUD, поле, трей.
//!
//! Всё считается из прямоугольника элемента, поэтому работает и в узком
//! портретном окне, и в широком, и на телефоне.

use crate::game::{Shape, SIZE};
use syngui::prelude::*;

#[derive(Clone, Copy, Debug)]
pub struct Geo {
    pub bounds: Rect,
    /// Колонка контента (ширина = сторона панели поля).
    pub col: Rect,
    /// Верхняя строка HUD: рекорд слева, меню справа.
    pub top: Rect,
    pub best: Rect,
    pub menu_btn: Rect,
    /// Большой счёт.
    pub score: Rect,
    /// Панель поля (с внутренним отступом).
    pub board: Rect,
    /// Левый-верхний угол клетки (0,0).
    pub grid: Point,
    pub cell: f32,
    pub pad: f32,
    pub tray: Rect,
    pub slots: [Rect; 3],
    pub tray_cell: f32,
    /// Панель способностей под треем.
    pub abil: Rect,
    pub abil_btns: [Rect; 3],
    pub energy_bar: Rect,
}

pub const HUD_H: f32 = 104.0;
pub const SIDE: f32 = 16.0;
/// Зазор HUD → поле; в нём живёт плашка режима блоков.
pub const GAP_BOARD: f32 = 34.0;
pub const GAP_TRAY: f32 = 18.0;
pub const BOTTOM: f32 = 16.0;
pub const ABIL_H: f32 = 62.0;
pub const MAX_COL: f32 = 560.0;

pub fn compute(b: Rect) -> Geo {
    let w = b.width();
    let h = b.height();
    let fixed = HUD_H + GAP_BOARD + GAP_TRAY + BOTTOM + ABIL_H;
    // трей = 0.42 стороны поля, минимум 100
    let col = (w - SIDE * 2.0)
        .min((h - fixed) / 1.42)
        .min(MAX_COL)
        .max(160.0);
    let tray_h = (col * 0.42).max(100.0);
    let total = fixed + col + tray_h;
    let free = (h - total).max(0.0);
    // Свободное место почти целиком уходит вниз: HUD прижат к верху экрана,
    // трей — ниже, под большим пальцем.
    let y0 = b.origin.y + free * 0.1;
    let x0 = b.origin.x + (w - col) * 0.5;

    let top = Rect::new(Point::new(x0, y0 + 8.0), Size::new(col, 40.0));
    let menu_btn = Rect::new(Point::new(x0 + col - 40.0, y0 + 8.0), Size::new(40.0, 40.0));
    let best = Rect::new(Point::new(x0, y0 + 8.0), Size::new(col - 48.0, 40.0));
    let score = Rect::new(Point::new(x0, y0 + 46.0), Size::new(col, 58.0));

    let board = Rect::new(Point::new(x0, y0 + HUD_H + GAP_BOARD), Size::new(col, col));
    let pad = (col * 0.028).max(6.0);
    let cell = (col - pad * 2.0) / SIZE as f32;
    let grid = Point::new(board.origin.x + pad, board.origin.y + pad);

    let tray_y = board.origin.y + col + GAP_TRAY;
    let tray = Rect::new(Point::new(x0, tray_y), Size::new(col, tray_h));
    let slot_w = col / 3.0;
    let slots = [
        Rect::new(Point::new(x0, tray_y), Size::new(slot_w, tray_h)),
        Rect::new(Point::new(x0 + slot_w, tray_y), Size::new(slot_w, tray_h)),
        Rect::new(Point::new(x0 + slot_w * 2.0, tray_y), Size::new(slot_w, tray_h)),
    ];
    // самая крупная фигура — 5 клеток в ряд / 3×3; должна влезть в слот
    let tray_cell = (cell * 0.62)
        .min((tray_h - 14.0) / 5.0)
        .min((slot_w - 12.0) / 5.0)
        .max(8.0);

    let abil = Rect::new(Point::new(x0, tray_y + tray_h), Size::new(col, ABIL_H));
    let bw = 100.0f32.min((col - 24.0) / 3.0);
    let gap = 12.0;
    let total_w = bw * 3.0 + gap * 2.0;
    let bx = x0 + (col - total_w) * 0.5;
    let by = abil.origin.y + 14.0;
    let abil_btns = [
        Rect::new(Point::new(bx, by), Size::new(bw, 44.0)),
        Rect::new(Point::new(bx + bw + gap, by), Size::new(bw, 44.0)),
        Rect::new(Point::new(bx + (bw + gap) * 2.0, by), Size::new(bw, 44.0)),
    ];
    let energy_bar = Rect::new(Point::new(bx, abil.origin.y + 3.0), Size::new(total_w, 6.0));
    Geo { bounds: b, col: Rect::new(Point::new(x0, y0), Size::new(col, total)), top, best, menu_btn, score, board, grid, cell, pad, tray, slots, tray_cell, abil, abil_btns, energy_bar }
}

impl Geo {
    /// Внешний прямоугольник клетки.
    pub fn cell_rect(&self, r: usize, c: usize) -> Rect {
        Rect::new(
            Point::new(self.grid.x + c as f32 * self.cell, self.grid.y + r as f32 * self.cell),
            Size::new(self.cell, self.cell),
        )
    }

    /// Зазор между кубиками.
    pub fn gap(&self) -> f32 {
        (self.cell * 0.045).max(1.0)
    }

    /// Прямоугольник кубика внутри клетки (с зазором).
    pub fn cube_rect(&self, r: usize, c: usize) -> Rect {
        inset(self.cell_rect(r, c), self.gap())
    }

    /// Клетка под точкой (только внутри сетки).
    pub fn point_to_cell(&self, p: Point) -> Option<(i32, i32)> {
        let lx = p.x - self.grid.x;
        let ly = p.y - self.grid.y;
        let side = self.cell * SIZE as f32;
        if lx < 0.0 || ly < 0.0 || lx >= side || ly >= side {
            return None;
        }
        Some(((ly / self.cell) as i32, (lx / self.cell) as i32))
    }

    /// Якорь фигуры (левый-верх рамки) по положению её левого-верхнего угла
    /// в пикселях — округление к ближайшей клетке.
    pub fn anchor_for_origin(&self, origin: Point) -> (i32, i32) {
        let c = ((origin.x - self.grid.x) / self.cell).round() as i32;
        let r = ((origin.y - self.grid.y) / self.cell).round() as i32;
        (r, c)
    }

    pub fn slot_at(&self, p: Point) -> Option<usize> {
        self.slots.iter().position(|s| s.contains(p))
    }

    pub fn ability_at(&self, p: Point) -> Option<usize> {
        self.abil_btns.iter().position(|r| inset(*r, -4.0).contains(p))
    }

    /// Левый-верх фигуры, отцентрированной в слоте (масштаб трея).
    pub fn piece_origin_in_slot(&self, slot: usize, s: &Shape) -> Point {
        let r = self.slots[slot];
        let w = s.w as f32 * self.tray_cell;
        let h = s.h as f32 * self.tray_cell;
        Point::new(r.origin.x + (r.width() - w) * 0.5, r.origin.y + (r.height() - h) * 0.5)
    }

    /// Прямоугольник, занимаемый фигурой в слоте (для хит-теста).
    pub fn piece_rect_in_slot(&self, slot: usize, s: &Shape) -> Rect {
        let o = self.piece_origin_in_slot(slot, s);
        Rect::new(o, Size::new(s.w as f32 * self.tray_cell, s.h as f32 * self.tray_cell))
    }

    /// Пилюля «КОМБО ×N» по центру верхней строки.
    pub fn combo_pill(&self) -> Rect {
        let pw = 132.0;
        Rect::new(Point::new(self.top.origin.x + (self.top.width() - pw) * 0.5, self.top.origin.y + 6.0), Size::new(pw, 28.0))
    }

    /// Сторона сетки в пикселях.
    pub fn grid_side(&self) -> f32 {
        self.cell * SIZE as f32
    }
}

pub fn inset(r: Rect, d: f32) -> Rect {
    Rect::new(
        Point::new(r.origin.x + d, r.origin.y + d),
        Size::new((r.width() - d * 2.0).max(0.0), (r.height() - d * 2.0).max(0.0)),
    )
}

pub fn scaled_about_center(r: Rect, s: f32) -> Rect {
    let cx = r.origin.x + r.width() * 0.5;
    let cy = r.origin.y + r.height() * 0.5;
    let w = r.width() * s;
    let h = r.height() * s;
    Rect::new(Point::new(cx - w * 0.5, cy - h * 0.5), Size::new(w, h))
}

pub fn offset(r: Rect, dx: f32, dy: f32) -> Rect {
    Rect::new(Point::new(r.origin.x + dx, r.origin.y + dy), r.size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::Family;

    #[test]
    fn portrait_fits_inside_window() {
        let g = compute(Rect::new(Point::new(0.0, 0.0), Size::new(440.0, 800.0)));
        assert!(g.board.width() <= 440.0 - SIDE * 2.0 + 0.01);
        assert!(g.abil.origin.y + g.abil.height() <= 800.0 + 0.01);
        assert!(g.cell > 38.0);
        assert!(g.tray_cell * 5.0 <= g.slots[0].width());
    }

    #[test]
    fn landscape_is_limited_by_height() {
        let g = compute(Rect::new(Point::new(0.0, 0.0), Size::new(1400.0, 700.0)));
        assert!(g.col.origin.y >= 0.0);
        assert!(g.abil.origin.y + g.abil.height() <= 700.0 + 0.01);
        // колонка центрирована
        let cx = g.board.origin.x + g.board.width() * 0.5;
        assert!((cx - 700.0).abs() < 1.0);
    }

    #[test]
    fn cell_mapping_roundtrips() {
        let g = compute(Rect::new(Point::new(20.0, 30.0), Size::new(400.0, 760.0)));
        for r in 0..SIZE {
            for c in 0..SIZE {
                let cr = g.cell_rect(r, c);
                let center = Point::new(cr.origin.x + cr.width() * 0.5, cr.origin.y + cr.height() * 0.5);
                assert_eq!(g.point_to_cell(center), Some((r as i32, c as i32)));
                assert_eq!(g.anchor_for_origin(Point::new(cr.origin.x + 3.0, cr.origin.y - 4.0)), (r as i32, c as i32));
            }
        }
        assert_eq!(g.point_to_cell(Point::new(0.0, 0.0)), None);
        let s = Family::I5.base();
        let pr = g.piece_rect_in_slot(1, &s);
        assert!(g.slots[1].contains(Point::new(pr.origin.x + 1.0, pr.origin.y + 1.0)));
    }
}
