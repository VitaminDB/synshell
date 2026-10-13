//! Мои пульты (`~/.local/share/synshell/remote/remotes.json`): кнопки копируются из базы, так что
//! пульт работает и без неё, и после её обновления.

use serde::{Deserialize, Serialize};

use crate::ir::Signal;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Remote {
    pub id: String,
    pub name: String,
    /// Тип техники — каталог базы («TVs»).
    pub category: String,
    pub brand: String,
    /// Модель пульта в базе (имя файла) и путь к файлу от корня базы.
    pub model: String,
    #[serde(default)]
    pub path: String,
    /// Другие пульты с тем же кодом питания — на случай, если часть кнопок не подошла.
    #[serde(default)]
    pub alternatives: Vec<String>,
    pub signals: Vec<Signal>,
    #[serde(default)]
    pub created: i64,
}

fn store_path() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| synshell_common::paths::expand_tilde("~/.local/share"));
    base.join("synshell/remote/remotes.json")
}

pub fn load() -> Vec<Remote> {
    std::fs::read_to_string(store_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save(list: &[Remote]) {
    let p = store_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = p.with_extension("json.tmp");
    let r = std::fs::write(&tmp, serde_json::to_string_pretty(list).unwrap_or_default()).and_then(|_| std::fs::rename(&tmp, &p));
    if let Err(e) = r {
        tracing::warn!("пульты не сохранены: {e}");
    }
}

pub fn new_id() -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{:x}", t)
}
