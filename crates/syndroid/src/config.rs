//! Настройки syndroid (`/var/lib/syndroid/config.toml`).

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Активный набор образов (каталог в `images/`).
    pub active: Option<String>,
    /// OTA-каналы (формат Waydroid: JSON со списком `response`).
    pub system_channel: String,
    pub vendor_channel: String,
    pub rom_type: String,
    /// VANILLA или GAPPS.
    pub system_type: String,
    /// MAINLINE — без HAL устройства (наш случай: Android-HAL на хосте не работают).
    pub vendor_type: String,
    pub arch: String,
    /// Плотность экрана Android (`ro.sf.lcd_density`), 0 — по умолчанию Android.
    pub dpi: u32,
    /// Каждое приложение — своим окном.
    pub multi_windows: bool,
    /// Сеть (мост, NAT, DHCP).
    pub network: bool,
    /// Узел DRM для gralloc (gbm); пусто — первый `renderD*`.
    pub drm_node: String,
    /// Дополнительные/переопределённые свойства Android.
    pub properties: BTreeMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            active: None,
            system_channel: "https://ota.waydro.id/system".into(),
            vendor_channel: "https://ota.waydro.id/vendor".into(),
            rom_type: "lineage".into(),
            system_type: "VANILLA".into(),
            vendor_type: "MAINLINE".into(),
            arch: "arm64".into(),
            dpi: 0,
            multi_windows: true,
            network: true,
            drm_node: String::new(),
            properties: BTreeMap::new(),
        }
    }
}

impl Config {
    pub fn load() -> Self {
        std::fs::read_to_string(paths::config())
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        std::fs::create_dir_all(paths::STATE)?;
        let tmp = paths::config().with_extension("toml.tmp");
        std::fs::write(&tmp, toml::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, paths::config()).context("config.toml")
    }
}
