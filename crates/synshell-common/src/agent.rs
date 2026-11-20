//! Настройки управления агентом (ИИ через synlink) — отдельный файл
//! `agent.toml` в каталоге настроек: новый ключ в общем конфиге сломал бы его
//! разбор у уже запущенного synwm старой версии.

use serde::{Deserialize, Serialize};

/// Сочетание, которым пользователь забирает управление (зашито в synwm).
pub const TAKEBACK: &str = "Ctrl+Alt+Esc";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AgentSettings {
    /// Полное управление: пока агент управляет, ввод пользователя
    /// блокируется (кроме сочетания `TAKEBACK`).
    pub full_control: bool,
    /// Искать элементы по описанию локальной моделью со зрением (synaptixd).
    /// По умолчанию выключено: память видеокарты обычно занята, а агент быстрее
    /// разбирает снимок сам.
    pub vision: bool,
    /// Модель со зрением (пусто — самая лёгкая из умеющих картинки).
    pub vision_model: String,
}

pub fn path() -> std::path::PathBuf {
    crate::paths::config_dir().join("agent.toml")
}

pub fn load() -> AgentSettings {
    std::fs::read_to_string(path()).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
}

pub fn save(s: &AgentSettings) -> std::io::Result<()> {
    let p = path();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(p, toml::to_string(s).map_err(std::io::Error::other)?)
}
