//! Многоголосие через службу: сценарий «Speaker N: …» и `speakers` —
//! N-я реплика голосом N-го персонажа. Поток: высота голоса и текст каждой
//! реплики; без потока — текст целиком.
//!
//! `cargo run --example dialog_check -- [модель] [персонажи через запятую] [wav]`

use std::time::Instant;
use synaptixd::{Client, Params};

const SCRIPT: &str = "Speaker 1: Good afternoon. Could you tell me where the nearest train station is?
Speaker 2: Of course. Walk straight for two blocks, then turn left at the bank.
Speaker 1: Thank you. How long does it take to get there on foot?
Speaker 2: About ten minutes. You can't miss it.";

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let model = a.get(1).cloned().unwrap_or_else(|| "vibevoice-1-5b".into());
    let cast = a.get(2).cloned().unwrap_or_else(|| "female_adult,male_adult".into());
    let out = a.get(3).cloned();
    let mut c = Client::connect().expect("служба");
    let p = Params::new().with("language", "en").with("speakers", cast.as_str());
    let t = Instant::now();
    let mut first = None;
    let mut chunks = Vec::new();
    let s = c
        .speak_stream(&model, SCRIPT, &p, |ch| {
            first.get_or_insert(t.elapsed().as_secs_f32());
            chunks.push(ch);
        })
        .expect("поток");
    println!("поток: первый звук через {:.1} с, всё за {:.1} с, звука {:.1} с", first.unwrap_or(0.0), t.elapsed().as_secs_f32(), s.audio.f32s().len() as f32 / s.sample_rate as f32);
    for ch in &chunks {
        let tr = c.transcribe("whisper-large-v3-turbo", &ch.pcm, ch.sample_rate, Some("en")).unwrap();
        println!("  #{} F0 {:3.0} Гц  «{}» → «{}»", ch.index, f0(&ch.pcm, ch.sample_rate), ch.text, tr.text.trim());
    }
    let t = Instant::now();
    let s = c.speak_with(&model, SCRIPT, &p).expect("озвучка");
    let pcm = s.audio.f32s();
    let tr = c.transcribe("whisper-large-v3-turbo", pcm, s.sample_rate, Some("en")).unwrap();
    println!("одним проходом: {:.1} с звука за {:.1} с\n  «{}»", pcm.len() as f32 / s.sample_rate as f32, t.elapsed().as_secs_f32(), tr.text.trim());
    if let Some(o) = out {
        let mut w = Vec::new();
        let data = pcm.len() as u32 * 2;
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&(36 + data).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes());
        w.extend_from_slice(&s.sample_rate.to_le_bytes());
        w.extend_from_slice(&(s.sample_rate * 2).to_le_bytes());
        w.extend_from_slice(&2u16.to_le_bytes());
        w.extend_from_slice(&16u16.to_le_bytes());
        w.extend_from_slice(b"data");
        w.extend_from_slice(&data.to_le_bytes());
        for x in pcm {
            w.extend_from_slice(&((x.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
        std::fs::write(o, w).unwrap();
    }
}

fn f0(pcm: &[f32], sr: u32) -> f32 {
    let win = (sr as f32 * 0.04) as usize;
    let (lo, hi) = ((sr / 450) as usize, (sr / 60) as usize);
    let mut f0s = Vec::new();
    let mut i = 0;
    while i + win + hi < pcm.len() {
        let w = &pcm[i..i + win + hi];
        i += win / 2;
        if w[..win].iter().map(|x| x * x).sum::<f32>() / (win as f32) < 1e-4 {
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
    f0s.sort_by(f32::total_cmp);
    f0s.get(f0s.len() / 2).copied().unwrap_or(0.0)
}
