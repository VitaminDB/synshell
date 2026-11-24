//! Ленты превью эффектов и переходов для панели «Эффекты»: тот же композитор,
//! мини-проект из двух кадров A и B, `FRAMES` положений. Переход — смена
//! A → B, эффект — нарастание параметра. Готовые ленты — PNG в кэше
//! (`~/.cache/syn-video-editor/fx/<вид>.png`), кадры склеены по горизонтали.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use super::compose::{self, FrameSource};
use crate::doc::{Clip, Effect, Media, MediaKind, Project, Source, Track, TrackKind, Transition};

pub const FRAMES: usize = 16;
const W: u32 = 320;
const H: u32 = 180;
/// Версия рисунка лент: смена — пересчёт кэша.
const VERSION: u32 = 1;

pub fn dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache"))).unwrap_or_else(std::env::temp_dir).join(format!("syn-video-editor/fx-v{VERSION}"))
}

/// Файл ленты вида (`transition:dissolve`, `effect:blur`).
pub fn strip_path(key: &str) -> PathBuf {
    dir().join(format!("{}.png", key.replace(':', "_")))
}

fn base(w: u32, h: u32, fps: f64) -> Project {
    let mut p = Project::new("preview");
    p.settings.width = w;
    p.settings.height = h;
    p.settings.fps = fps;
    p.tracks.clear();
    p
}

fn clip(id: u64, source: Source, start: f64, duration: f64) -> Clip {
    Clip { id, source, start, duration, offset: 0.0, speed: 1.0, volume: 1.0, name: String::new(), effects: Vec::new(), transition: None }
}

fn title_style() -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("size".into(), json!(110));
    m.insert("position".into(), "center".into());
    m.insert("anim".into(), json!(0));
    m
}

/// Кадр-образец: цвет и большая буква (через тот же композитор).
fn sample(off: &mut syngui::offscreen::Offscreen, letter: &str, color: &str, out: &Path) -> Result<(), String> {
    let mut p = base(W, H, 30.0);
    p.tracks.push(Track { id: 1, kind: TrackKind::Video, name: "V1".into(), muted: false, hidden: false, clips: vec![clip(10, Source::Color { color: color.into() }, 0.0, 1.0)] });
    p.tracks.push(Track { id: 2, kind: TrackKind::Video, name: "V2".into(), muted: false, hidden: false, clips: vec![clip(11, Source::Title { text: letter.into(), style: title_style() }, 0.0, 1.0)] });
    let rgba = off.render(move || compose::frame_widget(&p, 0.5, &Default::default(), 1.0))?;
    image::save_buffer(out, &rgba, W, H, image::ExtendedColorType::Rgba8).map_err(|e| e.to_string())
}

fn media(id: u64, path: &Path) -> Media {
    Media { id, path: path.to_path_buf(), kind: MediaKind::Image, duration: 0.0, width: W, height: H, fps: 0.0, has_audio: false, transcript: None }
}

/// Проект кадра `i` ленты вида `key` и время кадра.
fn scene(key: &str, i: usize, a: &Path, b: &Path) -> (Project, f64) {
    let f = i as f64 / (FRAMES - 1) as f64;
    let mut p = base(W, H, 30.0);
    p.media = vec![media(1, a), media(2, b)];
    let (kind, name) = key.split_once(':').unwrap_or(("effect", key));
    if kind == "transition" {
        let mut cb = clip(21, Source::Media { media: 2 }, 1.0, 1.0);
        cb.transition = Some(Transition { kind: name.into(), duration: 1.0 });
        p.tracks.push(Track { id: 1, kind: TrackKind::Video, name: "V1".into(), muted: false, hidden: false, clips: vec![clip(20, Source::Media { media: 1 }, 0.0, 1.0), cb] });
        return (p, 0.5 + f * 0.999);
    }
    let mut c = clip(20, Source::Media { media: 1 }, 0.0, 2.0);
    let mut params = Map::new();
    let mut t = 1.0;
    match name {
        "fade_in" => {
            params.insert("duration".into(), json!(1.0));
            t = f * 0.999;
        }
        "fade_out" => {
            params.insert("duration".into(), json!(1.0));
            t = 1.0 + f * 0.999;
        }
        "blur" => {
            params.insert("radius".into(), json!(f * 36.0));
        }
        "vignette" => {
            params.insert("amount".into(), json!(f));
        }
        "color" => {
            params.insert("saturation".into(), json!(-1.0 + 2.0 * f));
            params.insert("contrast".into(), json!(0.4 * f));
        }
        "transform" => {
            params.insert("scale".into(), json!(0.55 + 0.6 * f));
            params.insert("rotation".into(), json!(-12.0 + 24.0 * f));
        }
        _ => {}
    }
    c.effects.push(Effect { id: 30, kind: name.into(), params });
    p.tracks.push(Track { id: 1, kind: TrackKind::Video, name: "V1".into(), muted: false, hidden: false, clips: vec![c] });
    (p, t)
}

/// Все виды для панели: (ключ, подпись).
pub fn kinds() -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = compose::TRANSITIONS.iter().map(|(k, t)| (format!("transition:{k}"), t.to_string())).collect();
    for (k, _) in crate::cmd::EFFECTS {
        v.push((format!("effect:{k}"), k.to_string()));
    }
    v
}

/// Дочерний процесс: отрисовать недостающие ленты.
pub fn render_all() -> Result<String, String> {
    let d = dir();
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    let mut off = syngui::offscreen::Offscreen::new((W, H), "")?;
    let (a, b) = (d.join("sample-a.png"), d.join("sample-b.png"));
    if !a.exists() {
        sample(&mut off, "A", "#e07a5f", &a)?;
    }
    if !b.exists() {
        sample(&mut off, "B", "#3d85c6", &b)?;
    }
    let mut made = 0;
    let all = kinds();
    for (n, (key, _)) in all.iter().enumerate() {
        let out = strip_path(key);
        if out.exists() {
            continue;
        }
        let mut strip = image::RgbaImage::new(W * FRAMES as u32, H);
        let mut src = FrameSource::new(W, H);
        for i in 0..FRAMES {
            let (p, t) = scene(key, i, &a, &b);
            let frames = src.frames(&p, t);
            let rgba = off.render(move || compose::frame_widget(&p, t, &frames, 1.0))?;
            if let Some(img) = image::RgbaImage::from_raw(W, H, rgba) {
                image::imageops::replace(&mut strip, &img, (i as u32 * W) as i64, 0);
            }
        }
        strip.save(&out).map_err(|e| e.to_string())?;
        made += 1;
        println!("progress {} {}", n + 1, all.len());
    }
    Ok(format!("лент: {made}"))
}
