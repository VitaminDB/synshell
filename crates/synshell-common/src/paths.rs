//! Пути по XDG: конфиг, тема, сокет IPC.

use std::path::PathBuf;

/// Домашний каталог (`$HOME`).
pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// `$XDG_CONFIG_HOME/synshell` (по умолчанию `~/.config/synshell`).
pub fn config_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("SYNSHELL_CONFIG_DIR") {
        return PathBuf::from(p);
    }
    let base = xdg_config_home();
    let new = base.join("synshell");
    // Совместимость: до переименования проект звался syndesktop и хранил конфиг в
    // ~/.config/syndesktop. Пока новый каталог не создан, читаем старый.
    let old = base.join("syndesktop");
    if !new.exists() && old.exists() {
        return old;
    }
    new
}

/// `$XDG_CONFIG_HOME` (по умолчанию `~/.config`) — конфиги других программ.
pub fn xdg_config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
}

/// Главный файл конфигурации.
pub fn config_file() -> PathBuf {
    config_dir().join("config.toml")
}

/// Пользовательская MSS-тема, дописывается поверх встроенной.
pub fn user_theme_file() -> PathBuf {
    config_dir().join("theme.mss")
}

/// `$XDG_RUNTIME_DIR`.
pub fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

/// `$XDG_DATA_HOME` (по умолчанию `~/.local/share`).
pub fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local/share"))
}

/// `$XDG_CACHE_HOME` (по умолчанию `~/.cache`).
pub fn cache_home() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".cache"))
}

/// Все каталоги данных: `$XDG_DATA_HOME` + `$XDG_DATA_DIRS`.
pub fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![data_home()];
    let sys = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    dirs.extend(sys.split(':').filter(|s| !s.is_empty()).map(PathBuf::from));
    dirs
}

/// Путь сокета IPC. Композитор выставляет `SYNSHELL_SOCKET` своим детям;
/// клиенты вне сессии ищут сокет по имени Wayland-дисплея.
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("SYNSHELL_SOCKET") {
        return PathBuf::from(p);
    }
    // Сеанс композитора до переименования (syndesktop) — своя переменная.
    if let Some(p) = std::env::var_os("SYNDESKTOP_SOCKET") {
        return PathBuf::from(p);
    }
    let display = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".into());
    socket_path_for(&display)
}

/// Путь сокета IPC для заданного Wayland-дисплея.
pub fn socket_path_for(wayland_display: &str) -> PathBuf {
    runtime_dir().join(format!("synwm.{wayland_display}.sock"))
}

/// Раскрыть `~/` в начале пути.
pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        home().join(rest)
    } else if path == "~" {
        home()
    } else {
        PathBuf::from(path)
    }
}

/// Каталог пользователя по XDG (`DOWNLOAD`, `DOCUMENTS`, `DESKTOP`…) из
/// `user-dirs.dirs`; `None`, если не задан.
pub fn user_dir(kind: &str) -> Option<PathBuf> {
    let home = home();
    let conf = xdg_config_home().join("user-dirs.dirs");
    let key = format!("XDG_{kind}_DIR=");
    let text = std::fs::read_to_string(conf).ok()?;
    for line in text.lines() {
        if let Some(v) = line.trim().strip_prefix(&key) {
            let v = v.trim_matches('"').replace("$HOME", &home.to_string_lossy());
            let p = PathBuf::from(v);
            // `XDG_DESKTOP_DIR="$HOME/"` — каталог не задан.
            return (p != home).then_some(p);
        }
    }
    None
}

/// Каталог пользователя по XDG или обычное английское имя в `~`.
pub fn user_dir_or_default(kind: &str) -> PathBuf {
    user_dir(kind).unwrap_or_else(|| {
        let name = match kind {
            "DOWNLOAD" => "Downloads",
            "DOCUMENTS" => "Documents",
            "PICTURES" => "Pictures",
            "MUSIC" => "Music",
            "VIDEOS" => "Videos",
            "DESKTOP" => "Desktop",
            "TEMPLATES" => "Templates",
            "PUBLICSHARE" => "Public",
            _ => "",
        };
        home().join(name)
    })
}
