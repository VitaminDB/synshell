use elle_block::game::{generator, Board, Difficulty, Rng, SIZE};
use std::time::Instant;

/// Запуск: cargo test --release --test bench_deal -- --ignored --nocapture
#[test]
#[ignore]
fn bench_deal_near_empty_boards() {
    let mut rng = Rng::new(77);
    let mut worst = 0.0f64;
    let mut total = 0.0f64;
    let n = 200;
    for k in 0..n {
        let mut b = Board::new();
        let cells = 6 + (k % 20);
        let mut placed = 0;
        while placed < cells {
            let r = rng.index(SIZE);
            let c = rng.index(SIZE);
            if b.get(r, c) == 0 {
                b.set(r, c, 1);
                placed += 1;
            }
        }
        let t = Instant::now();
        let _ = generator::deal_for(&mut rng, &b, Difficulty::Normal);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        total += ms;
        worst = worst.max(ms);
    }
    eprintln!("deal_for near-empty: avg {:.2} ms, worst {:.2} ms", total / n as f64, worst);
}

/// Сравнение раздачи «под поле» со случайной: жадный бот играет партии,
/// считаем среднюю длину партии, очки и долю ходов с очисткой.
/// Запуск: cargo test --profile fast-release --test bench_deal -- --ignored --nocapture sim
#[test]
#[ignore]
fn sim_helpful_vs_random() {
    use elle_block::game::Game;
    for d in [Difficulty::Easy, Difficulty::Normal, Difficulty::Hard] {
        for helpful in [false, true] {
            generator::set_helpful(helpful);
            let (mut moves, mut score, mut clears, mut games) = (0u64, 0i64, 0u64, 0u64);
            let mut best_fit_sum = 0.0f64;
            let mut dealt = 0u64;
            for seed in 0..60u64 {
                let mut g = Game::with_difficulty(1000 + seed, d);
                let mut n = 0;
                while !g.over && n < 400 {
                    // жадно: ход с максимумом очков, при равенстве — плотнее
                    let mut best: Option<(f32, usize, i32, i32)> = None;
                    for slot in 0..3 {
                        let Some(p) = g.piece(slot).copied() else { continue };
                        if let Some(f) = generator::fit_score(&g.board, &p.shape) {
                            best_fit_sum += f as f64;
                            dealt += 1;
                        }
                        for r in 0..SIZE as i32 {
                            for c in 0..SIZE as i32 {
                                if !g.board.can_place(&p.shape, r, c) {
                                    continue;
                                }
                                let mut t = g.clone();
                                let pl = t.place(slot, r, c).unwrap();
                                let v = pl.clear.lines() as f32 * 100.0 + generator::fit_score(&g.board, &p.shape).unwrap_or(0.0) * 0.0
                                    - t.board.filled() as f32 * 0.5
                                    + if t.over { -1000.0 } else { 0.0 };
                                if best.map(|b| v > b.0).unwrap_or(true) {
                                    best = Some((v, slot, r, c));
                                }
                            }
                        }
                    }
                    let Some((_, slot, r, c)) = best else { break };
                    let pl = g.place(slot, r, c).unwrap();
                    if pl.clear.lines() > 0 {
                        clears += 1;
                    }
                    n += 1;
                }
                moves += n as u64;
                score += g.scoring.score as i64;
                games += 1;
            }
            eprintln!(
                "{:?} helpful={helpful}: ходов {:.0}, очков {:.0}, очисток {:.0}%, полезность фигур {:.2}",
                d,
                moves as f64 / games as f64,
                score as f64 / games as f64,
                clears as f64 * 100.0 / moves as f64,
                best_fit_sum / dealt.max(1) as f64
            );
        }
    }
    generator::set_helpful(true);
}
