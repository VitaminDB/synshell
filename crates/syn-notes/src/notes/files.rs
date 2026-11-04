//! Редактор файлов: текстовые документы с диска (TXT, Markdown, исходники,
//! конфиги, журналы) открываются в заметках на редактирование и
//! сохраняются обратно в тот же файл — в исходной кодировке (UTF-8,
//! UTF-16, CP1251, KOI8-R), с теми же концами строк и BOM.
//!
//! ```text
//! [←] имя.md ●  ~/Документы             [Вид|Код] [↶][↷] [🔍] [⏎] [💾] [⋮]
//! [имя.md ●] [notes.txt] [main.rs] …                          — вкладки
//! [Найти…      ] [Заменить на…   ] [Далее] [Заменить всё]      — Ctrl+F
//! ┌──────────────────────────────────────────────────────────────────┐
//! │ Markdown — оформленный вид (DocumentEditor) или исходник;          │
//! │ остальное — редактор кода с подсветкой, номерами строк и поиском   │
//! └──────────────────────────────────────────────────────────────────┘
//! Стр 12, стлб 4 · 120 строк          Markdown · UTF-8 · LF
//! ```
//!
//! Открытые файлы живут в контексте [`Files`] (переживают пересоздание
//! окна); экран — [`super::library::Screen::Files`]. Книжные форматы
//! (PDF, EPUB, FB2) сюда не идут — их открывает читалка.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use encoding_rs::Encoding;
use syngui::input::Key;
use syngui::prelude::*;
use syngui::widgets::input::code_editor::languages::detect_by_path;
use syngui::widgets::input::code_editor::{CodeEditor, CursorInfo, EditorCommand, EditorPersistedState, TextEdit};
use syngui::widgets::input::document_editor::{DocOp, DocumentEditor, DocumentEditorHandle};
use syngui::widgets::overlay::menu::MenuItem;
use syngui::widgets::{EventHook, KeyReply, Page, TextField, ToolButton};

use crate::context::AppCtx;
use crate::icons::*;

use super::library::{Library, Screen};
use super::state::NotesCtx;

type W = Box<dyn Widget>;

/// Предел размера: больше — не текстовый документ, а данные.
const MAX_BYTES: u64 = 64 * 1024 * 1024;

// ─── Кодировка ──────────────────────────────────────────────────────────────

/// Как файл лежал на диске — так же и сохраняется.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub encoding: &'static Encoding,
    pub bom: bool,
    pub crlf: bool,
}

impl Default for Format {
    fn default() -> Self {
        Self { encoding: encoding_rs::UTF_8, bom: false, crlf: false }
    }
}

/// Текст файла и его формат. Ошибка — файл двоичный.
pub fn decode(data: &[u8]) -> std::result::Result<(String, Format), String> {
    let (encoding, bom_len) = super::reader::formats::detect_encoding(data);
    let body = &data[bom_len..];
    // Двоичные данные: NUL в начале (кроме UTF-16, там нули — половинки ASCII).
    let utf16 = encoding == encoding_rs::UTF_16LE || encoding == encoding_rs::UTF_16BE;
    if !utf16 && body[..body.len().min(8192)].contains(&0) {
        return Err(tr!("notes.files.binary"));
    }
    let text = encoding.decode_without_bom_handling(body).0.into_owned();
    let crlf = text.contains("\r\n");
    let text = if crlf { text.replace("\r\n", "\n") } else { text };
    Ok((text, Format { encoding, bom: bom_len > 0, crlf }))
}

/// Байты для записи. Символы, которых нет в кодировке файла, переводят
/// его в UTF-8 (второе значение — новый формат).
pub fn encode(text: &str, f: Format) -> (Vec<u8>, Format) {
    let text = if f.crlf { text.replace('\n', "\r\n") } else { text.to_string() };
    let mut out = Vec::with_capacity(text.len() + 3);
    if f.encoding == encoding_rs::UTF_16LE || f.encoding == encoding_rs::UTF_16BE {
        let le = f.encoding == encoding_rs::UTF_16LE;
        if f.bom {
            out.extend_from_slice(if le { &[0xFF, 0xFE] } else { &[0xFE, 0xFF] });
        }
        for u in text.encode_utf16() {
            out.extend_from_slice(&if le { u.to_le_bytes() } else { u.to_be_bytes() });
        }
        return (out, f);
    }
    if f.encoding == encoding_rs::UTF_8 {
        if f.bom {
            out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
        }
        out.extend_from_slice(text.as_bytes());
        return (out, f);
    }
    let (bytes, _, lossy) = f.encoding.encode(&text);
    if lossy {
        let nf = Format { encoding: encoding_rs::UTF_8, bom: false, crlf: f.crlf };
        return (text.into_bytes(), nf);
    }
    (bytes.into_owned(), f)
}

fn encoding_label(f: Format) -> String {
    let name = match f.encoding.name() {
        "UTF-8" => "UTF-8",
        "UTF-16LE" => "UTF-16 LE",
        "UTF-16BE" => "UTF-16 BE",
        "windows-1251" => "CP1251",
        "KOI8-R" => "KOI8-R",
        other => other,
    };
    if f.bom && f.encoding == encoding_rs::UTF_8 {
        format!("{name} BOM")
    } else {
        name.to_string()
    }
}

/// Файл Markdown (оформленный вид доступен).
pub fn is_markdown(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    matches!(ext.as_str(), "md" | "markdown" | "mdown" | "mkd" | "mkdn" | "mdwn")
}

// ─── Открытый файл ──────────────────────────────────────────────────────────

pub struct FileBuf {
    pub id: u64,
    /// `None` — новый файл, ещё не сохранён.
    pub path: RwSignal<Option<PathBuf>>,
    pub format: Mutex<Format>,
    pub markdown: RwSignal<bool>,
    /// Текст исходника (вид «Код»), обновляется на каждую правку.
    text: Mutex<String>,
    /// Текст, совпадающий с диском.
    saved: Mutex<String>,
    pub dirty: RwSignal<bool>,
    /// Markdown в оформленном виде (иначе исходник).
    pub visual: RwSignal<bool>,
    pub handle: DocumentEditorHandle,
    /// Markdown модели оформленного вида сразу после загрузки — от него
    /// считается «изменён» (разбор и запись Markdown могут переставить
    /// пробелы и маркеры без правки пользователя).
    visual_base: Mutex<String>,
    /// Был ли текст изменён до перехода в оформленный вид.
    visual_carry: Mutex<bool>,
    pub epoch: RwSignal<u64>,
    pub cmd: RwSignal<Option<EditorCommand>>,
    pub state: RwSignal<EditorPersistedState>,
    pub cursor: RwSignal<(usize, usize, usize)>,
}

impl FileBuf {
    fn new(id: u64, path: Option<PathBuf>, text: String, format: Format, markdown: bool) -> Self {
        let handle = DocumentEditorHandle::new();
        let lines = text.split('\n').count().max(1);
        Self {
            id,
            path: use_signal(path),
            format: Mutex::new(format),
            markdown: use_signal(markdown),
            text: Mutex::new(text.clone()),
            saved: Mutex::new(text),
            dirty: use_signal(false),
            visual: use_signal(false),
            handle,
            visual_base: Mutex::new(String::new()),
            visual_carry: Mutex::new(false),
            epoch: use_signal(0),
            cmd: use_signal(None),
            state: use_signal(EditorPersistedState::default()),
            cursor: use_signal((0, 0, lines)),
        }
    }

    pub fn name(&self) -> String {
        match self.path.get_untracked() {
            Some(p) => p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
            None => tr!("notes.files.untitled"),
        }
    }

    /// Текущий текст (из оформленного вида — его Markdown).
    pub fn text(&self) -> String {
        if self.visual.get_untracked() {
            self.handle.serialize()
        } else {
            self.text.lock().unwrap().clone()
        }
    }

    fn set_source(&self, t: &str) {
        *self.text.lock().unwrap() = t.to_string();
        let d = *self.saved.lock().unwrap() != t;
        if self.dirty.get_untracked() != d {
            self.dirty.set(d);
        }
    }

    fn visual_changed(&self) {
        let d = *self.visual_carry.lock().unwrap() || *self.visual_base.lock().unwrap() != self.handle.serialize();
        if self.dirty.get_untracked() != d {
            self.dirty.set(d);
        }
    }

    /// Переключить Markdown между оформленным видом и исходником.
    pub fn set_visual(&self, on: bool) {
        if self.visual.get_untracked() == on || (on && !self.markdown.get_untracked()) {
            return;
        }
        if on {
            let src = self.text.lock().unwrap().clone();
            self.handle.replace_markdown(&src);
            *self.visual_base.lock().unwrap() = self.handle.serialize();
            *self.visual_carry.lock().unwrap() = self.dirty.get_untracked();
        } else {
            let md = self.handle.serialize();
            let changed = *self.visual_base.lock().unwrap() != md;
            if changed {
                self.set_source(&md);
                self.cmd.set(Some(EditorCommand::Reload(md)));
            }
        }
        self.visual.set(on);
    }

    /// Текст записан на диск.
    fn mark_saved(&self, text: &str) {
        *self.saved.lock().unwrap() = text.to_string();
        *self.text.lock().unwrap() = text.to_string();
        if self.visual.get_untracked() {
            *self.visual_base.lock().unwrap() = text.to_string();
            *self.visual_carry.lock().unwrap() = false;
        }
        self.dirty.set(false);
    }

    /// Заменить текст (перечитан с диска).
    fn reload_text(&self, text: String, format: Format) {
        *self.format.lock().unwrap() = format;
        *self.saved.lock().unwrap() = text.clone();
        *self.text.lock().unwrap() = text.clone();
        if self.visual.get_untracked() {
            self.handle.replace_markdown(&text);
            *self.visual_base.lock().unwrap() = self.handle.serialize();
            *self.visual_carry.lock().unwrap() = false;
            self.epoch.update(|e| *e += 1);
        } else {
            self.cmd.set(Some(EditorCommand::Reload(text)));
        }
        self.dirty.set(false);
    }

    fn undo(&self, redo: bool) {
        if self.visual.get_untracked() {
            self.handle.queue_op(if redo { DocOp::Redo } else { DocOp::Undo });
            self.epoch.update(|e| *e += 1);
        } else {
            self.cmd.set(Some(if redo { EditorCommand::Redo } else { EditorCommand::Undo }));
        }
    }
}

// ─── Контекст ───────────────────────────────────────────────────────────────

/// Открытые файлы окна заметок.
#[derive(Clone, Copy)]
pub struct Files {
    pub list: RwSignal<Vec<Arc<FileBuf>>>,
    pub current: RwSignal<Option<u64>>,
    pub wrap: RwSignal<bool>,
    pub find_open: RwSignal<bool>,
    pub find: RwSignal<String>,
    pub replace: RwSignal<String>,
    /// Закрыть файл с несохранёнными правками? (id файла).
    pub confirm_close: RwSignal<Option<u64>>,
    next: RwSignal<u64>,
}

impl Files {
    pub fn install() {
        provide_context(Files {
            list: use_signal(Vec::new()),
            current: use_signal(None),
            wrap: use_signal(true),
            find_open: use_signal(false),
            find: use_signal(String::new()),
            replace: use_signal(String::new()),
            confirm_close: use_signal(None),
            next: use_signal(1),
        });
    }

    pub fn get() -> Files {
        use_context::<Files>()
    }

    fn take_id(&self) -> u64 {
        let id = self.next.get_untracked();
        self.next.set(id + 1);
        id
    }

    pub fn buf(&self, id: u64) -> Option<Arc<FileBuf>> {
        self.list.get_untracked().into_iter().find(|b| b.id == id)
    }

    pub fn current_buf(&self) -> Option<Arc<FileBuf>> {
        let id = self.current.get_untracked()?;
        self.buf(id)
    }

    fn show(&self, id: u64) {
        self.current.set(Some(id));
        use_context::<Library>().screen.set(Screen::Files);
    }

    /// Открыть файл с диска (уже открытый — показать).
    pub fn open_path(&self, path: &Path) -> std::result::Result<(), String> {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if let Some(b) = self.list.get_untracked().iter().find(|b| b.path.get_untracked().as_deref() == Some(path.as_path())) {
            self.show(b.id);
            return Ok(());
        }
        let meta = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if meta.is_dir() {
            return Err(tr!("notes.files.is_dir", path = crate::paths::pretty(&path)));
        }
        if meta.len() > MAX_BYTES {
            return Err(tr!("notes.files.too_big", path = crate::paths::pretty(&path)));
        }
        let data = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let (text, format) = decode(&data).map_err(|e| format!("{}: {e}", crate::paths::pretty(&path)))?;
        let md = is_markdown(&path);
        let buf = Arc::new(FileBuf::new(self.take_id(), Some(path), text, format, md));
        if md {
            buf.set_visual(true);
        }
        let id = buf.id;
        self.list.update(|l| l.push(buf));
        self.show(id);
        Ok(())
    }

    /// Новый файл (Markdown — сразу в оформленном виде).
    pub fn new_file(&self, markdown: bool) {
        let buf = Arc::new(FileBuf::new(self.take_id(), None, String::new(), Format::default(), markdown));
        if markdown {
            buf.set_visual(true);
        }
        let id = buf.id;
        self.list.update(|l| l.push(buf));
        self.show(id);
    }

    /// Диалог «Открыть файл».
    pub fn open_dialog(&self) {
        let files = *self;
        let dir = self.current_buf().and_then(|b| b.path.get_untracked()).and_then(|p| p.parent().map(Path::to_path_buf));
        let mut dialog = rfd::AsyncFileDialog::new().set_title(tr!("notes.files.open"));
        if let Some(d) = dir {
            dialog = dialog.set_directory(d);
        }
        syngui::async_runtime::spawn(async move {
            let Some(files_picked) = dialog.pick_files().await else { return };
            let paths: Vec<PathBuf> = files_picked.iter().map(|f| f.path().to_path_buf()).collect();
            syngui::async_runtime::run_on_main_thread(move || {
                for p in paths {
                    open_any(&p, Some(files));
                }
            });
        });
    }

    /// Сохранить (новый файл — через «Сохранить как»). `then` — после
    /// удачной записи (закрыть вкладку и т. п.).
    pub fn save(&self, id: u64, then: Option<Box<dyn FnOnce() + Send>>) {
        let Some(buf) = self.buf(id) else { return };
        let Some(path) = buf.path.get_untracked() else {
            self.save_as(id, then);
            return;
        };
        if write(&buf, &path) {
            if let Some(f) = then {
                f();
            }
        }
    }

    pub fn save_as(&self, id: u64, then: Option<Box<dyn FnOnce() + Send>>) {
        let Some(buf) = self.buf(id) else { return };
        let files = *self;
        let cur = buf.path.get_untracked();
        let name = cur
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| if buf.markdown.get_untracked() { format!("{}.md", tr!("notes.files.untitled")) } else { format!("{}.txt", tr!("notes.files.untitled")) });
        let dir = cur
            .as_ref()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Documents")).filter(|p| p.is_dir()))
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from));
        let mut dialog = rfd::AsyncFileDialog::new().set_title(tr!("notes.files.save_as")).set_file_name(name);
        if let Some(d) = dir {
            dialog = dialog.set_directory(d);
        }
        syngui::async_runtime::spawn(async move {
            let Some(f) = dialog.save_file().await else { return };
            let path = f.path().to_path_buf();
            syngui::async_runtime::run_on_main_thread(move || {
                let Some(buf) = files.buf(id) else { return };
                if write(&buf, &path) {
                    buf.path.set(Some(path.clone()));
                    let md = is_markdown(&path);
                    if !md && buf.visual.get_untracked() {
                        buf.set_visual(false);
                    }
                    buf.markdown.set(md);
                    if let Some(f) = then {
                        f();
                    }
                }
            });
        });
    }

    /// Закрыть вкладку; с несохранёнными правками — спросить.
    pub fn close(&self, id: u64, force: bool) {
        let Some(buf) = self.buf(id) else { return };
        if buf.dirty.get_untracked() && !force {
            self.confirm_close.set(Some(id));
            return;
        }
        let list = self.list.get_untracked();
        let idx = list.iter().position(|b| b.id == id).unwrap_or(0);
        self.list.update(|l| l.retain(|b| b.id != id));
        if self.current.get_untracked() == Some(id) {
            let rest = self.list.get_untracked();
            let next = rest.get(idx.min(rest.len().saturating_sub(1))).map(|b| b.id);
            self.current.set(next);
            if next.is_none() {
                let lib = use_context::<Library>();
                if lib.screen.get_untracked() == Screen::Files {
                    lib.screen.set(Screen::Library);
                }
            }
        }
    }

    /// Перечитать с диска (правки теряются).
    pub fn reload(&self, id: u64) {
        let Some(buf) = self.buf(id) else { return };
        let Some(path) = buf.path.get_untracked() else { return };
        match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|d| decode(&d)) {
            Ok((text, f)) => buf.reload_text(text, f),
            Err(e) => notify_error(format!("{}: {e}", crate::paths::pretty(&path))),
        }
    }

    /// Сделать страницей текущего проекта заметок.
    pub fn to_notes(&self, id: u64) {
        let Some(buf) = self.buf(id) else { return };
        let ctx = use_context::<NotesCtx>();
        if let Err(e) = ctx.ensure_project() {
            notify_error(e.message());
            return;
        }
        let title = buf
            .path
            .get_untracked()
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
            .unwrap_or_else(|| tr!("notes.untitled"));
        let text = buf.text();
        let md = if buf.markdown.get_untracked() { text } else { format!("```\n{}\n```\n", text.trim_end_matches('\n')) };
        let page = ctx.insert_page_of_kind(None, None, &title, super::project::PageKind::Doc, false);
        ctx.set_page_markdown(&page, &md);
        use_context::<Library>().open(ctx, &page);
        use_context::<AppCtx>().notifications.success(tr!("notes.files.added", title = title));
    }
}

fn notify_error(msg: String) {
    use_context::<AppCtx>().notifications.error(msg);
}

/// Записать текст файла; ошибка — тостом.
fn write(buf: &FileBuf, path: &Path) -> bool {
    let text = buf.text();
    let f = *buf.format.lock().unwrap();
    let (bytes, nf) = encode(&text, f);
    match crate::fsutil::write_user_file(path, &bytes) {
        Ok(()) => {
            if nf != f {
                *buf.format.lock().unwrap() = nf;
                use_context::<AppCtx>().notifications.info(tr!("notes.files.to_utf8", from = encoding_label(f)));
            }
            buf.mark_saved(&text);
            true
        }
        Err(e) => {
            notify_error(tr!("notes.files.save_failed", path = crate::paths::pretty(path), error = e.to_string()));
            false
        }
    }
}

// ─── Что чем открывать ──────────────────────────────────────────────────────

/// Книга для читалки (иначе — текст в редактор).
pub fn is_book(path: &Path) -> bool {
    let name = path.file_name().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    if [".pdf", ".epub", ".fb2", ".fb2.zip", ".fbz"].iter().any(|e| name.ends_with(e)) {
        return true;
    }
    // По содержимому: PDF и ZIP (EPUB) без расширения.
    let mut head = [0u8; 4];
    use std::io::Read;
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).is_ok() && (&head == b"%PDF" || &head[..2] == b"PK")
}

/// Открыть файл, пришедший извне (Проводник, `syn-notes ФАЙЛ`): проект
/// `.syn`, книга — в читалку, остальное — в редактор.
pub fn open_any(path: &Path, files: Option<Files>) {
    let files = files.unwrap_or_else(Files::get);
    if is_book(path) {
        let ctx = use_context::<NotesCtx>();
        if let Err(e) = ctx.ensure_project() {
            notify_error(e.message());
            return;
        }
        super::reader::open_book_file(ctx, use_context::<Library>(), path.to_path_buf());
        return;
    }
    if let Err(e) = files.open_path(path) {
        notify_error(e);
    }
}

// ─── Вид ────────────────────────────────────────────────────────────────────

/// Экран редактора файлов.
pub fn view(wide: bool) -> W {
    let files = Files::get();
    let lib = use_context::<Library>();
    let body = Reactive::new(move || -> Vec<W> {
        let Some(id) = files.current.get() else { return vec![] };
        let Some(buf) = files.buf(id) else { return vec![] };
        vec![editor(files, buf)]
    });
    let find = Reactive::new(move || -> Vec<W> {
        if files.find_open.get() && files.current.get().is_some() {
            vec![find_bar(files)]
        } else {
            vec![]
        }
    });
    let tabs = Reactive::new(move || -> Vec<W> {
        let list = files.list.get();
        if list.len() < 2 {
            return vec![];
        }
        let cur = files.current.get();
        vec![tabs_row(files, &list, cur)]
    });
    let col = Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("lib-open")
        .child(bar(files, lib, wide))
        .child(tabs)
        .child(find)
        .child(DecoratedBox::new().class("grow lib-open-body notes-file-body").clip(true).child(Stack::new().fit(StackFit::Expand).child(body)))
        .child(status_bar(files));
    Box::new(
        Stack::new()
            .fit(StackFit::Expand)
            .clip(false)
            .child(EventHook::new().on_key_down(move |k, m| hotkey(files, k, m)).child(col))
            .child(confirm_dialog(files)),
    )
}

/// Сочетания вне редактора кода (оформленный вид их не знает).
fn hotkey(files: Files, k: Key, m: syngui::input::Modifiers) -> KeyReply {
    let ctrl = m.ctrl || m.meta;
    if !ctrl || m.alt {
        return KeyReply::Ignore;
    }
    let Some(id) = files.current.get_untracked() else { return KeyReply::Ignore };
    match k {
        Key::S if m.shift => files.save_as(id, None),
        Key::S => files.save(id, None),
        Key::O => files.open_dialog(),
        Key::N => files.new_file(false),
        Key::W => files.close(id, false),
        Key::H => files.find_open.set(true),
        Key::F if files.buf(id).is_some_and(|b| b.visual.get_untracked()) => files.find_open.set(true),
        _ => return KeyReply::Ignore,
    }
    KeyReply::Handled
}

fn editor(files: Files, buf: Arc<FileBuf>) -> W {
    let id = buf.id;
    if buf.visual.get() {
        let _ = buf.epoch.get();
        let b2 = buf.clone();
        let ed = DocumentEditor::new()
            .inline_toolbar(true)
            .markdown(buf.text())
            .handle(&buf.handle)
            .model_epoch(buf.epoch.get_untracked())
            .fill_height(true)
            .on_change(move || b2.visual_changed())
            .class("notes-editor notes-file-doc");
        return Box::new(Page::new().vertical().child(ed).class("notes-editor-page"));
    }
    let wrap = files.wrap.get();
    let b_change = buf.clone();
    let b_cursor = buf.clone();
    let mut ed = CodeEditor::new()
        .text(buf.text.lock().unwrap().clone())
        .soft_wrap(wrap)
        .tab_width(4)
        .on_change(move |c| b_change.set_source(c.full_text))
        .on_save(move |_| files.save(id, None))
        .on_cursor(move |c: CursorInfo| b_cursor.cursor.set((c.line, c.col, c.total_lines)))
        .command_signal(buf.cmd)
        .state_signal(buf.state)
        .class("notes-file-code");
    let lang = buf.path.get_untracked().as_deref().and_then(detect_by_path);
    if let Some(l) = lang {
        ed = ed.language(l);
    }
    // Код и Markdown-исходник — моноширинным, простой текст — обычным шрифтом.
    if lang.is_some_and(|l| l != syngui::widgets::input::code_editor::syntax::Language::PlainText) {
        ed = ed.class("mono");
    }
    // Редактору кода нужна колонка с растягиваемой ячейкой — так он
    // получает высоту окна, а не бесконечную.
    Box::new(Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).child(DecoratedBox::new().class("grow").child(ed)))
}

fn bar(files: Files, lib: Library, wide: bool) -> impl Widget {
    let title = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let Some(id) = files.current.get() else { return vec![] };
        let Some(buf) = files.buf(id) else { return vec![] };
        let path = buf.path.get();
        let dirty = buf.dirty.get();
        let name = buf.name();
        let sub = path
            .as_ref()
            .and_then(|p| p.parent())
            .map(crate::paths::pretty)
            .unwrap_or_else(|| tr!("notes.files.not_saved"));
        vec![Box::new(
            Column::new()
                .gap(1.0)
                .child(Text::new(if dirty { format!("{name} ●") } else { name }).max_lines(1).elide(Elide::End).class("lib-open-title"))
                .child(Text::new(sub).max_lines(1).elide(Elide::Middle).class("lib-open-sub")),
        )]
    });
    let actions = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let Some(id) = files.current.get() else { return vec![] };
        let Some(buf) = files.buf(id) else { return vec![] };
        let md = buf.markdown.get();
        let visual = buf.visual.get();
        let dirty = buf.dirty.get();
        let wrap = files.wrap.get();
        let find = files.find_open.get();
        let mut row = Row::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Center);
        if md {
            let b = buf.clone();
            row = row.child(
                ToolButton::new(if visual { MI_CODE } else { MI_VISIBILITY })
                    .tooltip(if visual { tr!("notes.files.source") } else { tr!("notes.files.visual") })
                    .on_click(move || b.set_visual(!visual))
                    .class("lib-icon-btn"),
            );
        }
        let (bu, br) = (buf.clone(), buf.clone());
        row = row
            .child(ToolButton::new(MI_UNDO).tooltip(tr!("notes.undo")).on_click(move || bu.undo(false)).class("lib-icon-btn"))
            .child(ToolButton::new(MI_REDO).tooltip(tr!("notes.redo")).on_click(move || br.undo(true)).class("lib-icon-btn"));
        if wide {
            row = row.child(
                ToolButton::new(MI_SEARCH)
                    .tooltip(tr!("notes.files.find"))
                    .on_click(move || files.find_open.set(!find))
                    .class(if find { "lib-icon-btn on" } else { "lib-icon-btn" }),
            );
            if !visual {
                row = row.child(
                    ToolButton::new(MI_WRAP_TEXT)
                        .tooltip(tr!("notes.files.wrap"))
                        .on_click(move || files.wrap.set(!wrap))
                        .class(if wrap { "lib-icon-btn on" } else { "lib-icon-btn" }),
                );
            }
        }
        row = row.child(
            ToolButton::new(MI_SAVE)
                .tooltip(tr!("notes.files.save"))
                .disabled(!dirty && buf.path.get_untracked().is_some())
                .on_click(move || files.save(id, None))
                .class("lib-icon-btn"),
        );
        row = row.child(file_menu(files, id, md, visual, wrap, wide));
        vec![Box::new(row)]
    });
    DecoratedBox::new().class("lib-openbar").child(
        Row::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(
                ToolButton::new(MI_ARROW_BACK)
                    .tooltip(tr!("notes.lib.back"))
                    .on_click(move || {
                        lib.back(use_context::<NotesCtx>());
                    })
                    .class("lib-back"),
            )
            .child(DecoratedBox::new().class("grow").child(title))
            .child(actions),
    )
}

fn file_menu(files: Files, id: u64, md: bool, visual: bool, wrap: bool, wide: bool) -> W {
    let mut items = vec![
        MenuItem::new("new", tr!("notes.files.new_text")).icon(MI_NOTE_ADD),
        MenuItem::new("new_md", tr!("notes.files.new_md")).icon(MI_DESCRIPTION),
        MenuItem::new("open", tr!("notes.files.open")).icon(MI_FOLDER_OPEN),
        MenuItem::separator(),
        MenuItem::new("save_as", tr!("notes.files.save_as")).icon(MI_SAVE_AS),
        MenuItem::new("reload", tr!("notes.files.reload")).icon(MI_AUTORENEW),
        MenuItem::new("to_notes", tr!("notes.files.to_notes")).icon(MI_EDIT_NOTE),
    ];
    if !wide {
        items.push(MenuItem::separator());
        items.push(MenuItem::new("find", tr!("notes.files.find")).icon(MI_SEARCH));
        if !visual {
            items.push(MenuItem::new("wrap", if wrap { tr!("notes.files.nowrap") } else { tr!("notes.files.wrap") }).icon(MI_WRAP_TEXT));
        }
    }
    if md {
        items.push(MenuItem::new("toggle", if visual { tr!("notes.files.source") } else { tr!("notes.files.visual") }).icon(if visual { MI_CODE } else { MI_VISIBILITY }));
    }
    items.push(MenuItem::separator());
    items.push(MenuItem::new("close", tr!("app.close")).icon(MI_CLOSE));
    Box::new(super::library::menu_button(MI_MORE_VERT, tr!("notes.lib.more"), "lib-icon-btn", items, move |key: &str| match key {
        "new" => files.new_file(false),
        "new_md" => files.new_file(true),
        "open" => files.open_dialog(),
        "save_as" => files.save_as(id, None),
        "reload" => files.reload(id),
        "to_notes" => files.to_notes(id),
        "find" => files.find_open.set(true),
        "wrap" => files.wrap.set(!wrap),
        "toggle" => {
            if let Some(b) = files.buf(id) {
                b.set_visual(!visual);
            }
        }
        "close" => files.close(id, false),
        _ => {}
    }))
}

fn tabs_row(files: Files, list: &[Arc<FileBuf>], cur: Option<u64>) -> W {
    let mut row = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center);
    for b in list {
        let id = b.id;
        let dirty = b.dirty.get();
        let name = b.name();
        let active = cur == Some(id);
        row = row.child(
            DecoratedBox::new().class(if active { "notes-file-tab active" } else { "notes-file-tab" }).child(
                Row::new()
                    .gap(4.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(
                        syngui::widgets::GestureDetector::new()
                            .cursor(syngui::input::CursorIcon::Pointer)
                            .on_click(move || files.current.set(Some(id)))
                            .child(Text::new(if dirty { format!("{name} ●") } else { name }).max_lines(1).class("notes-file-tab-text")),
                    )
                    .child(ToolButton::new(MI_CLOSE).tooltip(tr!("app.close")).on_click(move || files.close(id, false)).class("notes-file-tab-close")),
            ),
        );
    }
    Box::new(DecoratedBox::new().class("notes-file-tabs").child(ScrollView::new().horizontal().child(row)))
}

/// Поиск и замена.
fn find_bar(files: Files) -> W {
    let run_find = move |forward: bool| {
        let Some(buf) = files.current_buf() else { return };
        let q = files.find.get_untracked();
        if q.is_empty() {
            return;
        }
        if buf.visual.get_untracked() {
            // В оформленном виде — к исходнику, где виден найденный текст.
            buf.set_visual(false);
        }
        let text = buf.text.lock().unwrap().clone();
        let (line, col, _) = buf.cursor.get_untracked();
        let here = offset_of(&text, line, col);
        let here = here.min(text.len());
        let hit = if forward {
            text[here..].find(&q).map(|i| i + here).or_else(|| text.find(&q))
        } else {
            let end = here.saturating_sub(1);
            let end = (0..=end).rev().find(|&i| text.is_char_boundary(i)).unwrap_or(0);
            text[..end].rfind(&q).or_else(|| text.rfind(&q))
        };
        match hit {
            Some(i) => {
                let (l, c) = line_col(&text, i + if forward { q.len() } else { 0 });
                buf.cmd.set(Some(EditorCommand::SetCursor { line: l, col: c }));
            }
            None => use_context::<AppCtx>().notifications.info(tr!("notes.files.not_found")),
        }
    };
    let replace_all = move || {
        let Some(buf) = files.current_buf() else { return };
        let q = files.find.get_untracked();
        if q.is_empty() {
            return;
        }
        let r = files.replace.get_untracked();
        if buf.visual.get_untracked() {
            buf.set_visual(false);
        }
        let text = buf.text.lock().unwrap().clone();
        let edits: Vec<TextEdit> = text
            .match_indices(&q)
            .map(|(i, m)| {
                let (sl, sc) = line_col(&text, i);
                let (el, ec) = line_col(&text, i + m.len());
                TextEdit { start_line: sl, start_col: sc, end_line: el, end_col: ec, text: r.clone() }
            })
            .collect();
        let n = edits.len();
        if n == 0 {
            use_context::<AppCtx>().notifications.info(tr!("notes.files.not_found"));
            return;
        }
        buf.cmd.set(Some(EditorCommand::ApplyEdits(edits)));
        use_context::<AppCtx>().notifications.info(trn!("notes.files.replaced", n as i64, n = n));
    };
    let next = move || run_find(true);
    Box::new(
        DecoratedBox::new().class("notes-file-find").child(
            Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(
                    DecoratedBox::new().class("grow").child(
                        TextField::new()
                            .text(files.find.get_untracked())
                            .placeholder(tr!("notes.files.find_ph"))
                            .on_change(move |t| files.find.set(t.to_string()))
                            .on_submit(move |_| next())
                            .autofocus(true)
                            .class("lib-search"),
                    ),
                )
                .child(ToolButton::new(MI_EXPAND_LESS).tooltip(tr!("notes.files.prev")).on_click(move || run_find(false)).class("lib-icon-btn small"))
                .child(ToolButton::new(MI_EXPAND_MORE).tooltip(tr!("notes.files.next")).on_click(move || run_find(true)).class("lib-icon-btn small"))
                .child(
                    DecoratedBox::new().class("grow").child(
                        TextField::new()
                            .text(files.replace.get_untracked())
                            .placeholder(tr!("notes.files.replace_ph"))
                            .on_change(move |t| files.replace.set(t.to_string()))
                            .on_submit(move |_| replace_all())
                            .class("lib-search"),
                    ),
                )
                .child(Button::new(tr!("notes.files.replace_all")).on_click(move || replace_all()).class("lib-dialog-btn"))
                .child(ToolButton::new(MI_CLOSE).tooltip(tr!("app.close")).on_click(move || files.find_open.set(false)).class("lib-icon-btn small")),
        ),
    )
}

/// Байтовое смещение строки `line` и символа `col`.
fn offset_of(text: &str, line: usize, col: usize) -> usize {
    let mut off = 0;
    for (i, l) in text.split('\n').enumerate() {
        if i == line {
            return off + l.char_indices().nth(col).map(|(b, _)| b).unwrap_or(l.len());
        }
        off += l.len() + 1;
    }
    text.len()
}

/// Строка и символ байтового смещения.
fn line_col(text: &str, off: usize) -> (usize, usize) {
    let before = &text[..off.min(text.len())];
    let line = before.matches('\n').count();
    let start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    (line, before[start..].chars().count())
}

fn status_bar(files: Files) -> impl Widget {
    Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let Some(id) = files.current.get() else { return vec![] };
        let Some(buf) = files.buf(id) else { return vec![] };
        let visual = buf.visual.get();
        let _ = buf.dirty.get();
        let path = buf.path.get();
        let f = *buf.format.lock().unwrap();
        let left = if visual {
            tr!("notes.files.visual_mode")
        } else {
            let (l, c, n) = buf.cursor.get();
            tr!("notes.files.pos", line = l + 1, col = c + 1, lines = n)
        };
        let lang = if buf.markdown.get() {
            "Markdown".to_string()
        } else {
            path.as_deref().and_then(detect_by_path).map(|l| format!("{l:?}")).unwrap_or_else(|| tr!("notes.files.plain"))
        };
        let lang = if lang == "PlainText" { tr!("notes.files.plain") } else { lang };
        let right = format!("{lang} · {} · {}", encoding_label(f), if f.crlf { "CRLF" } else { "LF" });
        vec![Box::new(
            DecoratedBox::new().class("notes-file-status").child(
                Row::new()
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(DecoratedBox::new().class("grow").child(Text::new(left).max_lines(1).class("notes-file-status-text")))
                    .child(Text::new(right).max_lines(1).class("notes-file-status-text")),
            ),
        )]
    })
}

/// «Сохранить изменения?» при закрытии вкладки.
fn confirm_dialog(files: Files) -> impl Widget {
    let open = use_signal(false);
    create_effect(move || {
        let v = files.confirm_close.get().is_some();
        if open.get_untracked() != v {
            open.set(v);
        }
    });
    create_effect(move || {
        if !open.get() && files.confirm_close.get_untracked().is_some() {
            files.confirm_close.set(None);
        }
    });
    Portal::new().is_open(open).modal(true).backdrop(true).child(Reactive::new(move || -> Vec<W> {
        let Some(id) = files.confirm_close.get() else { return vec![] };
        let Some(buf) = files.buf(id) else { return vec![] };
        let name = buf.name();
        vec![Box::new(
            DecoratedBox::new().class("lib-dialog").child(
                Column::new()
                    .gap(14.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(Text::new(tr!("notes.files.ask_save", name = name)).class("lib-dialog-title"))
                    .child(
                        Row::new()
                            .gap(8.0)
                            .main_axis_alignment(MainAxisAlignment::End)
                            .child(Button::new(tr!("app.cancel")).class("lib-dialog-btn").on_click(move || files.confirm_close.set(None)))
                            .child(Button::new(tr!("notes.files.discard")).class("lib-dialog-btn").on_click(move || {
                                files.confirm_close.set(None);
                                files.close(id, true);
                            }))
                            .child(Button::new(tr!("notes.files.save")).class("lib-dialog-btn primary").on_click(move || {
                                files.confirm_close.set(None);
                                files.save(id, Some(Box::new(move || {
                                    syngui::async_runtime::run_on_main_thread(move || Files::get().close(id, true));
                                })));
                            })),
                    ),
            ),
        )]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_crlf_roundtrip() {
        let (t, f) = decode(b"a\r\nb\r\n").unwrap();
        assert_eq!(t, "a\nb\n");
        assert!(f.crlf);
        assert_eq!(encode(&t, f).0, b"a\r\nb\r\n");
    }

    #[test]
    fn cp1251_roundtrip() {
        let src = encoding_rs::WINDOWS_1251.encode("Привет, мир! Это проверка текста.").0.into_owned();
        let (t, f) = decode(&src).unwrap();
        assert_eq!(t, "Привет, мир! Это проверка текста.");
        assert_eq!(f.encoding, encoding_rs::WINDOWS_1251);
        assert_eq!(encode(&t, f).0, src);
        // Символ вне CP1251 — запись в UTF-8.
        let (b, nf) = encode("мир ✓", f);
        assert_eq!(nf.encoding, encoding_rs::UTF_8);
        assert_eq!(b, "мир ✓".as_bytes());
    }

    #[test]
    fn utf16_bom_roundtrip() {
        let mut src = vec![0xFF, 0xFE];
        for u in "Да\n".encode_utf16() {
            src.extend_from_slice(&u.to_le_bytes());
        }
        let (t, f) = decode(&src).unwrap();
        assert_eq!(t, "Да\n");
        assert_eq!(encode(&t, f).0, src);
    }

    #[test]
    fn utf8_bom_kept() {
        let src = b"\xEF\xBB\xBFhello".to_vec();
        let (t, f) = decode(&src).unwrap();
        assert_eq!(t, "hello");
        assert_eq!(encode(&t, f).0, src);
    }

    #[test]
    fn binary_rejected() {
        assert!(decode(b"\x7fELF\x00\x00\x01").is_err());
    }

    #[test]
    fn offsets() {
        let t = "аб\nвгд\nе";
        assert_eq!(offset_of(t, 1, 2), "аб\nвг".len());
        assert_eq!(line_col(t, "аб\nвг".len()), (1, 2));
    }
}
