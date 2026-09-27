//! Пути по XDG: конфиг, тема, сокет IPC.

use std::path::PathBuf;

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// `$XDG_CONFIG_HOME/syndesktop` (по умолчанию `~/.config/syndesktop`).
pub fn config_dir() -> PathBuf {
    std::env::var_os("SYNDESKTOP_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(|p| PathBuf::from(p).join("syndesktop")))
        .unwrap_or_else(|| home().join(".config/syndesktop"))
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

/// Путь сокета IPC. Композитор выставляет `SYNDESKTOP_SOCKET` своим детям;
/// клиенты вне сессии ищут сокет по имени Wayland-дисплея.
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("SYNDESKTOP_SOCKET") {
        return PathBuf::from(p);
    }
    let display = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".into());
    socket_path_for(&display)
}

/// Путь сокета IPC для заданного Wayland-дисплея.
pub fn socket_path_for(wayland_display: &str) -> PathBuf {
    runtime_dir().join(format!("syndesktop.{wayland_display}.sock"))
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
