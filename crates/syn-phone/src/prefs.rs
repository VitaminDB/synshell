//! Настройки «Телефона» — `~/.config/synphone.toml`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use synshell_common::paths;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RecordMode {
    /// Записи нет (кнопки в разговоре тоже).
    Off,
    /// Кнопка «Запись» в разговоре.
    #[default]
    Manual,
    /// Каждый разговор пишется с ответа.
    Auto,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub record: RecordMode,
    /// Папка записей разговоров; пусто — `$XDG_MUSIC_DIR/Звонки`.
    pub record_dir: String,
    /// Живая визуализация голосов на экране разговора.
    pub visualize: bool,
    /// Фото абонента на весь экран звонка (иначе — кружком).
    pub caller_photo: bool,
    /// Вкладка при открытии: 0 — набор, 1 — журнал, 2 — контакты.
    pub start_tab: u8,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { record: RecordMode::Manual, record_dir: String::new(), visualize: true, caller_photo: true, start_tab: 0 }
    }
}

fn path() -> PathBuf {
    paths::xdg_config_home().join("synphone.toml")
}

impl Prefs {
    pub fn load() -> Prefs {
        std::fs::read_to_string(path()).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(s) = toml::to_string_pretty(self) {
            let p = path();
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(p, s);
        }
    }

    /// Папка записей (без создания).
    pub fn record_dir(&self) -> PathBuf {
        let d = self.record_dir.trim();
        if d.is_empty() {
            default_record_dir()
        } else {
            paths::expand_tilde(d)
        }
    }
}

pub fn default_record_dir() -> PathBuf {
    paths::user_dir_or_default("MUSIC").join(syngui::t!("Звонки").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_roundtrip() {
        let p = Prefs { record: RecordMode::Auto, record_dir: "~/Calls".into(), visualize: false, caller_photo: true, start_tab: 2 };
        let s = toml::to_string_pretty(&p).unwrap();
        assert!(s.contains("record = \"auto\""));
        assert_eq!(toml::from_str::<Prefs>(&s).unwrap(), p);
        // неизвестные и отсутствующие ключи — по умолчанию
        let d: Prefs = toml::from_str("visualize = false").unwrap();
        assert_eq!(d.record, RecordMode::Manual);
        assert!(!d.visualize);
    }
}
