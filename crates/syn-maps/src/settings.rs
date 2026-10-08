//! Настройки «Карт» (`~/.config/synshell/maps.toml`): где была карта и какой слой.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub lat: f64,
    pub lon: f64,
    pub zoom: f64,
    /// Номер слоя в [`crate::ui::LAYERS`].
    pub layer: usize,
}

impl Default for Settings {
    fn default() -> Self {
        // Москва — пока не известно своё место
        Self { lat: 55.7558, lon: 37.6173, zoom: 11.0, layer: 0 }
    }
}

fn settings_file() -> PathBuf {
    synshell_common::paths::config_dir().join("maps.toml")
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
        if let Ok(s) = toml::to_string_pretty(self) {
            let _ = std::fs::write(f, s);
        }
    }
}
