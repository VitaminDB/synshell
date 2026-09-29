//! Фигуры: формы-полимино, семейства с весом и цветом, каталог ориентаций.
//!
//! Поворота во время игры нет (как в жанре) — вместо этого каталог заранее
//! содержит все уникальные ориентации каждого семейства.

use std::sync::OnceLock;

/// Самая крупная фигура — квадрат 3×3.
pub const MAX_CELLS: usize = 9;

/// Форма: клетки `(row, col)` в плотной рамке `w × h`, отсортированные.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Shape {
    cells: [(u8, u8); MAX_CELLS],
    n: u8,
    pub w: u8,
    pub h: u8,
}

impl Shape {
    /// Собирает форму из списка клеток; клетки сортируются, рамка считается.
    pub const fn from_cells(src: &[(u8, u8)]) -> Self {
        let mut cells = [(0u8, 0u8); MAX_CELLS];
        let mut n = 0usize;
        while n < src.len() && n < MAX_CELLS {
            cells[n] = src[n];
            n += 1;
        }
        // сортировка вставками (const fn)
        let mut i = 1;
        while i < n {
            let key = cells[i];
            let mut j = i;
            while j > 0 && (cells[j - 1].0 > key.0 || (cells[j - 1].0 == key.0 && cells[j - 1].1 > key.1)) {
                cells[j] = cells[j - 1];
                j -= 1;
            }
            cells[j] = key;
            i += 1;
        }
        let mut w = 0u8;
        let mut h = 0u8;
        let mut k = 0;
        while k < n {
            if cells[k].0 + 1 > h {
                h = cells[k].0 + 1;
            }
            if cells[k].1 + 1 > w {
                w = cells[k].1 + 1;
            }
            k += 1;
        }
        Self { cells, n: n as u8, w, h }
    }

    #[inline]
    pub fn cells(&self) -> &[(u8, u8)] {
        &self.cells[..self.n as usize]
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.n as usize
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// Поворот на 90° по часовой стрелке с нормализацией рамки.
    pub fn rotated(&self) -> Shape {
        let h = self.h;
        let v: Vec<(u8, u8)> = self.cells().iter().map(|&(r, c)| (c, h - 1 - r)).collect();
        Shape::from_cells(&v)
    }

    /// Зеркальное отражение по горизонтали.
    pub fn mirrored(&self) -> Shape {
        let w = self.w;
        let v: Vec<(u8, u8)> = self.cells().iter().map(|&(r, c)| (r, w - 1 - c)).collect();
        Shape::from_cells(&v)
    }

    /// Центр рамки в клетках (для центрирования в слоте и захвата).
    pub fn center(&self) -> (f32, f32) {
        (self.w as f32 * 0.5, self.h as f32 * 0.5)
    }

    /// Ограничивающий прямоугольник заполнен целиком (прямоугольник/квадрат).
    pub fn is_rect(&self) -> bool {
        self.len() == self.w as usize * self.h as usize
    }
}

/// Семейство фигур: базовая форма + вес выпадения + цвет.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    Mono,
    I2,
    I3,
    I4,
    I5,
    O2,
    O3,
    R23,
    L3,
    L4,
    J4,
    T4,
    S4,
    Z4,
    V5,
    /// Диагональ из двух клеток (касаются углами).
    D2,
    /// Диагональ из трёх клеток.
    D3,
}

impl Family {
    pub const ALL: [Family; 17] = [
        Family::Mono,
        Family::I2,
        Family::I3,
        Family::I4,
        Family::I5,
        Family::O2,
        Family::O3,
        Family::R23,
        Family::L3,
        Family::L4,
        Family::J4,
        Family::T4,
        Family::S4,
        Family::Z4,
        Family::V5,
        Family::D2,
        Family::D3,
    ];

    pub fn base(self) -> Shape {
        use Family::*;
        match self {
            Mono => Shape::from_cells(&[(0, 0)]),
            I2 => Shape::from_cells(&[(0, 0), (0, 1)]),
            I3 => Shape::from_cells(&[(0, 0), (0, 1), (0, 2)]),
            I4 => Shape::from_cells(&[(0, 0), (0, 1), (0, 2), (0, 3)]),
            I5 => Shape::from_cells(&[(0, 0), (0, 1), (0, 2), (0, 3), (0, 4)]),
            O2 => Shape::from_cells(&[(0, 0), (0, 1), (1, 0), (1, 1)]),
            O3 => Shape::from_cells(&[
                (0, 0),
                (0, 1),
                (0, 2),
                (1, 0),
                (1, 1),
                (1, 2),
                (2, 0),
                (2, 1),
                (2, 2),
            ]),
            R23 => Shape::from_cells(&[(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 2)]),
            L3 => Shape::from_cells(&[(0, 0), (1, 0), (1, 1)]),
            L4 => Shape::from_cells(&[(0, 0), (1, 0), (2, 0), (2, 1)]),
            J4 => Shape::from_cells(&[(0, 1), (1, 1), (2, 1), (2, 0)]),
            T4 => Shape::from_cells(&[(0, 0), (0, 1), (0, 2), (1, 1)]),
            S4 => Shape::from_cells(&[(0, 1), (0, 2), (1, 0), (1, 1)]),
            Z4 => Shape::from_cells(&[(0, 0), (0, 1), (1, 1), (1, 2)]),
            V5 => Shape::from_cells(&[(0, 0), (1, 0), (2, 0), (2, 1), (2, 2)]),
            D2 => Shape::from_cells(&[(0, 0), (1, 1)]),
            D3 => Shape::from_cells(&[(0, 0), (1, 1), (2, 2)]),
        }
    }

    /// Базовый вес выпадения (до поправки на заполненность поля).
    pub fn weight(self) -> f32 {
        use Family::*;
        match self {
            Mono => 5.0,
            I2 => 7.0,
            I3 => 8.0,
            I4 => 6.0,
            I5 => 4.0,
            O2 => 7.0,
            O3 => 2.0,
            R23 => 4.0,
            L3 => 8.0,
            L4 => 5.0,
            J4 => 5.0,
            T4 => 6.0,
            S4 => 4.0,
            Z4 => 4.0,
            V5 => 4.0,
            D2 => 4.0,
            D3 => 3.0,
        }
    }

    /// Индекс цвета в палитре (1..=7), фиксирован за семейством —
    /// по цвету на поле видно, из какой фигуры сложен участок.
    pub fn color(self) -> u8 {
        use Family::*;
        match self {
            R23 | J4 | D3 => 1, // blue
            O3 | S4 | Z4 => 2, // green
            I4 | I5 => 3,      // orange
            L3 | L4 | D2 => 4, // purple
            I2 | I3 => 5,      // yellow
            Mono | T4 => 6, // cyan
            O2 | V5 => 7,      // red
        }
    }

    pub fn index(self) -> usize {
        Family::ALL.iter().position(|f| *f == self).unwrap_or(0)
    }
}

/// Одна ориентация семейства.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Variant {
    pub family: Family,
    pub shape: Shape,
}

/// Каталог всех ориентаций всех семейств.
pub struct Catalog {
    pub variants: Vec<Variant>,
    by_family: Vec<Vec<usize>>,
}

impl Catalog {
    fn build() -> Self {
        let mut variants = Vec::new();
        let mut by_family = vec![Vec::new(); Family::ALL.len()];
        for (fi, family) in Family::ALL.iter().enumerate() {
            let mut s = family.base();
            let mut seen: Vec<Shape> = Vec::new();
            for _ in 0..4 {
                if !seen.contains(&s) {
                    seen.push(s);
                    by_family[fi].push(variants.len());
                    variants.push(Variant { family: *family, shape: s });
                }
                s = s.rotated();
            }
        }
        Catalog { variants, by_family }
    }

    pub fn variants_of(&self, family: Family) -> &[usize] {
        &self.by_family[family.index()]
    }

    pub fn len(&self) -> usize {
        self.variants.len()
    }

    pub fn is_empty(&self) -> bool {
        self.variants.is_empty()
    }
}

/// Глобальный каталог (строится один раз).
pub fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(Catalog::build)
}

/// Материал фигуры: меняет внешний вид всех её кубиков, частицы при
/// очистке и даёт бонус очков за каждую очищенную клетку.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Material {
    #[default]
    Plain,
    Wood,
    Crystal,
    Gold,
    Lava,
}

impl Material {
    pub const ALL: [Material; 5] = [Material::Plain, Material::Wood, Material::Crystal, Material::Gold, Material::Lava];

    pub fn index(self) -> u8 {
        match self {
            Material::Plain => 0,
            Material::Wood => 1,
            Material::Crystal => 2,
            Material::Gold => 3,
            Material::Lava => 4,
        }
    }

    pub fn from_index(i: u8) -> Material {
        match i {
            1 => Material::Wood,
            2 => Material::Crystal,
            3 => Material::Gold,
            4 => Material::Lava,
            _ => Material::Plain,
        }
    }

    /// Вес выпадения (доля от всех фигур).
    pub fn weight(self) -> f32 {
        match self {
            Material::Plain => 78.0,
            Material::Wood => 9.0,
            Material::Crystal => 6.0,
            Material::Lava => 4.0,
            Material::Gold => 3.0,
        }
    }

    /// Бонус за очищенную обычную клетку, пока действует режим блоков
    /// этого материала (меньше, чем у настоящих особых кубиков).
    pub fn mode_bonus(self) -> i32 {
        match self {
            Material::Plain => 0,
            Material::Wood => 5,
            Material::Crystal => 10,
            Material::Gold => 15,
            Material::Lava => 20,
        }
    }

    /// Бонус очков за каждую очищенную клетку этого материала.
    pub fn bonus(self) -> i32 {
        match self {
            Material::Plain => 0,
            Material::Wood => 5,
            Material::Crystal => 15,
            Material::Lava => 20,
            Material::Gold => 30,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Material::Plain => "",
            Material::Wood => "ДЕРЕВО",
            Material::Crystal => "ХРУСТАЛЬ",
            Material::Gold => "ЗОЛОТО",
            Material::Lava => "ЛАВА",
        }
    }

    /// Отделка (см. `FINISH_COUNT`), которой бонусный материал выглядит в
    /// виде игры «Люкс»: золото — золотом, хрусталь — хрусталём,
    /// дерево — дубом, лава — лавой. У обычного материала отделки нет.
    pub fn finish(self) -> u8 {
        match self {
            Material::Plain => 0,
            Material::Gold => FINISH_GOLD,
            Material::Crystal => FINISH_CRYSTAL,
            Material::Wood => FINISH_OAK,
            Material::Lava => FINISH_LAVA,
        }
    }
}

/// Число отделок (визуальных материалов «Люкса»); нумерация 1..=FINISH_COUNT,
/// 0 — не задана. Таблица с именами и видом — `ui::finish`.
pub const FINISH_COUNT: u8 = 24;
pub const FINISH_GOLD: u8 = 1;
pub const FINISH_CRYSTAL: u8 = 6;
pub const FINISH_OAK: u8 = 8;
pub const FINISH_LAVA: u8 = 13;

/// Фигура в трее: форма + семейство + цвет + материал + отделка.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece {
    pub shape: Shape,
    pub family: Family,
    pub color: u8,
    pub material: Material,
    /// Отделка кубиков (1..=FINISH_COUNT); 0 — не задана (старые сохранения).
    pub finish: u8,
}

impl Piece {
    pub fn new(family: Family, shape: Shape) -> Self {
        Piece { shape, family, color: family.color(), material: Material::Plain, finish: 0 }
    }

    pub fn with_material(mut self, material: Material) -> Self {
        self.material = material;
        self
    }

    pub fn with_finish(mut self, finish: u8) -> Self {
        self.finish = finish;
        self
    }

    /// Базовая ориентация семейства.
    pub fn of(family: Family) -> Self {
        Self::new(family, family.base())
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.shape.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.shape.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connected(s: &Shape) -> bool {
        let cells = s.cells();
        let mut seen = vec![false; cells.len()];
        let mut stack = vec![0usize];
        seen[0] = true;
        while let Some(i) = stack.pop() {
            let (r, c) = cells[i];
            for (j, &(r2, c2)) in cells.iter().enumerate() {
                // диагональные семейства связны по углам
                if !seen[j] && r.abs_diff(r2).max(c.abs_diff(c2)) == 1 {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        seen.iter().all(|v| *v)
    }

    #[test]
    fn bases_are_tight_and_connected() {
        for f in Family::ALL {
            let s = f.base();
            assert!(!s.is_empty());
            assert!(s.len() <= MAX_CELLS);
            let max_r = s.cells().iter().map(|c| c.0).max().unwrap();
            let max_c = s.cells().iter().map(|c| c.1).max().unwrap();
            assert_eq!(max_r + 1, s.h, "{f:?} not tight in rows");
            assert_eq!(max_c + 1, s.w, "{f:?} not tight in cols");
            assert!(connected(&s), "{f:?} not connected");
        }
    }

    #[test]
    fn rotation_is_cyclic_and_swaps_dims() {
        for f in Family::ALL {
            let s = f.base();
            let r1 = s.rotated();
            assert_eq!((r1.w, r1.h), (s.h, s.w));
            assert_eq!(r1.len(), s.len());
            let back = r1.rotated().rotated().rotated();
            assert_eq!(back, s, "{f:?} rotated 4× must be identity");
            assert_eq!(s.mirrored().mirrored(), s);
        }
    }

    #[test]
    fn catalog_has_expected_orientations() {
        let cat = catalog();
        assert_eq!(cat.variants_of(Family::Mono).len(), 1);
        assert_eq!(cat.variants_of(Family::O2).len(), 1);
        assert_eq!(cat.variants_of(Family::O3).len(), 1);
        assert_eq!(cat.variants_of(Family::I3).len(), 2);
        assert_eq!(cat.variants_of(Family::R23).len(), 2);
        assert_eq!(cat.variants_of(Family::S4).len(), 2);
        assert_eq!(cat.variants_of(Family::Z4).len(), 2);
        assert_eq!(cat.variants_of(Family::L4).len(), 4);
        assert_eq!(cat.variants_of(Family::J4).len(), 4);
        assert_eq!(cat.variants_of(Family::T4).len(), 4);
        assert_eq!(cat.variants_of(Family::V5).len(), 4);
        assert_eq!(cat.variants_of(Family::L3).len(), 4);
        assert_eq!(cat.variants_of(Family::D2).len(), 2);
        assert_eq!(cat.variants_of(Family::D3).len(), 2);
        // всего: 1+2+2+2+2+1+1+2+4+4+4+4+2+2+4+4 = 41
        assert_eq!(cat.len(), 41);
        // L и J — зеркальные семейства, а не повороты друг друга
        let l = Family::L4.base();
        let j = Family::J4.base();
        assert_ne!(l, j);
        assert_eq!(l.mirrored(), j);
    }

    #[test]
    fn no_duplicate_variants_within_family() {
        let cat = catalog();
        for f in Family::ALL {
            let idx = cat.variants_of(f);
            for i in 0..idx.len() {
                for j in (i + 1)..idx.len() {
                    assert_ne!(cat.variants[idx[i]].shape, cat.variants[idx[j]].shape);
                }
            }
        }
    }

    #[test]
    fn material_index_roundtrip() {
        for m in Material::ALL {
            assert_eq!(Material::from_index(m.index()), m);
            assert!(m.weight() > 0.0);
        }
        assert_eq!(Material::Plain.bonus(), 0);
        assert!(Material::Gold.bonus() > Material::Wood.bonus());
    }

    #[test]
    fn colors_in_palette_range() {
        for f in Family::ALL {
            let c = f.color();
            assert!((1..=7).contains(&c));
            assert!(f.weight() > 0.0);
        }
    }
}
