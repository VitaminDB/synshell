//! Где лежат снимки и видео: каталоги XDG пользователя — «Изображения» (`XDG_PICTURES_DIR`) и «Видео»
//! (`XDG_VIDEOS_DIR`), имена `IMG_ГГГГММДД_ЧЧММСС.jpg` / `VID_….mp4` (как у Android). Миниатюры
//! (последний кадр превью в момент съёмки) — `~/.cache/syncamera/<имя>.png`.
//!
//! [`Library`] — то же для любой программы: свой каталог, префикс имён и кэш миниатюр (записи сессий
//! synspeak — `Library::new("synspeak", videos_dir().join("synspeak"), "SESSION")`). Свободные функции —
//! библиотека «Камеры» ([`Library::camera`]).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Photo,
    Video,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub path: PathBuf,
    pub kind: Kind,
    pub mtime: SystemTime,
    pub size: u64,
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// Каталог XDG из `~/.config/user-dirs.dirs` (`XDG_PICTURES_DIR="$HOME/Изображения"`).
fn user_dir(key: &str, fallback: &str) -> PathBuf {
    if let Some(v) = std::env::var_os(key) {
        return PathBuf::from(v);
    }
    let cfg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".config"));
    if let Ok(s) = std::fs::read_to_string(cfg.join("user-dirs.dirs")) {
        for line in s.lines() {
            let line = line.trim();
            if let Some(v) = line.strip_prefix(key).and_then(|r| r.strip_prefix('=')) {
                let v = v.trim().trim_matches('"');
                let p = if let Some(rest) = v.strip_prefix("$HOME") { home().join(rest.trim_start_matches('/')) } else { PathBuf::from(v) };
                return p;
            }
        }
    }
    home().join(fallback)
}

/// Папки, выбранные в настройках (`None` — XDG пользователя).
static DIRS: std::sync::RwLock<(Option<PathBuf>, Option<PathBuf>)> = std::sync::RwLock::new((None, None));

pub fn set_dirs(photo: Option<PathBuf>, video: Option<PathBuf>) {
    *DIRS.write().unwrap() = (photo, video);
}

/// «Изображения» пользователя по XDG (папка по умолчанию).
pub fn default_pictures_dir() -> PathBuf {
    user_dir("XDG_PICTURES_DIR", "Изображения")
}

pub fn default_videos_dir() -> PathBuf {
    user_dir("XDG_VIDEOS_DIR", "Видео")
}

pub fn pictures_dir() -> PathBuf {
    DIRS.read().unwrap().0.clone().unwrap_or_else(default_pictures_dir)
}

pub fn videos_dir() -> PathBuf {
    DIRS.read().unwrap().1.clone().unwrap_or_else(default_videos_dir)
}

/// Можно ли писать в папку (создаётся, если её нет).
pub fn writable(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(".syncamera-probe");
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

fn cache_dir(app: &str) -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".cache")).join(app)
}

/// Где программа хранит снимки и видео.
#[derive(Clone, Debug, PartialEq)]
pub struct Library {
    pub pictures: PathBuf,
    pub videos: PathBuf,
    /// Префиксы имён: `IMG` → `IMG_ГГГГММДД_ЧЧММСС.jpg`.
    pub photo_prefix: String,
    pub video_prefix: String,
    /// Кэш миниатюр.
    pub cache: PathBuf,
}

/// Местное время «ГГГГММДД_ЧЧММСС».
fn stamp() -> String {
    // SAFETY: localtime_r в живую структуру
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        format!("{:04}{:02}{:02}_{:02}{:02}{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec)
    }
}

impl Library {
    /// «Камера»: «Изображения» и «Видео» (или папки из её настроек), `IMG_`/`VID_`.
    pub fn camera() -> Library {
        Library { pictures: pictures_dir(), videos: videos_dir(), photo_prefix: "IMG".into(), video_prefix: "VID".into(), cache: cache_dir("syncamera") }
    }

    /// Программа `app`: видео и снимки в `dir`, имена `<prefix>_ГГГГММДД_ЧЧММСС`, миниатюры в `~/.cache/<app>`.
    pub fn new(app: &str, dir: impl Into<PathBuf>, prefix: &str) -> Library {
        let dir = dir.into();
        Library { pictures: dir.clone(), videos: dir, photo_prefix: prefix.into(), video_prefix: prefix.into(), cache: cache_dir(app) }
    }

    /// Сколько байт занимают файлы библиотеки.
    pub fn used_bytes(&self) -> u64 {
        self.list().iter().map(|i| i.size).sum()
    }

    /// Новый свободный путь: `IMG_…jpg` в «Изображениях» или `VID_…mp4` в «Видео».
    pub fn new_path(&self, kind: Kind) -> PathBuf {
        let (dir, pre, ext) = match kind {
            Kind::Photo => (self.pictures.clone(), self.photo_prefix.as_str(), "jpg"),
            Kind::Video => (self.videos.clone(), self.video_prefix.as_str(), "mp4"),
        };
        let _ = std::fs::create_dir_all(&dir);
        let base = format!("{pre}_{}", stamp());
        let mut p = dir.join(format!("{base}.{ext}"));
        let mut n = 1;
        while p.exists() {
            p = dir.join(format!("{base}_{n}.{ext}"));
            n += 1;
        }
        p
    }

    fn is_ours(&self, name: &str, kind: Kind) -> bool {
        match kind {
            Kind::Photo => name.starts_with(&format!("{}_", self.photo_prefix)) && (name.ends_with(".jpg") || name.ends_with(".jpeg")),
            Kind::Video => name.starts_with(&format!("{}_", self.video_prefix)) && name.ends_with(".mp4"),
        }
    }

    /// Снимки и видео камеры, новые первыми.
    pub fn list(&self) -> Vec<Item> {
        let mut v = Vec::new();
        for (dir, kind) in [(self.pictures.clone(), Kind::Photo), (self.videos.clone(), Kind::Video)] {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if !self.is_ours(&name, kind) {
                    continue;
                }
                let Ok(md) = e.metadata() else { continue };
                if md.len() == 0 {
                    continue;
                }
                v.push(Item { path: e.path(), kind, mtime: md.modified().unwrap_or(SystemTime::UNIX_EPOCH), size: md.len() });
            }
        }
        v.sort_by(|a, b| b.mtime.cmp(&a.mtime).then(b.path.cmp(&a.path)));
        v
    }

    pub fn thumb_path(&self, p: &Path) -> PathBuf {
        self.cache.join(format!("{}.png", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()))
    }

    /// Сохранить миниатюру файла.
    pub fn save_thumb(&self, p: &Path, w: u32, h: u32, rgba: &[u8]) {
        let _ = std::fs::create_dir_all(&self.cache);
        if let Some(img) = image::RgbaImage::from_raw(w, h, rgba.to_vec()) {
            let _ = img.save(self.thumb_path(p));
        }
    }

    /// Миниатюра: из кэша, иначе (снимок) — уменьшенный JPEG с поворотом EXIF (и в кэш).
    pub fn thumb(&self, p: &Path, kind: Kind, max: u32) -> Option<(u32, u32, Vec<u8>)> {
        let tp = self.thumb_path(p);
        if let Ok(img) = image::open(&tp) {
            let img = img.to_rgba8();
            return Some((img.width(), img.height(), img.into_raw()));
        }
        if kind != Kind::Photo {
            return None;
        }
        let data = std::fs::read(p).ok()?;
        let img = image::load_from_memory_with_format(&data, image::ImageFormat::Jpeg).ok()?;
        let img = img.thumbnail(max, max);
        let img = match exif_orientation(&data) {
            3 => img.rotate180(),
            6 => img.rotate90(),
            8 => img.rotate270(),
            _ => img,
        };
        let img = img.to_rgba8();
        let (w, h) = (img.width(), img.height());
        let raw = img.into_raw();
        self.save_thumb(p, w, h, &raw);
        Some((w, h, raw))
    }

    pub fn delete(&self, p: &Path) -> std::io::Result<()> {
        let _ = std::fs::remove_file(self.thumb_path(p));
        std::fs::remove_file(p)
    }
}

/// Новый свободный путь «Камеры».
pub fn new_path(kind: Kind) -> PathBuf {
    Library::camera().new_path(kind)
}

/// Снимки и видео «Камеры», новые первыми.
pub fn list() -> Vec<Item> {
    Library::camera().list()
}

pub fn save_thumb(p: &Path, w: u32, h: u32, rgba: &[u8]) {
    Library::camera().save_thumb(p, w, h, rgba)
}

pub fn thumb(p: &Path, kind: Kind, max: u32) -> Option<(u32, u32, Vec<u8>)> {
    Library::camera().thumb(p, kind, max)
}

pub fn delete(p: &Path) -> std::io::Result<()> {
    Library::camera().delete(p)
}

/// Уменьшить RGBA до `max` по длинной стороне (ближайший пиксель).
pub fn shrink(w: u32, h: u32, rgba: &[u8], max: u32) -> (u32, u32, Vec<u8>) {
    let k = (w.max(h) as f32 / max as f32).max(1.0);
    let (tw, th) = (((w as f32 / k) as u32).max(1), ((h as f32 / k) as u32).max(1));
    let mut out = vec![0u8; (tw * th * 4) as usize];
    for y in 0..th {
        let sy = ((y as f32 * k) as u32).min(h - 1);
        for x in 0..tw {
            let sx = ((x as f32 * k) as u32).min(w - 1);
            let s = ((sy * w + sx) * 4) as usize;
            let d = ((y * tw + x) * 4) as usize;
            out[d..d + 4].copy_from_slice(&rgba[s..s + 4]);
        }
    }
    (tw, th, out)
}

/// Ориентация EXIF (тег 0x0112) из APP1 JPEG; 1 — нет поворота.
pub fn exif_orientation(d: &[u8]) -> u16 {
    let mut i = 2;
    while i + 4 < d.len() && d[i] == 0xFF {
        let marker = d[i + 1];
        let len = u16::from_be_bytes([d[i + 2], d[i + 3]]) as usize;
        if marker == 0xE1 && d.len() >= i + 4 + len && d[i + 4..].starts_with(b"Exif\0\0") {
            let t = &d[i + 10..i + 2 + len];
            if t.len() < 8 {
                return 1;
            }
            let le = &t[..2] == b"II";
            let r16 = |o: usize| if o + 2 > t.len() { 0 } else if le { u16::from_le_bytes([t[o], t[o + 1]]) } else { u16::from_be_bytes([t[o], t[o + 1]]) };
            let r32 = |o: usize| {
                if o + 4 > t.len() {
                    0
                } else if le {
                    u32::from_le_bytes([t[o], t[o + 1], t[o + 2], t[o + 3]])
                } else {
                    u32::from_be_bytes([t[o], t[o + 1], t[o + 2], t[o + 3]])
                }
            };
            let ifd = r32(4) as usize;
            let n = r16(ifd) as usize;
            for k in 0..n {
                let e = ifd + 2 + k * 12;
                if r16(e) == 0x0112 {
                    return r16(e + 8);
                }
            }
            return 1;
        }
        if marker == 0xDA {
            break;
        }
        i += 2 + len;
    }
    1
}

