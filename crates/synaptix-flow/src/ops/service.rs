//! Ноды генерации — запросы к службе synaptixd. Отмена прогона рвёт
//! соединение, служба по обрыву прерывает генерацию.

use std::process::Command;

use crate::run::{CancelGuard, Cx};
use crate::value::Val;

type R = Result<(), String>;

/// Клиент службы, отменяемый прогоном.
fn client(cx: &Cx) -> Result<(synaptixd::Client, Option<CancelGuard>), String> {
    cx.cancel.check()?;
    let c = synaptixd::Client::connect().map_err(|e| format!("служба нейросетей: {e}"))?;
    let guard = c.canceller().map(|k| cx.cancel.on_cancel(move || k.cancel()));
    Ok((c, guard))
}

/// Ошибка службы; после отмены — «остановлено».
fn err(cx: &Cx, e: impl ToString) -> String {
    if cx.cancel.is_cancelled() {
        "прогон остановлен".into()
    } else {
        e.to_string()
    }
}

pub fn llm(cx: &mut Cx) -> R {
    let mut msgs = Vec::new();
    if let Some(sys) = cx.str("system") {
        msgs.push(synaptixd::Message::system(sys));
    }
    msgs.push(synaptixd::Message::user(cx.need("prompt")?));
    let mut params = synaptixd::Params::new().with("thinking", false);
    if let Some(n) = cx.pos("max_tokens") {
        params = params.with("max_tokens", n as i64);
    }
    let (mut c, _g) = client(cx)?;
    let g = c.chat(&cx.str("model").unwrap_or_default(), msgs, Vec::new(), &params, |_| {}).map_err(|e| err(cx, e))?;
    cx.label = format!("{} токенов", g.tokens);
    cx.out("text", Val::Text(g.text.trim().to_string()));
    Ok(())
}

pub fn image(cx: &mut Cx) -> R {
    let mut images = cx.files("images");
    images.retain(|s| !s.is_empty());
    let p = synaptixd::ImageParams {
        negative: cx.str("negative"),
        width: cx.pos("width").map(|v| v as usize),
        height: cx.pos("height").map(|v| v as usize),
        steps: cx.pos("steps").map(|v| v as usize),
        guidance: cx.pos("guidance").map(|v| v as f32),
        seed: cx.pos("seed").map(|v| v as u64),
        images,
        init: cx.file("init"),
        strength: cx.pos("strength").map(|v| v as f32),
    };
    let prompt = cx.need("prompt")?;
    let model = cx.str("model").unwrap_or_default();
    let (mut c, _g) = client(cx)?;
    let (reply, png) = {
        let mut prog = |i, n| cx.progress(i, n);
        c.image(&model, &prompt, &p, &mut prog)
    }
    .map_err(|e| err(cx, e))?;
    let path = cx.out_path("png");
    std::fs::write(&path, png).map_err(|e| e.to_string())?;
    cx.label = reply.label;
    cx.out("image", Val::File(path));
    Ok(())
}

/// Модель видео по нужным возможностям (как у инструмента media).
pub fn video_model(cx: &Cx, p: &synaptixd::VideoParams) -> String {
    if let Some(m) = cx.str("model") {
        return m;
    }
    let restyle = cx.bool("restyle") && p.source_video.is_some();
    let ltx_only = !restyle && (p.audio.is_some() || p.source_video.is_some() || p.control_video.is_some() || p.two_stage);
    if ltx_only {
        "ltx-2-3-22b-distilled-1-1".into()
    } else if p.first_frame.is_some() || p.last_frame.is_some() || restyle {
        "minimax-h3-fl2va".into()
    } else {
        String::new()
    }
}

pub fn video(cx: &mut Cx) -> R {
    let p = synaptixd::VideoParams {
        negative: cx.str("negative"),
        width: cx.pos("width").map(|v| v as usize),
        height: cx.pos("height").map(|v| v as usize),
        seconds: cx.pos("seconds"),
        steps: cx.pos("steps").map(|v| v as usize),
        guidance: cx.pos("guidance").map(|v| v as f32),
        seed: cx.pos("seed").map(|v| v as u64),
        first_frame: cx.file("first_frame"),
        last_frame: cx.file("last_frame"),
        refs: cx.files("refs"),
        audio: cx.file("audio"),
        source_video: cx.file("source_video"),
        retake: match (cx.num("retake_start"), cx.pos("retake_end")) {
            (Some(a), Some(b)) if b > a => Some((a, b)),
            _ => None,
        },
        extend: cx.bool("extend"),
        control_video: cx.file("control_video"),
        control: cx.str("control"),
        two_stage: cx.bool("two_stage"),
        strength: cx.pos("strength").map(|v| v as f32),
    };
    let model = video_model(cx, &p);
    let prompt = cx.need("prompt")?;
    let (mut c, _g) = client(cx)?;
    let (reply, mp4) = {
        let mut prog = |i, n| cx.progress(i, n);
        c.video(&model, &prompt, &p, &mut prog)
    }
    .map_err(|e| err(cx, e))?;
    let path = cx.out_path("mp4");
    std::fs::write(&path, mp4).map_err(|e| e.to_string())?;
    cx.label = reply.label;
    cx.out("video", Val::File(path));
    Ok(())
}

pub fn music(cx: &mut Cx) -> R {
    let lyrics = cx.str("lyrics").unwrap_or_default();
    let prompt = cx.need("prompt")?;
    let (mut c, _g) = client(cx)?;
    // Инструментал — ACE-Step (YuE2 — песенная модель).
    let model = cx.str("model").or_else(|| {
        if lyrics.is_empty() {
            c.list().ok()?.into_iter().filter(|m| m.kind == synaptixd::ModelKind::Music && m.id.contains("acestep")).min_by_key(|m| !m.id.contains("turbo")).map(|m| m.id)
        } else {
            None
        }
    });
    let secs = cx.pos("seconds").map(|v| v as u32);
    let seed = cx.pos("seed").map(|v| v as u64);
    let (reply, wav) = {
        let mut prog = |i, n| cx.progress(i, n);
        c.music(&model.unwrap_or_default(), &prompt, &lyrics, secs, seed, &mut prog)
    }
    .map_err(|e| err(cx, e))?;
    let path = cx.out_path("wav");
    std::fs::write(&path, wav).map_err(|e| e.to_string())?;
    cx.label = reply.label;
    cx.out("audio", Val::File(path));
    Ok(())
}

pub fn speak(cx: &mut Cx) -> R {
    let mut params = synaptixd::Params::new();
    if let Some(v) = cx.file("voice") {
        params = params.with("voice", v);
    }
    if let Some(l) = cx.str("language") {
        params = params.with("language", l);
    }
    let text = cx.need("text")?;
    let (mut c, _g) = client(cx)?;
    let sp = c.speak_with(&cx.str("model").unwrap_or_default(), &text, &params).map_err(|e| err(cx, e))?;
    let pcm = sp.audio.f32s();
    let path = cx.out_path("wav");
    std::fs::write(&path, wav_mono(pcm, sp.sample_rate)).map_err(|e| e.to_string())?;
    cx.label = format!("речь {:.1} с", pcm.len() as f32 / sp.sample_rate.max(1) as f32);
    cx.out("audio", Val::File(path));
    Ok(())
}

pub fn transcribe(cx: &mut Cx) -> R {
    let src = cx.need_file("file")?;
    let out = Command::new("ffmpeg").args(["-v", "error", "-i", &src, "-vn", "-ac", "1", "-ar", "16000", "-f", "f32le", "-"]).output().map_err(|e| format!("ffmpeg: {e}"))?;
    let pcm: Vec<f32> = out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    if pcm.is_empty() {
        return Err(format!("в {src} нет звука"));
    }
    let lang = cx.str("language");
    let (mut c, _g) = client(cx)?;
    let t = c.transcribe("", &pcm, 16000, lang.as_deref()).map_err(|e| err(cx, e))?;
    cx.out("text", Val::Text(t.text));
    Ok(())
}

pub fn wav_mono(pcm: &[f32], rate: u32) -> Vec<u8> {
    let data_len = (pcm.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + pcm.len() * 2);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for &x in pcm {
        w.extend_from_slice(&((x.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    w
}
