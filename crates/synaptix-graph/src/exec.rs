//! Исполнение графа: узлы по порядку зависимостей, каждый — вызов операции
//! службы или ffmpeg; файлы — в каталоге прогона.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Map, Value};

use crate::{media_seconds, Graph, Node, Output, Report, Scope};

pub(crate) const OPS: &[&str] = &["text", "llm", "image", "video", "music", "speak", "transcribe", "concat", "mux", "frame", "trim", "still"];

type R<T> = Result<T, String>;

/// Поля узла после подстановок.
struct Args<'a>(&'a Map<String, Value>);

impl Args<'_> {
    fn str(&self, k: &str) -> Option<String> {
        match self.0.get(k)? {
            Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    }
    fn need(&self, k: &str) -> R<String> {
        self.str(k).ok_or_else(|| format!("нужно поле «{k}»"))
    }
    fn num(&self, k: &str) -> Option<f64> {
        match self.0.get(k)? {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => s.trim().parse().ok(),
            _ => None,
        }
    }
    fn bool(&self, k: &str) -> bool {
        match self.0.get(k) {
            Some(Value::Bool(b)) => *b,
            Some(Value::String(s)) => s == "true",
            _ => false,
        }
    }
    fn list(&self, k: &str) -> Vec<String> {
        match self.0.get(k) {
            Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str()).filter(|s| !s.trim().is_empty()).map(String::from).collect(),
            Some(Value::String(s)) if !s.trim().is_empty() => s.split_whitespace().map(String::from).collect(),
            _ => Vec::new(),
        }
    }
}

fn client() -> R<synaptixd::Client> {
    synaptixd::Client::connect().map_err(|e| e.to_string())
}

fn ffmpeg(args: &[&str]) -> R<()> {
    let out = Command::new("ffmpeg").args(["-v", "error", "-y"]).args(args).output().map_err(|e| format!("ffmpeg: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("ffmpeg: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("ошибка")))
    }
}

fn has_audio(p: &Path) -> bool {
    Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "a", "-show_entries", "stream=index", "-of", "csv=p=0"])
        .arg(p)
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false)
}

fn video_size(p: &Path) -> Option<(u32, u32)> {
    let out = Command::new("ffprobe").args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height", "-of", "csv=p=0"]).arg(p).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let mut it = s.trim().split(',');
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

fn is_video(p: &Path) -> bool {
    matches!(p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(), Some("mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v"))
}

/// Модель видео по нужным возможностям (как у инструмента media).
fn video_model(a: &Args, p: &synaptixd::VideoParams) -> String {
    if let Some(m) = a.str("model") {
        return m;
    }
    let restyle = a.bool("restyle") && p.source_video.is_some();
    let ltx_only = !restyle && (p.audio.is_some() || p.source_video.is_some() || p.control_video.is_some() || p.two_stage);
    if ltx_only {
        "ltx-2-3-22b-distilled-1-1".into()
    } else if p.first_frame.is_some() || p.last_frame.is_some() || restyle {
        "minimax-h3-fl2va".into()
    } else {
        String::new()
    }
}

fn op(node: &Node, a: &Args, out_base: &Path) -> R<Output> {
    let file = |ext: &str| out_base.with_extension(ext);
    match node.op.as_str() {
        "text" => Ok(Output { text: a.str("text").unwrap_or_default(), ..Default::default() }),
        "llm" => {
            let mut msgs = Vec::new();
            if let Some(sys) = a.str("system") {
                msgs.push(synaptixd::Message::system(sys));
            }
            msgs.push(synaptixd::Message::user(a.need("prompt")?));
            let mut params = synaptixd::Params::new().with("thinking", false);
            if let Some(n) = a.num("max_tokens") {
                params = params.with("max_tokens", n as i64);
            }
            let g = client()?.chat(&a.str("model").unwrap_or_default(), msgs, Vec::new(), &params, |_| {}).map_err(|e| e.to_string())?;
            Ok(Output { text: g.text.trim().to_string(), label: format!("{} токенов", g.tokens), ..Default::default() })
        }
        "image" => {
            let p = synaptixd::ImageParams {
                negative: a.str("negative"),
                width: a.num("width").map(|v| v as usize),
                height: a.num("height").map(|v| v as usize),
                steps: a.num("steps").map(|v| v as usize),
                guidance: a.num("guidance").map(|v| v as f32),
                seed: a.num("seed").map(|v| v as u64),
                images: a.list("images"),
                init: a.str("init"),
                strength: a.num("strength").map(|v| v as f32),
            };
            let (reply, png) = client()?.image(&a.str("model").unwrap_or_default(), &a.need("prompt")?, &p, |_, _| {}).map_err(|e| e.to_string())?;
            let path = file("png");
            std::fs::write(&path, png).map_err(|e| e.to_string())?;
            Ok(Output { files: vec![path], label: reply.label, ..Default::default() })
        }
        "video" => {
            let p = synaptixd::VideoParams {
                negative: a.str("negative"),
                width: a.num("width").map(|v| v as usize),
                height: a.num("height").map(|v| v as usize),
                seconds: a.num("seconds"),
                steps: a.num("steps").map(|v| v as usize),
                guidance: a.num("guidance").map(|v| v as f32),
                seed: a.num("seed").map(|v| v as u64),
                first_frame: a.str("first_frame"),
                last_frame: a.str("last_frame"),
                refs: a.list("refs"),
                audio: a.str("audio"),
                source_video: a.str("source_video"),
                retake: match (a.num("retake_start"), a.num("retake_end")) {
                    (Some(x), Some(y)) => Some((x, y)),
                    _ => None,
                },
                extend: a.bool("extend"),
                control_video: a.str("control_video"),
                control: a.str("control"),
                two_stage: a.bool("two_stage"),
                strength: a.num("strength").map(|v| v as f32),
            };
            let model = video_model(a, &p);
            let (reply, mp4) = client()?.video(&model, &a.need("prompt")?, &p, |_, _| {}).map_err(|e| e.to_string())?;
            let path = file("mp4");
            std::fs::write(&path, mp4).map_err(|e| e.to_string())?;
            Ok(Output { files: vec![path], label: reply.label, ..Default::default() })
        }
        "music" => {
            let lyrics = a.str("lyrics").unwrap_or_default();
            let mut c = client()?;
            // Инструментал — ACE-Step (YuE2 — песенная модель).
            let model = a.str("model").or_else(|| {
                if lyrics.is_empty() {
                    c.list().ok()?.into_iter().filter(|m| m.kind == synaptixd::ModelKind::Music && m.id.contains("acestep")).min_by_key(|m| !m.id.contains("turbo")).map(|m| m.id)
                } else {
                    None
                }
            });
            let (reply, wav) = c
                .music(&model.unwrap_or_default(), &a.need("prompt")?, &lyrics, a.num("seconds").map(|v| v as u32), a.num("seed").map(|v| v as u64), |_, _| {})
                .map_err(|e| e.to_string())?;
            let path = file("wav");
            std::fs::write(&path, wav).map_err(|e| e.to_string())?;
            Ok(Output { files: vec![path], label: reply.label, ..Default::default() })
        }
        "speak" => {
            let mut params = synaptixd::Params::new();
            if let Some(v) = a.str("voice") {
                params = params.with("voice", v);
            }
            if let Some(l) = a.str("language") {
                params = params.with("language", l);
            }
            let sp = client()?.speak_with(&a.str("model").unwrap_or_default(), &a.need("text")?, &params).map_err(|e| e.to_string())?;
            let pcm = sp.audio.f32s();
            let path = file("wav");
            std::fs::write(&path, wav_mono(pcm, sp.sample_rate)).map_err(|e| e.to_string())?;
            Ok(Output { files: vec![path], label: format!("речь {:.1} с", pcm.len() as f32 / sp.sample_rate.max(1) as f32), ..Default::default() })
        }
        "transcribe" => {
            let src = a.need("file")?;
            let out = Command::new("ffmpeg").args(["-v", "error", "-i", &src, "-vn", "-ac", "1", "-ar", "16000", "-f", "f32le", "-"]).output().map_err(|e| format!("ffmpeg: {e}"))?;
            let pcm: Vec<f32> = out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
            let t = client()?.transcribe("", &pcm, 16000, a.str("language").as_deref()).map_err(|e| e.to_string())?;
            Ok(Output { text: t.text, ..Default::default() })
        }
        "concat" => {
            let files: Vec<PathBuf> = a.list("files").into_iter().map(PathBuf::from).collect();
            if files.is_empty() {
                return Err("concat: пустой список files".into());
            }
            if files.iter().all(|f| is_video(f)) {
                // Ролики приводятся к размеру первого, 24 кадра/с, стерео-звук (тишина, где его нет).
                let (w, h) = video_size(&files[0]).unwrap_or((960, 544));
                let tmp = out_base.with_extension("parts");
                std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
                let mut list = String::new();
                for (i, f) in files.iter().enumerate() {
                    let part = tmp.join(format!("{i:03}.mp4"));
                    let fs = f.display().to_string();
                    let ps = part.display().to_string();
                    let vf = format!("scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,fps=24,setsar=1");
                    if has_audio(f) {
                        ffmpeg(&["-i", &fs, "-vf", &vf, "-c:v", "libx264", "-crf", "18", "-pix_fmt", "yuv420p", "-c:a", "aac", "-ar", "48000", "-ac", "2", &ps])?;
                    } else {
                        ffmpeg(&["-i", &fs, "-f", "lavfi", "-i", "anullsrc=r=48000:cl=stereo", "-shortest", "-vf", &vf, "-c:v", "libx264", "-crf", "18", "-pix_fmt", "yuv420p", "-c:a", "aac", &ps])?;
                    }
                    list.push_str(&format!("file '{}'\n", ps.replace('\'', "'\\''")));
                }
                let lp = tmp.join("list.txt");
                std::fs::write(&lp, list).map_err(|e| e.to_string())?;
                let path = file("mp4");
                ffmpeg(&["-f", "concat", "-safe", "0", "-i", &lp.display().to_string(), "-c", "copy", "-movflags", "+faststart", &path.display().to_string()])?;
                let _ = std::fs::remove_dir_all(&tmp);
                Ok(Output { label: format!("склеено {} роликов", files.len()), files: vec![path], ..Default::default() })
            } else {
                let mut args: Vec<String> = Vec::new();
                for f in &files {
                    args.push("-i".into());
                    args.push(f.display().to_string());
                }
                let inputs: String = (0..files.len()).map(|i| format!("[{i}:a]")).collect();
                args.extend(["-filter_complex".into(), format!("{inputs}concat=n={}:v=0:a=1[a]", files.len()), "-map".into(), "[a]".into()]);
                let path = file("wav");
                args.push(path.display().to_string());
                ffmpeg(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
                Ok(Output { label: format!("склеено {} дорожек", files.len()), files: vec![path], ..Default::default() })
            }
        }
        "mux" => {
            let video = a.need("video")?;
            let audio = a.need("audio")?;
            let path = file("mp4");
            let ps = path.display().to_string();
            let dur = media_seconds(Path::new(&video)).unwrap_or(10.0).to_string();
            if a.str("mode").as_deref() == Some("mix") && has_audio(Path::new(&video)) {
                let vol = a.num("volume").unwrap_or(0.8);
                ffmpeg(&["-i", &video, "-i", &audio, "-filter_complex", &format!("[1:a]volume={vol}[m];[0:a][m]amix=inputs=2:duration=first[a]"), "-map", "0:v", "-map", "[a]", "-c:v", "copy", "-c:a", "aac", "-t", &dur, &ps])?;
            } else {
                ffmpeg(&["-i", &video, "-i", &audio, "-map", "0:v", "-map", "1:a", "-c:v", "copy", "-c:a", "aac", "-t", &dur, &ps])?;
            }
            Ok(Output { files: vec![path], label: "звук наложен".into(), ..Default::default() })
        }
        "frame" => {
            let video = a.need("video")?;
            let path = file("png");
            let ps = path.display().to_string();
            match a.str("at").as_deref() {
                Some("last") | None => ffmpeg(&["-sseof", "-0.2", "-i", &video, "-frames:v", "1", "-update", "1", &ps])?,
                Some("first") => ffmpeg(&["-i", &video, "-frames:v", "1", &ps])?,
                Some(t) => ffmpeg(&["-ss", t, "-i", &video, "-frames:v", "1", &ps])?,
            }
            Ok(Output { files: vec![path], label: "кадр".into(), ..Default::default() })
        }
        "trim" => {
            let src = a.need("file")?;
            let ext = Path::new(&src).extension().and_then(|e| e.to_str()).unwrap_or("mp4").to_string();
            let path = file(&ext);
            let start = a.num("start").unwrap_or(0.0).to_string();
            let mut args = vec!["-ss".to_string(), start, "-i".into(), src];
            if let Some(end) = a.num("end") {
                args.extend(["-t".into(), (end - a.num("start").unwrap_or(0.0)).max(0.1).to_string()]);
            }
            args.push(path.display().to_string());
            ffmpeg(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
            Ok(Output { files: vec![path], label: "обрезано".into(), ..Default::default() })
        }
        "still" => {
            let image = a.need("image")?;
            let path = file("mp4");
            let ps = path.display().to_string();
            match a.str("audio") {
                Some(audio) => ffmpeg(&["-loop", "1", "-i", &image, "-i", &audio, "-c:v", "libx264", "-tune", "stillimage", "-pix_fmt", "yuv420p", "-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2", "-c:a", "aac", "-shortest", &ps])?,
                None => {
                    let secs = a.num("seconds").unwrap_or(5.0).to_string();
                    ffmpeg(&["-loop", "1", "-i", &image, "-t", &secs, "-c:v", "libx264", "-pix_fmt", "yuv420p", "-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2", &ps])?
                }
            }
            Ok(Output { files: vec![path], label: "картинка в ролик".into(), ..Default::default() })
        }
        other => Err(format!("неизвестная операция «{other}»")),
    }
}

fn wav_mono(pcm: &[f32], rate: u32) -> Vec<u8> {
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

/// Выполнить граф с параметрами; файлы — в `dir`. `on_step(текст)` — ход
/// выполнения (узел начат/готов).
pub fn run(graph: &Graph, params: &Map<String, Value>, dir: &Path, on_step: &mut dyn FnMut(&str)) -> Result<Report, String> {
    // Параметры: заданные + умолчания.
    let mut params = params.clone();
    for (k, p) in &graph.params {
        if !params.contains_key(k) {
            if let Some(d) = &p.default {
                params.insert(k.clone(), d.clone());
            }
        }
    }
    let mut graph = graph.clone();
    graph.normalize_output()?;
    let graph = &graph;
    graph.validate(&params)?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let order = graph.order()?;
    let mut outputs: HashMap<String, Output> = HashMap::new();
    let mut steps = Vec::new();
    for &i in &order {
        let node = &graph.nodes[i];
        on_step(&format!("{} ({})", node.id, node.op));
        let items: Option<Vec<Value>> = match &node.each {
            Some(e) => {
                let scope = Scope { params: &params, outputs: &outputs, item: None };
                match scope.resolve(&Value::String(e.clone())) {
                    Value::Array(a) => Some(a),
                    Value::String(s) if !s.trim().is_empty() => Some(s.split_whitespace().map(|x| Value::String(x.into())).collect()),
                    _ => Some(Vec::new()),
                }
            }
            None => None,
        };
        let result = match items {
            None => {
                let scope = Scope { params: &params, outputs: &outputs, item: None };
                let args = scope.resolve(&Value::Object(node.args.clone()));
                op(node, &Args(args.as_object().unwrap()), &dir.join(&node.id))
            }
            Some(items) => {
                let mut acc = Output::default();
                let mut texts = Vec::new();
                for (k, item) in items.iter().enumerate() {
                    on_step(&format!("{} ({}) — {} из {}", node.id, node.op, k + 1, items.len()));
                    let scope = Scope { params: &params, outputs: &outputs, item: Some(item) };
                    let args = scope.resolve(&Value::Object(node.args.clone()));
                    let o = op(node, &Args(args.as_object().unwrap()), &dir.join(format!("{}-{k:03}", node.id))).map_err(|e| format!("{} (элемент {}): {e}", node.id, k + 1))?;
                    acc.files.extend(o.files);
                    if !o.text.is_empty() {
                        texts.push(o.text);
                    }
                    acc.label = o.label;
                }
                acc.text = texts.join("\n\n");
                acc.label = format!("{} × {}", items.len(), acc.label);
                Ok(acc)
            }
        };
        let o = result.map_err(|e| format!("узел «{}» ({}): {e}", node.id, node.op))?;
        let what = if !o.files.is_empty() {
            format!("{} файл(ов){}", o.files.len(), if o.label.is_empty() { String::new() } else { format!(", {}", o.label) })
        } else {
            let t: String = o.text.chars().take(80).collect();
            format!("текст: {t}")
        };
        steps.push(format!("{} ({}): {what}", node.id, node.op));
        outputs.insert(node.id.clone(), o);
    }
    let out_ids: Vec<String> = if graph.output.is_empty() {
        order.last().map(|&i| vec![graph.nodes[i].id.clone()]).unwrap_or_default()
    } else {
        graph.output.clone()
    };
    let mut report = Report { steps, dir: dir.to_path_buf(), ..Default::default() };
    let mut texts = Vec::new();
    for id in out_ids {
        if let Some(o) = outputs.get(&id) {
            report.files.extend(o.files.iter().cloned());
            if o.files.is_empty() && !o.text.is_empty() {
                texts.push(o.text.clone());
            }
        }
    }
    report.text = texts.join("\n\n");
    Ok(report)
}
