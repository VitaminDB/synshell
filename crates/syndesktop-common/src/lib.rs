//! Общая часть syndesktop: конфигурация (`config.toml`), действия
//! (`Action` — то, что вешается на сочетания клавиш, кнопки панели и
//! команду `syndesktop msg action`), IPC-протокол между композитором,
//! оболочкой и настройками.

pub mod action;
pub mod config;
pub mod config_edit;
pub mod ipc;
pub mod paths;
pub mod theme;
pub mod watch;

pub use action::Action;
pub use config::Config;
