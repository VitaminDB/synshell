//! synpass — «Пароли»: хранилище паролей synshell под мастер-паролем.
//!
//! Записи синхронизируются с соединёнными устройствами через synlink
//! (`sync.rs`), а скопированный пароль сразу вставляется и там — общим
//! буфером обмена synlink (`[link] clipboard`); через заданное время буфер
//! очищается здесь и, следом, там. Телефон — стек «список → запись»,
//! рабочий стол — две колонки.

mod sync;
mod ui;
mod vault;

use serde::{Deserialize, Serialize};
use synshell_common::Config;
use syngui::prelude::*;

/// `~/.config/synshell/passwords.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Заблокировать после стольких минут без действий; 0 — не блокировать.
    pub autolock_min: u32,
    /// Очистить буфер обмена через столько секунд после копирования; 0 — не очищать.
    pub clear_secs: u32,
    pub generator: vault::GenOpts,
}

impl Default for Prefs {
    fn default() -> Self {
        Self { autolock_min: 5, clear_secs: 30, generator: vault::GenOpts::default() }
    }
}

impl Prefs {
    fn path() -> std::path::PathBuf {
        synshell_common::paths::config_dir().join("passwords.toml")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path()).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(s) = toml::to_string_pretty(self) {
            let p = Self::path();
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(p, s);
        }
    }
}

fn main() {
    synshell_common::i18n::init(&[include_str!("../i18n/en.lang")]);
    let (cfg, _) = Config::load();
    let mss = theme(&cfg);
    App::new()
        .title(t!("Пароли"))
        .app_id("synpass")
        .size(980, 680)
        .min_size(340, 480)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_styles_str(&mss)
        .run(move |_| ui::root());
}

fn theme(cfg: &Config) -> String {
    let a = &cfg.appearance;
    let mut s = a.mss_variables();
    let p = a.palette();
    let dark = a.is_dark();
    s.push_str(&format!(
        ":root {{\n  --input-bg: {};\n  --card: {};\n  --scrim: {};\n}}\n",
        if dark { p.bg.mix(p.surface, 0.35) } else { p.surface }.hex(),
        if dark { p.surface.mix(p.fg, 0.04) } else { p.surface }.hex(),
        if dark { "#000000b8" } else { "#00000080" },
    ));
    s.push_str(&a.theme_mss_variables());
    s.push_str(include_str!("../styles/synpass.mss"));
    if !a.font.trim().is_empty() {
        s.push_str(&format!("Text, Button, TextField {{ font-family: \"{}\"; }}\n", a.font.trim()));
    }
    s
}
