//! Индекс медиатеки (E1): сцены (склейки) с подписями модели со зрением,
//! дикторы (кто когда говорит), бит и темп, а для поиска по смыслу —
//! эмбеддинги фраз расшифровки и подписей сцен. Индекс лежит в проекте у
//! медиа (переносим вместе с ним), векторы — в кэше рядом с файлом по его
//! пути, размеру и времени изменения (большие и пересчитываются заново).

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Сцена: отрезок источника между склейками и что на нём.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Scene {
    pub start: f64,
    pub end: f64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub caption: String,
}

/// Реплика диктора.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Turn {
    pub speaker: String,
    pub start: f64,
    pub end: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct MediaIndex {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scenes: Vec<Scene>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub speakers: Vec<Turn>,
    /// Доли ударов (с), темп.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub beats: Vec<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bpm: Option<f64>,
    /// Модель эмбеддингов, которой посчитаны векторы.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub embed_model: String,
}

/// Склейки по ffmpeg (`select=gt(scene,порог)`): моменты смены кадра.
pub fn scene_cuts(path: &Path, threshold: f64) -> Result<Vec<f64>, String> {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-nostats", "-i"])
        .arg(path)
        .args(["-an", "-vf", &format!("scale=320:-2,select='gt(scene,{threshold})',showinfo"), "-f", "null", "-"])
        .output()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    let err = String::from_utf8_lossy(&out.stderr);
    let mut cuts: Vec<f64> = err
        .lines()
        .filter(|l| l.contains("showinfo") && l.contains("pts_time:"))
        .filter_map(|l| l.split("pts_time:").nth(1)?.split_whitespace().next()?.parse::<f64>().ok())
        .collect();
    cuts.sort_by(f64::total_cmp);
    cuts.dedup_by(|a, b| (*a - *b).abs() < 0.3);
    Ok(cuts)
}

/// Сцены из склеек: короткие (< 0.6 с) сливаются с соседней.
pub fn scenes_from_cuts(cuts: &[f64], duration: f64) -> Vec<Scene> {
    let mut bounds = vec![0.0];
    for &c in cuts {
        if c > 0.6 && duration - c > 0.6 && c - bounds.last().copied().unwrap_or(0.0) >= 0.6 {
            bounds.push(c);
        }
    }
    bounds.push(duration.max(0.0));
    bounds.windows(2).map(|w| Scene { start: w[0], end: w[1], caption: String::new() }).collect()
}

/// Бит по огибающей звука (16 кГц моно): всплески энергии и темп по
/// автокорреляции 60–180 уд/мин. Для музыки; у речи темп не уверенный — None.
pub fn beats(pcm: &[f32], rate: f64) -> (Vec<f64>, Option<f64>) {
    let hop = (rate / 100.0) as usize; // 10 мс
    if pcm.len() < hop * 300 {
        return (Vec::new(), None);
    }
    // поток всплесков по полосам (спектральный поток на банке фильтров):
    // у оркестра, хора, эмбиента удар литавр или атака струнных тонет в общей
    // громкости, а в своей полосе виден. Каждая полоса нормирована своим
    // средним, чтобы бас не забивал верх.
    let bands: &[f64] = &[60.0, 150.0, 400.0, 1000.0, 2500.0, 5500.0];
    let frames = pcm.len() / hop;
    let mut flux = vec![0f32; frames];
    for &fc in bands.iter().filter(|f| **f < rate * 0.45) {
        let y = bandpass(pcm, rate, fc, 1.0);
        let log: Vec<f32> = y.chunks(hop).take(frames).map(|c| (c.iter().map(|x| x * x).sum::<f32>() / c.len() as f32 + 1e-10).ln()).collect();
        let f: Vec<f32> = (0..log.len()).map(|i| if i == 0 { 0.0 } else { (log[i] - log[i - 1]).max(0.0) }).collect();
        let m = f.iter().sum::<f32>() / f.len().max(1) as f32;
        if m > 0.0 {
            for (o, v) in flux.iter_mut().zip(&f) {
                *o += v / m;
            }
        }
    }
    let mean = flux.iter().sum::<f32>() / flux.len() as f32;
    // автокорреляция: лаг 0.33–1.0 с
    let (lo, hi) = (33usize, 100usize);
    let mut best = (0usize, 0f32);
    let mut total = 0f32;
    for lag in lo..=hi.min(flux.len() / 2) {
        let s: f32 = (lag..flux.len()).map(|i| (flux[i] - mean) * (flux[i - lag] - mean)).sum();
        total += s.max(0.0);
        if s > best.1 {
            best = (lag, s);
        }
    }
    let n = (hi - lo + 1) as f32;
    let ratio = if total > 0.0 { best.1 / (total / n) } else { 0.0 };
    if std::env::var_os("SYN_VE_BEATLOG").is_some() {
        eprintln!("beats: лаг {} отношение {ratio:.2}", best.0);
    }
    // музыка с ударами — 15+, речь и шумы сцены — 5–6
    let confident = best.0 > 0 && best.1 > 0.0 && ratio > 9.0;
    let bpm = confident.then(|| (6000.0 / best.0 as f64 * 10.0).round() / 10.0);
    // удары — ровная сетка по темпу (для монтажа нужен ритм, а не каждый всплеск):
    // фаза — сдвиг, при котором сумма всплесков в узлах сетки наибольшая
    // ровного темпа нет (оркестр, эмбиент, свободный ритм) — сильные акценты:
    // пики потока выше местного среднего + 1.5σ (окно 1.5 с), не чаще 0.5 с
    let Some(_) = bpm else { return (accents(&flux, hop as f64 / rate), None) };
    let period = best.0;
    let score = |ph: usize| -> f32 { (ph..flux.len()).step_by(period).map(|i| flux[i.saturating_sub(1)].max(flux[i]).max(flux[(i + 1).min(flux.len() - 1)])).sum() };
    let phase = (0..period).max_by(|a, b| score(*a).total_cmp(&score(*b))).unwrap_or(0);
    let out: Vec<f64> = (phase..flux.len()).step_by(period).map(|i| i as f64 * hop as f64 / rate).collect();
    (out, bpm)
}

/// Акценты потока всплесков: местные пики выше среднего + 1.5σ окна ±0.75 с,
/// не ближе 0.5 с друг к другу (из двух близких — сильнейший). `step` — с на кадр.
fn accents(flux: &[f32], step: f64) -> Vec<f64> {
    let half = (0.75 / step) as usize;
    let gap = (0.5 / step) as usize;
    let mut picks: Vec<(usize, f32)> = Vec::new();
    for i in 1..flux.len().saturating_sub(1) {
        let v = flux[i];
        if v < flux[i - 1] || v < flux[i + 1] {
            continue;
        }
        let w = &flux[i.saturating_sub(half)..(i + half).min(flux.len())];
        let m = w.iter().sum::<f32>() / w.len() as f32;
        let sd = (w.iter().map(|x| (x - m) * (x - m)).sum::<f32>() / w.len() as f32).sqrt();
        if v <= m + 1.5 * sd || v <= 0.0 {
            continue;
        }
        match picks.last_mut() {
            Some(last) if i - last.0 < gap => {
                if v > last.1 {
                    *last = (i, v);
                }
            }
            _ => picks.push((i, v)),
        }
    }
    picks.into_iter().map(|(i, _)| (i as f64 * step * 1000.0).round() / 1000.0).collect()
}

/// Полосовой фильтр (biquad RBJ, пик 0 дБ) с добротностью `q`.
fn bandpass(x: &[f32], rate: f64, fc: f64, q: f64) -> Vec<f32> {
    let w = 2.0 * std::f64::consts::PI * fc / rate;
    let alpha = w.sin() / (2.0 * q);
    let a0 = 1.0 + alpha;
    let (b0, b2) = (alpha / a0, -alpha / a0);
    let (a1, a2) = (-2.0 * w.cos() / a0, (1.0 - alpha) / a0);
    let (mut x1, mut x2, mut y1, mut y2) = (0f64, 0f64, 0f64, 0f64);
    x.iter()
        .map(|&s| {
            let s = s as f64;
            let y = b0 * s + b2 * x2 - a1 * y1 - a2 * y2;
            x2 = x1;
            x1 = s;
            y2 = y1;
            y1 = y;
            y as f32
        })
        .collect()
}

/// Реплики дикторов из JSON разметки (`[{speaker, start_s, end_s, …}]`).
pub fn parse_speakers(json: &str) -> Vec<Turn> {
    let v: Value = serde_json::from_str(json).unwrap_or(Value::Null);
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| {
                    let sp = match s.get("speaker")? {
                        Value::String(x) => x.clone(),
                        x => format!("{}", x.as_i64().map(|n| n + 1).unwrap_or(0)),
                    };
                    Some(Turn { speaker: sp, start: s.get("start_s")?.as_f64()?, end: s.get("end_s")?.as_f64()? })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Кусок для поиска: файл, отрезок, вид (речь/сцена), текст.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Chunk {
    pub media: u64,
    pub start: f64,
    pub end: f64,
    pub kind: String,
    pub text: String,
}

/// Куски медиа для поиска: фразы расшифровки (с диктором) и подписи сцен.
pub fn chunks(m: &crate::doc::Media) -> Vec<Chunk> {
    let mut out = Vec::new();
    let speaker_at = |t: f64| m.index.as_ref().and_then(|i| i.speakers.iter().find(|s| t >= s.start && t < s.end).map(|s| s.speaker.clone()));
    if let Some(tr) = &m.transcript {
        for (s, e, text) in crate::cmd::audio::phrases(&tr.words, 120) {
            let who = speaker_at((s + e) / 2.0).map(|s| format!("диктор {s}: ")).unwrap_or_default();
            out.push(Chunk { media: m.id, start: s, end: e, kind: "речь".into(), text: format!("{who}{text}") });
        }
    }
    if let Some(ix) = &m.index {
        for sc in ix.scenes.iter().filter(|s| !s.caption.is_empty()) {
            out.push(Chunk { media: m.id, start: sc.start, end: sc.end, kind: "сцена".into(), text: sc.caption.clone() });
        }
    }
    out
}

/// Файл кэша векторов медиа (по пути, размеру, времени изменения и модели).
pub fn vectors_file(path: &Path, model: &str) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    if let Ok(md) = std::fs::metadata(path) {
        md.len().hash(&mut h);
        md.modified().ok().hash(&mut h);
    }
    model.hash(&mut h);
    let base = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".cache"))).unwrap_or_else(std::env::temp_dir);
    base.join("syn-video-editor/index").join(format!("{:016x}.json", h.finish()))
}

#[derive(Serialize, Deserialize, Default)]
pub struct Vectors {
    pub chunks: Vec<Chunk>,
    pub vectors: Vec<Vec<f32>>,
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let (mut d, mut na, mut nb) = (0f32, 0f32, 0f32);
    for (x, y) in a.iter().zip(b) {
        d += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        d / (na.sqrt() * nb.sqrt())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenes_and_speakers() {
        let s = scenes_from_cuts(&[0.2, 3.0, 3.3, 7.0, 9.8], 10.0);
        let b: Vec<(f64, f64)> = s.iter().map(|x| (x.start, x.end)).collect();
        assert_eq!(b, vec![(0.0, 3.0), (3.0, 7.0), (7.0, 10.0)]);
        let t = parse_speakers(r#"[{"speaker":0,"start_s":0.5,"end_s":2.0,"confidence":0.9},{"speaker":1,"start_s":2.1,"end_s":4.0}]"#);
        assert_eq!(t.len(), 2);
        assert_eq!(t[1].speaker, "2");
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn beats_on_click_track() {
        // щелчки 120 уд/мин (каждые 0.5 с) на 16 кГц, 8 с
        let rate = 16000.0;
        let mut pcm = vec![0.001f32; (rate * 8.0) as usize];
        for k in 0..16 {
            let i = (k as f64 * 0.5 * rate) as usize;
            for j in 0..400 {
                if i + j < pcm.len() {
                    pcm[i + j] = 0.8 * (1.0 - j as f32 / 400.0);
                }
            }
        }
        let (b, bpm) = beats(&pcm, rate);
        assert!(bpm.is_some_and(|x| (x - 120.0).abs() < 4.0), "{bpm:?}");
        assert!(b.len() >= 12, "{b:?}");
    }
}
