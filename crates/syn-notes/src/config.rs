//! Состояние приложения заметок: открытые проекты, активные страницы,
//! раскрытые узлы, положения разделителей и видимость панелей.
//!
//! Файл — `~/.local/state/syn-notes/state.json` (`SYN_NOTES_STATE` — другой
//! путь, для проверок на копиях). При первом запуске состояние берётся из
//! конфига synthos (`~/.config/synthos/config.json`), где заметки жили
//! раньше: те же проекты открываются на тех же страницах.
//!
//! Поля и их имена — как в `AppConfig` synthos: код проектов
//! (`notes::projects::restore`) читает их без изменений.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Открытый проект заметок: файл, плитка и где в нём остановились.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct NotesProjectConfig {
    pub path: String,
    /// Штамп появления плитки (unix-миллисекунды).
    pub opened_at: u64,
    /// Активная страница (id) и раскрытые узлы дерева.
    pub active_page: Option<String>,
    pub expanded: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub notes_left_split_ratio: f32,
    pub notes_right_split_ratio: f32,
    /// Папка `.md` времён до проектов — только источник миграции.
    pub notes_vault_path: String,
    /// Файл времён одного проекта — только источник миграции.
    pub notes_project_path: String,
    pub notes_active: Option<String>,
    pub notes_expanded: Vec<String>,
    pub notes_tile_opened_at: Option<u64>,
    pub notes_projects: Option<Vec<NotesProjectConfig>>,
    pub notes_active_project: String,
    pub notes_recent: Vec<String>,
    pub notes_left: bool,
    pub notes_right: bool,
}

pub fn default_notes_left_split_ratio() -> f32 {
    0.18
}

pub fn default_notes_right_split_ratio() -> f32 {
    0.72
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            notes_left_split_ratio: default_notes_left_split_ratio(),
            notes_right_split_ratio: default_notes_right_split_ratio(),
            notes_vault_path: String::new(),
            notes_project_path: String::new(),
            notes_active: None,
            notes_expanded: Vec::new(),
            notes_tile_opened_at: None,
            notes_projects: None,
            notes_active_project: String::new(),
            notes_recent: Vec::new(),
            notes_left: true,
            notes_right: true,
        }
    }
}

pub fn state_path() -> PathBuf {
    if let Some(p) = std::env::var_os("SYN_NOTES_STATE") {
        return PathBuf::from(p);
    }
    synshell_common::paths::state_home().join("syn-notes/state.json")
}

fn synthos_config() -> PathBuf {
    synshell_common::paths::home().join(".config/synthos/config.json")
}

static LOADER: std::sync::OnceLock<fn() -> AppConfig> = std::sync::OnceLock::new();

/// Программа, встраивающая заметки (synthos), хранит их состояние в своём
/// конфиге: `load` тогда берёт его отсюда.
pub fn set_loader(f: fn() -> AppConfig) {
    let _ = LOADER.set(f);
}

impl AppConfig {
    /// Состояние с диска; нет файла — из synthos, нет и его — по умолчанию.
    pub fn load() -> Self {
        if let Some(f) = LOADER.get() {
            return f();
        }
        let path = state_path();
        if let Ok(text) = std::fs::read_to_string(&path) {
            return serde_json::from_str(&text).unwrap_or_default();
        }
        if std::env::var_os("SYN_NOTES_STATE").is_none() {
            if let Some(cfg) = Self::from_synthos() {
                return cfg;
            }
        }
        Self::default()
    }

    /// Поля заметок из конфига synthos (json с теми же именами полей).
    fn from_synthos() -> Option<Self> {
        let text = std::fs::read_to_string(synthos_config()).ok()?;
        let v: serde_json::Value = serde_json::from_str(&text).ok()?;
        let mut cfg: Self = serde_json::from_value(v.clone()).ok()?;
        if let Some(p) = v.get("panels") {
            cfg.notes_left = p.get("notes_left").and_then(|b| b.as_bool()).unwrap_or(true);
            cfg.notes_right = p.get("notes_right").and_then(|b| b.as_bool()).unwrap_or(true);
        }
        Some(cfg)
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = state_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        crate::fsutil::write_atomic(&path, text.as_bytes())
    }
}
