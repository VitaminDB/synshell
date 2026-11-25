//! Галерея шаблонов Fusion (C4): поиск по имени и разделам, фильтр по
//! категориям и подразделам, фоновая генерация и дисковый кэш превью,
//! живое анимированное превью и скраббинг при наведении, перенос на таймлайн
//! (drag-and-drop) и наложение двойным щелчком.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::json;
use syngui::containers::Positioned;
use syngui::prelude::*;
use syngui::video::RgbaFrame;
use syngui::widgets::overlay::MenuItem;
use syngui::widgets::*;

use super::{coverage_ui, ed, icons, St};
use crate::fusion::{self, Color4, Image as FusionImage, Overrides};

/// Тип переноса «шаблон Fusion из галереи» (payload — путь к файлу `.setting`).
pub const DRAG_TEMPLATE: &str = "syn-video-editor/template";
/// Тип переноса «переход Fusion из галереи» (payload — путь к файлу `.setting`).
pub const DRAG_TRANSITION: &str = "syn-video-editor/transition";

pub const THUMB_W: u32 = 224;
pub const THUMB_H: u32 = 126;
pub const STRIP_FRAMES: usize = 12;
const PAGE_STEP: usize = 48;

/// Запись каталога шаблонов.
#[derive(Clone, Debug)]
pub struct TemplateEntry {
    pub path: PathBuf,
    pub name: String,
    pub display: String,
    pub top: String,
    pub sub: String,
    pub hay: String,
}

/// Просканировать библиотеку шаблонов один раз за запуск.
pub fn catalog() -> &'static [TemplateEntry] {
    static CAT: OnceLock<Vec<TemplateEntry>> = OnceLock::new();
    CAT.get_or_init(|| {
        let mut v = scan_catalog(&fusion::library());
        v.extend(entries_from(crate::lottie::scan()));
        v
    })
}

pub fn scan_catalog(root: &Path) -> Vec<TemplateEntry> {
    entries_from(fusion::scan(root))
}

fn entries_from(list: Vec<(String, String, PathBuf)>) -> Vec<TemplateEntry> {
    list.into_iter()
        .map(|(category, name, path)| {
            let display = name.replace('_', " ");
            let (top, sub) = match category.split_once('/') {
                Some((a, b)) => (a.to_string(), b.to_string()),
                None => (category.clone(), String::new()),
            };
            let hay = format!("{category} {name} {display}").to_lowercase();
            TemplateEntry { path, name, display, top, sub, hay }
        })
        .collect()
}

/// Отфильтровать каталог по словам поиска, верхнему разделу и подразделу.
pub fn filter_catalog(entries: &[TemplateEntry], query: &str, top: &str, sub: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    let words: Vec<&str> = q.split_whitespace().collect();
    entries
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            if !top.is_empty() && e.top != top {
                return false;
            }
            if !sub.is_empty() && e.sub != sub {
                return false;
            }
            words.iter().all(|w| e.hay.contains(w))
        })
        .map(|(i, _)| i)
        .collect()
}

// ── фоновый рендер превью (1 кадр для плитки + 12 кадров для наведения) ──

#[derive(Clone, Debug, PartialEq, Eq)]
enum Job {
    Hero(PathBuf),
    Strip(PathBuf),
}

struct PreviewPool {
    heroes: HashMap<PathBuf, Option<RgbaFrame>>,
    strips: HashMap<PathBuf, Option<Arc<Vec<RgbaFrame>>>>,
    high: VecDeque<Job>,
    low: VecDeque<Job>,
    in_flight: Vec<Job>,
    last_notify: Instant,
}

fn pool() -> &'static (Mutex<PreviewPool>, Condvar) {
    static P: OnceLock<(Mutex<PreviewPool>, Condvar)> = OnceLock::new();
    P.get_or_init(|| {
        let pair = (
            Mutex::new(PreviewPool {
                heroes: HashMap::new(),
                strips: HashMap::new(),
                high: VecDeque::new(),
                low: VecDeque::new(),
                in_flight: Vec::new(),
                last_notify: Instant::now() - Duration::from_secs(1),
            }),
            Condvar::new(),
        );
        // 2 рабочих потока: хватает для мгновенного отклика при наведении без перегрузки CPU
        for i in 0..2 {
            let _ = std::thread::Builder::new()
                .name(format!("tpl-preview-{i}"))
                .spawn(preview_worker);
        }
        pair
    })
}

fn cache_dir(kind: &str) -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|v| PathBuf::from(v).join(".cache")))
        .unwrap_or_else(std::env::temp_dir)
        .join("syn-video-editor")
        .join(kind)
}

fn cache_key(path: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    "c5v1".hash(&mut h);
    path.hash(&mut h);
    if let Ok(md) = std::fs::metadata(path) {
        md.len().hash(&mut h);
        md.modified().ok().hash(&mut h);
    }
    format!("{:016x}.png", h.finish())
}

fn demo_media_pair(w: u32, h: u32) -> [Arc<FusionImage>; 2] {
    let make = |a: [f32; 3], b: [f32; 3], accent: [f32; 3], shape_a: bool| -> Arc<FusionImage> {
        let mut img = FusionImage::empty(w, h);
        let (wf, hf) = (w.max(1) as f32, h.max(1) as f32);
        for y in 0..h {
            let fy = y as f32 / hf;
            for x in 0..w {
                let fx = x as f32 / wf;
                let t = (fx * 0.65 + fy * 0.35).clamp(0.0, 1.0);
                let mut r = a[0] + (b[0] - a[0]) * t;
                let mut g = a[1] + (b[1] - a[1]) * t;
                let mut bl = a[2] + (b[2] - a[2]) * t;
                // чёткий геометрический ориентир (круг для клипа A, ромб/полосы для клипа B),
                // чтобы зум/поворот/сдвиг/шторки Fusion-переходов мгновенно читались на миниатюре
                let dx = (fx - 0.5) * (wf / hf);
                let dy = fy - 0.5;
                let mark = if shape_a {
                    let d = (dx * dx + dy * dy).sqrt();
                    (d < 0.28 && d > 0.22) || (d < 0.10)
                } else {
                    let m = dx.abs() + dy.abs();
                    (m < 0.36 && m > 0.29) || ((fx * 8.0 + fy * 4.0).floor() as i32 % 2 == 0 && m < 0.14)
                };
                if mark {
                    r = r * 0.25 + accent[0] * 0.75;
                    g = g * 0.25 + accent[1] * 0.75;
                    bl = bl * 0.25 + accent[2] * 0.75;
                } else if x % 28 == 0 || y % 28 == 0 {
                    r = r * 0.86 + accent[0] * 0.14;
                    g = g * 0.86 + accent[1] * 0.14;
                    bl = bl * 0.86 + accent[2] * 0.14;
                }
                let c = Color4::from_straight(r, g, bl, 1.0);
                let idx = ((y * w + x) * 4) as usize;
                img.data[idx] = c.r;
                img.data[idx + 1] = c.g;
                img.data[idx + 2] = c.b;
                img.data[idx + 3] = c.a;
            }
        }
        Arc::new(img)
    };
    [
        // Клип A: глубокий сине-фиолетовый с ярким розовым кольцом
        make([0.14, 0.18, 0.48], [0.58, 0.16, 0.48], [1.0, 0.78, 0.92], true),
        // Клип B: контрастный бирюзово-янтарный с золотым ромбом
        make([0.06, 0.44, 0.48], [0.92, 0.54, 0.14], [1.0, 0.96, 0.72], false),
    ]
}

/// Подложить тёмно-графитовый градиент под прозрачный кадр (чтобы и светлые,
/// и тёмные титры читались одинаково чётко на плитке).
fn composite_over_backdrop(mut frame: RgbaFrame) -> RgbaFrame {
    let w = frame.width;
    let h = frame.height;
    let mut out = (*frame.rgba).clone();
    for y in 0..h {
        let fy = y as f32 / h.max(1) as f32;
        for x in 0..w {
            let fx = x as f32 / w.max(1) as f32;
            let idx = ((y * w + x) * 4) as usize;
            let a = out[idx + 3] as f32 / 255.0;
            // мягкий тёмный студийный градиент с едва заметной сеткой прозрачности
            let checker = if ((x / 10) + (y / 10)) % 2 == 0 { 4.0 } else { 0.0 };
            let bg_r = 22.0 + fx * 10.0 + fy * 8.0 + checker;
            let bg_g = 26.0 + fx * 10.0 + fy * 10.0 + checker;
            let bg_b = 38.0 + fx * 14.0 + fy * 14.0 + checker;
            out[idx] = (out[idx] as f32 * a + bg_r * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
            out[idx + 1] = (out[idx + 1] as f32 * a + bg_g * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
            out[idx + 2] = (out[idx + 2] as f32 * a + bg_b * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
            out[idx + 3] = 255;
        }
    }
    frame.rgba = Arc::new(out);
    frame
}

/// Кадр Lottie для плитки: по центру поля `THUMB_W×THUMB_H`, на подложке.
fn lottie_tile(comp: &syngui::lottie::Composition, t: f64) -> RgbaFrame {
    let f = crate::lottie::render(comp, t, (THUMB_W, THUMB_H));
    let mut canvas = image::RgbaImage::new(THUMB_W, THUMB_H);
    if let Some(img) = image::RgbaImage::from_raw(f.width, f.height, (*f.rgba).clone()) {
        let x = (THUMB_W.saturating_sub(f.width) / 2) as i64;
        let y = (THUMB_H.saturating_sub(f.height) / 2) as i64;
        image::imageops::replace(&mut canvas, &img, x, y);
    }
    // тёмный рисунок (контуры) — на светлой подложке, иначе не виден
    let (mut sum, mut wsum) = (0.0f64, 0.0f64);
    for px in canvas.pixels() {
        let a = px[3] as f64 / 255.0;
        sum += a * (0.2126 * px[0] as f64 + 0.7152 * px[1] as f64 + 0.0722 * px[2] as f64) / 255.0;
        wsum += a;
    }
    if wsum > 0.0 && sum / wsum < 0.3 {
        let mut out = canvas.clone();
        for (x, y, px) in out.enumerate_pixels_mut() {
            let a = px[3] as f32 / 255.0;
            let bg = if ((x / 10) + (y / 10)) % 2 == 0 { 236.0 } else { 226.0 };
            for c in 0..3 {
                px[c] = (px[c] as f32 * a + bg * (1.0 - a)).round() as u8;
            }
            px[3] = 255;
        }
        return RgbaFrame { width: THUMB_W, height: THUMB_H, pts: t, rgba: Arc::new(out.into_raw()) };
    }
    composite_over_backdrop(RgbaFrame { width: THUMB_W, height: THUMB_H, pts: t, rgba: Arc::new(canvas.into_raw()) })
}

fn save_png(file: &Path, f: &RgbaFrame) {
    if let Some(img) = image::RgbaImage::from_raw(f.width, f.height, (*f.rgba).clone()) {
        if let Some(d) = file.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = img.save(file);
    }
}

/// Отрендерить один показательный кадр шаблона (используется и в тестах).
pub fn render_hero(path: &Path) -> Option<RgbaFrame> {
    let file = cache_dir("tpl-thumbs").join(cache_key(path));
    if let Ok(img) = image::open(&file) {
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        if w > 0 && h > 0 {
            return Some(RgbaFrame { width: w, height: h, pts: 0.0, rgba: Arc::new(rgba.into_raw()) });
        }
    }
    if crate::lottie::is_lottie_ext(path) {
        let comp = crate::lottie::load(path).ok()?;
        let frame = lottie_tile(&comp, comp.duration_secs() as f64 * 0.45);
        save_png(&file, &frame);
        return Some(frame);
    }
    let comp = fusion::load_cached(path).ok()?;
    let is_tr = comp.is_transition();
    let needs_media = is_tr || !comp.main_inputs.is_empty() || comp.tools.values().any(|t| t.ty == "MediaIn");
    let media = if needs_media { demo_media_pair(THUMB_W, THUMB_H).to_vec() } else { Vec::new() };
    let (dur, at) = if is_tr { (1.0, 0.5) } else { (4.0, 4.0 * 0.45) };
    let raw = fusion::eval::render(&comp, THUMB_W, THUMB_H, at, dur, 30.0, &media, &Overrides::default());
    let frame = composite_over_backdrop(raw);
    if let Some(img) = image::RgbaImage::from_raw(frame.width, frame.height, (*frame.rgba).clone()) {
        if let Some(d) = file.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = img.save(&file);
    }
    Some(frame)
}

/// Отрендерить анимированную ленту шаблона из `STRIP_FRAMES` кадров.
pub fn render_strip(path: &Path) -> Option<Arc<Vec<RgbaFrame>>> {
    let file = cache_dir("tpl-strips").join(cache_key(path));
    if let Ok(img) = image::open(&file) {
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        let fw = w / STRIP_FRAMES as u32;
        if fw > 0 && h > 0 {
            let frames: Vec<RgbaFrame> = (0..STRIP_FRAMES as u32)
                .map(|i| {
                    let sub = image::imageops::crop_imm(&rgba, i * fw, 0, fw, h).to_image();
                    RgbaFrame { width: fw, height: h, pts: 0.0, rgba: Arc::new(sub.into_raw()) }
                })
                .collect();
            return Some(Arc::new(frames));
        }
    }
    let mut frames = Vec::with_capacity(STRIP_FRAMES);
    if crate::lottie::is_lottie_ext(path) {
        let comp = crate::lottie::load(path).ok()?;
        let d = comp.duration_secs() as f64;
        for i in 0..STRIP_FRAMES {
            frames.push(lottie_tile(&comp, d * i as f64 / STRIP_FRAMES as f64));
        }
    }
    let comp = if frames.is_empty() { Some(fusion::load_cached(path).ok()?) } else { None };
    let is_tr = comp.as_ref().is_some_and(|c| c.is_transition());
    let needs_media = comp.as_ref().is_some_and(|c| is_tr || !c.main_inputs.is_empty() || c.tools.values().any(|t| t.ty == "MediaIn"));
    let media = if needs_media { demo_media_pair(THUMB_W, THUMB_H).to_vec() } else { Vec::new() };
    let dur = if is_tr { 1.0 } else { 4.0 };
    for i in 0..comp.as_ref().map_or(0, |_| STRIP_FRAMES) {
        let comp = comp.as_ref().unwrap();
        let frac = if is_tr {
            i as f64 / (STRIP_FRAMES - 1).max(1) as f64
        } else {
            0.05 + 0.90 * (i as f64 / (STRIP_FRAMES - 1).max(1) as f64)
        };
        let raw = fusion::eval::render(&comp, THUMB_W, THUMB_H, dur * frac, dur, 30.0, &media, &Overrides::default());
        frames.push(composite_over_backdrop(raw));
    }
    let mut atlas = image::RgbaImage::new(THUMB_W * STRIP_FRAMES as u32, THUMB_H);
    for (i, f) in frames.iter().enumerate() {
        if let Some(sub) = image::RgbaImage::from_raw(f.width, f.height, (*f.rgba).clone()) {
            image::imageops::replace(&mut atlas, &sub, (i as u32 * THUMB_W) as i64, 0);
        }
    }
    if let Some(d) = file.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = atlas.save(&file);
    Some(Arc::new(frames))
}

fn preview_worker() {
    loop {
        let job = {
            let (m, cv) = pool();
            let mut p = m.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if let Some(j) = p.high.pop_front().or_else(|| p.low.pop_front()) {
                    p.in_flight.push(j.clone());
                    break j;
                }
                p = cv.wait(p).unwrap_or_else(|e| e.into_inner());
            }
        };
        let is_high = matches!(job, Job::Strip(_));
        match &job {
            Job::Hero(path) => {
                let res = render_hero(path);
                let (m, _) = pool();
                let mut p = m.lock().unwrap_or_else(|e| e.into_inner());
                p.in_flight.retain(|x| x != &job);
                p.heroes.insert(path.clone(), res);
            }
            Job::Strip(path) => {
                let res = render_strip(path);
                let (m, _) = pool();
                let mut p = m.lock().unwrap_or_else(|e| e.into_inner());
                p.in_flight.retain(|x| x != &job);
                if let Some(ref s) = res {
                    if let Some(mid) = s.get(STRIP_FRAMES / 2) {
                        p.heroes.insert(path.clone(), Some(mid.clone()));
                    }
                }
                p.strips.insert(path.clone(), res);
            }
        }
        let notify = {
            let (m, _) = pool();
            let mut p = m.lock().unwrap_or_else(|e| e.into_inner());
            let drained = p.high.is_empty() && p.low.is_empty();
            if is_high || drained || p.last_notify.elapsed() >= Duration::from_millis(80) {
                p.last_notify = Instant::now();
                true
            } else {
                false
            }
        };
        if notify {
            super::thumbs::notify();
        }
    }
}

/// Получить кадр превью плитки (или поставить в фоновую очередь).
fn hero_or_queue(path: &Path) -> Option<RgbaFrame> {
    let (m, cv) = pool();
    let mut p = m.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(Some(strip)) = p.strips.get(path) {
        if let Some(mid) = strip.get(STRIP_FRAMES / 2) {
            return Some(mid.clone());
        }
    }
    if let Some(hit) = p.heroes.get(path) {
        return hit.clone();
    }
    let job = Job::Hero(path.to_path_buf());
    if !p.in_flight.contains(&job) && !p.low.contains(&job) {
        // Ограничиваем фоновую очередь видимой страницей, чтобы старые фильтры не тормозили новые
        while p.low.len() >= 96 {
            p.low.pop_front();
        }
        p.low.push_back(job);
        cv.notify_one();
    }
    None
}

/// Получить анимированную ленту шаблона (или поставить в приоритетную очередь наведения).
fn strip_or_queue(path: &Path) -> Option<Arc<Vec<RgbaFrame>>> {
    let (m, cv) = pool();
    let mut p = m.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(hit) = p.strips.get(path) {
        return hit.clone();
    }
    let job = Job::Strip(path.to_path_buf());
    if !p.in_flight.contains(&job) && !p.high.contains(&job) {
        // Держим не больше 4 приоритетных наведений подряд
        while p.high.len() >= 4 {
            p.high.pop_back();
        }
        p.high.push_front(job);
        cv.notify_all();
    }
    None
}

// ── живое проигрывание при наведении (10 fps только пока курсор на плитке) ─

struct HoverPlayer {
    active_path: Option<PathBuf>,
    last_move: Instant,
    frame: usize,
    tick_sig: Option<RwSignal<usize>>,
}

static HOVER_PLAYER: OnceLock<Mutex<HoverPlayer>> = OnceLock::new();
static TICKER_RUNNING: AtomicBool = AtomicBool::new(false);

fn hover_state() -> &'static Mutex<HoverPlayer> {
    HOVER_PLAYER.get_or_init(|| {
        Mutex::new(HoverPlayer {
            active_path: None,
            last_move: Instant::now(),
            frame: 0,
            tick_sig: None,
        })
    })
}

fn ensure_hover_ticker() {
    if TICKER_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = std::thread::Builder::new().name("tpl-hover-play".into()).spawn(|| loop {
        std::thread::sleep(Duration::from_millis(95));
        let next = {
            let mut st = hover_state().lock().unwrap_or_else(|e| e.into_inner());
            let Some(path) = st.active_path.clone() else {
                TICKER_RUNNING.store(false, Ordering::SeqCst);
                break;
            };
            // После 260 мс неподвижного курсора запускаем плавный цикл превью
            if st.last_move.elapsed() >= Duration::from_millis(260) {
                let ready = {
                    let (m, _) = pool();
                    m.lock().unwrap_or_else(|e| e.into_inner()).strips.get(&path).and_then(|o| o.as_ref()).is_some()
                };
                if ready {
                    st.frame = (st.frame + 1) % STRIP_FRAMES;
                    st.tick_sig.map(|sig| (sig, st.frame))
                } else {
                    None
                }
            } else {
                None
            }
        };
        if let Some((sig, frame)) = next {
            syngui::async_runtime::run_on_main_thread(move || sig.set(frame));
        }
    });
}

/// Длительность шаблона (по `source_frames`, 4 с по умолчанию).
pub fn template_duration(path: &Path, fps: f64) -> f64 {
    if crate::lottie::is_lottie_ext(path) {
        return crate::lottie::load(path).map(|c| (c.duration_secs() as f64).max(0.5)).unwrap_or(3.0);
    }
    let fps = fps.max(1.0);
    fusion::load_cached(path)
        .ok()
        .map(|c| (c.source_frames / fps).clamp(2.0, 10.0))
        .unwrap_or(4.0)
}

/// Открыть галерею шаблонов сразу на разделе «Переходы».
pub fn open_transitions(st: St) {
    st.tpl_cat.set("Переходы".into());
    st.open_panel("templates");
}

/// Применить Fusion-переход к выделенному клипу (или к ближайшей склейке у позиции воспроизведения).
fn apply_transition_from_gallery(st: St, path: &Path) {
    let target = {
        let e = ed();
        let p = &e.project;
        let has_prev = |cid: u64| -> Option<(u64, f64)> {
            let (ti, ci) = p.find_clip(cid)?;
            if p.tracks[ti].kind != crate::doc::TrackKind::Video || ci == 0 {
                return None;
            }
            let c = &p.tracks[ti].clips[ci];
            let prev = &p.tracks[ti].clips[ci - 1];
            ((prev.end() - c.start).abs() < 1e-3).then_some((cid, c.start))
        };
        // 1. Выделенный клип со стыком в начале (или следующий клип встык после выделенного)
        e.selection
            .iter()
            .find_map(|id| has_prev(*id))
            .or_else(|| {
                e.selection.iter().find_map(|id| {
                    let (ti, ci) = p.find_clip(*id)?;
                    if p.tracks[ti].kind != crate::doc::TrackKind::Video {
                        return None;
                    }
                    let c = &p.tracks[ti].clips[ci];
                    let next = p.tracks[ti].clips.get(ci + 1)?;
                    ((c.end() - next.start).abs() < 1e-3).then_some((next.id, next.start))
                })
            })
            // 2. Ближайшая склейка на видеодорожках к позиции воспроизведения
            .or_else(|| {
                let ph = e.playhead;
                let mut best: Option<(f64, u64, f64)> = None;
                for tr in &p.tracks {
                    if tr.kind != crate::doc::TrackKind::Video {
                        continue;
                    }
                    for w in tr.clips.windows(2) {
                        if (w[0].end() - w[1].start).abs() < 1e-3 {
                            let dist = (w[1].start - ph).abs();
                            if best.is_none_or(|(bd, _, _)| dist < bd) {
                                best = Some((dist, w[1].id, w[1].start));
                            }
                        }
                    }
                }
                best.map(|(_, cid, t)| (cid, t))
            })
    };
    let Some((cid, cut_t)) = target else {
        st.toast.set(Some("Нет склейки клипов встык на видеодорожке — поставьте два клипа рядом".into()));
        return;
    };
    if st.exec("transition.set", json!({"id": cid, "kind": "fusion", "template": path.display().to_string()})).is_some() {
        st.exec("selection.set", json!({"ids": [cid]}));
        st.seek_live(cut_t);
        st.dock.activate("inspector");
    }
}

/// Применить шаблон в позиции воспроизведения, выделить клип и открыть инспектор.
fn apply_template_at_playhead(st: St, path: &Path) {
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let dur = template_duration(path, ed().project.settings.fps);
    if let Some(o) = st.exec("template.apply", json!({"name": name, "path": path.display().to_string(), "duration": dur})) {
        if let Some(id) = o.data.get("id").and_then(|v| v.as_u64()) {
            st.exec("selection.set", json!({"ids": [id]}));
            st.dock.activate("inspector");
        }
    }
}

// ── UI панели «Шаблоны» ─────────────────────────────────────────────────

pub fn view(st: St) -> impl Widget {
    let query = use_signal(String::new());
    let top_cat = st.tpl_cat;
    let sub_cat = use_signal(String::new());
    let page_limit = use_signal(PAGE_STEP);
    let search_gen = use_signal(0u64);

    let entries = catalog();
    // Подсчёт верхних категорий в стабильном порядке: Титры, Графика, Переходы, остальные
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for e in entries {
        *counts.entry(e.top.clone()).or_default() += 1;
    }
    let priority = |s: &str| match s {
        "Титры" => 0,
        "Графика" => 1,
        "Переходы" => 2,
        _ => 3,
    };
    let mut top_list: Vec<(String, usize)> = counts.into_iter().collect();
    top_list.sort_by(|a, b| priority(&a.0).cmp(&priority(&b.0)).then_with(|| a.0.cmp(&b.0)));
    // D5: графы сценария (рецепты ассистента) — свой раздел галереи
    top_list.push(("Графы".into(), 0));

    let search_bar = Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .class("panel-head")
        .child(DecoratedBox::new().class("grow").child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let _ = search_gen.get();
            vec![Box::new(
                TextField::new()
                    .text(query.get_untracked())
                    .prefix_icon(icons::SEARCH)
                    .placeholder("Поиск шаблонов…")
                    .on_change(move |s: &str| {
                        query.set(s.to_string());
                        page_limit.set(PAGE_STEP);
                    })
                    .class("tpl-search"),
            )]
        })))
        .child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            if query.get().is_empty() {
                Vec::new()
            } else {
                vec![Box::new(
                    ToolButton::new("\u{E5CD}")
                        .tooltip("Очистить поиск")
                        .on_click(move || {
                            query.set(String::new());
                            page_limit.set(PAGE_STEP);
                            search_gen.update(|g| *g += 1);
                        }),
                )]
            }
        }))
        .child(
            Slider::new()
                .range(100.0, 280.0)
                .value(st.tile_size.get_untracked().clamp(100.0, 280.0))
                .width(74.0)
                .on_change(move |v| st.tile_size.set(v)),
        )
        .child(Button::new("Покрытие").on_click(move || coverage_ui::open(st)).class("head-btn"));

    let cats_bar = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let cur_top = top_cat.get();
        let cur_sub = sub_cat.get();
        let mut chips = Flex::new().wrap().gap(4.0);
        let mk_chip = move |label: String, key: String, active: bool| {
            let cls = if active { "cv-chip cv-chip-on" } else { "cv-chip" };
            GestureDetector::new()
                .on_click(move || {
                    top_cat.set(key.clone());
                    sub_cat.set(String::new());
                    page_limit.set(PAGE_STEP);
                })
                .child(DecoratedBox::new().class(cls).child(Text::new(label).class("cv-chip-text")))
        };
        chips = chips.child(mk_chip("Все".into(), String::new(), cur_top.is_empty()));
        for (name, _cnt) in &top_list {
            if name.is_empty() {
                continue;
            }
            chips = chips.child(mk_chip(name.clone(), name.clone(), cur_top == *name));
        }

        let mut bar = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("tpl-cats").child(chips);
        if !cur_top.is_empty() {
            let mut subs: BTreeMap<String, usize> = BTreeMap::new();
            for e in catalog().iter().filter(|e| e.top == cur_top && !e.sub.is_empty()) {
                *subs.entry(e.sub.clone()).or_default() += 1;
            }
            if subs.len() > 1 {
                let mut items = vec![DropdownItem::new("", format!("Все подразделы ({})", subs.values().sum::<usize>()))];
                for (sname, cnt) in subs {
                    items.push(DropdownItem::new(sname.clone(), format!("{sname} ({cnt})")));
                }
                bar = bar.child(
                    Dropdown::new()
                        .items(items)
                        .selected(cur_sub)
                        .on_change(move |v: &str| {
                            sub_cat.set(v.to_string());
                            page_limit.set(PAGE_STEP);
                        })
                        .class("tpl-sub-select"),
                );
            }
        }
        vec![Box::new(bar)]
    });

    let body = ScrollView::new().vertical().child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let q = query.get();
        let top = top_cat.get();
        let sub = sub_cat.get();
        let limit = page_limit.get();
        let tile_size = st.tile_size.get().clamp(100.0, 280.0);
        if top == "Графы" {
            return vec![graphs_view(st, &q, tile_size)];
        }
        let all = catalog();
        if all.is_empty() {
            return vec![Box::new(
                Column::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .class("media-empty")
                    .child(Icon::new(icons::TITLE).class("media-empty-icon"))
                    .child(Text::new("Библиотека шаблонов не найдена").class("row-title"))
                    .child(Text::new(format!("Ожидаются файлы .setting в\n{}", fusion::library().display())).max_lines(3).class("panel-empty")),
            )];
        }
        let hits = filter_catalog(all, &q, &top, &sub);
        if hits.is_empty() {
            return vec![Box::new(
                Column::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .class("media-empty")
                    .child(Text::new("Ничего не найдено").class("row-title"))
                    .child(Text::new("Попробуйте другое слово или сбросьте раздел").class("panel-empty")),
            )];
        }

        let shown = hits.len().min(limit);
        let mut grid = Grid::new(16).min_column_width(tile_size).gap(8.0);
        for &idx in hits.iter().take(shown) {
            grid = grid.child(tpl_tile(st, &all[idx]));
        }

        let mut col = Column::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("panel-list")
            .child(Text::new(format!("Найдено {} · наведите для живого превью · перетащите на таймлайн или 2×клик", hits.len())).max_lines(2).class("panel-hint"))
            .child(grid);

        if shown < hits.len() {
            let rem = hits.len() - shown;
            let step = rem.min(PAGE_STEP);
            col = col.child(
                Row::new()
                    .main_axis_alignment(MainAxisAlignment::Center)
                    .class("tpl-more")
                    .child(
                        Button::new(format!("Показать ещё {step} (осталось {rem})"))
                            .on_click(move || page_limit.update(|n| *n += PAGE_STEP))
                            .class("head-btn"),
                    ),
            );
        }
        vec![Box::new(col)]
    }));

    Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("grow")
        .child(search_bar)
        .child(cats_bar)
        .child(DecoratedBox::new().class("grow").child(body))
}

fn tpl_tile(st: St, entry: &TemplateEntry) -> Box<dyn Widget> {
    let path = entry.path.clone();
    let path_hover = entry.path.clone();
    let path_dbl = entry.path.clone();
    let path_menu = entry.path.clone();
    let path_str = entry.path.display().to_string();
    let display = entry.display.clone();
    let drag_label = entry.display.clone();
    let sub_label = if entry.sub.is_empty() { entry.top.clone() } else { entry.sub.clone() };
    let name_inspect = entry.name.clone();

    let hover_frac = use_signal(None::<f32>);
    let auto_frame = use_signal(0usize);
    let tile_w = use_signal(160.0f32);
    let is_auto = use_signal(false);

    let is_transition = entry.top == "Переходы";
    let key_base = cache_key(&path);
    let thumb = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.thumbs.get();
        let frac = hover_frac.get();
        let auto_idx = auto_frame.get();
        let auto_on = is_auto.get();
        let mut stack = Stack::new().fit(StackFit::Expand);

        if let Some(f) = frac {
            if let Some(strip) = strip_or_queue(&path) {
                let n = strip.len().max(1);
                let idx = if auto_on { auto_idx.min(n - 1) } else { ((f * n as f32) as usize).min(n - 1) };
                let fr = &strip[idx];
                stack = stack.child(
                    Image::from_rgba_shared(format!("tpls:{key_base}:{idx}"), fr.width, fr.height, fr.rgba.clone())
                        .fit(ImageFit::Cover)
                        .placeholder(false),
                );
                let w = tile_w.get_untracked();
                let x = if auto_on { (idx as f32 + 0.5) / n as f32 * w } else { f * w };
                stack = stack.child(Positioned::new(DecoratedBox::new().class("tile-scrub")).at((x - 1.0).max(0.0), 0.0).dimensions(2.0, 4000.0));
            } else if let Some(fr) = hero_or_queue(&path) {
                stack = stack.child(
                    Image::from_rgba_shared(format!("tplh:{key_base}"), fr.width, fr.height, fr.rgba.clone())
                        .fit(ImageFit::Cover)
                        .placeholder(false),
                );
                stack = stack.child(
                    Column::new()
                        .main_axis_alignment(MainAxisAlignment::End)
                        .cross_axis_alignment(CrossAxisAlignment::End)
                        .child(DecoratedBox::new().class("tile-badge").child(Text::new("…").class("tile-badge-text"))),
                );
            } else {
                stack = stack.child(Center::new().child(Icon::new(icons::TITLE).class("tile-icon")));
            }
        } else if let Some(fr) = hero_or_queue(&path) {
            stack = stack.child(
                Image::from_rgba_shared(format!("tplh:{key_base}"), fr.width, fr.height, fr.rgba.clone())
                    .fit(ImageFit::Cover)
                    .placeholder(false),
            );
        } else {
            stack = stack.child(Center::new().child(Icon::new(icons::TITLE).class("tile-icon")));
        }
        if is_transition {
            stack = stack.child(
                Column::new()
                    .main_axis_alignment(MainAxisAlignment::Start)
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .child(DecoratedBox::new().class("tile-badge").child(Text::new("◇").class("tile-badge-text"))),
            );
        }
        vec![Box::new(stack)]
    });

    let body = Column::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(
            EventHook::new()
                .fill()
                .on_hover_at(move |p| {
                    if let Some((pt, sz)) = p {
                        tile_w.set(sz.width);
                        let frac = (pt.x / sz.width.max(1.0)).clamp(0.0, 0.999);
                        let prev = hover_frac.get_untracked();
                        let moved = prev.is_none_or(|o| (o - frac).abs() > 0.015);
                        hover_frac.set(Some(frac));
                        let _ = strip_or_queue(&path_hover);
                        let mut hs = hover_state().lock().unwrap_or_else(|e| e.into_inner());
                        let switched = hs.active_path.as_deref() != Some(path_hover.as_path());
                        if switched || moved {
                            hs.active_path = Some(path_hover.clone());
                            hs.last_move = Instant::now();
                            hs.frame = ((frac * STRIP_FRAMES as f32) as usize).min(STRIP_FRAMES - 1);
                            hs.tick_sig = Some(auto_frame);
                            is_auto.set(false);
                        } else if hs.last_move.elapsed() >= Duration::from_millis(260) {
                            is_auto.set(true);
                        }
                        drop(hs);
                        ensure_hover_ticker();
                    } else {
                        hover_frac.set(None);
                        is_auto.set(false);
                        let mut hs = hover_state().lock().unwrap_or_else(|e| e.into_inner());
                        if hs.active_path.as_deref() == Some(path_hover.as_path()) {
                            hs.active_path = None;
                            hs.tick_sig = None;
                        }
                    }
                })
                .child(AspectRatio::new(16.0 / 9.0).child(DecoratedBox::new().class("tile-thumb").child(thumb))),
        )
        .child(Text::new(display).max_lines(1).elide(Elide::End).class("row-title"))
        .child(Text::new(sub_label).max_lines(1).elide(Elide::End).class("row-sub"));

    let tile_box = DecoratedBox::new().class("tile").child(body);
    let drag_type = if is_transition { DRAG_TRANSITION } else { DRAG_TEMPLATE };
    let drag = Draggable::new(drag_type, path_str)
        .label(drag_label)
        .on_double_click(move || {
            if is_transition {
                apply_transition_from_gallery(st, &path_dbl);
            } else {
                apply_template_at_playhead(st, &path_dbl);
            }
        })
        .child(tile_box);

    let items = vec![
        MenuItem::new("apply", if is_transition { "Применить к склейке" } else { "Наложить в позиции" }),
        MenuItem::new("inspect", "Состав и параметры…"),
    ];
    Box::new(
        ContextMenu::new()
            .items(items)
            .on_select(move |a: &str| match a {
                "apply" => {
                    if is_transition {
                        apply_transition_from_gallery(st, &path_menu);
                    } else {
                        apply_template_at_playhead(st, &path_menu);
                    }
                }
                "inspect" => {
                    if let Some(o) = st.exec("template.inspect", json!({"name": name_inspect})) {
                        let first = o.text.lines().next().unwrap_or_default().to_string();
                        st.toast.set(Some(first));
                    }
                }
                _ => {}
            })
            .child(drag),
    )
}

// ── D5: шаблоны графов сценария (рецепты) ───────────────────────────────

/// Плитки рецептов: схема узлов, описание; двойной щелчок — открыть графом.
fn graphs_view(st: St, query: &str, tile: f32) -> Box<dyn Widget> {
    let q = query.trim().to_lowercase();
    let all: Vec<synaptix_graph::Graph> = synaptix_graph::recipes::list(&crate::recipe::dir()).into_iter().filter(|g| q.is_empty() || format!("{} {}", g.name, g.description).to_lowercase().contains(&q)).collect();
    let mut grid = Grid::new(16).min_column_width(tile.max(200.0)).gap(8.0);
    for g in &all {
        grid = grid.child(graph_tile(st, g.clone()));
    }
    Box::new(
        Column::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("panel-list")
            .child(Text::new(format!("Графов сценария: {} · 2×клик — открыть в «Графе», правый щелчок — прогнать в проект", all.len())).max_lines(2).class("panel-hint"))
            .child(grid),
    )
}

/// Схема рецепта: цепочка узлов «id·op» стрелками.
fn graph_chain(g: &synaptix_graph::Graph) -> String {
    g.nodes.iter().map(|n| format!("{}{}", n.op, if n.each.is_some() { "×" } else { "" })).collect::<Vec<_>>().join(" → ")
}

/// Открыть рецепт графом в панели «Граф» (общий редактор узлов, ноды ядра).
pub fn open_graph(st: St, g: &synaptix_graph::Graph) {
    let Some(ws) = super::graph::workspace() else {
        st.toast.set(Some("Редактор узлов не подключён".into()));
        return;
    };
    let params: serde_json::Map<String, serde_json::Value> = g.params.iter().filter_map(|(k, p)| p.default.clone().map(|d| (k.clone(), d))).collect();
    let t = synaptix_graph::synthos::to_template(g, &params).and_then(|v| serde_json::from_value::<syn_node_editor::model::Template>(v).map_err(|e| e.to_string()));
    match t {
        Ok(t) => {
            super::framegraph_ui::current().set(None);
            ws.open_template(&t);
            st.open_panel("graph");
            st.dock.activate("graph");
        }
        Err(e) => st.toast.set(Some(format!("{}: {e}", g.name))),
    }
}

fn graph_tile(st: St, g: synaptix_graph::Graph) -> Box<dyn Widget> {
    let chain = graph_chain(&g);
    let params = synaptix_graph::recipes::params_line(&g);
    let (g_dbl, g_menu) = (g.clone(), g.clone());
    let body = Column::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(AspectRatio::new(16.0 / 9.0).child(DecoratedBox::new().class("tile-thumb graph-thumb").child(Center::new().child(Text::new(chain).max_lines(4).class("graph-chain")))))
        .child(Text::new(g.name.clone()).max_lines(1).elide(Elide::End).class("row-title"))
        .child(Text::new(g.description.clone()).max_lines(2).elide(Elide::End).class("row-sub"))
        .child(Text::new(params).max_lines(1).elide(Elide::End).class("row-sub"));
    let menu = vec![MenuItem::new("open", "Открыть как граф"), MenuItem::new("run", "Прогнать в проект…")];
    Box::new(
        ContextMenu::new()
            .items(menu)
            .on_select(move |id: &str| match id {
                "open" => open_graph(st, &g_menu),
                "run" => {
                    st.open_panel("ai");
                    st.ask_ai(format!("Прогони рецепт «{}» в редактируемый проект (recipe.run). Если не хватает обязательных параметров ({}) — сначала спроси меня.", g_menu.name, synaptix_graph::recipes::params_line(&g_menu)));
                }
                _ => {}
            })
            .child(GestureDetector::new().on_double_click(move || open_graph(st, &g_dbl)).child(DecoratedBox::new().class("tile").child(body))),
    )
}
