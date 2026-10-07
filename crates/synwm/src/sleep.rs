//! Сон системы при погашенном экране (телефон).
//!
//! Усыпляет систему служба платформы (synmobile `syn-sleepd`, root): пока
//! есть файл `/run/syn-sleep/screen-off`, она раз за разом засыпает. Файл ставит
//! композитор, гася экран, и убирает, включая; внутри — pid композитора, чтобы
//! служба не усыпляла телефон без живого композитора, и второй строкой —
//! задержка `[idle] sleep_delay` (секунды после гашения экрана до сна; служба
//! отсчитывает её от времени файла). Нет каталога (рабочий стол, служба не
//! установлена) — ничего не делается.

use std::path::Path;

const DIR: &str = "/run/syn-sleep";
const FLAG: &str = "/run/syn-sleep/screen-off";

pub fn screen_power(on: bool, delay_secs: u32) {
    if !Path::new(DIR).is_dir() {
        return;
    }
    let r = if on {
        match std::fs::remove_file(FLAG) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            r => r,
        }
    } else {
        std::fs::write(FLAG, format!("{}\n{delay_secs}\n", std::process::id()))
    };
    if let Err(e) = r {
        tracing::warn!("сон: {FLAG}: {e}");
    }
}
