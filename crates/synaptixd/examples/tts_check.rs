//! Проверка озвучки через службу: голоса-персонажи (высота голоса по
//! основному тону — мужские ниже женских), скорость (длительность), поток по
//! предложениям (задержка первого куска против всей фразы).
//!
//! `cargo run --example tts_check -- <модель> [каталог для wav]`.

use std::time::Instant;
use synaptixd::{Client, Params};

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
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data).to_le_bytes());
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
    let args: Vec<String> = std::env::args().collect();
    let model = args.get(1).cloned().unwrap_or_else(|| "omnivoice".into());
    let dir = args.get(2).cloned();
    let mut c = Client::connect().expect("служба");
    let line = "Good morning! Thank you for coming in today. Could you tell me a little about yourself?";
    let base = Params::new().with("language", "en");
    // Прогрев.
    let _ = c.speak_with(&model, "Hello.", &base);

    println!("== персонажи ==");
    for sp in synaptixd::SPEAKERS {
        let t = Instant::now();
        let s = c.speak_with(&model, line, &base.clone().with("speaker", *sp)).expect("озвучка");
        let pcm = s.audio.f32s();
        println!("{sp:15} F0 {:5.0} Гц  {:4.1} с звука за {:4.1} с", median_f0(pcm, s.sample_rate), pcm.len() as f32 / s.sample_rate as f32, t.elapsed().as_secs_f32());
        save(&dir, &format!("{model}-{sp}"), pcm, s.sample_rate);
    }
    // Тот же персонаж второй раз — тот же голос (F0 близко).
    let s = c.speak_with(&model, "And what are your strongest skills?", &base.clone().with("speaker", "male_adult")).unwrap();
    println!("male_adult ещё раз: F0 {:.0} Гц", median_f0(s.audio.f32s(), s.sample_rate));
    save(&dir, &format!("{model}-male_adult-again"), s.audio.f32s(), s.sample_rate);

    println!("== скорость ==");
    for sp in [0.75f32, 1.0, 1.3] {
        let s = c.speak_with(&model, line, &base.clone().with("speaker", "female_young").with("speed", sp).with("seed", 7i64)).unwrap();
        let pcm = s.audio.f32s();
        println!("speed {sp:4}: {:4.2} с, F0 {:.0} Гц", pcm.len() as f32 / s.sample_rate as f32, median_f0(pcm, s.sample_rate));
        save(&dir, &format!("{model}-speed-{sp}"), pcm, s.sample_rate);
    }

    println!("== поток ==");
    let text = "Welcome to your mock IELTS speaking test. My name is Daniel, and I will be your examiner today. \
                In the first part, I will ask you some questions about yourself. Let's begin. \
                Do you work or are you a student? What do you enjoy most about it?";
    let p = base.clone().with("speaker", "male_adult");
    let t = Instant::now();
    let mut first = None;
    let s = c
        .speak_stream(&model, text, &p, |ch| {
            let at = t.elapsed().as_secs_f32();
            first.get_or_insert(at);
            println!("  кусок {} через {:4.1} с: {:4.1} с звука «{}»", ch.index, at, ch.pcm.len() as f32 / ch.sample_rate as f32, ch.text);
        })
        .unwrap();
    let total = t.elapsed().as_secs_f32();
    println!("первый звук через {:.1} с, вся речь {:.1} с за {total:.1} с", first.unwrap_or(0.0), s.audio.f32s().len() as f32 / s.sample_rate as f32);
    save(&dir, &format!("{model}-stream"), s.audio.f32s(), s.sample_rate);
}
