//! Экспорт графа в шаблон редактора узлов Synthos (уровень Б): граф
//! раскрывается в цепочки нод Synthos (LLM, FLUX.2, LTX-2.3, ACE-Step,
//! OmniVoice), значения параметров и тексты — в ноды «Текст», файлы — в
//! «Картинка»/«Аудиофайл». Шаблон кладётся в `~/.config/synthos/templates/`
//! (Synthos читает его при открытии окна шаблонов, раздел «Свои»), затем
//! Synthos запускается. Сам движок и генерация Synthos не меняются.
//! Утилиты ffmpeg (склейка, наложение звука…) в Synthos нод не имеют — для
//! них пишется нода-заметка с описанием шага.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::Graph;

/// Файлы моделей в библиотеке службы (по имени файла).
struct Models {
    roots: Vec<PathBuf>,
}

impl Models {
    fn load() -> Self {
        let mut roots = Vec::new();
        if let Ok(text) = std::fs::read_to_string("/var/lib/synaptix/state.toml") {
            for l in text.lines() {
                if let Some(p) = l.trim().strip_prefix("path = ") {
                    roots.push(PathBuf::from(p.trim().trim_matches('"')));
                }
            }
        }
        Self { roots }
    }

    /// Первый `.syn` с началом имени из списка (по порядку предпочтения).
    fn find(&self, prefixes: &[&str]) -> String {
        for pre in prefixes {
            for root in &self.roots {
                let mut stack = vec![root.clone()];
                while let Some(dir) = stack.pop() {
                    for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                        let p = e.path();
                        if p.is_dir() {
                            stack.push(p);
                            continue;
                        }
                        let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
                        if name.starts_with(pre) && name.ends_with(".syn") {
                            return p.display().to_string();
                        }
                    }
                }
            }
        }
        String::new()
    }
}

/// Сборщик шаблона: ноды, связи, раскладка по столбцам.
struct Builder {
    nodes: Vec<Value>,
    conns: Vec<Value>,
    next: u64,
}

impl Builder {
    fn node(&mut self, kind: &str, x: f32, y: f32, state: Option<(&str, Value)>) -> u64 {
        let id = self.next;
        self.next += 1;
        let state = state.map(|(k, d)| json!({"kind": k, "data": d})).unwrap_or(Value::Null);
        self.nodes.push(json!({"id": id, "kind": kind, "pos": {"x": x, "y": y}, "fields": {}, "style": {"tint": null, "shadow": true}, "enabled": true, "state": state}));
        id
    }
    fn link(&mut self, from: u64, fp: &str, to: u64, tp: &str) {
        self.conns.push(json!({"from_node": from, "from_port": fp, "to_node": to, "to_port": tp}));
    }
    fn text(&mut self, x: f32, y: f32, text: &str) -> u64 {
        self.node("text_view", x, y, Some(("TextView", json!({"output_text": text}))))
    }
}

/// Что граф-узел отдаёт наружу в Synthos: нода и порт (текст, картинка, звук, видео-кадры).
#[derive(Clone, Copy, Default)]
struct Ports {
    text: Option<(u64, &'static str)>,
    image: Option<(u64, &'static str)>,
    audio: Option<(u64, &'static str)>,
    frames: Option<(u64, &'static str)>,
}

/// Значение поля после подстановки параметров (ссылки на узлы — пустые).
fn field(n: &crate::Node, key: &str, params: &Map<String, Value>) -> String {
    let Some(v) = n.args.get(key) else { return String::new() };
    let outputs = HashMap::new();
    let scope = crate::Scope { params, outputs: &outputs, item: None };
    match scope.resolve(v) {
        Value::String(s) => s,
        Value::Number(x) => x.to_string(),
        other => other.to_string(),
    }
}

/// Узел-источник единственной ссылки поля (`{{id.text}}` → id).
fn source(n: &crate::Node, key: &str) -> Option<String> {
    let s = n.args.get(key)?.as_str()?;
    let a = s.find("{{")?;
    let b = s[a..].find("}}")?;
    let r = s[a + 2..a + b].trim();
    r.split_once('.').map(|(id, _)| id.to_string())
}

/// Граф → JSON шаблона Synthos.
pub fn to_template(g: &Graph, params: &Map<String, Value>) -> Result<Value, String> {
    let models = Models::load();
    let mut b = Builder { nodes: Vec::new(), conns: Vec::new(), next: 1 };
    let mut ports: HashMap<String, Ports> = HashMap::new();
    let order = g.order()?;
    // Параметры-файлы — ноды загрузки слева.
    let mut y_param = 40.0f32;
    let mut param_ports: HashMap<String, Ports> = HashMap::new();
    for (name, p) in &g.params {
        let v = params.get(name).cloned().or(p.default.clone()).unwrap_or(Value::Null);
        let s = match &v {
            Value::String(s) => s.clone(),
            Value::Array(a) => a.first().and_then(|x| x.as_str()).unwrap_or("").to_string(),
            Value::Number(n) => n.to_string(),
            _ => String::new(),
        };
        let lower = s.to_lowercase();
        let mut pp = Ports::default();
        if p.kind == "file" || p.kind == "files" {
            if lower.ends_with(".wav") || lower.ends_with(".mp3") || lower.ends_with(".flac") || lower.ends_with(".ogg") {
                let id = b.node("audio_file", 40.0, y_param, Some(("AudioFile", json!({"loaded_path": s}))));
                pp.audio = Some((id, "out"));
            } else if lower.ends_with(".mp4") || lower.ends_with(".mkv") || lower.ends_with(".webm") || lower.ends_with(".mov") {
                let id = b.node("ffmpeg_player", 40.0, y_param, Some(("FfmpegPlayer", json!({"source": s}))));
                pp.frames = Some((id, "video"));
                pp.audio = Some((id, "audio"));
            } else {
                let id = b.node("image_load", 40.0, y_param, Some(("ImageLoad", json!({"image_path": s}))));
                pp.image = Some((id, "image"));
            }
            y_param += 220.0;
            param_ports.insert(name.clone(), pp);
        }
    }
    let mut notes = Vec::new();
    for (col, &i) in order.iter().enumerate() {
        let n = &g.nodes[i];
        let x = 360.0 + col as f32 * 760.0;
        let mut y = 40.0f32;
        let mut row = |h: f32| {
            let v = y;
            y += h;
            v
        };
        // Вход-текст поля: из узла-источника или новой ноды «Текст» с подставленным значением.
        let text_in = |b: &mut Builder, key: &str, y: f32| -> (u64, &'static str) {
            if let Some(src) = source(n, key) {
                if let Some(t) = ports.get(&src).and_then(|p| p.text) {
                    return t;
                }
            }
            let id = b.text(x, y, &field(n, key, params));
            (id, "out")
        };
        let file_in = |key: &str, want_audio: bool| -> Option<(u64, &'static str)> {
            let s = n.args.get(key)?.as_str()?;
            let a = s.find("{{")?;
            let r = s[a + 2..a + s[a..].find("}}")?].trim().to_string();
            let p = match r.split_once('.') {
                Some((id, _)) => ports.get(id).copied(),
                None => param_ports.get(&r).copied(),
            }?;
            if want_audio {
                p.audio
            } else {
                p.image.or(p.frames)
            }
        };
        let mut out = Ports::default();
        match n.op.as_str() {
            "text" => {
                let id = b.text(x, row(200.0), &field(n, "text", params));
                out.text = Some((id, "out"));
            }
            "llm" => {
                let cp = b.node("syn_checkpoint", x, row(220.0), Some(("SynCheckpoint", json!({"model_path": models.find(&["gemma-4-26b", "gemma-4", "qwen3"])}))));
                let prompt = text_in(&mut b, "prompt", row(200.0));
                let llm = b.node("llm", x + 360.0, 40.0, Some(("Llm", json!({"system_prompt": field(n, "system", params), "max_tokens": 2048}))));
                b.link(cp, "model", llm, "model");
                b.link(prompt.0, prompt.1, llm, "prompt");
                let view = b.node("text_view", x + 360.0, 300.0, None);
                b.link(llm, "answer", view, "in");
                out.text = Some((view, "out"));
            }
            "image" => {
                let cp = b.node("flux2_checkpoint", x, row(220.0), Some(("Flux2Checkpoint", json!({"model_path": models.find(&["flux.2-klein-4b", "flux.2-klein", "flux.2"])}))));
                let prompt = text_in(&mut b, "prompt", row(200.0));
                let w = field(n, "width", params).parse::<u32>().unwrap_or(1024);
                let h = field(n, "height", params).parse::<u32>().unwrap_or(1024);
                let enc = b.node("flux2_text_encoder", x + 360.0, 40.0, None);
                let lat = b.node("flux_empty_latent", x + 360.0, 220.0, Some(("FluxEmptyLatent", json!({"width": w, "height": h}))));
                let smp = b.node("flux2_sampler", x + 360.0, 420.0, Some(("Flux2Sampler", json!({"seed": -1}))));
                let dec = b.node("flux2_vae_decode", x + 360.0, 640.0, None);
                b.link(cp, "model", enc, "model");
                b.link(cp, "model", smp, "model");
                b.link(cp, "model", dec, "model");
                b.link(prompt.0, prompt.1, enc, "prompt");
                b.link(enc, "conditioning", smp, "conditioning");
                b.link(lat, "latent", smp, "latent");
                b.link(smp, "latent", dec, "latent");
                if let Some(r) = file_in("images", false) {
                    b.link(r.0, r.1, smp, "references");
                }
                let save = b.node("image_save", x, row(200.0), None);
                b.link(dec, "image", save, "image");
                out.image = Some((dec, "image"));
            }
            "video" => {
                let cp = b.node(
                    "ltx_checkpoint",
                    x,
                    row(240.0),
                    Some((
                        "LtxCheckpoint",
                        json!({"model_path": models.find(&["ltx-2.3-22b-distilled", "ltx-2.3"]), "gemma_dir": models.find(&["ltx-gemma"]), "upscaler_path": models.find(&["ltx-2.3-spatial-upscaler"])}),
                    )),
                );
                let prompt = text_in(&mut b, "prompt", row(200.0));
                let enc = b.node("ltx_text_encoder", x + 360.0, 40.0, None);
                let secs = field(n, "seconds", params).parse::<f32>().unwrap_or(4.0);
                let w = field(n, "width", params).parse::<u32>().unwrap_or(960);
                let h = field(n, "height", params).parse::<u32>().unwrap_or(544);
                let s1 = b.node("ltx_sampler_stage1", x + 360.0, 260.0, Some(("LtxSamplerStage1", json!({"width": w, "height": h, "duration_seconds": secs, "seed": -1}))));
                let vd = b.node("ltx_vae_decode", x + 360.0, 520.0, None);
                let ad = b.node("ltx_audio_decode", x + 360.0, 700.0, None);
                let save = b.node("ltx_video_save", x, row(200.0), None);
                b.link(cp, "model", enc, "model");
                b.link(cp, "model", s1, "model");
                b.link(cp, "model", vd, "model");
                b.link(cp, "model", ad, "model");
                b.link(prompt.0, prompt.1, enc, "prompt");
                b.link(enc, "video_encoding", s1, "video_encoding");
                b.link(enc, "audio_encoding", s1, "audio_encoding");
                if let Some(img) = file_in("first_frame", false) {
                    b.link(img.0, img.1, s1, "image_cond");
                }
                b.link(s1, "video_latent", vd, "video_latent");
                b.link(s1, "audio_tokens", ad, "audio_tokens");
                b.link(vd, "frames", save, "frames");
                b.link(ad, "audio", save, "audio");
                if n.args.contains_key("audio") || n.args.contains_key("control_video") || n.args.contains_key("source_video") || n.args.contains_key("refs") {
                    notes.push(format!("{}: в Synthos собран обычный текст/кадр→видео; режим узла (звук, образец, референсы) настройте вручную", n.id));
                }
                out.frames = Some((vd, "frames"));
                out.audio = Some((ad, "audio"));
            }
            "music" => {
                let cp = b.node("ace_step_checkpoint", x, row(220.0), Some(("AceStepCheckpoint", json!({"models_dir": models.find(&["acestep_v15_xl_turbo"]).rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default()}))));
                let tags = text_in(&mut b, "prompt", row(200.0));
                let lyrics = text_in(&mut b, "lyrics", row(200.0));
                let secs = field(n, "seconds", params).parse::<f32>().unwrap_or(30.0);
                let gen = b.node("ace_step_generate", x + 360.0, 40.0, Some(("AceStepGenerate", json!({"duration_seconds": secs}))));
                b.link(cp, "model", gen, "model");
                b.link(tags.0, tags.1, gen, "tags");
                b.link(lyrics.0, lyrics.1, gen, "lyrics");
                let player = b.node("audio_player", x + 360.0, 300.0, None);
                b.link(gen, "audio", player, "in");
                out.audio = Some((gen, "audio"));
            }
            "speak" => {
                let cp = b.node("syn_checkpoint", x, row(220.0), Some(("SynCheckpoint", json!({"model_path": models.find(&["omnivoice"])}))));
                let text = text_in(&mut b, "text", row(200.0));
                let tts = b.node("omni_voice", x + 360.0, 40.0, Some(("OmniVoice", json!({"instruct": field(n, "voice", params)}))));
                b.link(cp, "model", tts, "model");
                b.link(text.0, text.1, tts, "text");
                let player = b.node("audio_player", x + 360.0, 300.0, None);
                b.link(tts, "audio", player, "in");
                out.audio = Some((tts, "audio"));
            }
            other => {
                // Утилиты без нод в Synthos — заметкой.
                let desc = match other {
                    "concat" => "склейка роликов/дорожек по порядку",
                    "mux" => "наложить звук на видео",
                    "frame" => "кадр из видео",
                    "trim" => "обрезка",
                    "still" => "картинка в ролик",
                    "transcribe" => "распознавание речи",
                    _ => "шаг",
                };
                let id = b.text(x, row(200.0), &format!("Шаг «{}» ({other}): {desc}. В редакторе узлов Synthos такой ноды нет — выполняет ассистент (рецепт в synshell).", n.id));
                out.text = Some((id, "out"));
                notes.push(format!("{}: «{other}» — без ноды в Synthos", n.id));
            }
        }
        if n.each.is_some() {
            notes.push(format!("{}: в графе выполняется для каждого элемента списка, в Synthos — один раз", n.id));
        }
        ports.insert(n.id.clone(), out);
    }
    let mut description = g.description.clone();
    if !notes.is_empty() {
        description.push_str("\n\nОграничения экспорта: ");
        description.push_str(&notes.join("; "));
    }
    let name = if g.name.is_empty() { "Сценарий ассистента".to_string() } else { format!("Рецепт: {}", g.name) };
    Ok(json!({"name": name, "description": description, "kind": "full", "nodes": b.nodes, "connections": b.conns, "viewport": null}))
}

fn templates_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/synthos/templates")
}

/// Записать шаблон, вернуть путь файла.
pub fn export(g: &Graph, params: &Map<String, Value>) -> Result<PathBuf, String> {
    let t = to_template(g, params)?;
    let dir = templates_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let slug: String = format!("synshell-{}", if g.name.is_empty() { "graph" } else { &g.name }).chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' }).collect();
    let path = dir.join(format!("{slug}.json"));
    std::fs::write(&path, serde_json::to_string_pretty(&t).map_err(|e| e.to_string())?).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// Экспорт и запуск Synthos (если ещё не открыт).
pub fn export_and_open(g: &Graph, params: &Map<String, Value>) -> Result<String, String> {
    let path = export(g, params)?;
    let running = std::process::Command::new("pgrep").args(["-x", "synthos"]).output().map(|o| o.status.success()).unwrap_or(false);
    if !running {
        std::process::Command::new("synthos").spawn().map_err(|e| format!("Synthos не запущен: {e}"))?;
    }
    let name = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(format!("Шаблон «{name}» сохранён в Synthos: редактор узлов → шаблоны → «Свои»"))
}

/// Путь файла шаблона по имени графа (для проверки).
pub fn template_path(name: &str) -> PathBuf {
    let slug: String = format!("synshell-{name}").chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' }).collect();
    templates_dir().join(format!("{slug}.json"))
}

#[allow(dead_code)]
fn _unused(_: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_recipes_export() {
        for g in crate::recipes::builtin() {
            let t = to_template(&g, &Map::new()).unwrap_or_else(|e| panic!("{}: {e}", g.name));
            let nodes = t["nodes"].as_array().unwrap();
            assert!(!nodes.is_empty(), "{}", g.name);
            let ids: Vec<u64> = nodes.iter().map(|n| n["id"].as_u64().unwrap()).collect();
            for c in t["connections"].as_array().unwrap() {
                assert!(ids.contains(&c["from_node"].as_u64().unwrap()) && ids.contains(&c["to_node"].as_u64().unwrap()), "{}: связь на несуществующую ноду", g.name);
            }
        }
    }
}
