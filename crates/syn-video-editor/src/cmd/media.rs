use std::path::{Path, PathBuf};

use serde_json::json;

use super::{req, Args, Command, Out, Res, Ty};
use crate::doc::{expand, Media, MediaKind};
use crate::editor::Editor;

pub static COMMANDS: &[&Command] = &[&IMPORT, &LIST, &REMOVE];

pub const IMAGE: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "gif", "tif", "tiff", "avif", "heic"];
pub const AUDIO: &[&str] = &["mp3", "wav", "flac", "ogg", "opus", "m4a", "aac", "wma", "aiff", "aif"];
pub const VIDEO: &[&str] = &["mp4", "mkv", "mov", "webm", "avi", "m4v", "mts", "m2ts", "ts", "mpg", "mpeg", "wmv", "flv", "3gp", "mxf"];

static IMPORT: Command = Command {
    id: "media.import",
    category: "Медиа",
    title: "Импорт медиа…",
    shortcut: "Ctrl+I",
    keywords: "import добавить файлы видео звук картинки медиатека",
    help: "Добавить файлы в медиатеку (видео, звук, картинки); папка — все медиафайлы в ней. Возвращает id (m…).",
    args: &[req("paths", Ty::Paths, "пути к файлам или папкам")],
    danger: false,
    run: import,
};

fn import(ed: &mut Editor, a: &Args) -> Res {
    let mut files = Vec::new();
    for p in a.strs("paths")?.unwrap_or_default() {
        let p = expand(&p);
        if p.is_dir() {
            let mut inner: Vec<PathBuf> = std::fs::read_dir(&p).map_err(|e| format!("{}: {e}", p.display()))?.flatten().map(|e| e.path()).filter(|f| kind_by_ext(f).is_some()).collect();
            inner.sort();
            files.extend(inner);
        } else {
            files.push(p);
        }
    }
    if files.is_empty() {
        return Err("нет медиафайлов".into());
    }
    let mut added = Vec::new();
    let mut errors = Vec::new();
    let mut probed = Vec::new();
    for f in files {
        let f = std::fs::canonicalize(&f).unwrap_or(f);
        if let Some(m) = ed.project.media.iter().find(|m| m.path == f) {
            added.push((m.id, m.name(), true));
            continue;
        }
        match probe(&f) {
            Ok(m) => probed.push(m),
            Err(e) => errors.push(e),
        }
    }
    if !probed.is_empty() {
        ed.begin("Импорт медиа");
        for mut m in probed {
            m.id = ed.project.alloc();
            added.push((m.id, m.name(), false));
            ed.project.media.push(m);
        }
    }
    let mut text: Vec<String> = added.iter().map(|(id, n, old)| format!("m{id} {n}{}", if *old { " (уже был)" } else { "" })).collect();
    text.extend(errors.iter().map(|e| format!("ошибка: {e}")));
    if added.is_empty() {
        return Err(text.join("\n"));
    }
    Ok(Out::with(text.join("\n"), json!({"ids": added.iter().map(|a| a.0).collect::<Vec<_>>()})))
}

fn kind_by_ext(p: &Path) -> Option<MediaKind> {
    let e = p.extension()?.to_str()?.to_ascii_lowercase();
    if IMAGE.contains(&e.as_str()) {
        Some(MediaKind::Image)
    } else if AUDIO.contains(&e.as_str()) {
        Some(MediaKind::Audio)
    } else if VIDEO.contains(&e.as_str()) {
        Some(MediaKind::Video)
    } else {
        None
    }
}

/// Сведения о файле: видео — через `syngui::video::probe`, звук — `probe_audio`.
pub fn probe(path: &Path) -> Result<Media, String> {
    if !path.is_file() {
        return Err(format!("{}: файл не найден", path.display()));
    }
    let s = path.to_string_lossy();
    let mut m = Media { id: 0, path: path.to_path_buf(), kind: MediaKind::Video, duration: 0.0, width: 0, height: 0, fps: 0.0, has_audio: false };
    match kind_by_ext(path) {
        Some(MediaKind::Image) => {
            m.kind = MediaKind::Image;
            if let Ok(img) = image::image_dimensions(path) {
                (m.width, m.height) = img;
            }
            return Ok(m);
        }
        Some(MediaKind::Audio) => {
            let meta = syngui::video::probe_audio(&s).map_err(|e| e.to_string())?;
            m.kind = MediaKind::Audio;
            m.duration = meta.duration_sec;
            m.has_audio = true;
            return Ok(m);
        }
        _ => {}
    }
    match syngui::video::probe(&s) {
        Ok(v) => {
            let (w, h) = if v.rotation % 180 == 90 { (v.height, v.width) } else { (v.width, v.height) };
            m.width = w;
            m.height = h;
            m.duration = v.duration_sec;
            m.fps = (v.fps_estimate as f64 * 1000.0).round() / 1000.0;
            m.has_audio = v.has_audio;
            Ok(m)
        }
        // без видеопотока — может быть звук в контейнере
        Err(_) => {
            let meta = syngui::video::probe_audio(&s).map_err(|e| format!("{}: не медиафайл ({e})", path.display()))?;
            m.kind = MediaKind::Audio;
            m.duration = meta.duration_sec;
            m.has_audio = true;
            Ok(m)
        }
    }
}

static LIST: Command = Command {
    id: "media.list",
    category: "Медиа",
    title: "Медиатека",
    shortcut: "",
    keywords: "media list файлы медиатека",
    help: "Список файлов медиатеки: id, вид, длительность, размер, частота, звук.",
    args: &[],
    danger: false,
    run: list,
};

fn list(ed: &mut Editor, _a: &Args) -> Res {
    if ed.project.media.is_empty() {
        return Ok(Out::text("Медиатека пуста"));
    }
    let lines: Vec<String> = ed
        .project
        .media
        .iter()
        .map(|m| {
            let used = ed.project.tracks.iter().flat_map(|t| &t.clips).filter(|c| c.media() == Some(m.id)).count();
            format!("m{} {} {} {:.3}s {}x{} {}fps звук:{} клипов:{} — {}", m.id, m.kind.as_str(), m.name(), m.duration, m.width, m.height, m.fps, if m.has_audio { "да" } else { "нет" }, used, m.path.display())
        })
        .collect();
    Ok(Out::with(lines.join("\n"), serde_json::to_value(&ed.project.media).unwrap_or_default()))
}

static REMOVE: Command = Command {
    id: "media.remove",
    category: "Медиа",
    title: "Убрать из медиатеки",
    shortcut: "",
    keywords: "remove удалить убрать медиа",
    help: "Убрать файл из медиатеки (сам файл на диске не трогается) вместе с его клипами на таймлайне.",
    args: &[req("id", Ty::Id, "id медиа (m…)")],
    danger: false,
    run: remove,
};

fn remove(ed: &mut Editor, a: &Args) -> Res {
    let id = a.need_id("id")?;
    let name = ed.project.media(id).ok_or_else(|| format!("нет медиа m{id}"))?.name();
    ed.begin("Убрать медиа");
    ed.project.media.retain(|m| m.id != id);
    let mut n = 0;
    for t in &mut ed.project.tracks {
        let before = t.clips.len();
        t.clips.retain(|c| c.media() != Some(id));
        n += before - t.clips.len();
    }
    Ok(Out::text(format!("Убрано m{id} {name} и клипов: {n}")))
}
