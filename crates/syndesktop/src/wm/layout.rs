//! Алгоритмы раскладок: по рабочей области и числу окон — прямоугольники
//! рамок окон (в тех же координатах, что и область).

use smithay::utils::{Logical, Point, Rectangle, Size};
use syndesktop_common::action::{Direction, LayoutKind};

pub type Rect = Rectangle<i32, Logical>;

#[derive(Debug, Clone, Copy)]
pub struct Params {
    pub gaps_inner: i32,
    pub gaps_outer: i32,
    pub master_ratio: f32,
    pub master_count: u32,
}

/// Прямоугольники для `n` окон в порядке их следования (первые — мастер).
pub fn arrange(kind: LayoutKind, area: Rect, n: usize, p: &Params) -> Vec<Rect> {
    if n == 0 {
        return Vec::new();
    }
    let area = shrink(area, p.gaps_outer);
    match kind {
        LayoutKind::Floating => Vec::new(),
        LayoutKind::Monocle => vec![area; n],
        LayoutKind::Tile => tile(area, n, p),
        LayoutKind::Columns => split(area, n, true, p.gaps_inner),
        LayoutKind::Grid => grid(area, n, p.gaps_inner),
    }
}

fn shrink(r: Rect, by: i32) -> Rect {
    let by = by.max(0);
    Rectangle::new(
        (r.loc.x + by, r.loc.y + by).into(),
        ((r.size.w - 2 * by).max(1), (r.size.h - 2 * by).max(1)).into(),
    )
}

/// Разрезать область на `n` равных полос (колонки или строки) с зазором.
fn split(area: Rect, n: usize, columns: bool, gap: i32) -> Vec<Rect> {
    let n_i = n as i32;
    let total = if columns { area.size.w } else { area.size.h };
    let avail = (total - gap * (n_i - 1)).max(n_i);
    let mut out = Vec::with_capacity(n);
    let mut pos = 0;
    for i in 0..n_i {
        // Остаток от деления раздаётся первым полосам по пикселю.
        let len = avail / n_i + if i < avail % n_i { 1 } else { 0 };
        let r = if columns {
            Rectangle::new((area.loc.x + pos, area.loc.y).into(), (len, area.size.h).into())
        } else {
            Rectangle::new((area.loc.x, area.loc.y + pos).into(), (area.size.w, len).into())
        };
        out.push(r);
        pos += len + gap;
    }
    out
}

fn tile(area: Rect, n: usize, p: &Params) -> Vec<Rect> {
    let m = (p.master_count as usize).clamp(1, n.max(1));
    if n <= m {
        return split(area, n, false, p.gaps_inner);
    }
    let ratio = p.master_ratio.clamp(0.1, 0.9);
    let master_w = ((area.size.w - p.gaps_inner) as f32 * ratio).round() as i32;
    let master = Rectangle::new(area.loc, (master_w, area.size.h).into());
    let stack = Rectangle::new(
        (area.loc.x + master_w + p.gaps_inner, area.loc.y).into(),
        ((area.size.w - master_w - p.gaps_inner).max(1), area.size.h).into(),
    );
    let mut out = split(master, m, false, p.gaps_inner);
    out.extend(split(stack, n - m, false, p.gaps_inner));
    out
}

fn grid(area: Rect, n: usize, gap: i32) -> Vec<Rect> {
    let cols = (n as f64).sqrt().ceil() as usize;
    let rows = n.div_ceil(cols);
    let row_rects = split(area, rows, false, gap);
    let mut out = Vec::with_capacity(n);
    let mut left = n;
    for (ri, row) in row_rects.into_iter().enumerate() {
        // Последняя строка может быть неполной — её окна шире.
        let in_row = if ri == rows - 1 { left } else { cols.min(left) };
        out.extend(split(row, in_row, true, gap));
        left -= in_row;
    }
    out
}

/// Зона прилипания окна (Super+стрелки, перетаскивание к краю).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapZone {
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl SnapZone {
    pub fn rect(self, area: Rect, gap: i32) -> Rect {
        let a = shrink(area, gap);
        let hw = (a.size.w - gap) / 2;
        let hh = (a.size.h - gap) / 2;
        let rx = a.loc.x + a.size.w - hw;
        let by = a.loc.y + a.size.h - hh;
        let (x, y, w, h) = match self {
            SnapZone::Left => (a.loc.x, a.loc.y, hw, a.size.h),
            SnapZone::Right => (rx, a.loc.y, hw, a.size.h),
            SnapZone::Top => (a.loc.x, a.loc.y, a.size.w, hh),
            SnapZone::Bottom => (a.loc.x, by, a.size.w, hh),
            SnapZone::TopLeft => (a.loc.x, a.loc.y, hw, hh),
            SnapZone::TopRight => (rx, a.loc.y, hw, hh),
            SnapZone::BottomLeft => (a.loc.x, by, hw, hh),
            SnapZone::BottomRight => (rx, by, hw, hh),
        };
        Rectangle::new((x, y).into(), (w.max(1), h.max(1)).into())
    }

    /// Super+стрелка: из текущей зоны в соседнюю (как в Plasma: влево из
    /// правой половины — вернуть, из левой — в левую и т.п.).
    pub fn step(current: Option<SnapZone>, dir: Direction) -> Option<SnapZone> {
        use SnapZone::*;
        Some(match (current, dir) {
            (None, Direction::Left) => Left,
            (None, Direction::Right) => Right,
            (None, Direction::Up) => Top,
            (None, Direction::Down) => Bottom,
            (Some(Left), Direction::Up) | (Some(Top), Direction::Left) => TopLeft,
            (Some(Left), Direction::Down) | (Some(Bottom), Direction::Left) => BottomLeft,
            (Some(Right), Direction::Up) | (Some(Top), Direction::Right) => TopRight,
            (Some(Right), Direction::Down) | (Some(Bottom), Direction::Right) => BottomRight,
            (Some(TopLeft), Direction::Down) | (Some(BottomLeft), Direction::Up) => Left,
            (Some(TopRight), Direction::Down) | (Some(BottomRight), Direction::Up) => Right,
            (Some(TopLeft), Direction::Right) | (Some(TopRight), Direction::Left) => Top,
            (Some(BottomLeft), Direction::Right) | (Some(BottomRight), Direction::Left) => Bottom,
            (Some(Left), Direction::Right) | (Some(Right), Direction::Left) => return None,
            (Some(Top), Direction::Down) | (Some(Bottom), Direction::Up) => return None,
            (Some(z), _) => z,
        })
    }

    /// Зона по положению указателя у края рабочей области при перетаскивании.
    pub fn from_pointer(area: Rect, p: Point<f64, Logical>, edge: f64) -> Option<SnapZone> {
        let (l, t) = (area.loc.x as f64, area.loc.y as f64);
        let (r, b) = (l + area.size.w as f64, t + area.size.h as f64);
        let corner = edge * 12.0;
        let near_l = p.x <= l + edge;
        let near_r = p.x >= r - edge;
        let near_t = p.y <= t + edge;
        Some(match () {
            _ if near_l && p.y <= t + corner => SnapZone::TopLeft,
            _ if near_l && p.y >= b - corner => SnapZone::BottomLeft,
            _ if near_r && p.y <= t + corner => SnapZone::TopRight,
            _ if near_r && p.y >= b - corner => SnapZone::BottomRight,
            _ if near_l => SnapZone::Left,
            _ if near_r => SnapZone::Right,
            // Верхний край — развернуть (вызывающий трактует Top как maximize).
            _ if near_t => SnapZone::Top,
            _ => return None,
        })
    }
}

/// Размер по умолчанию для нового плавающего окна без собственного размера.
pub fn default_float_size(area: Rect) -> Size<i32, Logical> {
    ((area.size.w as f64 * 0.6) as i32, (area.size.h as f64 * 0.65) as i32).into()
}

/// Индекс ближайшего соседа по направлению (для focus/move left/right…).
pub fn neighbor(rects: &[Rect], from: usize, dir: Direction) -> Option<usize> {
    let c = center(rects[from]);
    rects
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != from)
        .filter_map(|(i, r)| {
            let o = center(*r);
            let (dx, dy) = (o.0 - c.0, o.1 - c.1);
            let ok = match dir {
                Direction::Left => dx < -1.0,
                Direction::Right => dx > 1.0,
                Direction::Up => dy < -1.0,
                Direction::Down => dy > 1.0,
            };
            if !ok {
                return None;
            }
            // Основная ось весит меньше поперечной: предпочитаем соседей «в линию».
            let (main, cross) = match dir {
                Direction::Left | Direction::Right => (dx.abs(), dy.abs()),
                Direction::Up | Direction::Down => (dy.abs(), dx.abs()),
            };
            Some((i, main + cross * 2.0))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

fn center(r: Rect) -> (f64, f64) {
    (r.loc.x as f64 + r.size.w as f64 / 2.0, r.loc.y as f64 + r.size.h as f64 / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Rect {
        Rectangle::new((0, 0).into(), (1000, 600).into())
    }

    fn p() -> Params {
        Params { gaps_inner: 10, gaps_outer: 0, master_ratio: 0.5, master_count: 1 }
    }

    #[test]
    fn tile_master_and_stack() {
        let r = arrange(LayoutKind::Tile, area(), 3, &p());
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].loc, (0, 0).into());
        assert_eq!(r[0].size.h, 600);
        assert_eq!(r[1].loc.x, r[0].size.w + 10);
        assert_eq!(r[1].size.h + r[2].size.h + 10, 600);
    }

    #[test]
    fn columns_cover_width() {
        let r = arrange(LayoutKind::Columns, area(), 3, &p());
        let total: i32 = r.iter().map(|r| r.size.w).sum::<i32>() + 20;
        assert_eq!(total, 1000);
    }

    #[test]
    fn grid_counts() {
        for n in 1..10 {
            assert_eq!(arrange(LayoutKind::Grid, area(), n, &p()).len(), n);
        }
    }

    #[test]
    fn snap_steps() {
        assert_eq!(SnapZone::step(None, Direction::Left), Some(SnapZone::Left));
        assert_eq!(SnapZone::step(Some(SnapZone::Left), Direction::Up), Some(SnapZone::TopLeft));
        assert_eq!(SnapZone::step(Some(SnapZone::Left), Direction::Right), None);
    }

    #[test]
    fn neighbors() {
        let r = arrange(LayoutKind::Tile, area(), 3, &p());
        assert_eq!(neighbor(&r, 0, Direction::Right), Some(1));
        assert_eq!(neighbor(&r, 2, Direction::Up), Some(1));
        assert_eq!(neighbor(&r, 1, Direction::Left), Some(0));
    }
}
