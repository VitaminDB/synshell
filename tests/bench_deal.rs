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
