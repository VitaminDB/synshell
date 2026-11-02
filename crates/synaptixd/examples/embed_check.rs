//! Эмбеддинги en/ru/kk через службу: близкие по смыслу фразы на разных
//! языках ближе, чем далёкие; скорость пачки.
//!
//! `cargo run --example embed_check -- [модель]`.

use std::time::Instant;
use synaptixd::Client;

fn main() {
    let model = std::env::args().nth(1).unwrap_or_else(|| "bge-m3".into());
    let mut c = Client::connect().expect("служба");
    let texts: Vec<String> = [
        "I would like to book a hotel room for two nights.",
        "Я хотел бы забронировать номер в отеле на две ночи.",
        "Мен қонақүйден екі түнге бөлме брондағым келеді.",
        "Can I reserve a room for a couple of nights?",
        "The weather is terrible today, it's raining heavily.",
        "Бүгін ауа райы нашар, қатты жаңбыр жауып тұр.",
        "happy",
        "glad",
        "sad",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let _ = c.embed(&model, &texts[..1]);
    let t = Instant::now();
    let e = c.embed(&model, &texts).expect("эмбеддинги");
    println!("{} текстов, dim {}, {:.0} мс", texts.len(), e.dim, t.elapsed().as_secs_f64() * 1e3);
    let cos = |a: usize, b: usize| e.vectors[a].iter().zip(&e.vectors[b]).map(|(x, y)| x * y).sum::<f32>();
    for (a, b) in [(0, 1), (0, 2), (0, 3), (1, 2), (0, 4), (2, 5), (4, 5), (6, 7), (6, 8)] {
        println!("{:.3}  «{}» ~ «{}»", cos(a, b), texts[a], texts[b]);
    }
    let big: Vec<String> = (0..256).map(|i| format!("Example sentence number {i} about learning English words.")).collect();
    let t = Instant::now();
    c.embed(&model, &big).unwrap();
    println!("256 фраз: {:.0} мс", t.elapsed().as_secs_f64() * 1e3);
}
