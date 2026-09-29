//! Общая часть synshell: конфигурация (`config.toml`), действия
//! (`Action` — то, что вешается на сочетания клавиш, кнопки панели и
//! команду `synwm msg action`), IPC-протокол между композитором,
//! оболочкой и настройками.

pub mod action;
pub mod app_theme;
pub mod config;
pub mod config_edit;
pub mod ipc;
pub mod mime;
pub mod paths;
pub mod theme;
pub mod watch;
pub mod xdg;

pub use action::Action;
pub use config::Config;
