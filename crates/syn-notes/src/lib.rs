//! syn-notes — заметки synshell (перенесены из synthos целиком, см.
//! `docs/NOTES.md`): страницы-холсты, канбан, Гант, mindmap, календарь,
//! графики, журнал, вложения и напоминания в одном `.syn`-проекте;
//! инструмент `notes` для ассистента.

pub mod app;
pub mod attach;
pub mod audio_decode;
pub mod components;
pub mod config;
pub mod context;
pub mod fsutil;
pub mod i18n;
pub mod icons;
pub mod notes;
pub mod paths;
pub mod rail;
pub mod time;
pub mod tool;

pub use notes::NotesCtx;

/// Стили заметок для программы, встраивающей их (имена классов и
/// переменных — как в synthos).
pub const NOTES_MSS: &str = include_str!("../styles/notes.mss");
