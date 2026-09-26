//! Поле 8×8: размещение, полные линии, превью очистки.

use super::piece::{Material, Piece, Shape};

pub const SIZE: usize = 8;

/// 0 — пусто; иначе младший ниббл — цвет (1..=7), старший — материал.
pub type Cell = u8;

#[inline]
pub fn make_cell(color: u8, material: Material) -> Cell {
    (color & 0x0F) | (material.index() << 4)
}

#[inline]
pub fn cell_color(cell: Cell) -> u8 {
    cell & 0x0F
}

#[inline]
pub fn cell_material(cell: Cell) -> Material {
    Material::from_index(cell >> 4)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Board {
    cells: [[Cell; SIZE]; SIZE],
}

impl Default for Board {
    fn default() -> Self {
        Self::new()
    }
}

/// Результат схлопывания линий.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Clear {
    pub rows: Vec<usize>,
    pub cols: Vec<usize>,
    /// Реально очищенные клетки `(row, col, цвет)` — пересечения один раз.
    pub cells: Vec<(usize, usize, Cell)>,
}

impl Clear {
    pub fn lines(&self) -> usize {
        self.rows.len() + self.cols.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty() && self.cols.is_empty()
    }
}

impl Board {
    pub const fn new() -> Self {
        Board { cells: [[0; SIZE]; SIZE] }
    }

    #[inline]
    pub fn in_bounds(r: i32, c: i32) -> bool {
        r >= 0 && r < SIZE as i32 && c >= 0 && c < SIZE as i32
    }

    #[inline]
    pub fn get(&self, r: usize, c: usize) -> Cell {
        self.cells[r][c]
    }

    #[inline]
    pub fn set(&mut self, r: usize, c: usize, v: Cell) {
        self.cells[r][c] = v;
    }

    #[inline]
    pub fn is_free(&self, r: i32, c: i32) -> bool {
        Self::in_bounds(r, c) && self.cells[r as usize][c as usize] == 0
    }

    pub fn rows(&self) -> &[[Cell; SIZE]; SIZE] {
        &self.cells
    }

    /// Можно ли поставить форму якорем (левый-верх рамки) в `(r, c)`.
    pub fn can_place(&self, s: &Shape, r: i32, c: i32) -> bool {
        s.cells()
            .iter()
            .all(|&(dr, dc)| self.is_free(r + dr as i32, c + dc as i32))
    }

    /// Ставит фигуру (без проверки) и возвращает занятые клетки.
    pub fn place(&mut self, p: &Piece, r: i32, c: i32) -> Vec<(usize, usize)> {
        let mut out = Vec::with_capacity(p.len());
        let v = make_cell(p.color, p.material);
        for &(dr, dc) in p.shape.cells() {
            let (rr, cc) = ((r + dr as i32) as usize, (c + dc as i32) as usize);
            self.cells[rr][cc] = v;
            out.push((rr, cc));
        }
        out
    }

    pub fn full_rows(&self) -> Vec<usize> {
        (0..SIZE)
            .filter(|&r| self.cells[r].iter().all(|&v| v != 0))
            .collect()
    }

    pub fn full_cols(&self) -> Vec<usize> {
        (0..SIZE)
            .filter(|&c| (0..SIZE).all(|r| self.cells[r][c] != 0))
            .collect()
    }

    /// Схлопывает все полные ряды и колонки.
    pub fn clear_full_lines(&mut self) -> Clear {
        let rows = self.full_rows();
        let cols = self.full_cols();
        let mut cells = Vec::new();
        if rows.is_empty() && cols.is_empty() {
            return Clear::default();
        }
        for r in 0..SIZE {
            for c in 0..SIZE {
                if rows.contains(&r) || cols.contains(&c) {
                    let v = self.cells[r][c];
                    if v != 0 {
                        cells.push((r, c, v));
                    }
                    self.cells[r][c] = 0;
                }
            }
        }
        Clear { rows, cols, cells }
    }

    /// Какие ряды/колонки заполнятся, если поставить форму в `(r, c)`.
    /// Поле не меняется. Пустой результат, если ставить нельзя.
    pub fn preview_lines(&self, s: &Shape, r: i32, c: i32) -> (Vec<usize>, Vec<usize>) {
        if !self.can_place(s, r, c) {
            return (Vec::new(), Vec::new());
        }
        let mut tmp = *self;
        for &(dr, dc) in s.cells() {
            tmp.cells[(r + dr as i32) as usize][(c + dc as i32) as usize] = 1;
        }
        (tmp.full_rows(), tmp.full_cols())
    }

    /// Ставится ли форма хоть куда-нибудь.
    pub fn fits_anywhere(&self, s: &Shape) -> bool {
        self.first_fit(s).is_some()
    }

    pub fn first_fit(&self, s: &Shape) -> Option<(i32, i32)> {
        for r in 0..=(SIZE as i32 - s.h as i32) {
            for c in 0..=(SIZE as i32 - s.w as i32) {
                if self.can_place(s, r, c) {
                    return Some((r, c));
                }
            }
        }
        None
    }

    /// Число позиций, куда ставится форма (мера «свободы»).
    pub fn fit_count(&self, s: &Shape) -> usize {
        let mut n = 0;
        for r in 0..=(SIZE as i32 - s.h as i32) {
            for c in 0..=(SIZE as i32 - s.w as i32) {
                if self.can_place(s, r, c) {
                    n += 1;
                }
            }
        }
        n
    }

    pub fn filled(&self) -> usize {
        self.cells
            .iter()
            .map(|row| row.iter().filter(|&&v| v != 0).count())
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.filled() == 0
    }

    pub fn fill_ratio(&self) -> f32 {
        self.filled() as f32 / (SIZE * SIZE) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::piece::Family;

    fn sq() -> Piece {
        Piece::of(Family::O2)
    }

    #[test]
    fn can_place_bounds_and_occupancy() {
        let mut b = Board::new();
        let bar = Family::I5.base();
        assert!(b.can_place(&bar, 0, 3));
        assert!(!b.can_place(&bar, 0, 4));
        assert!(!b.can_place(&sq().shape, 7, 7));
        assert!(b.can_place(&sq().shape, 6, 6));
        b.set(2, 2, 7);
        assert!(!b.can_place(&sq().shape, 1, 1));
        assert!(b.can_place(&sq().shape, 0, 0));
    }

    #[test]
    fn clear_row_col_and_intersection() {
        let mut b = Board::new();
        for c in 0..SIZE {
            b.set(2, c, 2);
        }
        for r in 0..SIZE {
            b.set(r, 1, 4);
        }
        let cl = b.clear_full_lines();
        assert_eq!(cl.rows, vec![2]);
        assert_eq!(cl.cols, vec![1]);
        assert_eq!(cl.cells.len(), 15);
        assert_eq!(cl.lines(), 2);
        assert!(b.is_empty());
    }

    #[test]
    fn partial_lines_stay() {
        let mut b = Board::new();
        for c in 0..SIZE {
            b.set(0, c, 1);
        }
        b.set(1, 0, 1);
        let cl = b.clear_full_lines();
        assert_eq!(cl.lines(), 1);
        assert_eq!(b.get(1, 0), 1);
    }

    #[test]
    fn preview_matches_actual_clear() {
        let mut b = Board::new();
        for c in 0..6 {
            b.set(5, c, 3);
        }
        let bar = Family::I2.base();
        let (rows, cols) = b.preview_lines(&bar, 5, 6);
        assert_eq!(rows, vec![5]);
        assert!(cols.is_empty());
        assert_eq!(b.filled(), 6, "preview must not mutate");
        // невалидная позиция — пусто
        let (r2, c2) = b.preview_lines(&bar, 5, 7);
        assert!(r2.is_empty() && c2.is_empty());
    }

    #[test]
    fn checkerboard_traps_dominoes() {
        let mut b = Board::new();
        for r in 0..SIZE {
            for c in 0..SIZE {
                if (r + c) % 2 == 1 {
                    b.set(r, c, 9);
                }
            }
        }
        assert!(!b.fits_anywhere(&Family::I2.base()));
        assert!(b.fits_anywhere(&Family::Mono.base()));
        assert_eq!(b.fit_count(&Family::Mono.base()), 32);
        assert!((b.fill_ratio() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn place_returns_cells() {
        let mut b = Board::new();
        let cells = b.place(&sq(), 3, 4);
        assert_eq!(cells, vec![(3, 4), (3, 5), (4, 4), (4, 5)]);
        assert_eq!(b.get(4, 5), Family::O2.color());
        assert_eq!(cell_material(b.get(4, 5)), Material::Plain);
        let gold = sq().with_material(Material::Gold);
        b.place(&gold, 0, 0);
        assert_eq!(cell_color(b.get(0, 0)), Family::O2.color());
        assert_eq!(cell_material(b.get(0, 0)), Material::Gold);
        assert_ne!(b.get(0, 0), 0);
    }
}
