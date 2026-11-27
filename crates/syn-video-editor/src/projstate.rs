//! Состояние окна у каждого проекта своё: открытые графы панели «Граф»,
//! раскладка панелей, вид, масштабы таймлайна и просмотра. Хранится рядом с
//! программой — `$XDG_STATE_HOME/syn-video-editor/projects/<ключ>.*`, ключ —
//! хеш пути проекта (несохранённый проект — `untitled`). Новый проект
//! открывается с чистым состоянием, проект с диска — с тем, каким его оставили.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// Каталог состояний проектов.
pub fn dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .unwrap_or_else(std::env::temp_dir)
        .join("syn-video-editor/projects")
}

/// Ключ проекта: хеш канонического пути; `None` — несохранённый.
pub fn key(path: Option<&Path>) -> String {
    match path {
        None => "untitled".into(),
        Some(p) => {
            use std::hash::{Hash, Hasher};
            let c = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
            let mut h = std::collections::hash_map::DefaultHasher::new();
            c.hash(&mut h);
            let stem = p.file_stem().map(|s| s.to_string_lossy().chars().filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_').take(32).collect::<String>()).unwrap_or_default();
            format!("{stem}-{:016x}", h.finish())
        }
    }
}

static CURRENT: Mutex<Option<String>> = Mutex::new(None);

/// Ключ проекта, чьё состояние сейчас в окне.
pub fn current() -> String {
    CURRENT.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_else(|| "untitled".into())
}

pub fn set_current(k: &str) {
    *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(k.to_string());
}

/// Файл открытых графов проекта.
pub fn graphs_file(k: &str) -> PathBuf {
    dir().join(format!("{k}.graphs.json"))
}

/// Файл состояния окна проекта.
pub fn ui_file(k: &str) -> PathBuf {
    dir().join(format!("{k}.ui.json"))
}

/// Состояние окна проекта.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Ui {
    /// Вид (пресет) и раскладка панелей (`layout::encode_current`).
    #[serde(default)]
    pub layout: String,
    /// Масштаб таймлайна, пикс./с.
    #[serde(default)]
    pub zoom: Option<f64>,
    /// Масштаб и сдвиг просмотра.
    #[serde(default)]
    pub viewer_zoom: Option<f64>,
    #[serde(default)]
    pub viewer_pan: Option<(f64, f64)>,
    /// Размер плиток медиатеки.
    #[serde(default)]
    pub tile_size: Option<f64>,
}

pub fn load_ui(k: &str) -> Option<Ui> {
    serde_json::from_str(&std::fs::read_to_string(ui_file(k)).ok()?).ok()
}

pub fn save_ui(k: &str, ui: &Ui) {
    let p = ui_file(k);
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = p.with_extension("json.tmp");
    if std::fs::write(&tmp, serde_json::to_string_pretty(ui).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

/// Перенести состояние несохранённого проекта под ключ его нового файла
/// (первое «Сохранить как»), если у файла своего состояния ещё нет.
pub fn adopt(from: &str, to: &str) -> bool {
    if graphs_file(to).exists() || ui_file(to).exists() {
        return false;
    }
    let _ = std::fs::create_dir_all(dir());
    let mut any = false;
    for (a, b) in [(graphs_file(from), graphs_file(to)), (ui_file(from), ui_file(to))] {
        if a.exists() && std::fs::copy(&a, &b).is_ok() {
            any = true;
        }
    }
    any
}

#[cfg(test)]
mod tests {
    #[test]
    fn keys() {
        assert_eq!(super::key(None), "untitled");
        let a = super::key(Some(std::path::Path::new("/tmp/a/мультик.syn")));
        let b = super::key(Some(std::path::Path::new("/tmp/b/мультик.syn")));
        assert_ne!(a, b);
        assert!(a.starts_with("мультик-"));
    }
}
