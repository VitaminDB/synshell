//! Настройки терминала: `~/.config/synshell/terminal.toml`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Семейство моноширинного шрифта; пусто — системный `monospace`.
    pub font_family: String,
    /// Размер шрифта, логические пиксели.
    pub font_size: f32,
    /// Межстрочный интервал (доля размера шрифта).
    pub line_height: f32,
    /// Схема цветов (`schemes::SCHEMES`); `system` — по оформлению synshell.
    pub scheme: String,
    /// Непрозрачность фона, 0.5–1.0.
    pub opacity: f32,
    /// Строк истории над экраном.
    pub scrollback: usize,
    /// Оболочка новых вкладок; пусто — `$SHELL`.
    pub shell: String,
    /// Новая вкладка открывается в папке текущей.
    pub new_tab_here: bool,
    /// Спрашивать, прежде чем закрыть вкладку или окно с работающей программой.
    pub confirm_close: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            font_family: String::new(),
            font_size: 14.0,
            line_height: 1.2,
            scheme: "system".into(),
            opacity: 1.0,
            scrollback: 10_000,
            shell: String::new(),
            new_tab_here: true,
            confirm_close: true,
        }
    }
}

pub const FONT_MIN: f32 = 8.0;
pub const FONT_MAX: f32 = 32.0;
/// Длина истории на выбор.
pub const SCROLLBACK_STEPS: [usize; 4] = [1_000, 10_000, 50_000, 100_000];

fn file() -> PathBuf {
    synshell_common::paths::config_dir().join("terminal.toml")
}

impl Settings {
    pub fn load() -> Self {
        let mut s: Self = std::fs::read_to_string(file()).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default();
        s.font_size = s.font_size.clamp(FONT_MIN, FONT_MAX);
        s.line_height = s.line_height.clamp(1.0, 2.0);
        s.opacity = s.opacity.clamp(0.5, 1.0);
        s
    }

    pub fn save(&self) {
        let f = file();
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

    /// Оболочка новых вкладок.
    pub fn shell(&self) -> String {
        let s = self.shell.trim();
        if !s.is_empty() {
            return s.to_string();
        }
        std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/sh".into())
    }
}
