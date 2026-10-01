//! Сон системы при погашенном экране (телефон).
//!
//! Усыпляет систему служба платформы (arch-mobile-port `syn-sleepd`, root): пока
//! есть файл `/run/syn-sleep/screen-off`, она раз за разом засыпает. Файл ставит
//! композитор, гася экран, и убирает, включая; внутри — pid композитора, чтобы
//! служба не усыпляла телефон без живого композитора. Нет каталога (рабочий
//! стол, служба не установлена) — ничего не делается.

use std::path::Path;

const DIR: &str = "/run/syn-sleep";
const FLAG: &str = "/run/syn-sleep/screen-off";

pub fn screen_power(on: bool) {
    if !Path::new(DIR).is_dir() {
        return;
    }
    let r = if on {
        match std::fs::remove_file(FLAG) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            r => r,
        }
    } else {
        std::fs::write(FLAG, std::process::id().to_string())
    };
    if let Err(e) = r {
        tracing::warn!("сон: {FLAG}: {e}");
    }
}
