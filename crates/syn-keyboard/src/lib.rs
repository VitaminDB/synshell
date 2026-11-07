//! Экранная клавиатура synshell: раскладки ([`layout`]) и виджет ([`ui`]). Демон `synkeyboard`
//! (src/main.rs) рисует её своей поверхностью и шлёт клавиши композитору; [`ui::embedded`] —
//! та же клавиатура внутри чужой поверхности (экран блокировки), набранное — обработчику.

pub mod layout;
pub mod ui;

pub use ui::{embedded, keys, overlay, Typed, STYLE};
