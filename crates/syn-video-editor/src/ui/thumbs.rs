//! Ленты превью медиатеки: по файлу — `FRAMES` кадров равномерно по длине,
//! готовятся в фоне по очереди (один поток). Плитка показывает кадр по
//! положению курсора — промотка без декодирования на лету. Готовые ленты
//! хранятся на диске (`~/.cache/syn-video-editor/thumbs`) и не считаются заново.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use syngui::prelude::*;
use syngui::video::RgbaFrame;

pub const FRAMES: usize = 16;
const SIDE: u32 = 240;

pub type Strip = Arc<Vec<RgbaFrame>>;

struct State {
    ready: HashMap<PathBuf, Strip>,
    /// Файлы, для которых лента не построилась (аудио, битый/неподдерживаемый
    /// формат): не ставим повторно в очередь на каждом rebuild UI, иначе
    /// `worker` и главный поток замыкаются в бесконечную петлю. Сбрасывается в `forget`.
    failed: HashSet<PathBuf>,
    queued: VecDeque<PathBuf>,
    busy: Option<PathBuf>,
}

fn state() -> &'static (Mutex<State>, Condvar) {
    static S: OnceLock<(Mutex<State>, Condvar)> = OnceLock::new();
    S.get_or_init(|| {
        std::thread::Builder::new().name("thumbs".into()).spawn(worker).ok();
        (Mutex::new(State { ready: HashMap::new(), failed: HashSet::new(), queued: VecDeque::new(), busy: None }), Condvar::new())
    })
}

/// Сигнал «готова новая лента» — панели перерисовываются по нему.
static REV: OnceLock<RwSignal<u64>> = OnceLock::new();

pub fn bind(rev: RwSignal<u64>) {
    let _ = REV.set(rev);
}

fn is_audio_path(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    crate::cmd::media::AUDIO.contains(&ext.as_str())
}

/// Лента файла или `None` (поставлена в очередь).
pub fn strip(path: &Path) -> Option<Strip> {
    if is_audio_path(path) {
        return None;
    }
    let (m, cv) = state();
    let mut s = m.lock().unwrap();
    if let Some(st) = s.ready.get(path) {
        return Some(st.clone());
    }
    if s.failed.contains(path) {
        return None;
    }
    if s.busy.as_deref() != Some(path) && !s.queued.iter().any(|q| q == path) {
        s.queued.push_back(path.to_path_buf());
        cv.notify_one();
    }
    None
}

/// Забыть готовую ленту файла (файл правили — построить заново).
pub fn forget(path: &Path) {
    let (m, cv) = state();
    let mut s = m.lock().unwrap();
    s.ready.remove(path);
    s.failed.remove(path);
    if is_audio_path(path) {
        return;
    }
    if s.busy.as_deref() != Some(path) && !s.queued.iter().any(|q| q == path) {
        s.queued.push_back(path.to_path_buf());
        cv.notify_one();
    }
}

fn cache_file(path: &Path) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    if let Ok(md) = std::fs::metadata(path) {
        md.len().hash(&mut h);
        md.modified().ok().hash(&mut h);
    }
    let dir = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|v| PathBuf::from(v).join(".cache"))).unwrap_or_else(std::env::temp_dir).join("syn-video-editor/thumbs");
    dir.join(format!("{:016x}.png", h.finish()))
}

/// Лента: кадры склеены по горизонтали в один PNG.
fn load_cached(file: &Path) -> Option<Vec<RgbaFrame>> {
    let img = image::open(file).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    let fw = w / FRAMES as u32;
    if fw == 0 {
        return None;
    }
    Some((0..FRAMES as u32).map(|i| {
        let sub = image::imageops::crop_imm(&img, i * fw, 0, fw, h).to_image();
        RgbaFrame { width: fw, height: h, pts: 0.0, rgba: Arc::new(sub.into_raw()) }
    }).collect())
}

fn save_cached(file: &Path, frames: &[RgbaFrame]) {
    let (fw, fh) = (frames[0].width, frames[0].height);
    if frames.iter().any(|f| f.width != fw || f.height != fh) {
        return;
    }
    let mut out = image::RgbaImage::new(fw * frames.len() as u32, fh);
    for (i, f) in frames.iter().enumerate() {
        if let Some(sub) = image::RgbaImage::from_raw(fw, fh, (*f.rgba).clone()) {
            image::imageops::replace(&mut out, &sub, (i as u32 * fw) as i64, 0);
        }
    }
    if let Some(d) = file.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = out.save(file);
}

fn build(path: &Path) -> Option<Vec<RgbaFrame>> {
    let file = cache_file(path);
    if let Some(v) = load_cached(&file) {
        return Some(v);
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    if crate::cmd::media::AUDIO.contains(&ext.as_str()) {
        return None;
    }
    if crate::lottie::EXT.contains(&ext.as_str()) {
        let comp = crate::lottie::load(path).ok()?;
        let dur = comp.duration_secs() as f64;
        let v: Vec<RgbaFrame> = (0..FRAMES).map(|i| crate::lottie::render(&comp, dur * (i as f64 + 0.5) / FRAMES as f64, (SIDE, SIDE))).collect();
        save_cached(&file, &v);
        return Some(v);
    }
    if crate::cmd::media::IMAGE.contains(&ext.as_str()) {
        let img = image::DynamicImage::ImageRgba8(crate::cmd::media::open_image(path, SIDE * 2)?).thumbnail(SIDE, SIDE).to_rgba8();
        let (w, h) = img.dimensions();
        let f = RgbaFrame { width: w, height: h, pts: 0.0, rgba: Arc::new(img.into_raw()) };
        return Some(vec![f; FRAMES]);
    }
    let mut r = syngui::video::FrameReader::open(&path.to_string_lossy(), SIDE, SIDE).ok()?;
    let dur = r.duration.max(0.1);
    let mut v = Vec::with_capacity(FRAMES);
    for i in 0..FRAMES {
        let t = dur * (i as f64 + 0.5) / FRAMES as f64;
        match r.frame_at(t) {
            Ok(f) => v.push(f),
            Err(_) => break,
        }
    }
    if v.is_empty() {
        return None;
    }
    while v.len() < FRAMES {
        v.push(v.last().unwrap().clone());
    }
    save_cached(&file, &v);
    Some(v)
}

fn worker() {
    loop {
        let path = {
            let (m, cv) = state();
            let mut s = m.lock().unwrap();
            loop {
                if let Some(p) = s.queued.pop_front() {
                    s.busy = Some(p.clone());
                    break p;
                }
                s = cv.wait(s).unwrap();
            }
        };
        let strip = build(&path).unwrap_or_default();
        let built = !strip.is_empty();
        {
            let (m, _) = state();
            let mut s = m.lock().unwrap();
            s.busy = None;
            if built {
                s.failed.remove(&path);
                s.ready.insert(path, Arc::new(strip));
            } else {
                s.failed.insert(path);
            }
        }
        if built {
            if let Some(rev) = REV.get().copied() {
                syngui::async_runtime::run_on_main_thread_wake_only(move || rev.update(|r| *r += 1));
            }
        }
    }
}

/// Лента из готового PNG (ленты эффектов): кэш в памяти по пути.
pub fn strip_file(path: &Path) -> Option<Strip> {
    static FILES: OnceLock<Mutex<HashMap<PathBuf, Strip>>> = OnceLock::new();
    let m = FILES.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(s) = m.lock().unwrap().get(path) {
        return Some(s.clone());
    }
    let v = Arc::new(load_cached(path)?);
    m.lock().unwrap().insert(path.to_path_buf(), v.clone());
    Some(v)
}

/// Отрисовать недостающие ленты эффектов (один раз, отдельным процессом);
/// по готовности — сигнал `REV`.
pub fn ensure_fx_strips() {
    static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    if crate::render::preview::kinds().iter().all(|(k, _)| crate::render::preview::strip_path(k).exists()) {
        return;
    }
    std::thread::spawn(|| {
        {
            let exe = synshell_common::paths::self_exe("syn-video-editor");
            let _ = std::process::Command::new(exe).arg("--preview-strips").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
        }
        if let Some(rev) = REV.get().copied() {
            syngui::async_runtime::run_on_main_thread(move || rev.update(|r| *r += 1));
        }
    });
}

/// Сообщить окну: готовы новые превью (ленты, волны).
pub fn notify() {
    if let Some(rev) = REV.get().copied() {
        syngui::async_runtime::run_on_main_thread(move || rev.update(|r| *r += 1));
    }
}
