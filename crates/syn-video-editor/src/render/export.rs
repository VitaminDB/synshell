//! Экспорт: кадры таймлайна рисует тот же композитор, что и просмотр
//! (`Offscreen`, масштаб 1), RGBA идёт в ffmpeg; звук клипов сводится его
//! фильтрами (обрезка, скорость, громкость, затухания, сдвиг, смешение).

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::Value;

use super::compose::{self, FrameSource};
use crate::doc::{Clip, MediaKind, Project, TrackKind};

/// Параметры экспорта.
#[derive(Clone, Debug)]
pub struct Opts {
    /// Качество x264 (меньше — лучше), 18 — почти без потерь.
    pub crf: u32,
    /// Только часть таймлайна, с.
    pub range: Option<(f64, f64)>,
}

impl Default for Opts {
    fn default() -> Self {
        Self { crf: 18, range: None }
    }
}

/// Клипы со звуком: (клип, путь, громкость с учётом дорожки).
fn audio_clips(p: &Project) -> Vec<(&Clip, String)> {
    let mut out = Vec::new();
    for tr in &p.tracks {
        if tr.muted || (tr.kind == TrackKind::Video && tr.hidden) {
            continue;
        }
        for c in &tr.clips {
            let Some(m) = c.media().and_then(|id| p.media(id)) else { continue };
            let has = match m.kind {
                MediaKind::Audio => true,
                MediaKind::Video => m.has_audio,
                MediaKind::Image => false,
            };
            if has && c.volume > 0.0 {
                out.push((c, m.path.display().to_string()));
            }
        }
    }
    out
}

/// Цепочка `atempo` для скорости (у фильтра предел 0.5–2 за шаг).
fn atempo(mut s: f64) -> Vec<String> {
    let mut v = Vec::new();
    while s > 2.0 {
        v.push("atempo=2.0".into());
        s /= 2.0;
    }
    while s < 0.5 {
        v.push("atempo=0.5".into());
        s /= 0.5;
    }
    if (s - 1.0).abs() > 1e-6 {
        v.push(format!("atempo={s:.6}"));
    }
    v
}

fn eff(c: &Clip, kind: &str) -> Option<f64> {
    c.effects.iter().find(|e| e.kind == kind).map(|e| e.params.get("duration").and_then(Value::as_f64).unwrap_or(0.5))
}

/// Экспортировать проект в `out` (mp4/mov/mkv/webm по расширению). `progress(кадр, всего)`.
pub fn export(p: &Project, out: &Path, opts: &Opts, mut progress: impl FnMut(usize, usize)) -> Result<Vec<String>, String> {
    let (w, h) = (p.settings.width & !1, p.settings.height & !1);
    let fps = p.settings.fps;
    let (t0, t1) = opts.range.unwrap_or((0.0, p.duration()));
    if t1 - t0 <= 0.0 {
        return Err("таймлайн пуст — нечего экспортировать".into());
    }
    let total = ((t1 - t0) * fps).ceil() as usize;
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }

    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-hide_banner", "-loglevel", "error", "-y"]);
    cmd.args(["-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{w}x{h}"), "-r", &format!("{fps}"), "-i", "-"]);
    if let Some((inputs, filter)) = audio_graph(p, t0, t1, 1) {
        cmd.args(&inputs);
        cmd.args(["-filter_complex", &filter, "-map", "0:v", "-map", "[aout]", "-c:a", "aac", "-b:a", "192k"]);
    } else {
        cmd.args(["-map", "0:v", "-an"]);
    }
    let ext = out.extension().and_then(|e| e.to_str()).unwrap_or("mp4").to_ascii_lowercase();
    match ext.as_str() {
        "webm" => cmd.args(["-c:v", "libvpx-vp9", "-b:v", "0", "-crf", &format!("{}", opts.crf + 14)]),
        _ => cmd.args(["-c:v", "libx264", "-preset", "medium", "-crf", &opts.crf.to_string(), "-pix_fmt", "yuv420p", "-movflags", "+faststart"]),
    };
    cmd.arg(out).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("ffmpeg: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("ffmpeg: нет stdin")?;

    let mut off = syngui::offscreen::Offscreen::new((w, h), "").map_err(|e| format!("композитор: {e}"))?;
    let mut src = FrameSource::new(w, h);
    src.neural = true;
    let mut result = Ok(());
    for i in 0..total {
        let t = t0 + i as f64 / fps;
        let frames = src.frames(p, t);
        let pc = p.clone();
        let rgba = match off.render(move || compose::frame_widget(&pc, t, &frames, 1.0)) {
            Ok(v) => v,
            Err(e) => {
                result = Err(e);
                break;
            }
        };
        if let Err(e) = stdin.write_all(&rgba) {
            result = Err(format!("ffmpeg: {e}"));
            break;
        }
        progress(i + 1, total);
    }
    drop(stdin);
    let out_ = child.wait_with_output().map_err(|e| format!("ffmpeg: {e}"))?;
    result?;
    if !out_.status.success() {
        return Err(format!("ffmpeg: {}", String::from_utf8_lossy(&out_.stderr).trim()));
    }
    src.notes.dedup();
    Ok(src.notes)
}

/// Сведение звука клипов в `[t0, t1]`: аргументы входов ffmpeg (`-ss … -t … -i файл`)
/// и `filter_complex` с выходом `[aout]`; `first` — номер первого входа звука.
/// `None` — звука нет.
pub fn audio_graph(p: &Project, t0: f64, t1: f64, first: usize) -> Option<(Vec<String>, String)> {
    let mut inputs = Vec::new();
    let mut filters = Vec::new();
    let mut labels = Vec::new();
    for (c, path) in audio_clips(p) {
        // кусок источника, видимый в диапазоне
        let vis0 = c.start.max(t0);
        let vis1 = c.end().min(t1);
        if vis1 <= vis0 {
            continue;
        }
        let src0 = c.offset + (vis0 - c.start) * c.speed;
        let src_len = (vis1 - vis0) * c.speed;
        inputs.extend(["-ss".to_string(), format!("{src0:.6}"), "-t".into(), format!("{src_len:.6}"), "-i".into(), path]);
        let mut chain = atempo(c.speed);
        chain.push(format!("volume={:.4}", c.volume));
        if let Some(d) = eff(c, "audio_fade_in") {
            chain.push(format!("afade=t=in:st={:.4}:d={d:.4}", (c.start - vis0).max(0.0)));
        }
        if let Some(d) = eff(c, "audio_fade_out") {
            chain.push(format!("afade=t=out:st={:.4}:d={d:.4}", (c.duration - d - (vis0 - c.start)).max(0.0)));
        }
        let delay = ((vis0 - t0) * 1000.0).round() as i64;
        chain.push(format!("adelay={delay}:all=1"));
        chain.push(format!("aresample={}", p.settings.sample_rate));
        let label = format!("a{}", labels.len());
        filters.push(format!("[{}:a]{}[{label}]", first + labels.len(), chain.join(",")));
        labels.push(format!("[{label}]"));
    }
    if labels.is_empty() {
        return None;
    }
    filters.push(format!("{}amix=inputs={}:normalize=0:duration=longest,apad,atrim=0:{:.6}[aout]", labels.join(""), labels.len(), t1 - t0));
    Some((inputs, filters.join(";")))
}

/// Звук таймлайна `[t0, t1]` для воспроизведения: стерео f32 с чередованием
/// на частоте проекта. `None` — звука нет.
pub fn mix_pcm(p: &Project, t0: f64, t1: f64) -> Result<Option<Vec<f32>>, String> {
    let Some((inputs, filter)) = audio_graph(p, t0, t1, 0) else { return Ok(None) };
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error"])
        .args(&inputs)
        .args(["-filter_complex", &filter, "-map", "[aout]", "-ac", "2", "-ar", &p.settings.sample_rate.to_string(), "-f", "f32le", "-"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    if !out.status.success() {
        return Err(format!("ffmpeg: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(Some(out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect()))
}
