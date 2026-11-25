//! syn-video-editor — ИИ-видеоредактор synshell. Ядро без окна: документ
//! ([`doc`]), состояние и отмена ([`editor`]), команды ([`cmd`]), файл
//! проекта ([`store`]), встроенный агент ([`agent`]).

pub mod agent;
pub mod blocks;
pub mod cmd;
pub mod doc;
pub mod editor;
pub mod fonts;
pub mod framegraph;
pub mod fusion;
pub mod index;
pub mod layout;
pub mod lottie;
pub mod motion;
pub mod recipe;
pub mod render;
pub mod script;
pub mod store;
pub mod ui;
