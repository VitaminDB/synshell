//! Настройки «Видео» (`~/.config/synshell/video-player.toml`) и места остановки роликов
//! (`$XDG_STATE_HOME/syn-video-player/positions.json`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Аппаратное декодирование, где есть чем (иначе — только программно).
    pub hardware: bool,
    /// Кадр заполняет окно (края обрезаются); иначе вписан целиком.
    pub fill: bool,
    /// Шаг перемотки кнопками и двойным тапом по краю, с.
    pub seek_step: u32,
    /// Ролик дошёл до конца — начать сначала.
    pub repeat: bool,
    /// Открывать ролик с места, где остановились.
    pub resume: bool,
    /// Чип «как идёт воспроизведение» в шапке плеера.
    pub show_info: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { hardware: true, fill: false, seek_step: 10, repeat: false, resume: true, show_info: true }
    }
}

/// Шаги перемотки на выбор.
pub const SEEK_STEPS: [u32; 4] = [5, 10, 15, 30];

fn settings_file() -> PathBuf {
    synshell_common::paths::config_dir().join("video-player.toml")
}

impl Settings {
    pub fn load() -> Self {
        std::fs::read_to_string(settings_file()).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let f = settings_file();
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        match toml::to_string_pretty(self) {
            Ok(s) => {
                if let Err(e) = std::fs::write(&f, s) {
                    tracing::warn!("{}: {e}", f.display());
                }
            }
            Err(e) => tracing::warn!("настройки: {e}"),
        }
    }
}

// ─── места остановки ────────────────────────────────────────────────────────

/// Сколько роликов помнить.
const POSITIONS_MAX: usize = 300;
/// Ближе к началу или к концу — не запоминать (смотреть сначала).
const RESUME_MARGIN: f64 = 5.0;

fn positions_file() -> PathBuf {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| synshell_common::paths::home().join(".local/state"));
    state.join("syn-video-player/positions.json")
}

#[derive(Default, Serialize, Deserialize)]
struct Positions {
    /// путь → (секунда, когда записано — unix-время)
    items: HashMap<String, (f64, u64)>,
}

fn load_positions() -> Positions {
    std::fs::read(positions_file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Где остановились в этом ролике (`None` — смотреть сначала).
pub fn saved_position(path: &Path) -> Option<f64> {
    load_positions().items.get(&path.to_string_lossy().into_owned()).map(|(s, _)| *s)
}

/// Запомнить место остановки; у начала и конца ролика — забыть.
pub fn save_position(path: &Path, sec: f64, duration: f64) {
    let mut p = load_positions();
    let key = path.to_string_lossy().into_owned();
    if sec < RESUME_MARGIN || (duration > 0.0 && sec > duration - RESUME_MARGIN) {
        if p.items.remove(&key).is_none() {
            return;
        }
    } else {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        p.items.insert(key, (sec, now));
        if p.items.len() > POSITIONS_MAX {
            let mut v: Vec<(String, u64)> = p.items.iter().map(|(k, (_, t))| (k.clone(), *t)).collect();
            v.sort_by_key(|(_, t)| *t);
            for (k, _) in v.into_iter().take(p.items.len() - POSITIONS_MAX) {
                p.items.remove(&k);
            }
        }
    }
    let f = positions_file();
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(b) = serde_json::to_vec(&p) {
        let _ = std::fs::write(&f, b);
    }
}
