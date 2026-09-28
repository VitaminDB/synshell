//! Правила партии: очки, комбо, состояние игры.
//!
//! Очки: 10 за клетку; линии — `100·n + 50·n·(n−1)` (1→100, 2→300, 3→600,
//! 4→1000), умноженные на комбо; «идеальная очистка» поля — +300.
//! Комбо растёт с каждым ходом с очисткой и прощает до двух ходов без неё.

use super::board::{cell_material, Board, Clear};
use super::generator;
use super::piece::{Material, Piece};
use super::rng::Rng;

pub const CELL_POINTS: i32 = 10;
pub const PERFECT_BONUS: i32 = 300;
/// Сколько ходов подряд без очистки не сбрасывают комбо (обычная сложность).
pub const COMBO_GRACE: u32 = 2;
/// Потолок энергии способностей (1 единица за очищенную линию).
pub const MAX_ENERGY: u32 = 12;

/// Уровень сложности партии.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Difficulty {
    Easy,
    #[default]
    Normal,
    Hard,
}

impl Difficulty {
    pub const ALL: [Difficulty; 3] = [Difficulty::Easy, Difficulty::Normal, Difficulty::Hard];

    pub fn index(self) -> usize {
        match self {
            Difficulty::Easy => 0,
            Difficulty::Normal => 1,
            Difficulty::Hard => 2,
        }
    }

    pub fn from_index(i: usize) -> Difficulty {
        match i {
            0 => Difficulty::Easy,
            2 => Difficulty::Hard,
            _ => Difficulty::Normal,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Difficulty::Easy => "ЛЕГКО",
            Difficulty::Normal => "НОРМА",
            Difficulty::Hard => "СЛОЖНО",
        }
    }

    /// Сколько промахов подряд прощает комбо.
    pub fn grace(self) -> u32 {
        match self {
            Difficulty::Easy => 3,
            Difficulty::Normal => COMBO_GRACE,
            Difficulty::Hard => 1,
        }
    }

    /// Гарантирует ли раздача, что вся тройка размещаема.
    pub fn guarantees_triple(self) -> bool {
        !matches!(self, Difficulty::Hard)
    }
}

/// Способности за энергию линий.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ability {
    /// Заменить все фигуры в трее.
    Reroll,
    /// Убрать один кубик.
    Hammer,
    /// Убрать область 3×3.
    Bomb,
}

impl Ability {
    pub const ALL: [Ability; 3] = [Ability::Reroll, Ability::Hammer, Ability::Bomb];

    pub fn cost(self) -> u32 {
        match self {
            Ability::Reroll => 3,
            Ability::Hammer => 4,
            Ability::Bomb => 7,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Ability::Reroll => "Обмен",
            Ability::Hammer => "Молот",
            Ability::Bomb => "Бомба",
        }
    }

    /// Нужна ли цель на поле.
    pub fn needs_target(self) -> bool {
        !matches!(self, Ability::Reroll)
    }
}

/// Результат применения способности.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AbilityOutcome {
    pub removed: Vec<(usize, usize, u8)>,
    pub rerolled: bool,
}

pub fn line_points(lines: usize) -> i32 {
    let n = lines as i32;
    100 * n + 50 * n * (n - 1)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scoring {
    pub score: i32,
    pub combo: u32,
    pub misses: u32,
    pub best_combo: u32,
    pub lines: u32,
    pub moves: u32,
    pub perfects: u32,
    /// Сколько промахов подряд прощается (зависит от сложности).
    pub grace: u32,
}

impl Default for Scoring {
    fn default() -> Self {
        Scoring { score: 0, combo: 0, misses: 0, best_combo: 0, lines: 0, moves: 0, perfects: 0, grace: COMBO_GRACE }
    }
}

/// Разбор очков за один ход.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Gain {
    pub placed: i32,
    pub lines: i32,
    pub perfect: i32,
    /// Бонус за очищенные клетки особых материалов.
    pub material: i32,
    /// Комбо, применённое к этому ходу (0 — линий не было).
    pub combo: u32,
}

impl Gain {
    pub fn total(&self) -> i32 {
        self.placed + self.lines + self.perfect + self.material
    }
}

impl Scoring {
    pub fn apply(&mut self, cells: usize, lines: usize, perfect: bool) -> Gain {
        self.apply_with_bonus(cells, lines, perfect, 0)
    }

    pub fn apply_with_bonus(&mut self, cells: usize, lines: usize, perfect: bool, material: i32) -> Gain {
        self.moves += 1;
        let placed = cells as i32 * CELL_POINTS;
        let (line_pts, combo) = if lines > 0 {
            self.combo += 1;
            self.misses = 0;
            self.lines += lines as u32;
            self.best_combo = self.best_combo.max(self.combo);
            (line_points(lines) * self.combo as i32, self.combo)
        } else {
            self.misses += 1;
            if self.misses > self.grace {
                self.combo = 0;
                self.misses = 0;
            }
            (0, 0)
        };
        let perfect_pts = if perfect && lines > 0 {
            self.perfects += 1;
            PERFECT_BONUS
        } else {
            0
        };
        let material = if lines > 0 { material } else { 0 };
        self.score += placed + line_pts + perfect_pts + material;
        Gain { placed, lines: line_pts, perfect: perfect_pts, material, combo }
    }
}

/// Всё, что произошло за один ход — для анимаций и статистики.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    pub slot: usize,
    pub piece: Piece,
    pub anchor: (i32, i32),
    pub cells: Vec<(usize, usize)>,
    pub clear: Clear,
    pub gain: Gain,
    /// Бонусы по материалам: (материал, очищенных клеток, очки).
    pub material_bonus: Vec<(Material, usize, i32)>,
    /// Бонус режима блоков: (материал режима, обычных клеток, очки).
    pub mode_bonus: Option<(Material, usize, i32)>,
    pub perfect: bool,
    pub refilled: bool,
    pub over: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Game {
    pub board: Board,
    pub tray: [Option<Piece>; 3],
    pub scoring: Scoring,
    pub rng: Rng,
    pub over: bool,
    pub seed: u64,
    pub difficulty: Difficulty,
    /// Энергия способностей: +1 за каждую очищенную линию.
    pub energy: u32,
    pub abilities_used: u32,
    /// Режим блоков: пока он действует, обычные кубики считаются этим
    /// материалом и дают `Material::mode_bonus` за очищенную клетку.
    pub mode: Option<Material>,
}

impl Game {
    pub fn new(seed: u64) -> Self {
        Self::with_difficulty(seed, Difficulty::Normal)
    }

    pub fn with_difficulty(seed: u64, difficulty: Difficulty) -> Self {
        let mut rng = Rng::new(seed);
        let board = Board::new();
        let three = generator::deal_for(&mut rng, &board, difficulty);
        Game {
            board,
            tray: [Some(three[0]), Some(three[1]), Some(three[2])],
            scoring: Scoring { grace: difficulty.grace(), ..Default::default() },
            rng,
            over: false,
            seed,
            difficulty,
            energy: 0,
            abilities_used: 0,
            mode: None,
        }
    }

    pub fn can_use(&self, a: Ability) -> bool {
        !self.over && self.energy >= a.cost()
    }

    /// Применить способность; `target` нужен молоту и бомбе.
    pub fn use_ability(&mut self, a: Ability, target: Option<(i32, i32)>) -> Option<AbilityOutcome> {
        if !self.can_use(a) {
            return None;
        }
        let mut out = AbilityOutcome::default();
        match a {
            Ability::Reroll => {
                let three = generator::deal_for(&mut self.rng, &self.board, self.difficulty);
                self.tray = [Some(three[0]), Some(three[1]), Some(three[2])];
                out.rerolled = true;
            }
            Ability::Hammer => {
                let (r, c) = target?;
                if !Board::in_bounds(r, c) || self.board.get(r as usize, c as usize) == 0 {
                    return None;
                }
                out.removed.push((r as usize, c as usize, self.board.get(r as usize, c as usize)));
                self.board.set(r as usize, c as usize, 0);
            }
            Ability::Bomb => {
                let (r, c) = target?;
                if !Board::in_bounds(r, c) {
                    return None;
                }
                for dr in -1..=1 {
                    for dc in -1..=1 {
                        let (rr, cc) = (r + dr, c + dc);
                        if Board::in_bounds(rr, cc) {
                            let v = self.board.get(rr as usize, cc as usize);
                            if v != 0 {
                                out.removed.push((rr as usize, cc as usize, v));
                                self.board.set(rr as usize, cc as usize, 0);
                            }
                        }
                    }
                }
                if out.removed.is_empty() {
                    return None;
                }
            }
        }
        self.energy -= a.cost();
        self.abilities_used += 1;
        self.over = !self.any_fits();
        Some(out)
    }

    pub fn piece(&self, slot: usize) -> Option<&Piece> {
        self.tray.get(slot).and_then(|s| s.as_ref())
    }

    pub fn can_place(&self, slot: usize, r: i32, c: i32) -> bool {
        match self.piece(slot) {
            Some(p) => !self.over && self.board.can_place(&p.shape, r, c),
            None => false,
        }
    }

    /// Ставится ли фигура слота хоть куда-нибудь.
    pub fn slot_fits(&self, slot: usize) -> bool {
        self.piece(slot)
            .map(|p| self.board.fits_anywhere(&p.shape))
            .unwrap_or(false)
    }

    pub fn any_fits(&self) -> bool {
        (0..3).any(|i| self.slot_fits(i))
    }

    pub fn tray_is_empty(&self) -> bool {
        self.tray.iter().all(|s| s.is_none())
    }

    pub fn preview(&self, slot: usize, r: i32, c: i32) -> Option<(Vec<usize>, Vec<usize>)> {
        let p = self.piece(slot)?;
        if !self.board.can_place(&p.shape, r, c) {
            return None;
        }
        Some(self.board.preview_lines(&p.shape, r, c))
    }

    /// Ход: поставить фигуру слота `slot` якорем в `(r, c)`.
    pub fn place(&mut self, slot: usize, r: i32, c: i32) -> Option<Placement> {
        if self.over {
            return None;
        }
        let piece = *self.piece(slot)?;
        if !self.board.can_place(&piece.shape, r, c) {
            return None;
        }
        let cells = self.board.place(&piece, r, c);
        let clear = self.board.clear_full_lines();
        let perfect = !clear.is_empty() && self.board.is_empty();
        let mut material_bonus: Vec<(Material, usize, i32)> = Vec::new();
        let mut mode_bonus: Option<(Material, usize, i32)> = None;
        for &(_, _, cell) in &clear.cells {
            let m = cell_material(cell);
            if m == Material::Plain {
                if let Some(mm) = self.mode.filter(|mm| *mm != Material::Plain) {
                    let e = mode_bonus.get_or_insert((mm, 0, 0));
                    e.1 += 1;
                    e.2 += mm.mode_bonus();
                }
                continue;
            }
            match material_bonus.iter_mut().find(|e| e.0 == m) {
                Some(e) => {
                    e.1 += 1;
                    e.2 += m.bonus();
                }
                None => material_bonus.push((m, 1, m.bonus())),
            }
        }
        let bonus_total: i32 = material_bonus.iter().map(|e| e.2).sum::<i32>() + mode_bonus.map(|e| e.2).unwrap_or(0);
        let gain = self.scoring.apply_with_bonus(piece.len(), clear.lines(), perfect, bonus_total);
        self.tray[slot] = None;
        self.energy = (self.energy + clear.lines() as u32).min(MAX_ENERGY);
        let refilled = if self.tray_is_empty() {
            let three = generator::deal_for(&mut self.rng, &self.board, self.difficulty);
            self.tray = [Some(three[0]), Some(three[1]), Some(three[2])];
            true
        } else {
            false
        };
        self.over = !self.any_fits();
        Some(Placement {
            slot,
            piece,
            anchor: (r, c),
            cells,
            clear,
            gain,
            material_bonus,
            mode_bonus,
            perfect,
            refilled,
            over: self.over,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::board::SIZE;
    use crate::game::piece::Family;

    #[test]
    fn line_points_grow_superlinearly() {
        assert_eq!(line_points(0), 0);
        assert_eq!(line_points(1), 100);
        assert_eq!(line_points(2), 300);
        assert_eq!(line_points(3), 600);
        assert_eq!(line_points(4), 1000);
    }

    #[test]
    fn scoring_combo_with_grace() {
        let mut s = Scoring::default();
        assert_eq!(s.apply(4, 0, false).total(), 40);
        assert_eq!(s.combo, 0);
        let g = s.apply(4, 2, false);
        assert_eq!(g.total(), 40 + 300);
        assert_eq!(s.combo, 1);
        let g = s.apply(4, 1, false);
        assert_eq!(g.total(), 40 + 200);
        assert_eq!(s.combo, 2);
        // два промаха прощаются
        s.apply(1, 0, false);
        s.apply(1, 0, false);
        assert_eq!(s.combo, 2);
        // третий — сброс
        s.apply(1, 0, false);
        assert_eq!(s.combo, 0);
        assert_eq!(s.best_combo, 2);
        assert_eq!(s.lines, 3);
        assert_eq!(s.moves, 6);
    }

    #[test]
    fn perfect_bonus_only_with_lines() {
        let mut s = Scoring::default();
        assert_eq!(s.apply(1, 0, true).perfect, 0);
        let g = s.apply(2, 1, true);
        assert_eq!(g.perfect, PERFECT_BONUS);
        assert_eq!(s.perfects, 1);
    }

    #[test]
    fn game_place_clears_and_reports() {
        let mut g = Game::new(3);
        for c in 0..6 {
            g.board.set(7, c, 2);
        }
        g.tray = [Some(Piece::of(Family::I2)), None, None];
        assert!(g.can_place(0, 7, 6));
        assert!(!g.can_place(0, 7, 7));
        let pl = g.place(0, 7, 6).expect("valid move");
        assert_eq!(pl.clear.rows, vec![7]);
        assert_eq!(pl.clear.cells.len(), 8);
        assert!(pl.perfect, "поле стало пустым");
        assert_eq!(pl.gain.total(), 20 + 100 + PERFECT_BONUS);
        assert!(pl.refilled, "трей был пуст — добор");
        assert!(g.tray.iter().all(|s| s.is_some()));
        assert!(!g.over);
        assert!(g.board.is_empty());
    }

    #[test]
    fn game_over_when_nothing_fits() {
        let mut g = Game::new(1);
        for r in 0..SIZE {
            for c in 0..SIZE {
                if !(r == 0 && c == 0) {
                    g.board.set(r, c, 1);
                }
            }
        }
        g.tray = [Some(Piece::of(Family::Mono)), Some(Piece::of(Family::O2)), None];
        assert!(g.slot_fits(0));
        assert!(!g.slot_fits(1));
        let pl = g.place(0, 0, 0).unwrap();
        // последняя клетка заполнила все 8 рядов и 8 колонок → идеальная очистка
        assert_eq!(pl.clear.lines(), 16);
        assert!(pl.perfect);
        assert!(g.board.is_empty());
        assert!(!pl.over);
        // повторяем: забить всё и оставить только O2 без места
        let mut g2 = Game::new(2);
        for r in 0..SIZE {
            for c in 0..SIZE {
                if (r + c) % 2 == 1 {
                    g2.board.set(r, c, 1);
                }
            }
        }
        g2.tray = [Some(Piece::of(Family::Mono)), Some(Piece::of(Family::O2)), Some(Piece::of(Family::I2))];
        let pl = g2.place(0, 0, 0).unwrap();
        assert!(pl.over, "остались O2 и I2 — на шахматном поле не ставятся");
        assert!(g2.over);
        assert!(g2.place(1, 0, 0).is_none());
    }

    #[test]
    fn material_bonus_counts_cleared_special_cells() {
        let mut g = Game::new(5);
        for c in 0..5 {
            g.board.set(7, c, crate::game::board::make_cell(2, Material::Gold));
        }
        g.board.set(7, 5, crate::game::board::make_cell(2, Material::Wood));
        g.tray = [Some(Piece::of(Family::I2).with_material(Material::Crystal)), None, None];
        let pl = g.place(0, 7, 6).unwrap();
        assert_eq!(pl.clear.lines(), 1);
        let gold = pl.material_bonus.iter().find(|e| e.0 == Material::Gold).unwrap();
        assert_eq!((gold.1, gold.2), (5, 5 * Material::Gold.bonus()));
        let crystal = pl.material_bonus.iter().find(|e| e.0 == Material::Crystal).unwrap();
        assert_eq!(crystal.1, 2);
        let expected = 5 * Material::Gold.bonus() + Material::Wood.bonus() + 2 * Material::Crystal.bonus();
        assert_eq!(pl.gain.material, expected);
        assert_eq!(pl.gain.total(), 20 + 100 + PERFECT_BONUS + expected);
        // без линий бонуса нет
        let mut g2 = Game::new(6);
        g2.tray = [Some(Piece::of(Family::Mono).with_material(Material::Gold)), None, None];
        assert_eq!(g2.place(0, 0, 0).unwrap().gain.material, 0);
    }

    #[test]
    fn mode_bonus_counts_plain_cells_only() {
        let mut g = Game::new(7);
        g.mode = Some(Material::Wood);
        for c in 0..5 {
            g.board.set(7, c, 3);
        }
        g.board.set(7, 5, crate::game::board::make_cell(2, Material::Gold));
        g.tray = [Some(Piece::of(Family::I2)), None, None];
        let pl = g.place(0, 7, 6).unwrap();
        // 5 обычных на поле + 2 от фигуры; золото считается своим бонусом
        assert_eq!(pl.mode_bonus, Some((Material::Wood, 7, 7 * Material::Wood.mode_bonus())));
        assert_eq!(pl.gain.material, 7 * Material::Wood.mode_bonus() + Material::Gold.bonus());
        // без режима бонуса нет
        let mut g2 = Game::new(7);
        for c in 0..6 {
            g2.board.set(7, c, 3);
        }
        g2.tray = [Some(Piece::of(Family::I2)), None, None];
        assert_eq!(g2.place(0, 7, 6).unwrap().mode_bonus, None);
    }

    #[test]
    fn difficulty_sets_grace_and_hard_skips_guarantee() {
        assert_eq!(Game::with_difficulty(1, Difficulty::Easy).scoring.grace, 3);
        assert_eq!(Game::with_difficulty(1, Difficulty::Hard).scoring.grace, 1);
        assert!(!Difficulty::Hard.guarantees_triple());
        for d in Difficulty::ALL {
            assert_eq!(Difficulty::from_index(d.index()), d);
        }
    }

    #[test]
    fn energy_and_abilities() {
        let mut g = Game::new(9);
        for c in 0..6 {
            g.board.set(7, c, 2);
        }
        g.tray = [Some(Piece::of(Family::I2)), Some(Piece::of(Family::O2)), None];
        assert!(!g.can_use(Ability::Reroll));
        g.place(0, 7, 6).unwrap();
        assert_eq!(g.energy, 1);
        g.energy = 5;
        assert!(g.can_use(Ability::Hammer) && !g.can_use(Ability::Bomb));
        // молот по пустой клетке — отказ, энергия не тратится
        assert!(g.use_ability(Ability::Hammer, Some((0, 0))).is_none());
        assert_eq!(g.energy, 5);
        g.board.set(3, 3, 4);
        let out = g.use_ability(Ability::Hammer, Some((3, 3))).unwrap();
        assert_eq!(out.removed, vec![(3, 3, 4)]);
        assert_eq!(g.board.get(3, 3), 0);
        assert_eq!(g.energy, 1);
        // бомба 3×3
        g.energy = MAX_ENERGY;
        for r in 0..3 {
            for c in 0..3 {
                g.board.set(r, c, 5);
            }
        }
        let out = g.use_ability(Ability::Bomb, Some((1, 1))).unwrap();
        assert_eq!(out.removed.len(), 9);
        assert_eq!(g.energy, MAX_ENERGY - Ability::Bomb.cost());
        // обмен меняет трей
        let before = g.tray;
        let out = g.use_ability(Ability::Reroll, None).unwrap();
        assert!(out.rerolled);
        assert!(g.tray.iter().all(|s| s.is_some()));
        assert_ne!(g.tray, before);
        assert_eq!(g.abilities_used, 3);
    }

    #[test]
    fn preview_reports_lines() {
        let mut g = Game::new(4);
        for c in 1..SIZE {
            g.board.set(4, c, 5);
        }
        g.tray[0] = Some(Piece::of(Family::Mono));
        assert_eq!(g.preview(0, 4, 0), Some((vec![4], vec![])));
        assert_eq!(g.preview(0, 4, 1), None);
    }

    #[test]
    fn same_seed_same_game() {
        let a = Game::new(777);
        let b = Game::new(777);
        assert_eq!(a, b);
    }
}
