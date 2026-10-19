//! Навигация. В syn-notes приложение целиком — заметки, и «перейти в
//! заметки» значит показать окно (оно может быть спрятано: процесс живёт в
//! фоне). Программа, встраивающая заметки (synthos), ставит свою навигацию
//! и переименование плиток через [`set_host`].

use std::path::Path;
use std::sync::OnceLock;

/// Навигация программы-хозяина: перейти на маршрут, переименовать ключ плитки.
pub struct RailHost {
    pub navigate: fn(&str),
    pub rename_key: fn(&str, &str),
}

static HOST: OnceLock<RailHost> = OnceLock::new();

pub fn set_host(host: RailHost) {
    let _ = HOST.set(host);
}

pub fn navigate(route: &str) {
    match HOST.get() {
        Some(h) => (h.navigate)(route),
        None => {
            syngui::app::show_main_window();
        }
    }
}

/// Ключ проекта — для порядка плиток и памяти о них.
pub fn notes_key(path: &Path) -> String {
    format!("notes:{}", path.display())
}

/// Переименование проекта меняет ключ его плитки.
pub fn rename_key(old: &str, new: &str) {
    if let Some(h) = HOST.get() {
        (h.rename_key)(old, new);
    }
}
