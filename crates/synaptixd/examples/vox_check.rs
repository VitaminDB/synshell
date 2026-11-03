//! Проверка озвучки персонажами: каждая фраза каждым персонажем — длительность
//! против нормы по тексту и сверка whisper-текста с исходным (WER). Ловит
//! «убегание» синтеза (бормотание, чужой язык, растянутые слова).
//!
//! `cargo run --example vox_check -- [модель] [персонажи через запятую] [каталог wav]`;
//! STREAM=1 — через поток (speak_stream), как synspeak.

use std::time::Instant;
use synaptixd::{Client, Params};

const PHRASES: &[&str] = &[
    "Hello! Thank you for coming in today. Could you please introduce yourself briefly?",
    "That sounds great. What do you enjoy most about your current role?",
    "Can you tell me about a time when you solved a difficult problem at work?",
    "Why do you want to work for our company?",
    "Where do you see yourself in five years?",
    "Good morning. Please take a seat.",
    "I see. And how do you usually handle stress and tight deadlines?",
    "Thank you, that was very helpful. Do you have any questions for me?",
];

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

/// Медиана основного тона (Гц), YIN по окнам 40 мс с голосом.
fn median_f0(pcm: &[f32], sr: u32) -> f32 {
    let win = (sr as f32 * 0.04) as usize;
    let (lo, hi) = ((sr / 450) as usize, (sr / 60) as usize);
    let mut f0s = Vec::new();
    let mut i = 0;
    while i + win + hi < pcm.len() {
        let w = &pcm[i..i + win + hi];
        i += win / 2;
        let e: f32 = w[..win].iter().map(|x| x * x).sum::<f32>() / win as f32;
        if e < 1e-4 {
            continue;
        }
        let d: Vec<f32> = (0..=hi).map(|tau| (0..win).map(|j| (w[j] - w[j + tau]).powi(2)).sum()).collect();
        let mut run = 0f32;
        let cm: Vec<f32> = d.iter().enumerate().map(|(tau, &v)| if tau == 0 { 1.0 } else { run += v; v * tau as f32 / run.max(1e-9) }).collect();
        let mut tau = lo;
        while tau < hi {
            if cm[tau] < 0.15 {
                while tau + 1 < hi && cm[tau + 1] < cm[tau] {
                    tau += 1;
                }
                f0s.push(sr as f32 / tau as f32);
                break;
            }
            tau += 1;
        }
    }
    f0s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    f0s.get(f0s.len() / 2).copied().unwrap_or(0.0)
}

fn save(dir: &Option<String>, name: &str, pcm: &[f32], sr: u32) {
    let Some(d) = dir else { return };
    let mut w = Vec::new();
    let data = pcm.len() as u32 * 2;
    for (k, v) in [(b"RIFF", 36 + data)] {
        w.extend_from_slice(k);
        w.extend_from_slice(&v.to_le_bytes());
    }
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&sr.to_le_bytes());
    w.extend_from_slice(&(sr * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data.to_le_bytes());
    for x in pcm {
        w.extend_from_slice(&((x.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    let _ = std::fs::create_dir_all(d);
    std::fs::write(format!("{d}/{name}.wav"), w).unwrap();
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let model = a.get(1).cloned().unwrap_or_else(|| "voxcpm2".into());
    let speakers: Vec<String> = match a.get(2).filter(|s| !s.is_empty() && *s != "all") {
        Some(s) => s.split(',').map(String::from).collect(),
        None => synaptixd::SPEAKERS.iter().map(|s| s.to_string()).collect(),
    };
    let dir = a.get(3).cloned();
    let stream = std::env::var_os("STREAM").is_some();
    let steps: Option<i64> = std::env::var("STEPS").ok().and_then(|s| s.parse().ok());
    let n: usize = std::env::var("N").ok().and_then(|s| s.parse().ok()).unwrap_or(PHRASES.len());
    let (mut synth_s, mut audio_s) = (0f32, 0f32);
    let mut c = Client::connect().expect("служба");
    let (mut bad, mut total) = (0, 0);
    for sp in &speakers {
        let mut p = Params::new().with("language", "en").with("speaker", sp.as_str());
        if let Some(st) = steps {
            p = p.with("steps", st);
        }
        let mut f0s = Vec::new();
        let mut line = Vec::new();
        for (i, text) in PHRASES.iter().enumerate().take(n) {
            let t = Instant::now();
            let s = if stream { c.speak_stream(&model, text, &p, |_| {}) } else { c.speak_with(&model, text, &p) }.expect("озвучка");
            let el = t.elapsed().as_secs_f32();
            let pcm = s.audio.f32s();
            let dur = pcm.len() as f32 / s.sample_rate as f32;
            let tr = c.transcribe("whisper-large-v3-turbo", pcm, s.sample_rate, Some("en")).expect("asr");
            let w = wer(&norm(text), &norm(&tr.text));
            // Норма: ~0.075 с на символ, с запасом.
            let limit = text.chars().count() as f32 * 0.12 + 1.5;
            let peak = pcm.iter().fold(0f32, |m, x| m.max(x.abs()));
            let ok = dur <= limit && w <= 0.25 && peak >= 0.1;
            total += 1;
            if !ok {
                bad += 1;
                println!("  ПЛОХО {sp} #{i}: {dur:.1} с (предел {limit:.1}), пик {peak:.3}, WER {:.0}% «{}»", w * 100.0, tr.text.trim());
            }
            line.push(format!("{dur:4.1}{}", if ok { " " } else { "!" }));
            save(&dir, &format!("{model}-{sp}-{i}"), pcm, s.sample_rate);
            if i > 0 {
                synth_s += el;
                audio_s += dur;
            }
            f0s.push(median_f0(pcm, s.sample_rate));
        }
        f0s.sort_by(f32::total_cmp);
        println!("{sp:15} {}  F0 {:.0} Гц", line.join(" "), f0s[f0s.len() / 2]);
    }
    println!("плохих: {bad}/{total}, RTF {:.2} (без первой фразы персонажа)", synth_s / audio_s.max(1e-3));
}
