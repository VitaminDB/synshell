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
pub use synshell_tr::{n_, t, tn};
