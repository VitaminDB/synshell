//! Цвета программ под тему (`[appearance] app_colors`): файлы GTK, Qt/KDE и
//! GIMP пишет `syndesktop_common::app_theme`, здесь — запуск в фоне и
//! оповещение уже запущенных программ.

use std::collections::HashMap;
use std::sync::Mutex;

use syndesktop_common::{app_theme, Config};

/// Одна запись за раз: перечитывания конфига могут идти подряд.
static BUSY: Mutex<()> = Mutex::new(());

pub fn sync(cfg: &Config) {
    // Отладочные запуски с отдельным конфигом не трогают настройки программ.
    if !cfg.appearance.app_colors || std::env::var_os("SYNDESKTOP_CONFIG_DIR").is_some() {
        return;
    }
    let cfg = cfg.clone();
    std::thread::spawn(move || {
        let _g = BUSY.lock().unwrap_or_else(|e| e.into_inner());
        let ch = app_theme::apply(&cfg);
        if !ch.any() {
            return;
        }
        log::info!("цвета программ обновлены по теме (GTK: {}, Qt/KDE: {}, GIMP: {})", ch.gtk, ch.kde, ch.gimp);
        if ch.gtk {
            let scheme = if cfg.appearance.is_dark() { "prefer-dark" } else { "prefer-light" };
            let _ = std::process::Command::new("gsettings")
                .args(["set", "org.gnome.desktop.interface", "color-scheme", scheme])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
        if ch.kde {
            if let Err(e) = notify_kde(&cfg) {
                log::debug!("оповещение KDE о смене цветов: {e}");
            }
        }
    });
}

/// Программы KDE/Qt перечитывают палитру по `KGlobalSettings.notifyChange`,
/// kded6 (он же пересобирает `colors.css` GTK) — по `kconfig.notify`.
fn notify_kde(cfg: &Config) -> zbus::Result<()> {
    let conn = zbus::blocking::Connection::session()?;
    let groups: HashMap<String, Vec<Vec<u8>>> = app_theme::kde_groups(&app_theme::Colors::from_config(cfg))
        .into_iter()
        .map(|(g, kv)| (g, kv.into_iter().map(|(k, _)| k.as_bytes().to_vec()).collect()))
        .chain(std::iter::once(("General".to_string(), vec![b"ColorScheme".to_vec()])))
        .collect();
    conn.emit_signal(None::<&str>, "/kdeglobals", "org.kde.kconfig.notify", "ConfigChanged", &(groups,))?;
    // 0 — PaletteChanged.
    conn.emit_signal(None::<&str>, "/KGlobalSettings", "org.kde.KGlobalSettings", "notifyChange", &(0i32, 0i32))?;
    Ok(())
}
