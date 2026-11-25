//! Рецепты (D4): прогон рецепта synaptix-graph возвращает не только
//! склейку, а редактируемый проект — ролики сцен клипами по порядку на V1
//! (имена — строки сценария), музыка и речь на звуковых дорожках, если
//! ролики сами без звука. Раскладку задаёт поле рецепта `timeline`, без него
//! она угадывается по графу: вход склейки `concat` или узлы `video`.

use std::path::{Path, PathBuf};

use serde_json::json;
use synaptix_graph::{Graph, Report};

use crate::editor::Editor;

/// Ссылка `{{узел.поле}}` в значении поля узла → id узла.
fn ref_node(v: Option<&serde_json::Value>) -> Option<String> {
    let s = v?.as_str()?;
    let a = s.find("{{")? + 2;
    let b = s[a..].find("}}")? + a;
    Some(s[a..b].split('.').next()?.trim().to_string()).filter(|x| !x.is_empty())
}

/// Раскладка: (узел роликов, узлы звука, узел имён).
pub fn plan(g: &Graph) -> (Option<String>, Vec<String>, Option<String>) {
    if let Some(t) = &g.timeline {
        return ((!t.video.is_empty()).then(|| t.video.clone()), t.audio.clone(), (!t.names.is_empty()).then(|| t.names.clone()));
    }
    let node = |id: &str| g.nodes.iter().find(|n| n.id == id);
    // ролики: что подано на склейку результата, иначе узлы video результата, иначе любые video/image
    let outputs: Vec<&str> = if g.output.is_empty() { g.nodes.last().map(|n| n.id.as_str()).into_iter().collect() } else { g.output.iter().map(String::as_str).collect() };
    let mut video = None;
    for o in &outputs {
        if let Some(n) = node(o) {
            match n.op.as_str() {
                "concat" => video = ref_node(n.args.get("files")).or(video),
                "video" | "image" | "still" | "mux" => video = video.or(Some(n.id.clone())),
                _ => {}
            }
        }
    }
    let video = video.or_else(|| g.nodes.iter().rev().find(|n| n.op == "video").map(|n| n.id.clone())).or_else(|| g.nodes.iter().rev().find(|n| matches!(n.op.as_str(), "image" | "still")).map(|n| n.id.clone()));
    // имена: список строк, по которому шли ролики (`each: {{plan.lines}}`)
    let names = video.as_deref().and_then(node).and_then(|n| n.each.as_ref()).and_then(|e| ref_node(Some(&serde_json::Value::String(e.clone()))));
    let names = names.filter(|id| node(id).is_some_and(|n| n.op == "llm" || n.op == "text"));
    let audio: Vec<String> = g.nodes.iter().filter(|n| matches!(n.op.as_str(), "music" | "speak")).map(|n| n.id.clone()).collect();
    (video, audio, names)
}

/// Проект из прогона: команды редактора над пустым проектом (как руками).
pub fn build(g: &Graph, r: &Report, name: &str) -> Result<Editor, String> {
    let (vnode, anodes, names) = plan(g);
    let vfiles: Vec<PathBuf> = vnode.as_ref().and_then(|v| r.nodes.get(v)).map(|o| o.files.clone()).filter(|f| !f.is_empty()).unwrap_or_else(|| r.files.clone());
    if vfiles.is_empty() {
        return Err("в прогоне нет роликов и картинок для таймлайна".into());
    }
    let lines: Vec<String> = names.as_ref().and_then(|n| r.nodes.get(n)).map(|o| o.text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect()).unwrap_or_default();
    let mut ed = Editor::new();
    ed.project.name = name.to_string();
    let paths: Vec<String> = vfiles.iter().map(|p| p.display().to_string()).collect();
    let imp = crate::cmd::run(&mut ed, "media.import", &json!({"paths": paths}))?;
    let ids: Vec<u64> = imp.data["ids"].as_array().cloned().unwrap_or_default().iter().filter_map(|v| v.as_u64()).collect();
    // кадр проекта — как у первого ролика
    if let Some(m) = ids.first().and_then(|i| ed.project.media(*i)) {
        if m.width > 0 && m.height > 0 {
            let (w, h, fps) = (m.width & !1, m.height & !1, if m.fps > 0.0 { m.fps.round() } else { 30.0 });
            crate::cmd::run(&mut ed, "project.settings", &json!({"width": w, "height": h, "fps": fps}))?;
        }
    }
    let mut any_audio = false;
    for (i, id) in ids.iter().enumerate() {
        let o = crate::cmd::run(&mut ed, "clip.add", &json!({"media": id}))?;
        let _ = o;
        any_audio |= ed.project.media(*id).is_some_and(|m| m.has_audio);
        if let Some(line) = lines.get(i) {
            let cid = ed.project.tracks.iter().find(|t| t.kind == crate::doc::TrackKind::Video).and_then(|t| t.clips.last()).map(|c| c.id);
            if let Some(cid) = cid {
                let short: String = line.chars().take(80).collect();
                crate::cmd::run(&mut ed, "clip.set", &json!({"id": cid, "name": short}))?;
            }
        }
    }
    // звук отдельно — только если ролики без своего (иначе он уже в них)
    if !any_audio {
        for a in &anodes {
            for f in r.nodes.get(a).map(|o| o.files.clone()).unwrap_or_default() {
                let imp = crate::cmd::run(&mut ed, "media.import", &json!({"paths": [f.display().to_string()]}))?;
                for id in imp.data["ids"].as_array().cloned().unwrap_or_default().iter().filter_map(|v| v.as_u64()) {
                    if ed.project.media(id).is_some_and(|m| m.kind == crate::doc::MediaKind::Audio) {
                        crate::cmd::run(&mut ed, "clip.add", &json!({"media": id, "at": 0}))?;
                    }
                }
            }
        }
    }
    Ok(ed)
}

/// Каталог рецептов (как у ассистента).
pub fn dir() -> PathBuf {
    syn_assist::pipeline::resolve(&synshell_common::Config::load().0.assistant.recipes_dir)
}

/// Куда сохранить проект прогона.
pub fn project_path(run_dir: &Path, name: &str) -> PathBuf {
    let base = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| run_dir.to_path_buf()).join("Videos").join("syn-video-editor");
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    base.join(format!("{}-{stamp}.syn", synaptix_graph::recipes::slug(name)))
}
