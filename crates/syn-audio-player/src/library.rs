//! Библиотека: аудиофайлы из «Музыки» (XDG_MUSIC_DIR, вглубь до 6 уровней) и «Загрузок» (2 уровня).
//! Теги и длительность — `syngui::video::probe_audio` в фоне (сортировка: исполнитель, альбом, номер, имя).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use synshell_common::paths;

pub const EXTENSIONS: &[&str] = &[
    "mp3", "mp2", "mpga", "flac", "ogg", "oga", "opus", "spx", "m4a", "m4b", "m4p", "mp4a", "aac", "adts", "3ga", "amr", "awb",
    "wav", "wave", "aif", "aiff", "aifc", "caf", "au", "snd", "wma", "asf", "ape", "wv", "mpc", "mp+", "tta", "tak", "shn",
    "mka", "weba", "ac3", "eac3", "dts", "dtshd", "thd", "mlp", "mid", "midi", "mod", "s3m", "xm", "it", "voc", "gsm", "dsf",
    "dff", "ra", "rm",
];

pub fn is_audio(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Сведения о треке (теги — после `probe`).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Track {
    pub path: PathBuf,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub track_no: Option<u32>,
    pub duration: f64,
    pub probed: bool,
}

impl Track {
    pub fn from_path(path: PathBuf) -> Self {
        let title = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        Track { path, title, ..Default::default() }
    }

    /// Прочитать теги (медленно — не из главного потока). Обложка возвращается отдельно.
    pub fn probe(&mut self) -> Option<Arc<Vec<u8>>> {
        self.probed = true;
        let m = syngui::video::probe_audio(&self.path.to_string_lossy()).ok()?;
        if let Some(t) = m.title {
            self.title = t;
        }
        self.artist = m.artist;
        self.album = m.album;
        self.track_no = m.track.and_then(|t| t.split('/').next().and_then(|n| n.trim().parse().ok()));
        self.duration = m.duration_sec;
        m.cover.map(Arc::new)
    }

    pub fn subtitle(&self) -> String {
        match (&self.artist, &self.album) {
            (Some(a), Some(b)) => format!("{a} — {b}"),
            (Some(a), None) => a.clone(),
            (None, Some(b)) => b.clone(),
            (None, None) => String::new(),
        }
    }
}

pub fn roots() -> Vec<PathBuf> {
    let mut v = vec![paths::user_dir_or_default("MUSIC"), paths::user_dir_or_default("DOWNLOAD")];
    v.dedup();
    v
}

pub fn scan() -> Vec<Track> {
    let mut out = Vec::new();
    let roots = roots();
    for (i, r) in roots.iter().enumerate() {
        walk(r, if i == 0 { 6 } else { 2 }, &mut out);
    }
    out.sort();
    out.dedup();
    out.into_iter().map(Track::from_path).collect()
}

fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
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
        } else if is_audio(&p) && md.len() > 0 {
            out.push(p);
        }
    }
}

/// Порядок библиотеки: исполнитель, альбом, номер дорожки, название.
pub fn sort(tracks: &mut [Track]) {
    let key = |t: &Track| {
        (
            t.artist.clone().unwrap_or_default().to_lowercase(),
            t.album.clone().unwrap_or_default().to_lowercase(),
            t.track_no.unwrap_or(u32::MAX),
            t.title.to_lowercase(),
        )
    };
    tracks.sort_by_key(key);
}

/// «1:05», «1:02:03».
pub fn format_time(sec: f64) -> String {
    let s = sec.max(0.0) as u64;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}
