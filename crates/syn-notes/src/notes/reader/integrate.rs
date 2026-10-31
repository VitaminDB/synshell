//! Книга как страница проекта: импорт файла, состояние в
//! `notes/objects/<id страницы>.book.json`, читалка на весь экран, кнопки
//! шапки, обложка карточки и подробности для библиотеки.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use syngui::prelude::*;
use syngui::widgets::{Image, ImageFit};

use super::book::{Book, BookState, Cover};
use super::view::{header_actions, reader_view, Env, Reader};
use crate::context::AppCtx;
use crate::notes::library::Library;
use crate::notes::project::{self, PageKind};
use crate::notes::{autosave, media, NotesCtx};

type W = Box<dyn Widget>;

pub const OBJECT_KIND: &str = "book";

fn state_cache() -> &'static Mutex<HashMap<(PathBuf, String), BookState>> {
    static C: OnceLock<Mutex<HashMap<(PathBuf, String), BookState>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// Состояние книги страницы: из памяти или из проекта.
pub fn load_state(project_path: &Path, page_id: &str) -> Option<BookState> {
    let key = (project_path.to_path_buf(), page_id.to_string());
    if let Some(s) = state_cache().lock().unwrap().get(&key) {
        return Some(s.clone());
    }
    let json = project::read_text(project_path, &project::object_path(OBJECT_KIND, page_id))?;
    let s = BookState::parse(&json).ok()?;
    state_cache().lock().unwrap().insert(key, s.clone());
    Some(s)
}

/// Записать состояние (очередь автосохранения проекта).
pub fn save_state(project_path: &Path, page_id: &str, s: &BookState) {
    state_cache().lock().unwrap().insert((project_path.to_path_buf(), page_id.to_string()), s.clone());
    autosave::queue_bytes_to(project_path, &project::object_path(OBJECT_KIND, page_id), s.serialize().into_bytes());
}

/// Книги, разобранные при импорте (чтобы не разбирать второй раз при открытии).
fn preloaded() -> &'static Mutex<HashMap<String, Arc<Book>>> {
    static P: OnceLock<Mutex<HashMap<String, Arc<Book>>>> = OnceLock::new();
    P.get_or_init(Default::default)
}

/// Открытая читалка (одна на окно) — общая для вида и кнопок шапки.
fn current() -> &'static Mutex<Option<(PathBuf, String, Reader)>> {
    static R: OnceLock<Mutex<Option<(PathBuf, String, Reader)>>> = OnceLock::new();
    R.get_or_init(Default::default)
}

fn reader_for(ctx: NotesCtx, lib: Library, page_id: &str) -> Option<Reader> {
    let project = ctx.project_path.get_untracked();
    if let Some((p, id, r)) = current().lock().unwrap().as_ref() {
        if *p == project && id == page_id {
            return Some(r.clone());
        }
    }
    let state = load_state(&project, page_id)?;
    let book = preloaded().lock().unwrap().remove(page_id);
    let load: Option<Box<dyn FnOnce() -> std::result::Result<Book, String> + Send>> = if book.is_none() {
        let project = project.clone();
        let st = state.clone();
        Some(Box::new(move || Book::open_asset(&project, &st)))
    } else {
        None
    };
    let save_project = project.clone();
    let save_id = page_id.to_string();
    let env = Env {
        key: page_id.to_string(),
        on_state: Arc::new(move |s| save_state(&save_project, &save_id, s)),
        on_close: Some(Arc::new(move || {
            lib.back(ctx);
        })),
    };
    let mut state = state;
    state.opened_at = now();
    let r = Reader::new(env, state, book, load);
    *current().lock().unwrap() = Some((project, page_id.to_string(), r.clone()));
    Some(r)
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Читалка страницы-книги на весь экран.
pub fn page_view(ctx: NotesCtx, lib: Library, page_id: &str, kind: PageKind) -> Option<W> {
    if kind != PageKind::Book {
        return None;
    }
    let r = reader_for(ctx, lib, page_id)?;
    Some(Box::new(reader_view(r)))
}

/// Кнопки шапки: оглавление, поиск, «Аа», пометки пером, закладка.
pub fn actions(ctx: NotesCtx, lib: Library, page_id: &str, kind: PageKind) -> Option<W> {
    if kind != PageKind::Book {
        return None;
    }
    let r = reader_for(ctx, lib, page_id)?;
    Some(Box::new(header_actions(r)))
}

/// «Автор · 37 %».
pub fn details(ctx: NotesCtx, page_id: &str, kind: PageKind) -> Option<String> {
    if kind != PageKind::Book {
        return None;
    }
    let s = load_state(&ctx.project_path.get_untracked(), page_id)?;
    let pct = format!("{}%", (s.progress * 100.0).round() as i32);
    Some(if s.author.trim().is_empty() { pct } else { format!("{} · {pct}", s.author.trim()) })
}

// ─────────────────────────── Обложки ───────────────────────────

fn sha_of(asset: &str) -> Option<&str> {
    media::parse_asset_url(asset).and_then(|n| n.split('.').next())
}

/// Файл обложки в кэше вложений проекта (`cover-<sha>.png|jpg|…`).
fn cover_file(project_path: &Path, asset: &str) -> Option<PathBuf> {
    let sha = sha_of(asset)?;
    let dir = media::assets_cache_dir(project_path);
    for ext in ["png", "jpg", "gif", "webp", "bmp", "none"] {
        let p = dir.join(format!("cover-{sha}.{ext}"));
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn write_cover(project_path: &Path, asset: &str, cover: Option<Cover>) -> Option<PathBuf> {
    let sha = sha_of(asset)?;
    let dir = media::assets_cache_dir(project_path);
    std::fs::create_dir_all(&dir).ok()?;
    let (ext, bytes) = match cover {
        Some(Cover::Encoded(b)) => {
            let ext = match b.get(..4) {
                Some([0x89, b'P', ..]) => "png",
                Some([0xff, 0xd8, ..]) => "jpg",
                Some([b'G', b'I', ..]) => "gif",
                Some([b'R', b'I', ..]) => "webp",
                Some([b'B', b'M', ..]) => "bmp",
                _ => "png",
            };
            (ext, (*b).clone())
        }
        Some(Cover::Rgba { width, height, rgba }) => ("png", encode_png(width, height, &rgba)),
        // Обложки нет — метка, чтобы не искать снова.
        None => ("none", Vec::new()),
    };
    let path = dir.join(format!("cover-{sha}.{ext}"));
    crate::fsutil::write_atomic(&path, &bytes).ok()?;
    Some(path)
}

thread_local! {
    static COVER_REV: std::cell::OnceCell<RwSignal<u64>> = const { std::cell::OnceCell::new() };
}

fn cover_rev() -> RwSignal<u64> {
    COVER_REV.with(|c| *c.get_or_init(|| use_signal(0)))
}

fn generating() -> &'static Mutex<HashSet<String>> {
    static G: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    G.get_or_init(Default::default)
}

/// Обложка карточки 3:4: картинка книги, первая страница PDF или набранная
/// «титульная страница» (название и автор) для книг без картинок.
pub fn thumbnail(ctx: NotesCtx, page_id: &str, kind: PageKind) -> Option<W> {
    if kind != PageKind::Book {
        return None;
    }
    let project = ctx.project_path.get_untracked();
    let state = load_state(&project, page_id)?;
    let rev = cover_rev();
    Some(Box::new(Reactive::new(move || -> Vec<W> {
        let _ = rev.get();
        match cover_file(&project, &state.asset) {
            Some(p) if p.extension().is_some_and(|e| e != "none") => {
                vec![Box::new(Stack::new().fit(StackFit::Expand).child(Image::new(p.to_string_lossy().to_string()).fit(ImageFit::Cover)).class("reader-cover"))]
            }
            found => {
                if found.is_none() && !state.asset.is_empty() && generating().lock().unwrap().insert(state.asset.clone()) {
                    // Старый проект без обложки в кэше — сделать в фоне.
                    let (project, st) = (project.clone(), state.clone());
                    std::thread::spawn(move || {
                        let cover = Book::open_asset(&project, &st).ok().and_then(|b| b.cover());
                        write_cover(&project, &st.asset, cover);
                        syngui::async_runtime::run_on_main_thread(move || rev.update(|v| *v += 1));
                    });
                }
                vec![Box::new(title_cover(&state))]
            }
        }
    })))
}

/// Набранная обложка: название, автор, формат.
fn title_cover(s: &BookState) -> impl Widget {
    DecoratedBox::new().class("reader-cover reader-cover-typeset").child(
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .main_axis_alignment(MainAxisAlignment::Center)
            .child(Text::new(s.title.clone()).max_lines(5).elide(Elide::End).class("reader-cover-title"))
            .child(Text::new(s.author.clone()).max_lines(2).elide(Elide::End).class("reader-cover-author"))
            .child(Text::new(s.format.to_uppercase()).class("reader-cover-format")),
    )
}

// ─────────────────────────── Импорт ───────────────────────────

/// Импорт файла книги в проект: разбор в фоне, вложение, страница-книга с
/// текстом для поиска (автор, оглавление) — и открыть её.
pub fn import_book(ctx: NotesCtx, lib: Library, parent: Option<String>, path: PathBuf) {
    let project = ctx.project_path.get_untracked();
    std::thread::Builder::new()
        .name("reader-import".into())
        .spawn(move || {
            let res = (|| -> std::result::Result<(Book, Vec<u8>, Option<Cover>), String> {
                let data = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                let name = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                let book = Book::open_bytes(&name, data.clone())?;
                let cover = book.cover();
                Ok((book, data, cover))
            })();
            syngui::async_runtime::run_on_main_thread(move || match res {
                Ok((book, data, cover)) => {
                    let name = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                    let mut state = book.new_state(&name);
                    state.asset = media::ingest_bytes(&project, data, book.format.ext());
                    let (p2, a2) = (project.clone(), state.asset.clone());
                    generating().lock().unwrap().insert(a2.clone());
                    let rev = cover_rev();
                    std::thread::spawn(move || {
                        write_cover(&p2, &a2, cover);
                        syngui::async_runtime::run_on_main_thread(move || rev.update(|v| *v += 1));
                    });
                    let title = if state.title.trim().is_empty() { name.clone() } else { state.title.clone() };
                    let id = ctx.insert_page_of_kind(parent.as_deref(), None, &title, PageKind::Book, false);
                    save_state(&project, &id, &state);
                    ctx.set_page_markdown(&id, &book_markdown(&state));
                    preloaded().lock().unwrap().insert(id.clone(), Arc::new(book));
                    lib.open(ctx, &id);
                }
                Err(e) => {
                    log::warn!("reader: импорт {}: {e}", path.display());
                    use_context::<AppCtx>().notifications.error(tr!("notes.reader.import_failed", error = e));
                }
            });
        })
        .ok();
}

/// Текст страницы-книги для поиска и ассистента.
pub fn book_markdown(s: &BookState) -> String {
    let mut md = String::new();
    if !s.author.trim().is_empty() {
        md.push_str(&format!("**{}:** {}\n\n", tr!("notes.reader.author"), s.author.trim()));
    }
    md.push_str(&format!("**{}:** {} ({})\n\n", tr!("notes.reader.file"), s.file_name, s.format.to_uppercase()));
    if !s.toc.is_empty() {
        md.push_str(&format!("## {}\n\n", tr!("notes.reader.toc")));
        for t in s.toc.iter().take(300) {
            let indent = "  ".repeat(t.level.saturating_sub(1) as usize);
            md.push_str(&format!("{indent}- {}\n", t.title.replace(['[', ']'], "")));
        }
    }
    md
}

// ─────────────────────────── PNG ───────────────────────────

/// PNG без сжатия (deflate «stored») — для обложек PDF в кэше.
pub fn encode_png(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut c = 0xffff_ffffu32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    fn chunk(out: &mut Vec<u8>, ty: &[u8], data: &[u8]) {
        out.extend((data.len() as u32).to_be_bytes());
        let mut c = ty.to_vec();
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend(crc32(&c).to_be_bytes());
    }
    let mut raw = Vec::with_capacity((w as usize * 4 + 1) * h as usize);
    for y in 0..h as usize {
        raw.push(0);
        raw.extend_from_slice(&rgba[y * w as usize * 4..(y + 1) * w as usize * 4]);
    }
    let mut z = vec![0x78, 0x01];
    for (i, block) in raw.chunks(65535).enumerate() {
        let last = (i + 1) * 65535 >= raw.len();
        z.push(last as u8);
        z.extend((block.len() as u16).to_le_bytes());
        z.extend((!(block.len() as u16)).to_le_bytes());
        z.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend(((b << 16) | a).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend(w.to_be_bytes());
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_header_and_size() {
        let px = vec![255u8; 3 * 2 * 4];
        let png = encode_png(3, 2, &px);
        assert_eq!(crate::notes::reader::model::image_size(&png), Some((3, 2)));
    }

    #[test]
    fn markdown_for_search() {
        let mut s = BookState { author: "Лев Толстой".into(), file_name: "war.fb2".into(), format: "fb2".into(), ..Default::default() };
        s.toc.push(super::super::book::TocItem { title: "Часть 1".into(), level: 1, loc: Default::default() });
        let md = book_markdown(&s);
        assert!(md.contains("Лев Толстой") && md.contains("- Часть 1"));
    }

    #[test]
    fn content_kinds() {
        let b = Book::open_bytes("a.txt", b"x".to_vec()).unwrap();
        assert!(matches!(b.content, super::super::book::Content::Flow(_)));
    }
}
