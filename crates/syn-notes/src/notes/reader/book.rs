//! Книга: открытие из байтов/файла/вложения проекта и её состояние
//! ([`BookState`] — `notes/objects/<id>.book.json`).

use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::formats::{parse_flow, Format};
use super::layout::TextSettings;
use super::model::{FlowDoc, Pos};
use super::pdf::PdfWorker;

/// Место в книге: страница PDF или позиция в тексте.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "kind")]
pub enum Location {
    Pdf { page: usize },
    Flow { block: u32, ch: u32 },
}

impl Default for Location {
    fn default() -> Self {
        Location::Flow { block: 0, ch: 0 }
    }
}

impl Location {
    pub fn flow(p: Pos) -> Self {
        Location::Flow { block: p.block, ch: p.ch }
    }
    pub fn pos(&self) -> Pos {
        match *self {
            Location::Flow { block, ch } => Pos { block, ch },
            Location::Pdf { .. } => Pos::START,
        }
    }
    pub fn page(&self) -> usize {
        match *self {
            Location::Pdf { page } => page,
            Location::Flow { .. } => 0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Bookmark {
    pub loc: Location,
    /// Подпись: начало текста страницы.
    pub label: String,
    #[serde(default)]
    pub progress: f32,
    #[serde(default)]
    pub created: i64,
}

/// Выделение маркером (с заметкой). Для PDF `start`/`end` — страница, а
/// символы — в `pdf_chars`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Highlight {
    pub id: String,
    pub start: Location,
    pub end: Location,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pdf_chars: Option<(usize, usize)>,
    pub text: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// `#rrggbb`; пусто — маркер по умолчанию.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub color: String,
    #[serde(default)]
    pub created: i64,
}

/// Масштаб страницы PDF.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PdfFit {
    /// Страница целиком.
    #[default]
    Page,
    /// По ширине (страница режется на части по высоте).
    Width,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TocItem {
    pub title: String,
    pub level: u8,
    pub loc: Location,
}

/// Состояние книги в проекте. Новые поля — только с `#[serde(default)]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BookState {
    pub version: u32,
    /// Файл книги во вложениях проекта: `asset:<sha256>.<ext>`.
    pub asset: String,
    pub file_name: String,
    /// `pdf` | `epub` | `fb2` | `fb2.zip` | `txt` | `md`.
    pub format: String,
    pub title: String,
    pub author: String,
    pub pos: Location,
    /// Прочитано 0…1 (для библиотеки без открытия книги).
    pub progress: f32,
    pub bookmarks: Vec<Bookmark>,
    pub highlights: Vec<Highlight>,
    pub settings: TextSettings,
    pub pdf_fit: PdfFit,
    /// Разворот (две страницы) на широком экране.
    pub spread: bool,
    /// Полная очистка E Ink каждые N перелистываний (0 — никогда).
    pub refresh_every: u32,
    /// Оглавление (кэш для показа без разбора книги).
    pub toc: Vec<TocItem>,
    pub opened_at: i64,
}

impl Default for BookState {
    fn default() -> Self {
        Self {
            version: 1,
            asset: String::new(),
            file_name: String::new(),
            format: String::new(),
            title: String::new(),
            author: String::new(),
            pos: Location::default(),
            progress: 0.0,
            bookmarks: Vec::new(),
            highlights: Vec::new(),
            settings: TextSettings::default(),
            pdf_fit: PdfFit::Page,
            spread: false,
            refresh_every: 8,
            toc: Vec::new(),
            opened_at: 0,
        }
    }
}

impl BookState {
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
    pub fn serialize(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into())
    }
    pub fn has_bookmark(&self, loc: Location) -> bool {
        self.bookmarks.iter().any(|b| b.loc == loc)
    }
}

pub enum Content {
    Flow(Arc<FlowDoc>),
    Pdf(PdfWorker),
}

pub struct Book {
    pub format: Format,
    pub content: Content,
    pub title: String,
    pub author: String,
}

/// Картинка обложки: сжатые байты (EPUB/FB2) или растр первой страницы PDF.
pub enum Cover {
    Encoded(Arc<Vec<u8>>),
    Rgba { width: u32, height: u32, rgba: Arc<Vec<u8>> },
}

impl Book {
    pub fn open_bytes(name: &str, data: Vec<u8>) -> Result<Book, String> {
        let format = Format::detect(name, &data);
        let stem = Path::new(name).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let stem = stem.split('.').next().unwrap_or("").to_string();
        if format == Format::Pdf {
            let w = PdfWorker::open(data)?;
            let title = if w.info.title.trim().is_empty() { stem } else { w.info.title.trim().to_string() };
            let author = w.info.author.trim().to_string();
            return Ok(Book { format, content: Content::Pdf(w), title, author });
        }
        let doc = parse_flow(format, &data)?;
        let title = if doc.meta.title.trim().is_empty() { stem } else { doc.meta.title.trim().to_string() };
        let author = doc.meta.author.clone();
        Ok(Book { format, content: Content::Flow(Arc::new(doc)), title, author })
    }

    pub fn open_path(path: &Path) -> Result<Book, String> {
        let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Book::open_bytes(&path.to_string_lossy(), data)
    }

    /// Книга из вложения проекта (`state.asset`).
    pub fn open_asset(project_path: &Path, state: &BookState) -> Result<Book, String> {
        let file = crate::notes::media::asset_file(project_path, &state.asset).ok_or_else(|| format!("нет вложения {}", state.asset))?;
        let data = std::fs::read(&file).map_err(|e| e.to_string())?;
        let name = if state.file_name.is_empty() { file.to_string_lossy().to_string() } else { state.file_name.clone() };
        Book::open_bytes(&name, data)
    }

    pub fn toc(&self) -> Vec<TocItem> {
        match &self.content {
            Content::Flow(d) => d.toc.iter().map(|t| TocItem { title: t.title.clone(), level: t.level, loc: Location::flow(t.pos) }).collect(),
            Content::Pdf(w) => w.info.toc.iter().map(|t| TocItem { title: t.title.clone(), level: t.level, loc: Location::Pdf { page: t.page } }).collect(),
        }
    }

    pub fn cover(&self) -> Option<Cover> {
        match &self.content {
            Content::Flow(d) => {
                let id = d.meta.cover.clone().or_else(|| {
                    // Без явной обложки — первая картинка в начале книги.
                    d.blocks.iter().take(8).find_map(|b| match &b.kind {
                        super::model::BlockKind::Image { id, .. } => Some(id.clone()),
                        _ => None,
                    })
                })?;
                d.images.get(&id).map(|i| Cover::Encoded(i.data.clone()))
            }
            Content::Pdf(w) => {
                let (pw, _) = *w.info.sizes.first()?;
                let b = w.render_blocking(0, 240.0 / pw.max(1.0))?;
                Some(Cover::Rgba { width: b.width, height: b.height, rgba: b.rgba })
            }
        }
    }

    /// Новое состояние книги (до записи вложения).
    pub fn new_state(&self, file_name: &str) -> BookState {
        BookState {
            file_name: file_name.to_string(),
            format: self.format.ext().to_string(),
            title: self.title.clone(),
            author: self.author.clone(),
            pos: match self.content {
                Content::Pdf(_) => Location::Pdf { page: 0 },
                Content::Flow(_) => Location::default(),
            },
            toc: self.toc(),
            ..Default::default()
        }
    }
}

/// Импорт файла книги в проект: вложение + новое состояние.
pub fn import_file(project_path: &Path, file: &Path) -> Result<(Book, BookState), String> {
    let data = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let name = file.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let book = Book::open_bytes(&name, data.clone())?;
    let mut state = book.new_state(&name);
    state.asset = crate::notes::media::ingest_bytes(project_path, data, book.format.ext());
    Ok((book, state))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::reader::formats::tests::{sample_epub, SAMPLE_FB2};

    #[test]
    fn state_roundtrip_and_defaults() {
        let mut s = BookState::default();
        s.pos = Location::Pdf { page: 3 };
        s.bookmarks.push(Bookmark { loc: Location::flow(Pos { block: 2, ch: 5 }), label: "x".into(), ..Default::default() });
        let back = BookState::parse(&s.serialize()).unwrap();
        assert_eq!(back, s);
        // Старый/неполный файл читается с умолчаниями.
        let old = BookState::parse(r#"{"asset":"asset:x.epub","pos":{"kind":"flow","block":1,"ch":2}}"#).unwrap();
        assert_eq!(old.pos, Location::Flow { block: 1, ch: 2 });
        assert_eq!(old.settings, TextSettings::default());
    }

    #[test]
    fn open_formats() {
        let b = Book::open_bytes("x.epub", sample_epub()).unwrap();
        assert_eq!(b.title, "Тестовая книга");
        assert_eq!(b.toc().len(), 2);
        assert!(matches!(b.cover(), Some(Cover::Encoded(_))));
        let b = Book::open_bytes("x.fb2", SAMPLE_FB2.as_bytes().to_vec()).unwrap();
        assert_eq!(b.author, "Лев Толстой");
        let st = b.new_state("x.fb2");
        assert_eq!(st.format, "fb2");
        assert!(!st.toc.is_empty());
        let b = Book::open_bytes("заметки.txt", "просто текст".as_bytes().to_vec()).unwrap();
        assert_eq!(b.title, "заметки");
    }
}
