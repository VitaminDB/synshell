//! Честная раздача фигур.
//!
//! Правила, которых нет в оригинале:
//! * тройка **гарантированно размещаема целиком**: существует порядок и
//!   позиции, при которых все три фигуры встают на текущее поле (с учётом
//!   схлопывания линий между ходами) — проигрыш возможен только из-за
//!   ошибки игрока, а не из-за раздачи;
//! * чем полнее поле, тем реже крупные фигуры и чаще мелкие;
//! * три одинаковые формы подряд не выпадают.

use super::board::{Board, SIZE};
use super::piece::{catalog, Family, Material, Piece};
use super::rng::Rng;
use super::rules::Difficulty;

pub const DEAL_ATTEMPTS: usize = 60;
/// Предел перебора позиций в проверке размещаемости (защита от взрыва).
const SOLVE_BUDGET: usize = 200_000;
/// Идеальная очистка ищется, только если на поле не больше стольких кубиков.
pub const PERFECT_MAX_FILLED: usize = 22;
/// Вероятность (в процентах), что при возможности раздача будет «под идеал».
pub const PERFECT_CHANCE: u32 = 40;
const PERFECT_ATTEMPTS: usize = 12;
const PERFECT_BUDGET: usize = 40_000;

/// Вес семейства с поправкой на заполненность поля `fill` (0..1).
pub fn family_weight(f: Family, fill: f32) -> f32 {
    family_weight_for(f, fill, Difficulty::Normal)
}

/// То же с учётом сложности: легко — мелкие чаще, сложно — крупные чаще
/// и теснота почти не помогает.
pub fn family_weight_for(f: Family, fill: f32, d: Difficulty) -> f32 {
    let crowd = ((fill - 0.4) / 0.5).clamp(0.0, 1.0);
    let n = f.base().len() as f32;
    let size_factor = if n >= 6.0 {
        1.0 - crowd * 0.75
    } else {
        1.0 + crowd * (5.0 - n).max(0.0) * 0.45
    };
    let diff_factor = match d {
        Difficulty::Easy => 1.0 + (4.0 - n).max(0.0) * 0.25,
        Difficulty::Normal => 1.0,
        Difficulty::Hard => 1.0 + (n - 3.0).max(0.0) * 0.35,
    };
    f.weight() * size_factor * diff_factor
}

/// Случайная фигура с учётом заполненности (обычная сложность).
pub fn pick(rng: &mut Rng, fill: f32) -> Piece {
    pick_for(rng, fill, Difficulty::Normal)
}

pub fn pick_for(rng: &mut Rng, fill: f32, d: Difficulty) -> Piece {
    let weights: Vec<f32> = Family::ALL.iter().map(|f| family_weight_for(*f, fill, d)).collect();
    let family = Family::ALL[rng.weighted(&weights)];
    let cat = catalog();
    let variants = cat.variants_of(family);
    let v = cat.variants[variants[rng.index(variants.len())]];
    Piece::new(v.family, v.shape).with_material(pick_material(rng))
}

/// Материал фигуры по весам (`Material::weight`).
pub fn pick_material(rng: &mut Rng) -> Material {
    let weights: Vec<f32> = Material::ALL.iter().map(|m| m.weight()).collect();
    Material::ALL[rng.weighted(&weights)]
}

/// Можно ли поставить все фигуры `pieces` по очереди (в любом порядке) на
/// `board`, учитывая схлопывание линий после каждого хода.
pub fn all_placeable(board: &Board, pieces: &[Piece]) -> bool {
    let mut remaining: Vec<Piece> = pieces.to_vec();
    let mut budget = SOLVE_BUDGET;
    solve(*board, &mut remaining, &mut budget)
}

/// Можно ли этими фигурами (в любом порядке) довести поле до пустого —
/// «идеальная очистка» по ходу раздачи.
pub fn can_clear_all(board: &Board, pieces: &[Piece]) -> bool {
    if board.is_empty() {
        return false;
    }
    let mut remaining: Vec<Piece> = pieces.to_vec();
    let mut budget = PERFECT_BUDGET;
    solve_to_empty(*board, &mut remaining, &mut budget)
}

fn solve_to_empty(board: Board, remaining: &mut Vec<Piece>, budget: &mut usize) -> bool {
    if remaining.is_empty() || *budget == 0 {
        return false;
    }
    // Сначала дешёвая проверка: не опустошает ли поле один ход любой фигурой.
    for p in remaining.iter() {
        let sh = p.shape;
        for r in 0..=(SIZE as i32 - sh.h as i32) {
            for c in 0..=(SIZE as i32 - sh.w as i32) {
                if !board.can_place(&sh, r, c) {
                    continue;
                }
                let mut b = board;
                b.place(p, r, c);
                let cl = b.clear_full_lines();
                if !cl.is_empty() && b.is_empty() {
                    return true;
                }
            }
        }
    }
    for i in 0..remaining.len() {
        if remaining[..i].iter().any(|q| q.shape == remaining[i].shape) {
            continue;
        }
        let p = remaining.remove(i);
        let sh = p.shape;
        let mut found = false;
        'anchors: for r in 0..=(SIZE as i32 - sh.h as i32) {
            for c in 0..=(SIZE as i32 - sh.w as i32) {
                if *budget == 0 {
                    break 'anchors;
                }
                *budget -= 1;
                if !board.can_place(&sh, r, c) {
                    continue;
                }
                let mut b = board;
                b.place(&p, r, c);
                let cl = b.clear_full_lines();
                if !cl.is_empty() && b.is_empty() {
                    found = true;
                    break 'anchors;
                }
                // без очистки поле только растёт — дальше есть смысл, лишь если
                // оставшиеся фигуры смогут закрыть всё
                if solve_to_empty(b, remaining, budget) {
                    found = true;
                    break 'anchors;
                }
            }
        }
        remaining.insert(i, p);
        if found {
            return true;
        }
    }
    false
}

/// Подобрать тройку, которой можно собрать всё поле (если получится).
fn deal_perfect(rng: &mut Rng, board: &Board, d: Difficulty) -> Option<[Piece; 3]> {
    let fill = board.fill_ratio();
    for _ in 0..PERFECT_ATTEMPTS {
        let three = [pick_for(rng, fill, d), pick_for(rng, fill, d), pick_for(rng, fill, d)];
        if can_clear_all(board, &three) && (!d.guarantees_triple() || all_placeable(board, &three)) {
            return Some(three);
        }
    }
    None
}

fn solve(board: Board, remaining: &mut Vec<Piece>, budget: &mut usize) -> bool {
    if remaining.is_empty() {
        return true;
    }
    if *budget == 0 {
        return false;
    }
    for i in 0..remaining.len() {
        // одинаковые формы в одном списке — один и тот же ход
        if remaining[..i].iter().any(|q| q.shape == remaining[i].shape) {
            continue;
        }
        let p = remaining.remove(i);
        let sh = p.shape;
        let mut found = false;
        'anchors: for r in 0..=(SIZE as i32 - sh.h as i32) {
            for c in 0..=(SIZE as i32 - sh.w as i32) {
                if *budget == 0 {
                    break 'anchors;
                }
                *budget -= 1;
                if !board.can_place(&sh, r, c) {
                    continue;
                }
                let mut b = board;
                b.place(&p, r, c);
                b.clear_full_lines();
                if solve(b, remaining, budget) {
                    found = true;
                    break 'anchors;
                }
            }
        }
        remaining.insert(i, p);
        if found {
            return true;
        }
    }
    false
}

/// Раздать три фигуры под текущее поле (обычная сложность).
pub fn deal(rng: &mut Rng, board: &Board) -> [Piece; 3] {
    deal_for(rng, board, Difficulty::Normal)
}

/// Раздача с учётом сложности: на «сложно» гарантируется лишь одна
/// размещаемая фигура (как в оригинале), иначе — вся тройка.
pub fn deal_for(rng: &mut Rng, board: &Board, d: Difficulty) -> [Piece; 3] {
    let fill = board.fill_ratio();
    // Иногда — раздача «под идеальную очистку»: поле почти пустое, и тройка
    // подобрана так, что при верных ходах поле собирается целиком.
    let filled = board.filled();
    if filled > 0 && filled <= PERFECT_MAX_FILLED && rng.below(100) < PERFECT_CHANCE {
        if let Some(three) = deal_perfect(rng, board, d) {
            return three;
        }
    }
    let mut best: Option<([Piece; 3], usize)> = None;
    for attempt in 0..DEAL_ATTEMPTS {
        let three = [pick_for(rng, fill, d), pick_for(rng, fill, d), pick_for(rng, fill, d)];
        if three[0].shape == three[1].shape && three[1].shape == three[2].shape {
            continue;
        }
        let fits = three.iter().filter(|p| board.fits_anywhere(&p.shape)).count();
        if !d.guarantees_triple() {
            if fits >= 1 && (fits >= 2 || attempt >= 6 || fill < 0.5) {
                return three;
            }
        } else if fits == 3 && all_placeable(board, &three) {
            return three;
        }
        if best.map(|(_, f)| fits > f).unwrap_or(true) {
            best = Some((three, fits));
        }
    }
    // Размещаемой тройки не нашлось: берём ту, где ставится больше всего фигур.
    if let Some((three, fits)) = best {
        if fits >= 1 {
            return three;
        }
    }
    // Крайний случай: собираем тройку из того, что вообще ставится.
    let cat = catalog();
    let fitting: Vec<usize> = (0..cat.len())
        .filter(|&i| board.fits_anywhere(&cat.variants[i].shape))
        .collect();
    if fitting.is_empty() {
        return [Piece::of(Family::Mono); 3];
    }
    let mut out = [Piece::of(Family::Mono); 3];
    for slot in out.iter_mut() {
        let v = cat.variants[fitting[rng.index(fitting.len())]];
        *slot = Piece::new(v.family, v.shape);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_yields_valid_pieces() {
        let mut rng = Rng::new(2027);
        for _ in 0..5_000 {
            let p = pick(&mut rng, 0.2);
            assert!(!p.is_empty());
            assert!((1..=7).contains(&p.color));
            assert_eq!(p.color, p.family.color());
        }
    }

    #[test]
    fn materials_are_mostly_plain_but_present() {
        let mut rng = Rng::new(11);
        let mut counts = [0usize; 5];
        for _ in 0..10_000 {
            counts[pick(&mut rng, 0.3).material.index() as usize] += 1;
        }
        assert!(counts[0] > 7_000, "{counts:?}");
        for c in &counts[1..] {
            assert!(*c > 100, "{counts:?}");
        }
    }

    #[test]
    fn difficulty_shifts_piece_sizes() {
        assert!(family_weight_for(Family::Mono, 0.2, Difficulty::Easy) > family_weight_for(Family::Mono, 0.2, Difficulty::Normal));
        assert!(family_weight_for(Family::O3, 0.2, Difficulty::Hard) > family_weight_for(Family::O3, 0.2, Difficulty::Normal));
        assert_eq!(family_weight_for(Family::T4, 0.2, Difficulty::Normal), Family::T4.weight());
        let mut rng = Rng::new(3);
        let b = Board::new();
        let three = deal_for(&mut rng, &b, Difficulty::Hard);
        assert!(three.iter().any(|p| b.fits_anywhere(&p.shape)));
    }

    #[test]
    fn crowding_shrinks_big_pieces() {
        assert!(family_weight(Family::O3, 0.9) < family_weight(Family::O3, 0.0));
        assert!(family_weight(Family::Mono, 0.9) > family_weight(Family::Mono, 0.0));
        assert_eq!(family_weight(Family::I3, 0.1), Family::I3.weight());
    }

    #[test]
    fn deal_always_fits_at_least_one() {
        let mut rng = Rng::new(99);
        // шахматное поле: ставятся только мономино
        let mut b = Board::new();
        for r in 0..SIZE {
            for c in 0..SIZE {
                if (r + c) % 2 == 1 {
                    b.set(r, c, 3);
                }
            }
        }
        for _ in 0..200 {
            let three = deal(&mut rng, &b);
            assert!(three.iter().any(|p| b.fits_anywhere(&p.shape)));
        }
    }

    #[test]
    fn deal_avoids_triple_identical_and_prefers_two_fits_when_crowded() {
        let mut rng = Rng::new(5);
        let mut b = Board::new();
        // заполняем 5 нижних рядов и ещё немного — теснота ~0.7
        for r in 3..SIZE {
            for c in 0..SIZE {
                b.set(r, c, 2);
            }
        }
        b.set(2, 0, 2);
        b.set(2, 1, 2);
        let mut two_or_more = 0;
        for _ in 0..300 {
            let three = deal(&mut rng, &b);
            assert!(!(three[0].shape == three[1].shape && three[1].shape == three[2].shape));
            let fits = three.iter().filter(|p| b.fits_anywhere(&p.shape)).count();
            assert!(fits >= 1);
            if fits >= 2 {
                two_or_more += 1;
            }
        }
        assert!(two_or_more > 250, "{two_or_more}");
    }

    #[test]
    fn all_placeable_respects_order_and_clears() {
        // ряд 7 без клетки (7,7); над ней колонка 7 забита кроме (7,7):
        // Mono в (7,7) чистит ряд и колонку, после чего O3 влезает.
        let mut b = Board::new();
        for c in 0..7 {
            b.set(7, c, 1);
        }
        for r in 0..7 {
            b.set(r, 7, 1);
        }
        for r in 0..7 {
            for c in 0..7 {
                if r < 5 {
                    b.set(r, c, 2);
                }
            }
        }
        // свободно: ряды 5,6 × колонки 0..7 (2×7) и (7,7)
        let mono = Piece::of(Family::Mono);
        let o3 = Piece::of(Family::O3);
        assert!(!b.fits_anywhere(&o3.shape));
        assert!(all_placeable(&b, &[mono, o3]), "сначала Mono очищает линии, потом O3");
        assert!(all_placeable(&b, &[o3, mono]), "порядок в списке не важен");
        // шахматное поле: домино не встаёт ни в каком порядке
        let mut ch = Board::new();
        for r in 0..SIZE {
            for c in 0..SIZE {
                if (r + c) % 2 == 1 {
                    ch.set(r, c, 3);
                }
            }
        }
        let i2 = Piece::of(Family::I2);
        assert!(!all_placeable(&ch, &[mono, i2]));
        assert!(all_placeable(&ch, &[mono, mono, mono]));
    }

    #[test]
    fn dealt_triples_are_fully_placeable_on_crowded_boards() {
        let mut rng = Rng::new(31337);
        let mut boards = 0;
        let mut solvable = 0;
        for game in 0..60 {
            // случайное тесное поле: ~55–70 % клеток, без полных линий
            let mut b = Board::new();
            for r in 0..SIZE {
                for c in 0..SIZE {
                    if rng.unit() < 0.62 {
                        b.set(r, c, 1 + ((r + c + game) % 7) as u8);
                    }
                }
            }
            b.clear_full_lines();
            if b.filled() < 20 {
                continue;
            }
            let three = deal(&mut rng, &b);
            boards += 1;
            if all_placeable(&b, &three) {
                solvable += 1;
            } else {
                assert!(three.iter().any(|p| b.fits_anywhere(&p.shape)), "хотя бы одна должна ставиться");
            }
        }
        assert!(boards >= 40, "{boards}");
        assert!(solvable * 10 >= boards * 9, "размещаемых троек мало: {solvable}/{boards}");
    }

    #[test]
    fn perfect_clear_search_finds_obvious_case() {
        // ряд 7 без двух клеток: I2 закрывает ряд → поле пустое
        let mut b = Board::new();
        for c in 0..6 {
            b.set(7, c, 2);
        }
        let i2 = Piece::of(Family::I2);
        let o3 = Piece::of(Family::O3);
        assert!(can_clear_all(&b, &[o3, i2, o3]));
        assert!(!can_clear_all(&b, &[o3, o3, o3]));
        assert!(!can_clear_all(&Board::new(), &[i2]));
        // раздача под идеал: на таком поле она должна находиться часто
        let mut rng = Rng::new(2024);
        let mut hits = 0;
        for _ in 0..40 {
            if let Some(three) = deal_perfect(&mut rng, &b, Difficulty::Normal) {
                assert!(can_clear_all(&b, &three));
                hits += 1;
            }
        }
        assert!(hits >= 30, "{hits}");
    }

    #[test]
    fn deal_on_full_board_falls_back() {
        let mut rng = Rng::new(1);
        let mut b = Board::new();
        for r in 0..SIZE {
            for c in 0..SIZE {
                b.set(r, c, 1);
            }
        }
        let three = deal(&mut rng, &b);
        assert_eq!(three.len(), 3);
    }
}
