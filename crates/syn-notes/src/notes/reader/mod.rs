//! Читалка книг (тип страницы «Книга»): PDF, EPUB, FB2 (и .fb2.zip), TXT,
//! Markdown. Ориентир — Onyx NeoReader: постраничное листание без анимаций,
//! меню по тапу в центре, оглавление, закладки, поиск, выделения маркером
//! с заметками, настройки текста, пометки пером поверх страницы.
//!
//! - [`book`] — [`Book`] (открытие из байтов/файла/вложения проекта),
//!   [`BookState`] (позиция, закладки, выделения, настройки) — хранится как
//!   `notes/objects/<id>.book.json`, файл книги — вложение `asset:<sha>.<ext>`;
//! - [`formats`] + [`dom`] — разбор EPUB/FB2/TXT/MD в [`model::FlowDoc`];
//! - [`layout`] — вёрстка в страницы, переносы ([`hyph`]);
//! - [`pdf`] — poppler-glib через dlopen, рендер в своём потоке;
//! - [`view`] — экран читалки ([`reader_view`]), слой пометок пером
//!   ([`set_overlay_factory`]);
//! - [`integrate`] — книга как страница проекта: импорт, вид, кнопки шапки,
//!   обложка и подробности для библиотеки.

pub mod book;
pub mod dom;
pub mod formats;
pub mod hyph;
pub mod integrate;
pub mod layout;
pub mod model;
pub mod pdf;
pub mod view;
#[cfg(all(test, feature = "testing"))]
mod view_tests;

pub use book::{import_file, Book, BookState, Bookmark, Content, Cover, Highlight, Location, PdfFit, TocItem};
pub use formats::Format;
pub use layout::TextSettings;
pub use integrate::{actions, details, import_book, page_view, thumbnail};
pub use view::{reader_view, set_overlay_factory, OverlayFactory, OverlayPage, Reader};
/// Стили читалки (подключаются в теме окна).
pub const READER_MSS: &str = include_str!("../../../styles/reader.mss");
