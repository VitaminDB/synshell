//! syn-video-editor — ИИ-видеоредактор synshell. Ядро без окна: документ
//! ([`doc`]), состояние и отмена ([`editor`]), команды ([`cmd`]), файл
//! проекта ([`store`]), встроенный агент ([`agent`]).

pub mod agent;
pub mod cmd;
pub mod doc;
pub mod editor;
pub mod store;
pub mod ui;
