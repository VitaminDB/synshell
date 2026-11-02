//! Распознавание со словами через службу: слова, таймкоды, уверенность,
//! WER против известного текста, темп и паузы (метрики беглости).
//!
//! `cargo run --example asr_check -- <модель> <wav> [язык] [эталон] [verbatim]`.

use std::time::Instant;
use synaptixd::Client;

/// WAV PCM16 → моно f32.
fn read_wav(path: &str) -> (Vec<f32>, u32) {
    let b = std::fs::read(path).expect("wav");
    let (mut sr, mut ch, mut i) = (16000u32, 1usize, 12usize);
    while i + 8 <= b.len() {
        let id = &b[i..i + 4];
        let n = u32::from_le_bytes(b[i + 4..i + 8].try_into().unwrap()) as usize;
        if id == b"fmt " {
            ch = u16::from_le_bytes([b[i + 10], b[i + 11]]) as usize;
            sr = u32::from_le_bytes(b[i + 12..i + 16].try_into().unwrap());
        } else if id == b"data" {
            let d = &b[i + 8..(i + 8 + n).min(b.len())];
            let s: Vec<f32> = d.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect();
            return (s.chunks(ch).map(|f| f.iter().sum::<f32>() / ch as f32).collect(), sr);
        }
        i += 8 + n + (n & 1);
    }
    panic!("нет data в wav");
}

fn norm(s: &str) -> Vec<String> {
    s.to_lowercase().split_whitespace().map(|w| w.chars().filter(|c| c.is_alphanumeric() || *c == '\'').collect::<String>()).filter(|w| !w.is_empty()).collect()
}

fn wer(r: &[String], h: &[String]) -> f32 {
    let mut d: Vec<usize> = (0..=h.len()).collect();
    for i in 1..=r.len() {
        let mut prev = d[0];
        d[0] = i;
        for j in 1..=h.len() {
            let t = d[j];
            d[j] = (d[j] + 1).min(d[j - 1] + 1).min(prev + (r[i - 1] != h[j - 1]) as usize);
            prev = t;
        }
    }
    d[h.len()] as f32 / r.len().max(1) as f32
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (model, path) = (a[1].clone(), a[2].clone());
    let lang = a.get(3).filter(|s| !s.is_empty() && *s != "-").cloned();
    let reference = a.get(4).cloned();
    let verbatim = a.get(5).is_some_and(|v| v == "verbatim");
    let (pcm, sr) = read_wav(&path);
    let mut c = Client::connect().expect("служба");
    let t = Instant::now();
    let plain = c.transcribe(&model, &pcm, sr, lang.as_deref()).expect("asr");
    let t_plain = t.elapsed().as_secs_f32();
    let t = Instant::now();
    let tr = c.transcribe_words(&model, &pcm, sr, lang.as_deref(), verbatim).expect("asr слова");
    let t_words = t.elapsed().as_secs_f32();
    let dur = pcm.len() as f32 / sr as f32;
    println!("язык {:?}, {dur:.1} с звука: текст за {t_plain:.2} с, со словами за {t_words:.2} с", tr.language);
    println!("{}", tr.text);
    for w in &tr.words {
        println!("  {:6.2}–{:6.2}  {:.2}  {}", w.start, w.end, w.conf, w.text);
    }
    if let Some(r) = &reference {
        println!("WER {:.1} % (без слов: {:.1} %)", wer(&norm(r), &norm(&tr.text)) * 100.0, wer(&norm(r), &norm(&plain.text)) * 100.0);
    }
    if tr.words.len() > 1 {
        let speech: f32 = tr.words.iter().map(|w| w.end - w.start).sum();
        let pauses: Vec<f32> = tr.words.windows(2).map(|p| p[1].start - p[0].end).filter(|g| *g >= 0.25).collect();
        let span = tr.words.last().unwrap().end - tr.words[0].start;
        println!(
            "темп {:.0} слов/мин, доля речи {:.0} %, пауз ≥250 мс: {} (средн. {:.2} с), низкая уверенность (<0,7): {}",
            tr.words.len() as f32 * 60.0 / span,
            speech / span * 100.0,
            pauses.len(),
            pauses.iter().sum::<f32>() / pauses.len().max(1) as f32,
            tr.words.iter().filter(|w| w.conf < 0.7).count()
        );
    }
}
