//! synpass — «Пароли»: хранилище паролей synshell под мастер-паролем.
//!
//! Записи синхронизируются с соединёнными устройствами через synlink
//! (`sync.rs`), а скопированный пароль сразу вставляется и там — общим
//! буфером обмена synlink (`[link] clipboard`); через заданное время буфер
//! очищается здесь и, следом, там. Телефон — стек «список → запись»,
//! рабочий стол — две колонки.

mod sync;
mod ui;
mod vault;

use serde::{Deserialize, Serialize};
use synshell_common::Config;
use syngui::prelude::*;

/// `~/.config/synshell/passwords.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Заблокировать после стольких минут без действий; 0 — не блокировать.
    pub autolock_min: u32,
    /// Очистить буфер обмена через столько секунд после копирования; 0 — не очищать.
    pub clear_secs: u32,
    pub generator: vault::GenOpts,
}

impl Default for Prefs {
    fn default() -> Self {
        Self { autolock_min: 5, clear_secs: 30, generator: vault::GenOpts::default() }
    }
}

impl Prefs {
    fn path() -> std::path::PathBuf {
        synshell_common::paths::config_dir().join("passwords.toml")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path()).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(s) = toml::to_string_pretty(self) {
            let p = Self::path();
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(p, s);
        }
    }
}

fn main() {
    synshell_common::i18n::init(&synshell_common::catalogs!("../i18n"));
    let (cfg, _) = Config::load();
    let mss = theme(&cfg);
    let args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(args.first().map(String::as_str), Some("-h" | "--help")) {
        println!("{}", t!("synpass [--screenshot СНИМОК.png [--size ШxВ] [--scale N] [--view locked|list|edit|big|settings]]"));
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--screenshot") {
        shot(&args, args.get(i + 1).cloned().unwrap_or_else(|| "synpass.png".into()), &mss);
        return;
    }
    let win = use_signal(syngui::window::WindowState::default());
    App::new()
        .title(t!("Пароли"))
        .app_id("synpass")
        .size(980, 680)
        .min_size(340, 480)
        .frameless()
        .transparent(true)
        .background(Color::from_srgb(0, 0, 0, 0.0))
        .with_window_state(win)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_live_styles(synshell_common::live::styles(mss, theme))
        .run(move |_| ui::root(win));
}

fn theme(cfg: &Config) -> String {
    let a = &cfg.appearance;
    let p = a.palette();
    let dark = a.is_dark();
    synshell_common::style::AppStyle::new(cfg, "synpass")
        .vars(&[
            ("input-bg", if dark { p.bg.mix(p.surface, 0.35) } else { p.surface }.hex()),
            ("card", if dark { p.surface.mix(p.fg, 0.04) } else { p.surface }.hex()),
            ("scrim", (if dark { "#000000b8" } else { "#00000080" }).to_string()),
        ])
        .base(include_str!("../styles/synpass.mss"))
        .build()
}

/// `--screenshot`: снимок без окна на демо-хранилище в памяти (настоящее
/// хранилище не читается, синхронизации нет) — см. [`ui::shot_root`].
fn shot(args: &[String], out: String, mss: &str) {
    let mut size = (980u32, 680u32);
    let mut scale = 1.0f64;
    let mut view = "list".to_string();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--size" => {
                if let Some((w, h)) = it.next().and_then(|s| s.split_once('x')) {
                    size = (w.parse().unwrap_or(size.0), h.parse().unwrap_or(size.1));
                }
            }
            "--scale" => scale = it.next().and_then(|s| s.parse().ok()).unwrap_or(scale),
            "--view" => view = it.next().cloned().unwrap_or(view),
            _ => {}
        }
    }
    match syngui::shot::render_png(move || ui::shot_root(&view), mss, size, scale, &out) {
        Ok((w, h)) => println!("{out}: {w}x{h}"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
