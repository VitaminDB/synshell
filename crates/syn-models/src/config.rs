//! Настройки «Моделей»: Hugging Face (токен, кэш, загрузки) и пакеты `.syn`
//! (закладки, режим эксперта), положения разделителей и видимость панелей.
//!
//! Файл — `~/.config/syn-models/config.json` (`SYN_MODELS_CONFIG` — другой
//! путь, для проверок на копиях). При первом запуске настройки берутся из
//! конфига synthos (`~/.config/synthos/config.json`), где Hugging Face и
//! пакеты жили раньше: имена полей те же, лишние поля synthos пропускаются.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub syn_explorer_bookmarks: Vec<String>,
    pub syn_explorer_left_split_ratio: f32,
    pub syn_explorer_right_split_ratio: f32,
    pub syn_explorer_expert: bool,
    pub syn_explorer_left: bool,
    pub syn_explorer_right: bool,
    /// Пусто = `~/.local/share/synthos/hf` (см. [`resolve_hf_cache_dir`]) —
    /// прежний кэш synthos, уже скачанное не теряется.
    pub hf_cache_dir: String,
    pub hf_concurrent_downloads: u32,
    pub hf_segments_per_file: u32,
    pub hf_speed_limit_mbps: u32,
    pub hf_skip_unwanted_formats: bool,
    pub hf_gguf_support: bool,
    pub hf_files_view: String,
    pub hf_dock_expanded: bool,
    pub hf_token: String,
    pub hf_left_split_ratio: f32,
    pub hf_right_split_ratio: f32,
    pub huggingface_left: bool,
    pub huggingface_right: bool,
    /// «Обучение»: видимость и ширина (доля окна) левой панели.
    pub training_left: bool,
    pub training_left_split_ratio: f32,
    /// Каталог моделей (`.syn`, LoRA, каталоги HF).
    pub models_dir: String,
    /// Открытый раздел: `huggingface`, `syn_explorer`, `settings` или `training`.
    pub page: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            syn_explorer_bookmarks: Vec::new(),
            syn_explorer_left_split_ratio: 0.19,
            syn_explorer_right_split_ratio: 0.78,
            syn_explorer_expert: false,
            syn_explorer_left: true,
            syn_explorer_right: true,
            hf_cache_dir: String::new(),
            hf_concurrent_downloads: 3,
            hf_segments_per_file: 4,
            hf_speed_limit_mbps: 0,
            hf_skip_unwanted_formats: true,
            hf_gguf_support: false,
            hf_files_view: "list".into(),
            hf_dock_expanded: false,
            hf_token: String::new(),
            hf_left_split_ratio: 0.26,
            hf_right_split_ratio: 0.70,
            huggingface_left: true,
            huggingface_right: true,
            training_left: true,
            training_left_split_ratio: 0.24,
            models_dir: default_models_dir(),
            page: "huggingface".into(),
        }
    }
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
}

/// Каталог настроек программы.
pub fn dir() -> PathBuf {
    match std::env::var_os("SYN_MODELS_CONFIG") {
        Some(p) => PathBuf::from(p).parent().map(|p| p.to_path_buf()).unwrap_or_else(|| home().join(".config/syn-models")),
        None => home().join(".config/syn-models"),
    }
}

/// Прежний конфиг synthos — источник миграции.
fn synthos_config() -> PathBuf {
    home().join(".config/synthos/config.json")
}

pub fn default_models_dir() -> String {
    format!("{}/Storage/syn_models", home().display())
}

/// Каталог моделей с раскрытием `~` в начале пути.
pub fn resolve_models_dir(raw: &str) -> PathBuf {
    let raw = raw.trim();
    if raw.is_empty() {
        return PathBuf::from(default_models_dir());
    }
    crate::paths::expand_home(raw)
}

/// Кэш скачанного с Hugging Face.
pub fn resolve_hf_cache_dir(stored: &str) -> PathBuf {
    if !stored.trim().is_empty() {
        return crate::paths::expand_home(stored);
    }
    home().join(".local/share/synthos/hf")
}

impl AppConfig {
    pub fn path() -> PathBuf {
        match std::env::var_os("SYN_MODELS_CONFIG") {
            Some(p) => PathBuf::from(p),
            None => dir().join("config.json"),
        }
    }

    pub fn load() -> Self {
        let path = Self::path();
        if let Ok(s) = std::fs::read_to_string(&path) {
            return serde_json::from_str(&s).unwrap_or_else(|e| {
                tracing::warn!("{}: {e} — настройки по умолчанию", path.display());
                Self::default()
            });
        }
        // Первый запуск: перенести из synthos (тех же имён поля).
        if std::env::var_os("SYN_MODELS_CONFIG").is_none() {
            if let Ok(s) = std::fs::read_to_string(synthos_config()) {
                if let Ok(cfg) = serde_json::from_str::<Self>(&s) {
                    let cfg = Self { page: "huggingface".into(), ..cfg };
                    cfg.save();
                    tracing::info!("настройки Hugging Face и пакетов перенесены из synthos");
                    return cfg;
                }
            }
        }
        Self::default()
    }

    pub fn save(&self) {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(self) {
            Ok(s) => {
                if let Err(e) = crate::fsutil::write_atomic(&path, s) {
                    tracing::warn!("{}: {e}", path.display());
                }
            }
            Err(e) => tracing::warn!("настройки не записаны: {e}"),
        }
    }
}
