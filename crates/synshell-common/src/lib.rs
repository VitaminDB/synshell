//! Общая часть synshell: конфигурация (`config.toml`), действия
//! (`Action` — то, что вешается на сочетания клавиш, кнопки панели и
//! команду `synwm msg action`), IPC-протокол между композитором,
//! оболочкой и настройками.

pub mod action;
pub mod app_theme;
pub mod config;
pub mod config_edit;
#[cfg(feature = "drives")]
pub mod drives;
pub mod haptics;
pub mod ipc;
pub mod link;
pub mod mime;
pub mod paths;
pub mod theme;
#[cfg(feature = "thumbs")]
pub mod thumbs;
pub mod wallpaper;
pub mod watch;
pub mod xdg;

pub use action::Action;
pub use config::Config;

#[cfg(feature = "i18n")]
pub mod i18n;

/// Перевод строк библиотек: `synshell_common::t!`, `tn!`, `n_!` (см. `synshell_tr`).
pub use synshell_tr::{decimal, decimal_separator, n_, t, tn};

/// Язык программы без syngui (synwm, CLI): `SYNSHELL_LANG`, `[general]
/// language`, иначе `LC_ALL`/`LC_MESSAGES`/`LANG`; каталоги — свои `i18n/*.lang`
/// и общий (строки synshell-common, synsystem, synmodem).
pub fn tr_init(catalogs: &[&str]) {
    let (cfg, _) = Config::load();
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    let lang = env("SYNSHELL_LANG")
        .or_else(|| Some(cfg.general.language.trim().to_string()).filter(|l| !l.is_empty()))
        .or_else(|| env("LC_ALL"))
        .or_else(|| env("LC_MESSAGES"))
        .or_else(|| env("LANG"))
        .unwrap_or_else(|| "en".into());
    let mut all: Vec<&str> = vec![include_str!("../i18n/en.lang")];
    all.extend_from_slice(catalogs);
    synshell_tr::init("ru", &lang, &all);
}
