//! Анимации Lottie (C6): `.json` (bodymovin) и `.lottie` (dotLottie) как
//! медиа. Разбор и растеризация — `syngui::lottie` (без сторонних
//! библиотек); здесь — кэш разобранных анимаций, сведения для медиатеки,
//! кадр на момент времени (по кругу) и библиотека
//! `~/Storage/Видеомонтаж/Анимации Lottie`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use syngui::lottie::{Composition, Renderer};
use syngui::video::RgbaFrame;

/// Расширения: у `.json` проверяется, что это Lottie.
pub const EXT: &[&str] = &["lottie", "json"];

/// Похоже ли на Lottie: `.lottie` — да; `.json` — есть `layers` и частота `fr`.
pub fn is_lottie(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("lottie") => true,
        Some("json") => {
            // читаем начало: у bodymovin ключи `v`, `fr`, `ip`, `op`, `layers`
            let Ok(text) = std::fs::read(path) else { return false };
            let head = String::from_utf8_lossy(&text[..text.len().min(1 << 16)]);
            head.contains("\"fr\"") && head.contains("\"layers\"") && head.contains("\"op\"")
        }
        _ => false,
    }
}

type Cache = Mutex<HashMap<PathBuf, (Option<std::time::SystemTime>, Arc<Composition>)>>;

/// Разобранная анимация (кэш по пути и времени изменения файла).
pub fn load(path: &Path) -> Result<Arc<Composition>, String> {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    if let Some((t, c)) = cache.lock().unwrap().get(path) {
        if *t == mtime {
            return Ok(c.clone());
        }
    }
    let c = Arc::new(syngui::lottie::load_file(path)?);
    cache.lock().unwrap().insert(path.to_path_buf(), (mtime, c.clone()));
    Ok(c)
}

/// Сведения для медиатеки: ширина, высота, частота, длительность (с).
pub fn probe(path: &Path) -> Result<(u32, u32, f64, f64), String> {
    let c = load(path)?;
    if c.frame_rate <= 0.0 || c.frame_count() <= 0.0 {
        return Err(format!("{}: пустая анимация Lottie", path.display()));
    }
    Ok((c.width.round().max(1.0) as u32, c.height.round().max(1.0) as u32, c.frame_rate as f64, c.duration_secs() as f64))
}

/// Номер кадра анимации на момент `t` с её начала (по кругу).
pub fn frame_at(c: &Composition, t: f64) -> f32 {
    let n = c.frame_count().max(1e-3) as f64;
    let f = (t * c.frame_rate as f64).rem_euclid(n);
    c.in_point + f as f32
}

/// Размер кадра: пропорции анимации, вписано в `max` (не больше 2× исходного).
pub fn fit(c: &Composition, max: (u32, u32)) -> (u32, u32) {
    let (cw, ch) = (c.width.max(1.0), c.height.max(1.0));
    let k = (max.0 as f32 / cw).min(max.1 as f32 / ch).min(2.0);
    (((cw * k).round() as u32).max(1), ((ch * k).round() as u32).max(1))
}

/// Кадр в момент `t` (с начала анимации, по кругу), вписанный в `max`.
/// Прямая альфа, как у картинок.
pub fn render(c: &Composition, t: f64, max: (u32, u32)) -> RgbaFrame {
    thread_local! {
        static R: std::cell::RefCell<Renderer> = std::cell::RefCell::new(Renderer::new());
    }
    let (w, h) = fit(c, max);
    let rgba = R.with(|r| r.borrow_mut().render_rgba8(c, frame_at(c, t), w, h));
    RgbaFrame { width: w, height: h, pts: t, rgba: Arc::new(rgba) }
}

/// Папка библиотеки анимаций: `SYN_VIDEO_EDITOR_LOTTIE` или
/// `~/Storage/Видеомонтаж/Анимации Lottie`.
pub fn library_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("SYN_VIDEO_EDITOR_LOTTIE") {
        return PathBuf::from(p);
    }
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join("Storage/Видеомонтаж/Анимации Lottie")
}

/// Анимации библиотеки (рекурсивно, по имени).
pub fn library() -> Vec<PathBuf> {
    fn walk(d: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if is_lottie(&p) {
                out.push(p);
            }
        }
    }
    let mut v = Vec::new();
    walk(&library_dir(), &mut v);
    v.sort();
    v
}

/// Каталог для галереи и поиска: (раздел «Lottie[/подпапка]», имя, путь).
pub fn scan() -> Vec<(String, String, PathBuf)> {
    let root = library_dir();
    library()
        .into_iter()
        .map(|p| {
            let sub = p.parent().and_then(|d| d.strip_prefix(&root).ok()).map(|d| d.display().to_string()).unwrap_or_default();
            let cat = if sub.is_empty() { "Lottie".to_string() } else { format!("Lottie/{sub}") };
            let name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            (cat, name, p)
        })
        .collect()
}

/// Анимация ли это из каталога (по расширению, без чтения файла).
pub fn is_lottie_ext(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| EXT.contains(&e.to_ascii_lowercase().as_str()))
}
