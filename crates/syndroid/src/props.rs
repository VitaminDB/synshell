//! `waydroid.prop` — свойства, которые образ читает из `/vendor/waydroid.prop`: графика, пути сокетов
//! сеанса, пользователь хоста, плотность экрана. Имена — протокол образов Waydroid.

use std::path::Path;

use crate::api::Session;
use crate::config::Config;
use crate::paths;

/// Первый DRM render node хоста (для gralloc gbm).
pub fn drm_node(c: &Config) -> Option<String> {
    if !c.drm_node.is_empty() {
        return Some(c.drm_node.clone());
    }
    let mut v: Vec<String> = std::fs::read_dir("/dev/dri")
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("renderD"))
        .collect();
    v.sort();
    v.first().map(|n| format!("/dev/dri/{n}"))
}

pub fn build(c: &Config, s: &Session) -> String {
    let mut p: Vec<(String, String)> = Vec::new();
    let mut set = |k: &str, v: &str| p.push((k.to_string(), v.to_string()));
    if !Path::new("/dev/ashmem").exists() {
        set("sys.use_memfd", "true");
    }
    set("ro.adb.secure", "1");
    set("ro.debuggable", "0");
    match drm_node(c) {
        Some(node) => {
            set("ro.hardware.gralloc", "gbm");
            set("ro.hardware.egl", "mesa");
            set("gralloc.gbm.device", &node);
            set("ro.hardware.vulkan", "freedreno");
        }
        None => {
            set("ro.hardware.gralloc", "default");
            set("ro.hardware.egl", "swiftshader");
        }
    }
    set("debug.stagefright.ccodec", "0");
    set("ro.opengles.version", "196610");
    // Образы обновляет syndroid, а не апдейтер внутри Android
    set("waydroid.updater.disabled", "true");
    set("waydroid.tools_version", env!("CARGO_PKG_VERSION"));
    set("ro.vndk.lite", "true");
    // Сеанс
    set("waydroid.host.user", &s.user);
    set("waydroid.host.uid", &s.uid.to_string());
    set("waydroid.host.gid", &s.gid.to_string());
    set("waydroid.host_data_path", &paths::data(&crate::images::instance_of(c.active.as_deref().unwrap_or(""))).to_string_lossy());
    set("waydroid.background_start", "true");
    set("waydroid.xdg_runtime_dir", paths::CONTAINER_XDG_RUNTIME_DIR);
    set("waydroid.pulse_runtime_path", &format!("{}/pulse", paths::CONTAINER_XDG_RUNTIME_DIR));
    set("waydroid.wayland_display", paths::CONTAINER_WAYLAND_DISPLAY);
    set("waydroid.stub_sensors_hal", "1");
    // persist-свойство: значение из /data перекрывает это; при загрузке демон его выравнивает (android::apply_window_mode)
    set("persist.waydroid.multi_windows", if c.multi_windows { "true" } else { "false" });
    if c.dpi > 0 {
        set("ro.sf.lcd_density", &c.dpi.to_string());
    }
    for (k, v) in platform_props().into_iter().chain(c.properties.clone()) {
        p.retain(|(pk, _)| *pk != k);
        p.push((k, v));
    }
    p.into_iter().map(|(k, v)| format!("{k}={v}\n")).collect()
}

/// Свойства из `/etc/syndroid/*.prop` (по алфавиту файлов; `#` — комментарий).
fn platform_props() -> Vec<(String, String)> {
    let mut files: Vec<_> = std::fs::read_dir(paths::PLATFORM_PROPS)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "prop"))
        .collect();
    files.sort();
    files
        .iter()
        .filter_map(|f| std::fs::read_to_string(f).ok())
        .flat_map(|s| {
            s.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .filter_map(|l| l.split_once('=').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
                .collect::<Vec<_>>()
        })
        .collect()
}

