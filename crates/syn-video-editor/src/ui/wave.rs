//! Волна звука для клипов таймлайна: пики модуля по 10 мс (100 значений в
//! секунду, 0–1), считаются в фоне по очереди и хранятся на диске
//! (`~/.cache/syn-video-editor/wave`). Готово — сигнал тот же, что у лент превью.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

/// Пиков в секунду.
pub const RATE: f64 = 100.0;

pub type Peaks = Arc<Vec<f32>>;

struct State {
    ready: HashMap<PathBuf, Peaks>,
    queued: VecDeque<PathBuf>,
    busy: Option<PathBuf>,
}

fn state() -> &'static (Mutex<State>, Condvar) {
    static S: OnceLock<(Mutex<State>, Condvar)> = OnceLock::new();
    S.get_or_init(|| {
        std::thread::Builder::new().name("wave".into()).spawn(worker).ok();
        (Mutex::new(State { ready: HashMap::new(), queued: VecDeque::new(), busy: None }), Condvar::new())
    })
}

/// Пики файла или `None` (поставлен в очередь).
pub fn peaks(path: &Path) -> Option<Peaks> {
    let (m, cv) = state();
    let mut s = m.lock().unwrap();
    if let Some(p) = s.ready.get(path) {
        return Some(p.clone());
    }
    if s.busy.as_deref() != Some(path) && !s.queued.iter().any(|q| q == path) {
        s.queued.push_back(path.to_path_buf());
        cv.notify_one();
    }
    None
}

fn cache_file(path: &Path) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    if let Ok(md) = std::fs::metadata(path) {
        md.len().hash(&mut h);
        md.modified().ok().hash(&mut h);
    }
    let dir = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|v| PathBuf::from(v).join(".cache"))).unwrap_or_else(std::env::temp_dir).join("syn-video-editor/wave");
    dir.join(format!("{:016x}.peaks", h.finish()))
}

fn build(path: &Path) -> Option<Vec<f32>> {
    let file = cache_file(path);
    if let Ok(b) = std::fs::read(&file) {
        return Some(b.into_iter().map(|v| v as f32 / 255.0).collect());
    }
    // моно 8 кГц f32: на пик — 80 отсчётов
    let out = std::process::Command::new("ffmpeg").args(["-v", "error", "-i"]).arg(path).args(["-vn", "-ac", "1", "-ar", "8000", "-f", "f32le", "-"]).output().ok()?;
    if !out.status.success() || out.stdout.is_empty() {
        return None;
    }
    let samples: Vec<f32> = out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]).abs()).collect();
    let per = (8000.0 / RATE) as usize;
    let mut peaks: Vec<f32> = samples.chunks(per).map(|c| c.iter().copied().fold(0.0, f32::max)).collect();
    // нормировать по громкому месту (99-й перцентиль), чтобы тихие записи были видны
    let mut sorted = peaks.clone();
    sorted.sort_by(f32::total_cmp);
    let top = sorted.get(sorted.len() * 99 / 100).copied().unwrap_or(1.0).max(1e-3);
    for p in &mut peaks {
        *p = (*p / top).min(1.0);
    }
    if let Some(d) = file.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&file, peaks.iter().map(|p| (p * 255.0).round() as u8).collect::<Vec<u8>>());
    Some(peaks)
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
        let peaks = build(&path).unwrap_or_default();
        {
            let (m, _) = state();
            let mut s = m.lock().unwrap();
            s.busy = None;
            s.ready.insert(path, Arc::new(peaks));
        }
        super::thumbs::notify();
    }
}
