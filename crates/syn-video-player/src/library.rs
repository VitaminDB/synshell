//! Библиотека: видео из «Видео» (XDG_VIDEOS_DIR) и «Загрузок» (два уровня вглубь), новые сверху.
//! Миниатюры — `syngui::video::thumbnail`, кэш PNG в `~/.cache/syn-video-player/` (ключ — путь,
//! время изменения и размер файла).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use synshell_common::paths;

pub const EXTENSIONS: &[&str] =
    &["mp4", "m4v", "mkv", "mk3d", "webm", "mov", "qt", "avi", "divx", "ts", "m2ts", "mts", "m2v", "3gp", "3g2", "mpg", "mpeg",
      "mpe", "vob", "wmv", "asf", "flv", "f4v", "ogv", "ogm", "dv", "rm", "rmvb", "mxf", "nut", "y4m", "mjpeg", "mj2", "ivf",
      "264", "h264", "265", "h265", "hevc", "bik", "nsv"];

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub path: PathBuf,
    pub mtime: SystemTime,
    pub size: u64,
}

impl Item {
    pub fn name(&self) -> String {
        self.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    }

    fn key(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.path.hash(&mut h);
        self.mtime.hash(&mut h);
        self.size.hash(&mut h);
        h.finish()
    }
}

pub fn is_video(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Каталоги, где ищем видео.
pub fn roots() -> Vec<PathBuf> {
    let mut v = vec![paths::user_dir_or_default("VIDEOS"), paths::user_dir_or_default("DOWNLOAD")];
    v.dedup();
    v
}

pub fn scan() -> Vec<Item> {
    let mut out = Vec::new();
    for r in roots() {
        walk(&r, 2, &mut out);
    }
    out.sort_by(|a, b| b.mtime.cmp(&a.mtime));
    out
}

fn walk(dir: &Path, depth: u32, out: &mut Vec<Item>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.')) {
            continue;
        }
        let Ok(md) = e.metadata() else { continue };
        if md.is_dir() {
            if depth > 0 {
                walk(&p, depth - 1, out);
            }
        } else if is_video(&p) && md.len() > 0 {
            out.push(Item { path: p, mtime: md.modified().unwrap_or(SystemTime::UNIX_EPOCH), size: md.len() });
        }
    }
}

fn cache_dir() -> PathBuf {
    paths::cache_home().join("syn-video-player")
}

/// Миниатюра: (ширина, высота, RGBA, длительность с).
pub type Thumb = (u32, u32, Vec<u8>, f64);

/// Из кэша или заново (медленно — звать не из главного потока).
pub fn thumb(item: &Item, max: u32) -> Option<Thumb> {
    let file = cache_dir().join(format!("{:016x}.png", item.key()));
    if let Ok(img) = image::open(&file) {
        let img = img.to_rgba8();
        let dur = std::fs::read_to_string(file.with_extension("dur")).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0.0);
        return Some((img.width(), img.height(), img.into_raw(), dur));
    }
    let t = match syngui::video::thumbnail(&item.path.to_string_lossy(), max, 0.1) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("миниатюра {}: {e}", item.path.display());
            return None;
        }
    };
    let _ = std::fs::create_dir_all(cache_dir());
    if let Some(img) = image::RgbaImage::from_raw(t.width, t.height, t.rgba.clone()) {
        let _ = img.save(&file);
        let _ = std::fs::write(file.with_extension("dur"), format!("{}", t.duration_sec));
    }
    Some((t.width, t.height, t.rgba, t.duration_sec))
}

/// «1:05», «1:02:03».
pub fn format_time(sec: f64) -> String {
    // меньше секунды — «0:01», а не «0:00»
    let s = if sec > 0.0 && sec < 1.0 { 1 } else { sec.max(0.0) as u64 };
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}
