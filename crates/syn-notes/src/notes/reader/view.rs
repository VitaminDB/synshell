//! Экран читалки: страница (или разворот), подвал с номером страницы,
//! меню по тапу в центре, панели (оглавление/закладки/выделения, поиск,
//! настройки текста), всплывающая панель выделения.
//!
//! Жесты как в NeoReader: тап слева (30 %) — назад, справа — вперёд, в
//! центре — меню; свайп влево/вправо — листание; удержание — выделение
//! слова, ведение — расширение; тап по ссылке — переход. Клавиши: →, ↓,
//! PageDown, Пробел — вперёд; ←, ↑, PageUp — назад; Esc — закрыть меню.
//! Без анимаций; на E Ink — полная очистка экрана каждые N перелистываний.

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use syngui::core::{Color, Point, Rect, Size};
use syngui::input::{CursorIcon, Event, EventResult, Key, MouseButton};
use syngui::layout::Constraints;
use syngui::mss::{ComputedStyle, MssFields, StyleValue, TextAlign, TextDecoration};
use syngui::prelude::*;
use syngui::render::DisplayList;
use syngui::widget::context::{EventContext, UpdateContext};
use syngui::widget::{DirtyFlags, Element, ElementId, ElementTree, LayoutHint, StyledWidget};
use syngui::containers::Positioned;
use syngui::widgets::{GestureDetector, Image, ImageFit, Slider, TextField, ToolButton};

use super::book::{Book, BookState, Bookmark, Content, Highlight, Location, PdfFit};
use super::layout::{Item, Measure, Page, Paginator, Run, TmMeasure};
use super::model::{FlowDoc, Pos, LINK};
use super::pdf::{PdfBitmap, PdfLink, PdfLinkTarget, PdfText};
use crate::icons::*;

type W = Box<dyn Widget>;

// Глифы Material, которых нет в общем наборе значков.
const MI_BOOKMARK: &str = "\u{e866}";
const MI_BOOKMARK_BORDER: &str = "\u{e867}";
const MI_TOC: &str = "\u{e8de}";
const MI_FORMAT_SIZE: &str = "\u{e245}";
const MI_HIGHLIGHT: &str = "\u{e25f}";

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::new(Point::new(x, y), Size::new(w, h))
}

// ─────────────────────────── Подключение пометок пером ───────────────────────────

/// Страница, поверх которой просят слой пометок.
#[derive(Clone, Debug, PartialEq)]
pub struct OverlayPage {
    /// Страница проекта (книга).
    pub book_page: String,
    /// Ключ страницы книги: `pdf-<n>` (страница PDF, с 0) или
    /// `flow-<блок>-<символ>` (начало страницы при текущих настройках
    /// текста: при смене шрифта пометки остаются у начала той же позиции,
    /// но страница может начаться в другом месте).
    pub key: String,
    /// Размер слоя (логические px).
    pub size: Size,
    /// Режим пометок включён: слой берёт перо/палец; выключен — только показ.
    pub annotate: bool,
}

pub type OverlayFactory = Arc<dyn Fn(&OverlayPage) -> Option<W> + Send + Sync>;

static OVERLAY: OnceLock<OverlayFactory> = OnceLock::new();

/// Слой пометок пером поверх страниц книги (ставит модуль рукописного ввода).
pub fn set_overlay_factory(f: OverlayFactory) {
    let _ = OVERLAY.set(f);
}

// ─────────────────────────── Состояние экрана ───────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    None,
    Menu,
    Toc,
    Bookmarks,
    Highlights,
    Search,
    Text,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sel {
    Flow { a: Pos, b: Pos },
    Pdf { page: usize, a: usize, b: usize },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub loc: Location,
    pub label: String,
    /// Подсветка: для PDF — символы страницы, для текста — позиции.
    pub pdf_chars: Option<(usize, usize)>,
    pub flow: Option<(Pos, Pos)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LoadState {
    Loading,
    Ready,
    Failed(String),
}

/// Сигналы экрана (копируемые — для замыканий виджетов).
#[derive(Clone, Copy)]
pub struct Ctl {
    pub state: RwSignal<BookState>,
    pub panel: RwSignal<Panel>,
    pub annotate: RwSignal<bool>,
    pub selection: RwSignal<Option<Sel>>,
    /// Размер области страницы.
    pub size: RwSignal<(f32, f32)>,
    /// Счётчик перерисовки (пришёл растр, подсчёт страниц, книга открылась).
    pub rev: RwSignal<u64>,
    pub load: RwSignal<LoadState>,
    pub query: RwSignal<String>,
    pub hits: RwSignal<Vec<Hit>>,
    /// Текущий поиск подсвечивается на странице.
    pub hit: RwSignal<Option<Hit>>,
    /// Часть страницы PDF в режиме «по ширине».
    pub part: RwSignal<usize>,
    /// Правка заметки выделения (id).
    pub note_edit: RwSignal<Option<String>>,
}

/// Хозяйство, которое живёт вне сигналов: книга, вёрстка, растры PDF.
struct Core {
    book: Option<Arc<Book>>,
    measure: Option<Arc<dyn Measure>>,
    pag: Option<Paginator>,
    counting: Option<String>,
    bitmaps: HashMap<usize, PdfBitmap>,
    pending: HashSet<(usize, u32)>,
    texts: HashMap<usize, Arc<(PdfText, Vec<PdfLink>)>>,
    text_pending: HashSet<usize>,
    turns: u32,
    /// Физических пикселей на логический (резкость растра PDF).
    pixel_ratio: f32,
}

/// Окружение: куда сохранять состояние, что делать по «Назад».
pub struct Env {
    /// Ключ книги (id страницы проекта) — для ключей картинок и слоя пометок.
    pub key: String,
    pub on_state: Arc<dyn Fn(&BookState) + Send + Sync>,
    pub on_close: Option<Arc<dyn Fn() + Send + Sync>>,
}

/// Открытая книга на экране.
#[derive(Clone)]
pub struct Reader {
    core: Arc<Mutex<Core>>,
    pub ctl: Ctl,
    env: Arc<Env>,
}

/// Кадр страницы в области чтения.
#[derive(Clone)]
pub struct Frame {
    /// Относительно области страницы.
    pub rect: Rect,
    pub kind: FrameKind,
}

#[derive(Clone)]
pub enum FrameKind {
    Flow(Page),
    Pdf { page: usize, scale: f32, offset: f32, bitmap: Option<PdfBitmap>, text: Option<Arc<(PdfText, Vec<PdfLink>)>> },
}

impl Frame {
    pub fn overlay_key(&self) -> String {
        match &self.kind {
            FrameKind::Flow(p) => format!("flow-{}-{}", p.start.block, p.start.ch),
            FrameKind::Pdf { page, .. } => format!("pdf-{page}"),
        }
    }
}

impl Reader {
    /// Новый экран; книга открывается в фоне из `load` (или сразу из `book`).
    pub fn new(env: Env, state: BookState, book: Option<Arc<Book>>, load: Option<Box<dyn FnOnce() -> std::result::Result<Book, String> + Send>>) -> Reader {
        let ctl = Ctl {
            state: use_signal(state),
            panel: use_signal(Panel::None),
            annotate: use_signal(false),
            selection: use_signal(None),
            size: use_signal((0.0, 0.0)),
            rev: use_signal(0),
            load: use_signal(if book.is_some() { LoadState::Ready } else { LoadState::Loading }),
            query: use_signal(String::new()),
            hits: use_signal(Vec::new()),
            hit: use_signal(None),
            part: use_signal(0),
            note_edit: use_signal(None),
        };
        let core = Core {
            book,
            measure: None,
            pag: None,
            counting: None,
            bitmaps: HashMap::new(),
            pending: HashSet::new(),
            texts: HashMap::new(),
            text_pending: HashSet::new(),
            turns: 0,
            pixel_ratio: syngui::scale::pixel_ratio().max(1.0),
        };
        let r = Reader { core: Arc::new(Mutex::new(core)), ctl, env: Arc::new(env) };
        if let Some(load) = load {
            if r.book().is_none() {
                let core = r.core.clone();
                let rev = ctl.rev;
                let st = ctl.load;
                let state = ctl.state;
                std::thread::Builder::new()
                    .name("reader-open".into())
                    .spawn(move || {
                        let res = load();
                        syngui::async_runtime::run_on_main_thread(move || match res {
                            Ok(b) => {
                                // Страница PDF за пределами книги — на начало.
                                if let Content::Pdf(w) = &b.content {
                                    if state.get_untracked().pos.page() >= w.page_count() || !matches!(state.get_untracked().pos, Location::Pdf { .. }) {
                                        state.update(|s| s.pos = Location::Pdf { page: 0 });
                                    }
                                }
                                core.lock().unwrap().book = Some(Arc::new(b));
                                st.set(LoadState::Ready);
                                rev.update(|v| *v += 1);
                            }
                            Err(e) => st.set(LoadState::Failed(e)),
                        });
                    })
                    .ok();
            }
        }
        r
    }

    pub fn book(&self) -> Option<Arc<Book>> {
        self.core.lock().unwrap().book.clone()
    }

    fn flow(&self) -> Option<Arc<FlowDoc>> {
        match &self.book()?.content {
            Content::Flow(d) => Some(d.clone()),
            _ => None,
        }
    }

    fn bump(&self) {
        self.ctl.rev.update(|v| *v += 1);
    }

    /// Изменить состояние книги и сохранить.
    pub fn update_state(&self, f: impl FnOnce(&mut BookState)) {
        let mut s = self.ctl.state.get_untracked();
        f(&mut s);
        (self.env.on_state)(&s);
        self.ctl.state.set(s);
    }

    fn spread(&self) -> bool {
        let (w, h) = self.ctl.size.get_untracked();
        self.ctl.state.get_untracked().spread && w > h * 1.1 && w > 700.0
    }

    fn frame_sizes(&self) -> Vec<Rect> {
        let (w, h) = self.ctl.size.get_untracked();
        if w < 10.0 || h < 10.0 {
            return Vec::new();
        }
        if self.spread() {
            let gap = 16.0;
            let fw = (w - gap) / 2.0;
            vec![rect(0.0, 0.0, fw, h), rect(fw + gap, 0.0, fw, h)]
        } else {
            vec![rect(0.0, 0.0, w, h)]
        }
    }

    /// Вёрстка при текущем размере и настройках (пересоздаётся при смене).
    fn with_pag<T>(&self, f: impl FnOnce(&mut Paginator) -> T) -> Option<T> {
        let doc = self.flow()?;
        let sizes = self.frame_sizes();
        let fr = sizes.first()?;
        let size = (fr.size.width, fr.size.height);
        let settings = self.ctl.state.get_untracked().settings;
        let mut core = self.core.lock().unwrap();
        let measure = core.measure.clone()?;
        let stale = core.pag.as_ref().map_or(true, |p| p.size != size || p.settings != settings);
        if stale {
            core.pag = Some(Paginator::new(doc.clone(), settings.clone(), size, measure.clone()));
            // Подсчёт всех страниц — в фоне.
            let key = core.pag.as_ref().unwrap().key().to_string();
            if core.counting.as_deref() != Some(&key) {
                core.counting = Some(key.clone());
                let core_arc = self.core.clone();
                let rev = self.ctl.rev;
                std::thread::Builder::new()
                    .name("reader-count".into())
                    .spawn(move || {
                        let counts = Paginator::count_pages(&doc, &settings, size, &*measure);
                        syngui::async_runtime::run_on_main_thread(move || {
                            let mut c = core_arc.lock().unwrap();
                            if let Some(p) = c.pag.as_mut().filter(|p| p.key() == key) {
                                p.counts = Some(Arc::new(counts));
                                drop(c);
                                rev.update(|v| *v += 1);
                            }
                        });
                    })
                    .ok();
            }
        }
        Some(f(core.pag.as_mut().unwrap()))
    }

    /// Кадры текущего экрана.
    pub fn frames(&self) -> Vec<Frame> {
        let Some(book) = self.book() else { return Vec::new() };
        let sizes = self.frame_sizes();
        if sizes.is_empty() {
            return Vec::new();
        }
        let state = self.ctl.state.get_untracked();
        match &book.content {
            Content::Flow(_) => {
                let Some(pages) = self.with_pag(|p| {
                    let first = p.page(state.pos.pos());
                    let mut v = vec![first.clone()];
                    if sizes.len() > 1 {
                        if let Some(n) = p.next(first.start) {
                            v.push(p.page(n));
                        }
                    }
                    v
                }) else {
                    return Vec::new();
                };
                pages.into_iter().zip(sizes).map(|(p, r)| Frame { rect: r, kind: FrameKind::Flow(p) }).collect()
            }
            Content::Pdf(w) => {
                let first = state.pos.page().min(w.page_count().saturating_sub(1));
                let mut out = Vec::new();
                for (i, r) in sizes.iter().enumerate() {
                    let page = first + i;
                    let Some(&(pw, ph)) = w.info.sizes.get(page) else { break };
                    let fit_w = state.pdf_fit == PdfFit::Width && sizes.len() == 1;
                    let scale = if fit_w { r.size.width / pw } else { (r.size.width / pw).min(r.size.height / ph) };
                    let (dw, dh) = (pw * scale, ph * scale);
                    let parts = if fit_w { (dh / r.size.height).ceil().max(1.0) as usize } else { 1 };
                    let part = self.ctl.part.get_untracked().min(parts - 1);
                    let offset = if fit_w { (part as f32 * r.size.height * 0.9).min((dh - r.size.height).max(0.0)) } else { 0.0 };
                    let fr = if fit_w {
                        rect(r.origin.x, r.origin.y, dw, r.size.height.min(dh))
                    } else {
                        rect(r.origin.x + (r.size.width - dw) / 2.0, r.origin.y + (r.size.height - dh) / 2.0, dw, dh)
                    };
                    let (bitmap, text) = self.pdf_page(page, scale);
                    out.push(Frame { rect: fr, kind: FrameKind::Pdf { page, scale, offset, bitmap, text } });
                }
                // Соседние — заранее.
                if let Some(Frame { kind: FrameKind::Pdf { page, scale, .. }, .. }) = out.last().cloned() {
                    if page + 1 < w.page_count() {
                        let _ = self.pdf_page(page + 1, scale);
                    }
                }
                out
            }
        }
    }

    /// Растр и текст страницы PDF: из кэша или запрос в фон.
    fn pdf_page(&self, page: usize, scale: f32) -> (Option<PdfBitmap>, Option<Arc<(PdfText, Vec<PdfLink>)>>) {
        let Some(book) = self.book() else { return (None, None) };
        let Content::Pdf(w) = &book.content else { return (None, None) };
        let mut core = self.core.lock().unwrap();
        let px_scale = scale * core.pixel_ratio;
        let key = (page, (px_scale * 100.0) as u32);
        let have = core.bitmaps.get(&page).cloned();
        let fresh = have.as_ref().is_some_and(|b| {
            let want_w = (w.info.sizes[page].0 * px_scale).ceil() as u32;
            (b.width as i64 - want_w as i64).abs() <= 2
        });
        if !fresh && !core.pending.contains(&key) {
            core.pending.insert(key);
            let core_arc = self.core.clone();
            let rev = self.ctl.rev;
            w.render(page, px_scale, move |b| {
                syngui::async_runtime::run_on_main_thread(move || {
                    let mut c = core_arc.lock().unwrap();
                    c.pending.remove(&key);
                    if let Some(b) = b {
                        if c.bitmaps.len() > 6 {
                            let far: Vec<usize> = c.bitmaps.keys().copied().filter(|p| p.abs_diff(page) > 2).collect();
                            for p in far {
                                c.bitmaps.remove(&p);
                            }
                        }
                        c.bitmaps.insert(page, b);
                    }
                    drop(c);
                    rev.update(|v| *v += 1);
                });
            });
        }
        let text = core.texts.get(&page).cloned();
        if text.is_none() && !core.text_pending.contains(&page) {
            core.text_pending.insert(page);
            let core_arc = self.core.clone();
            let rev = self.ctl.rev;
            w.text(page, move |t, l| {
                syngui::async_runtime::run_on_main_thread(move || {
                    let mut c = core_arc.lock().unwrap();
                    c.text_pending.remove(&page);
                    c.texts.insert(page, Arc::new((t, l)));
                    drop(c);
                    rev.update(|v| *v += 1);
                });
            });
        }
        (have, text)
    }

    // ─── Навигация ───

    /// Перейти к месту (оглавление, закладка, поиск, ссылка).
    pub fn go(&self, loc: Location) {
        let loc = match loc {
            Location::Flow { .. } => self.with_pag(|p| Location::flow(p.page_start(loc.pos()))).unwrap_or(loc),
            l => l,
        };
        self.ctl.part.set(0);
        self.turned(loc);
    }

    fn turned(&self, loc: Location) {
        let progress = self.progress_of(loc);
        self.ctl.selection.set(None);
        self.update_state(|s| {
            s.pos = loc;
            s.progress = progress;
        });
        let every = self.ctl.state.get_untracked().refresh_every;
        let mut core = self.core.lock().unwrap();
        core.turns += 1;
        if every > 0 && core.turns >= every {
            core.turns = 0;
            refresh_eink();
        }
        drop(core);
        self.bump();
    }

    fn progress_of(&self, loc: Location) -> f32 {
        match (loc, self.book().map(|b| match &b.content {
            Content::Flow(d) => (Some(d.clone()), 0),
            Content::Pdf(w) => (None, w.page_count()),
        })) {
            (Location::Flow { .. }, Some((Some(d), _))) => d.progress(loc.pos()),
            (Location::Pdf { page }, Some((None, n))) if n > 0 => (page + 1) as f32 / n as f32,
            _ => 0.0,
        }
    }

    pub fn next(&self) {
        let Some(book) = self.book() else { return };
        let state = self.ctl.state.get_untracked();
        let step = if self.spread() { 2 } else { 1 };
        match &book.content {
            Content::Flow(_) => {
                let n = self.with_pag(|p| {
                    let mut pos = p.page_start(state.pos.pos());
                    for _ in 0..step {
                        pos = p.next(pos)?;
                    }
                    Some(pos)
                });
                if let Some(Some(pos)) = n {
                    self.turned(Location::flow(pos));
                }
            }
            Content::Pdf(w) => {
                let page = state.pos.page();
                // По ширине — сначала части страницы.
                if state.pdf_fit == PdfFit::Width && step == 1 {
                    if let Some(Frame { rect: r, kind: FrameKind::Pdf { scale, offset, .. } }) = self.frames().first().cloned() {
                        let full = w.info.sizes[page].1 * scale;
                        if offset + r.size.height < full - 1.0 {
                            self.ctl.part.update(|p| *p += 1);
                            self.bump();
                            return;
                        }
                    }
                }
                if page + step < w.page_count() + if step == 2 { 1 } else { 0 } && page + 1 < w.page_count() {
                    self.ctl.part.set(0);
                    self.turned(Location::Pdf { page: (page + step).min(w.page_count() - 1) });
                }
            }
        }
    }

    pub fn prev(&self) {
        let Some(book) = self.book() else { return };
        let state = self.ctl.state.get_untracked();
        let step = if self.spread() { 2 } else { 1 };
        match &book.content {
            Content::Flow(_) => {
                let n = self.with_pag(|p| {
                    let mut pos = p.page_start(state.pos.pos());
                    for _ in 0..step {
                        pos = p.prev(pos)?;
                    }
                    Some(pos)
                });
                if let Some(Some(pos)) = n {
                    self.turned(Location::flow(pos));
                }
            }
            Content::Pdf(_) => {
                let page = state.pos.page();
                if state.pdf_fit == PdfFit::Width && step == 1 && self.ctl.part.get_untracked() > 0 {
                    self.ctl.part.update(|p| *p -= 1);
                    self.bump();
                    return;
                }
                if page > 0 {
                    self.ctl.part.set(0);
                    self.turned(Location::Pdf { page: page.saturating_sub(step) });
                }
            }
        }
    }

    /// Номер страницы (с 1) и всего, если известно; прогресс 0…1.
    pub fn page_info(&self) -> (Option<(usize, usize)>, f32) {
        let state = self.ctl.state.get_untracked();
        let progress = self.progress_of(state.pos);
        let Some(book) = self.book() else { return (None, progress) };
        match &book.content {
            Content::Pdf(w) => (Some((state.pos.page() + 1, w.page_count())), progress),
            Content::Flow(_) => (self.with_pag(|p| p.page_number(state.pos.pos())).flatten().map(|(n, t)| (n + 1, t)), progress),
        }
    }

    /// Начало текста текущей страницы (подпись закладки).
    fn page_label(&self) -> String {
        let frames = self.frames();
        let text = match frames.first().map(|f| &f.kind) {
            Some(FrameKind::Flow(p)) => p.runs().take(12).map(|r| r.text.as_str()).collect::<Vec<_>>().join(" "),
            Some(FrameKind::Pdf { text: Some(t), page, .. }) => {
                let s: String = t.0.chars.iter().take(80).collect();
                let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
                if s.is_empty() { tr!("notes.reader.page_n", n = page + 1) } else { s }
            }
            Some(FrameKind::Pdf { page, .. }) => tr!("notes.reader.page_n", n = page + 1),
            None => String::new(),
        };
        text.chars().take(80).collect()
    }

    pub fn toggle_bookmark(&self) {
        let loc = self.ctl.state.get_untracked().pos;
        let label = self.page_label();
        let progress = self.progress_of(loc);
        self.update_state(|s| {
            if let Some(i) = s.bookmarks.iter().position(|b| b.loc == loc) {
                s.bookmarks.remove(i);
            } else {
                s.bookmarks.push(Bookmark { loc, label, progress, created: now() });
                s.bookmarks.sort_by(|a, b| a.progress.partial_cmp(&b.progress).unwrap_or(std::cmp::Ordering::Equal));
            }
        });
        self.bump();
    }

    /// Текст выделения.
    pub fn selection_text(&self) -> String {
        match self.ctl.selection.get_untracked() {
            Some(Sel::Flow { a, b }) => {
                let (a, b) = if a <= b { (a, b) } else { (b, a) };
                self.flow().map(|d| d.text_between(a, Pos { block: b.block, ch: b.ch + 1 })).unwrap_or_default()
            }
            Some(Sel::Pdf { page, a, b }) => self.core.lock().unwrap().texts.get(&page).map(|t| t.0.text(a, b)).unwrap_or_default(),
            None => String::new(),
        }
    }

    /// Выделение → маркер (с заметкой). Возвращает id.
    pub fn highlight_selection(&self, note: &str) -> Option<String> {
        let sel = self.ctl.selection.get_untracked()?;
        let text = self.selection_text();
        let id = format!("h{:x}", now_nanos());
        let h = match sel {
            Sel::Flow { a, b } => {
                let (a, b) = if a <= b { (a, b) } else { (b, a) };
                Highlight { id: id.clone(), start: Location::flow(a), end: Location::flow(b), text, note: note.into(), created: now(), ..Default::default() }
            }
            Sel::Pdf { page, a, b } => Highlight {
                id: id.clone(),
                start: Location::Pdf { page },
                end: Location::Pdf { page },
                pdf_chars: Some((a.min(b), a.max(b))),
                text,
                note: note.into(),
                created: now(),
                ..Default::default()
            },
        };
        self.update_state(|s| s.highlights.push(h));
        self.ctl.selection.set(None);
        self.bump();
        Some(id)
    }

    pub fn search(&self, q: &str) {
        let q = q.trim().to_string();
        self.ctl.query.set(q.clone());
        if q.is_empty() {
            self.ctl.hits.set(Vec::new());
            return;
        }
        let Some(book) = self.book() else { return };
        match &book.content {
            Content::Flow(d) => {
                let hits = d
                    .search(&q, 300)
                    .into_iter()
                    .map(|(a, b)| {
                        let text = d.blocks[a.block as usize].text();
                        let chars: Vec<char> = text.chars().collect();
                        let from = (a.ch as usize).saturating_sub(30);
                        let to = (b.ch as usize + 40).min(chars.len());
                        let label: String = chars[from..to].iter().collect();
                        Hit { loc: Location::flow(a), label: format!("{}{}…", if from > 0 { "…" } else { "" }, label.trim()), pdf_chars: None, flow: Some((a, Pos { block: b.block, ch: b.ch.saturating_sub(1) })) }
                    })
                    .collect();
                self.ctl.hits.set(hits);
            }
            Content::Pdf(w) => {
                let hits = self.ctl.hits;
                let from = self.ctl.state.get_untracked().pos.page();
                w.search(&q, from, move |res| {
                    let list: Vec<Hit> = res
                        .into_iter()
                        .map(|h| Hit { loc: Location::Pdf { page: h.page }, label: format!("{} · {}…", h.page + 1, h.snippet), pdf_chars: Some((h.a, h.b)), flow: None })
                        .collect();
                    syngui::async_runtime::run_on_main_thread(move || hits.set(list));
                });
            }
        }
    }

    /// Ссылка внутри книги или наружу.
    fn follow(&self, link: &str) {
        if link.contains("://") || link.starts_with("mailto:") {
            let _ = syngui::open_url(link);
            return;
        }
        if let Some(d) = self.flow() {
            if let Some(block) = d.resolve_link(link) {
                self.go(Location::flow(Pos::new(block, 0)));
            }
        }
    }

    fn close(&self) {
        if let Some(f) = &self.env.on_close {
            f();
        }
    }

    fn measure(&self) -> Option<Arc<dyn Measure>> {
        self.core.lock().unwrap().measure.clone()
    }
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn now_nanos() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
}

/// Полная очистка E Ink (после того как новый кадр дошёл до панели).
pub fn refresh_eink() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(400));
        if let Some(e) = synshell_common::display::Eink::find() {
            let _ = e.refresh();
        }
    });
}

// ─────────────────────────── Элемент страницы ───────────────────────────

/// Текст, выделения и жесты страницы. Картинки (иллюстрации, растр PDF)
/// лежат под ним отдельными виджетами.
pub struct PageCanvas {
    pub reader: Reader,
    pub frames: Arc<Vec<Frame>>,
    pub rev: u64,
}

impl Widget for PageCanvas {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(PageElement {
            id: ElementId::new(),
            bounds: Rect::zero(),
            dirty: DirtyFlags::LAYOUT | DirtyFlags::RENDER,
            classes: Vec::new(),
            mss: MssFields::new(),
            reader: self.reader.clone(),
            frames: self.frames.clone(),
            rev: self.rev,
            press: None,
            selecting: None,
            wheel: 0.0,
            focus_request: true,
            touch_down: None,
        })
    }
    fn can_update(&self, other: &dyn Any) -> bool {
        other.is::<Self>()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn mount(&self, _tree: &mut ElementTree, _parent_id: ElementId) {}
}

struct Press {
    at: Point,
    time: Instant,
    moved: bool,
}

pub struct PageElement {
    id: ElementId,
    bounds: Rect,
    dirty: DirtyFlags,
    classes: Vec<String>,
    mss: MssFields,
    reader: Reader,
    frames: Arc<Vec<Frame>>,
    rev: u64,
    press: Option<Press>,
    /// Якорь выделения (символ, с которого начали).
    selecting: Option<Sel>,
    wheel: f32,
    focus_request: bool,
    touch_down: Option<Instant>,
}

const HOLD: Duration = Duration::from_millis(450);

impl PageElement {
    fn local(&self, p: Point) -> Point {
        Point::new(p.x - self.bounds.origin.x, p.y - self.bounds.origin.y)
    }

    fn frame_at(&self, p: Point) -> Option<&Frame> {
        self.frames.iter().find(|f| f.rect.contains(p))
    }

    /// Символ под точкой (локальные координаты).
    fn char_at(&self, p: Point) -> Option<Sel> {
        let f = self.frame_at(p).or(self.frames.first())?;
        match &f.kind {
            FrameKind::Flow(page) => {
                let m = self.reader.measure()?;
                let lp = Point::new(p.x - f.rect.origin.x, p.y - f.rect.origin.y);
                // Строка — по вертикали, отрезок — ближайший по горизонтали.
                let mut best: Option<(&Run, f32)> = None;
                for r in page.runs().filter(|r| r.ch1 > r.ch0) {
                    let dy = if lp.y < r.rect.origin.y { r.rect.origin.y - lp.y } else if lp.y > r.rect.origin.y + r.rect.size.height { lp.y - r.rect.origin.y - r.rect.size.height } else { 0.0 };
                    let dx = if lp.x < r.rect.origin.x { r.rect.origin.x - lp.x } else if lp.x > r.rect.origin.x + r.rect.size.width { lp.x - r.rect.origin.x - r.rect.size.width } else { 0.0 };
                    let d = dy * 3.0 + dx;
                    if best.map_or(true, |(_, bd)| d < bd) {
                        best = Some((r, d));
                    }
                }
                let (r, _) = best?;
                let chars: Vec<char> = r.text.chars().collect();
                let fam = if r.mono { Some("monospace") } else { None };
                let x = lp.x - r.rect.origin.x;
                let mut k = 0;
                for i in 1..=chars.len() {
                    let w = m.width(&chars[..i].iter().collect::<String>(), r.size, r.bold, fam);
                    if w > x {
                        break;
                    }
                    k = i;
                }
                let k = (k as u32).min(r.ch1 - r.ch0 - 1);
                Some(Sel::Flow { a: Pos { block: r.block, ch: r.ch0 + k }, b: Pos { block: r.block, ch: r.ch0 + k } })
            }
            FrameKind::Pdf { page, scale, offset, text: Some(t), .. } => {
                let x = (p.x - f.rect.origin.x) / scale;
                let y = (p.y - f.rect.origin.y + offset) / scale;
                let i = t.0.char_at(x, y)?;
                Some(Sel::Pdf { page: *page, a: i, b: i })
            }
            _ => None,
        }
    }

    /// Слово вокруг символа.
    fn word_at(&self, s: Sel) -> Sel {
        let is_word = |c: char| c.is_alphanumeric() || c == '-' || c == '\'' || c == '’';
        match s {
            Sel::Flow { a, .. } => {
                let Some(d) = self.reader.flow() else { return s };
                let text: Vec<char> = d.blocks.get(a.block as usize).map(|b| b.text().chars().collect()).unwrap_or_default();
                let mut i = a.ch as usize;
                let mut j = a.ch as usize;
                if i >= text.len() || !is_word(text[i]) {
                    return s;
                }
                while i > 0 && is_word(text[i - 1]) {
                    i -= 1;
                }
                while j + 1 < text.len() && is_word(text[j + 1]) {
                    j += 1;
                }
                Sel::Flow { a: Pos { block: a.block, ch: i as u32 }, b: Pos { block: a.block, ch: j as u32 } }
            }
            Sel::Pdf { page, a, .. } => {
                let Some(t) = self.reader.core.lock().unwrap().texts.get(&page).cloned() else { return s };
                let c = &t.0.chars;
                let (mut i, mut j) = (a, a);
                if a >= c.len() || !is_word(c[a]) {
                    return s;
                }
                while i > 0 && is_word(c[i - 1]) {
                    i -= 1;
                }
                while j + 1 < c.len() && is_word(c[j + 1]) {
                    j += 1;
                }
                Sel::Pdf { page, a: i, b: j }
            }
        }
    }

    fn link_at(&self, p: Point) -> Option<String> {
        let f = self.frame_at(p)?;
        match &f.kind {
            FrameKind::Flow(page) => {
                let lp = Point::new(p.x - f.rect.origin.x, p.y - f.rect.origin.y);
                page.runs()
                    .find(|r| r.link.is_some() && r.rect.inflate(6.0, 6.0).contains(lp))
                    .and_then(|r| r.link.as_deref().map(str::to_string))
            }
            FrameKind::Pdf { scale, offset, text: Some(t), .. } => {
                let x = (p.x - f.rect.origin.x) / scale;
                let y = (p.y - f.rect.origin.y + offset) / scale;
                t.1.iter().find(|l| x >= l.rect[0] && x <= l.rect[0] + l.rect[2] && y >= l.rect[1] && y <= l.rect[1] + l.rect[3]).map(|l| match &l.target {
                    PdfLinkTarget::Page(n) => format!("#page:{n}"),
                    PdfLinkTarget::Uri(u) => u.clone(),
                })
            }
            _ => None,
        }
    }

    fn start_selection(&mut self, p: Point) {
        if let Some(s) = self.char_at(self.local(p)) {
            let w = self.word_at(s);
            self.selecting = Some(w);
            self.reader.ctl.selection.set(Some(w));
            self.reader.ctl.panel.set(Panel::None);
            self.mark_dirty(DirtyFlags::RENDER);
        }
    }

    fn extend_selection(&mut self, p: Point) {
        let Some(anchor) = self.selecting else { return };
        let Some(cur) = self.char_at(self.local(p)) else { return };
        let sel = match (anchor, cur) {
            (Sel::Flow { a, b }, Sel::Flow { a: c, .. }) => {
                if c < a { Sel::Flow { a: b, b: c } } else { Sel::Flow { a, b: c.max(b) } }
            }
            (Sel::Pdf { page, a, b }, Sel::Pdf { page: p2, a: c, .. }) if page == p2 => {
                if c < a { Sel::Pdf { page, a: b, b: c } } else { Sel::Pdf { page, a, b: c.max(b) } }
            }
            _ => return,
        };
        if self.reader.ctl.selection.get_untracked() != Some(sel) {
            self.reader.ctl.selection.set(Some(sel));
        }
    }

    fn tap(&mut self, p: Point) {
        let ctl = self.reader.ctl;
        if ctl.selection.get_untracked().is_some() {
            ctl.selection.set(None);
            return;
        }
        if ctl.panel.get_untracked() != Panel::None {
            ctl.panel.set(Panel::None);
            return;
        }
        let lp = self.local(p);
        if let Some(link) = self.link_at(lp) {
            if let Some(n) = link.strip_prefix("#page:").and_then(|n| n.parse().ok()) {
                self.reader.go(Location::Pdf { page: n });
            } else {
                self.reader.follow(&link);
            }
            return;
        }
        let w = self.bounds.size.width.max(1.0);
        let fx = lp.x / w;
        if fx < 0.3 {
            self.reader.prev();
        } else if fx > 0.7 {
            self.reader.next();
        } else {
            ctl.panel.set(Panel::Menu);
        }
    }

    // ─── Рисование ───

    /// Прямоугольники диапазона `[a, b]` (включительно) на текстовой странице.
    fn flow_rects(&self, page: &Page, a: Pos, b: Pos, origin: Point) -> Vec<Rect> {
        let Some(m) = self.reader.measure() else { return Vec::new() };
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let mut out: Vec<Rect> = Vec::new();
        for r in page.runs().filter(|r| r.ch1 > r.ch0) {
            let rs = Pos { block: r.block, ch: r.ch0 };
            let re = Pos { block: r.block, ch: r.ch1 - 1 };
            if re < a || rs > b {
                continue;
            }
            let chars: Vec<char> = r.text.chars().collect();
            let fam = if r.mono { Some("monospace") } else { None };
            let from = if a > rs { (a.ch - r.ch0) as usize } else { 0 };
            let to = if b < re { (b.ch - r.ch0 + 1) as usize } else { (r.ch1 - r.ch0) as usize };
            let x0 = if from == 0 { 0.0 } else { m.width(&chars[..from].iter().collect::<String>(), r.size, r.bold, fam) };
            let x1 = if to >= chars.len() { r.rect.size.width } else { m.width(&chars[..to].iter().collect::<String>(), r.size, r.bold, fam) };
            let rr = rect(origin.x + r.rect.origin.x + x0, origin.y + r.rect.origin.y, (x1 - x0).max(1.0), r.rect.size.height);
            // Соседние на одной строке — слить (просвет между словами тоже).
            match out.last_mut() {
                Some(l) if (l.origin.y - rr.origin.y).abs() < 1.0 && rr.origin.x >= l.origin.x && rr.origin.x - (l.origin.x + l.size.width) < r.size * 1.5 => {
                    l.size.width = rr.origin.x + rr.size.width - l.origin.x;
                }
                _ => out.push(rr),
            }
        }
        out
    }

    fn pdf_rects(&self, f: &Frame, a: usize, b: usize, origin: Point) -> Vec<Rect> {
        let FrameKind::Pdf { scale, offset, text: Some(t), .. } = &f.kind else { return Vec::new() };
        t.0.selection_rects(a, b)
            .into_iter()
            .map(|r| rect(origin.x + r[0] * scale, origin.y + r[1] * scale - offset, r[2] * scale, r[3] * scale))
            .collect()
    }

    fn draw_marks(&self, list: &mut DisplayList, f: &Frame, origin: Point) {
        let ctl = self.reader.ctl;
        let state = ctl.state.get_untracked();
        // Маркер — цветом текста с прозрачностью: виден и в тёмной теме.
        let ink = self.mss.color.unwrap_or(Color::from_srgb(0, 0, 0, 1.0));
        let hl = Color { a: 0.18, ..ink };
        let sel_c = self.mss.accent_color.map(|c| Color { a: 0.35, ..c }).unwrap_or(Color::from_srgb(40, 90, 200, 0.35));
        let hit_c = Color { a: 0.32, ..ink };
        let fill = |list: &mut DisplayList, rs: Vec<Rect>, c: Color| {
            for r in rs {
                list.push_rect(r, c, [2.0; 4]);
            }
        };
        match &f.kind {
            FrameKind::Flow(page) => {
                for h in &state.highlights {
                    if let (Location::Flow { .. }, Location::Flow { .. }) = (h.start, h.end) {
                        let rs = self.flow_rects(page, h.start.pos(), h.end.pos(), origin);
                        fill(list, rs, hl);
                    }
                }
                if let Some(Hit { flow: Some((a, b)), .. }) = ctl.hit.get_untracked() {
                    let rs = self.flow_rects(page, a, b, origin);
                    fill(list, rs, hit_c);
                }
                if let Some(Sel::Flow { a, b }) = ctl.selection.get_untracked() {
                    let rs = self.flow_rects(page, a, b, origin);
                    fill(list, rs, sel_c);
                }
            }
            FrameKind::Pdf { page, .. } => {
                for h in &state.highlights {
                    if let (Location::Pdf { page: p }, Some((a, b))) = (h.start, h.pdf_chars) {
                        if p == *page {
                            let rs = self.pdf_rects(f, a, b, origin);
                            fill(list, rs, hl);
                        }
                    }
                }
                if let Some(Hit { loc: Location::Pdf { page: p }, pdf_chars: Some((a, b)), .. }) = ctl.hit.get_untracked() {
                    if p == *page {
                        let rs = self.pdf_rects(f, a, b, origin);
                        fill(list, rs, hit_c);
                    }
                }
                if let Some(Sel::Pdf { page: p, a, b }) = ctl.selection.get_untracked() {
                    if p == *page {
                        let rs = self.pdf_rects(f, a, b, origin);
                        fill(list, rs, sel_c);
                    }
                }
            }
        }
    }
}

impl Element for PageElement {
    fn update(&mut self, widget: &dyn Widget, ctx: &mut UpdateContext) {
        let Some(w) = widget.as_any().downcast_ref::<PageCanvas>() else { return };
        self.reader = w.reader.clone();
        self.frames = w.frames.clone();
        if w.rev != self.rev {
            self.rev = w.rev;
        }
        self.mark_dirty(DirtyFlags::RENDER);
        let _ = ctx;
    }

    fn mount(&mut self, tree: &mut ElementTree) {
        if let Some(tm) = tree.text_measure.clone() {
            let mut core = self.reader.core.lock().unwrap();
            if core.measure.is_none() {
                core.measure = Some(Arc::new(TmMeasure(tm)));
                drop(core);
                let rev = self.reader.ctl.rev;
                syngui::async_runtime::run_on_main_thread(move || rev.update(|v| *v += 1));
            }
        }
    }

    fn layout(&mut self, constraints: Constraints) -> Size {
        let w = if constraints.max_width.is_finite() { constraints.max_width } else { 600.0 };
        let h = if constraints.max_height.is_finite() { constraints.max_height } else { 800.0 };
        let size = Size::new(w, h);
        self.bounds.size = size;
        let cur = self.reader.ctl.size.get_untracked();
        if ((cur.0 - w).abs() > 0.5 || (cur.1 - h).abs() > 0.5) && w > 10.0 && h > 10.0 {
            let sig = self.reader.ctl.size;
            syngui::async_runtime::run_on_main_thread(move || sig.set((w, h)));
        }
        size
    }

    fn layout_hint(&self) -> LayoutHint {
        LayoutHint::Stack { expand: true }
    }

    fn fills_available(&self) -> bool {
        true
    }

    fn take_focus_request(&mut self) -> bool {
        std::mem::take(&mut self.focus_request)
    }

    fn wants_key(&self, key: Key) -> bool {
        matches!(key, Key::Left | Key::Right | Key::Up | Key::Down | Key::PageUp | Key::PageDown | Key::Space | Key::Escape | Key::Home | Key::End)
    }

    fn accessibility_info(&self) -> Option<syngui::a11y::AccessibilityInfo> {
        Some(syngui::a11y::AccessibilityInfo {
            role: syngui::a11y::Role::TextField,
            state: syngui::a11y::NodeState::default(),
            properties: syngui::a11y::NodeProperties::default(),
        })
    }

    fn build_display_list(&self, list: &mut DisplayList, _clip: Rect) {
        let color = self.mss.color.unwrap_or(Color::from_srgb(0, 0, 0, 1.0));
        let link_c = self.mss.accent_color.unwrap_or(color);
        let o = self.bounds.origin;
        for f in self.frames.iter() {
            let origin = Point::new(o.x + f.rect.origin.x, o.y + f.rect.origin.y);
            self.draw_marks(list, f, origin);
            if let FrameKind::Flow(page) = &f.kind {
                let state = self.reader.ctl.state.get_untracked();
                let fam = state.settings.family.clone();
                // Общий образец высоты: иначе каждое слово центрируется по
                // своим глифам и строка «прыгает».
                let prev_sample = list.set_text_bbox_sample(Some("HgЙу".into()));
                for it in &page.items {
                    match it {
                        Item::Text(r) => {
                            let rr = rect(origin.x + r.rect.origin.x, origin.y + r.rect.origin.y, r.rect.size.width + r.size, r.rect.size.height);
                            let c = if r.style & LINK != 0 { link_c } else { color };
                            let italic = r.style & super::model::ITALIC != 0;
                            let prev = list.set_text_italic(italic);
                            let family = if r.mono { Some("monospace".to_string()) } else { fam.clone() };
                            list.push_text_full(
                                &r.text,
                                rr,
                                c,
                                r.size,
                                TextAlign::VCENTER,
                                if r.style & LINK != 0 { TextDecoration::Underline } else { TextDecoration::None },
                                if r.bold { 700 } else { 400 },
                                family,
                                0.0,
                                None,
                                true,
                            );
                            list.set_text_italic(prev);
                        }
                        Item::Rule { rect: r } => {
                            list.push_rect(rect(origin.x + r.origin.x, origin.y + r.origin.y, r.size.width, r.size.height), color, [0.0; 4]);
                        }
                        Item::Image { .. } => {}
                    }
                }
                list.set_text_bbox_sample(prev_sample);
            }
        }
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut EventContext) -> EventResult {
        match event {
            Event::TouchStart { .. } => {
                self.touch_down = Some(Instant::now());
                EventResult::Ignored
            }
            Event::MouseDown { button: MouseButton::Left, position } => {
                if !self.bounds.contains(*position) {
                    return EventResult::Ignored;
                }
                self.focus_request = true;
                self.press = Some(Press { at: *position, time: Instant::now(), moved: false });
                ctx.capture();
                EventResult::Handled
            }
            Event::LongPress { position } => {
                if !self.bounds.contains(*position) {
                    return EventResult::Ignored;
                }
                self.start_selection(*position);
                if let Some(p) = &mut self.press {
                    p.moved = true;
                }
                EventResult::Handled
            }
            Event::MouseMove(position) => {
                let Some(press) = &mut self.press else { return EventResult::Ignored };
                if self.selecting.is_some() {
                    let p = *position;
                    self.extend_selection(p);
                    return EventResult::Handled;
                }
                let d = ((position.x - press.at.x).powi(2) + (position.y - press.at.y).powi(2)).sqrt();
                if !press.moved {
                    if press.time.elapsed() >= HOLD && d < 14.0 {
                        let at = press.at;
                        press.moved = true;
                        self.start_selection(at);
                        self.extend_selection(*position);
                    } else if d >= 14.0 {
                        press.moved = true;
                    }
                }
                EventResult::Handled
            }
            Event::MouseUp { button: MouseButton::Left, position } => {
                let Some(press) = self.press.take() else { return EventResult::Ignored };
                if self.selecting.take().is_some() {
                    // Выделение готово — панель выделения покажет обёртка.
                    if self.reader.ctl.selection.get_untracked().is_some() {
                        self.reader.ctl.rev.update(|v| *v += 1);
                    }
                    return EventResult::Handled;
                }
                let dx = position.x - press.at.x;
                let dy = position.y - press.at.y;
                if press.moved {
                    if dx.abs() > 40.0 && dx.abs() > dy.abs() {
                        if dx < 0.0 {
                            self.reader.next();
                        } else {
                            self.reader.prev();
                        }
                    } else if dy.abs() > 60.0 && dy.abs() > dx.abs() * 1.5 && self.reader.ctl.state.get_untracked().pdf_fit == PdfFit::Width {
                        if dy < 0.0 {
                            self.reader.next();
                        } else {
                            self.reader.prev();
                        }
                    }
                    return EventResult::Handled;
                }
                if press.time.elapsed() >= HOLD {
                    self.start_selection(press.at);
                    self.selecting = None;
                    return EventResult::Handled;
                }
                self.tap(*position);
                EventResult::Handled
            }
            Event::MouseWheel { delta, position, .. } => {
                if !self.bounds.contains(*position) {
                    return EventResult::Ignored;
                }
                self.wheel += delta;
                if self.wheel.abs() >= 1.0 {
                    if self.wheel < 0.0 {
                        self.reader.next();
                    } else {
                        self.reader.prev();
                    }
                    self.wheel = 0.0;
                }
                EventResult::Handled
            }
            Event::KeyDown(key) => match key {
                Key::Right | Key::Down | Key::PageDown | Key::Space => {
                    self.reader.next();
                    EventResult::Handled
                }
                Key::Left | Key::Up | Key::PageUp => {
                    self.reader.prev();
                    EventResult::Handled
                }
                Key::Home => {
                    self.reader.go(match self.reader.book().map(|b| matches!(b.content, Content::Pdf(_))) {
                        Some(true) => Location::Pdf { page: 0 },
                        _ => Location::default(),
                    });
                    EventResult::Handled
                }
                Key::Escape => {
                    let ctl = self.reader.ctl;
                    if ctl.selection.get_untracked().is_some() {
                        ctl.selection.set(None);
                    } else if ctl.panel.get_untracked() != Panel::None {
                        ctl.panel.set(Panel::None);
                    } else {
                        self.reader.close();
                    }
                    EventResult::Handled
                }
                _ => EventResult::Ignored,
            },
            _ => EventResult::Ignored,
        }
    }

    fn element_type_name(&self) -> &str {
        "reader-page"
    }
    fn set_classes(&mut self, c: Vec<String>) {
        self.classes = c;
    }
    fn get_classes(&self) -> &[String] {
        &self.classes
    }
    fn reset_mss_styles(&mut self) {
        self.mss.reset();
    }
    fn mss(&self) -> Option<&MssFields> {
        Some(&self.mss)
    }
    fn apply_computed_style(&mut self, s: &ComputedStyle) {
        self.mss.apply(s);
        self.mark_dirty(DirtyFlags::RENDER);
    }
    fn id(&self) -> ElementId {
        self.id
    }
    fn set_id(&mut self, id: ElementId) {
        self.id = id;
    }
    fn bounds(&self) -> Rect {
        self.bounds
    }
    fn set_position(&mut self, pos: Point) {
        self.bounds.origin = pos;
    }
    fn set_content_size(&mut self, size: Size) {
        self.bounds.size = size;
    }
    fn children(&self) -> &[ElementId] {
        &[]
    }
    fn mark_dirty(&mut self, flags: DirtyFlags) {
        self.dirty |= flags;
    }
    fn clear_dirty(&mut self, flags: DirtyFlags) {
        self.dirty.remove(flags);
    }
    fn is_dirty(&self, flags: DirtyFlags) -> bool {
        self.dirty.contains(flags)
    }
}

// ─────────────────────────── Сборка экрана ───────────────────────────

fn fill<M>(w: impl syngui::widgets::containers::IntoWidget<M>) -> Stack {
    Stack::new().fit(StackFit::Expand).child(w)
}

fn icon_btn(icon: &'static str, tip: String, f: impl FnMut() + Send + 'static) -> StyledWidget<ToolButton> {
    ToolButton::new(icon).tooltip(tip).on_click(f).class("reader-btn")
}

/// Экран читалки целиком.
pub fn reader_view(r: Reader) -> impl Widget {
    let ctl = r.ctl;
    let page = {
        let r = r.clone();
        Reactive::new(move || -> Vec<W> {
            let _ = ctl.rev.get();
            let _ = ctl.state.get();
            let _ = ctl.size.get();
            let _ = ctl.part.get();
            let annotate = ctl.annotate.get();
            let _ = ctl.selection.get();
            let _ = ctl.hit.get();
            match ctl.load.get() {
                LoadState::Loading => return vec![Box::new(fill(Center::new().child(Text::new(tr!("notes.reader.loading")).class("reader-msg"))))],
                LoadState::Failed(e) => return vec![Box::new(fill(Center::new().child(Text::new(e).class("reader-msg"))))],
                LoadState::Ready => {}
            }
            let frames = Arc::new(r.frames());
            let rev = ctl.rev.get_untracked();
            let mut stack = Stack::new().fit(StackFit::Expand).clip(true);
            stack = stack.child(DecoratedBox::new().class("reader-paper"));
            let key = r.env.key.clone();
            let book = r.book();
            for f in frames.iter() {
                match &f.kind {
                    FrameKind::Pdf { page, bitmap: Some(b), offset, .. } => {
                        let img = Image::from_rgba_shared(format!("reader:{key}:pdf:{page}:{}", b.width), b.width, b.height, b.rgba.clone()).fit(ImageFit::Fill).placeholder(false);
                        let scale = f.rect.size.width / b.width as f32;
                        stack = stack.child(
                            Positioned::new(
                                Stack::new().clip(true).child(
                                    Positioned::new(img).at(0.0, -offset).size(Size::new(f.rect.size.width, b.height as f32 * scale)),
                                ),
                            )
                            .at(f.rect.origin.x, f.rect.origin.y)
                            .size(f.rect.size),
                        );
                    }
                    FrameKind::Flow(p) => {
                        if let Some(Content::Flow(doc)) = book.as_ref().map(|b| &b.content) {
                            for it in &p.items {
                                if let Item::Image { id, rect: ir, .. } = it {
                                    if let Some(im) = doc.images.get(id) {
                                        stack = stack.child(
                                            Positioned::new(Image::from_bytes(format!("reader:{key}:{id}"), (*im.data).clone()).fit(ImageFit::Contain).placeholder(false))
                                                .at(f.rect.origin.x + ir.origin.x, f.rect.origin.y + ir.origin.y)
                                                .size(ir.size),
                                        );
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            stack = stack.child(PageCanvas { reader: r.clone(), frames: frames.clone(), rev }.class("reader-page"));
            // Слой пометок пером — поверх страницы.
            if let Some(factory) = OVERLAY.get() {
                for f in frames.iter() {
                    let op = OverlayPage { book_page: key.clone(), key: f.overlay_key(), size: f.rect.size, annotate };
                    if let Some(w) = factory(&op) {
                        stack = stack.child(Positioned::new(w).at(f.rect.origin.x, f.rect.origin.y).size(f.rect.size));
                    }
                }
            }
            vec![Box::new(stack)]
        })
    };
    let footer = {
        let r = r.clone();
        Reactive::new(move || -> Vec<W> {
            let _ = ctl.rev.get();
            let state = ctl.state.get();
            let (num, progress) = r.page_info();
            let pages = match num {
                Some((n, t)) => format!("{n} / {t}"),
                None => "…".into(),
            };
            let mark = if state.has_bookmark(state.pos) { "🔖 " } else { "" };
            vec![Box::new(
                Row::new()
                    .gap(12.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .class("reader-footer")
                    .child(DecoratedBox::new().class("grow").child(Text::new(state.title.clone()).max_lines(1).elide(Elide::End).class("reader-footer-text")))
                    .child(Text::new(format!("{mark}{pages} · {}%", (progress * 100.0).round() as i32)).class("reader-footer-text")),
            )]
        })
    };
    let overlay = {
        let r = r.clone();
        Reactive::new(move || -> Vec<W> {
            syngui::i18n::subscribe();
            let panel = ctl.panel.get();
            let sel = ctl.selection.get();
            let mut out: Vec<W> = Vec::new();
            if sel.is_some() && panel == Panel::None {
                out.push(Box::new(selection_bar(r.clone())));
            }
            if let Some(id) = ctl.note_edit.get() {
                out.push(Box::new(note_dialog(r.clone(), id)));
            }
            match panel {
                Panel::None => {}
                Panel::Menu => out.push(Box::new(menu(r.clone()))),
                Panel::Toc | Panel::Bookmarks | Panel::Highlights => out.push(Box::new(lists_panel(r.clone(), panel))),
                Panel::Search => out.push(Box::new(search_panel(r.clone()))),
                Panel::Text => out.push(Box::new(text_panel(r.clone()))),
            }
            out
        })
    };
    DecoratedBox::new().class("reader").child(
        Stack::new()
            .fit(StackFit::Expand)
            .child(
                Column::new()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(DecoratedBox::new().class("grow reader-area").clip(true).child(page))
                    .child(footer)
                    .class("grow"),
            )
            .child(fill(overlay)),
    )
}

/// Меню по тапу в центре: верх — название и действия, низ — прогресс и панели.
fn menu(r: Reader) -> impl Widget {
    let ctl = r.ctl;
    let state = ctl.state.get_untracked();
    let (num, progress) = r.page_info();
    let mut top = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center);
    if r.env.on_close.is_some() {
        let r2 = r.clone();
        top = top.child(icon_btn(MI_ARROW_BACK, tr!("notes.reader.back"), move || r2.close()));
    }
    top = top
        .child(
            DecoratedBox::new().class("grow").child(
                Column::new()
                    .child(Text::new(state.title.clone()).max_lines(1).elide(Elide::End).class("reader-menu-title"))
                    .child(Text::new(state.author.clone()).max_lines(1).elide(Elide::End).class("reader-menu-sub")),
            ),
        )
        .child({
            let r = r.clone();
            let marked = state.has_bookmark(state.pos);
            icon_btn(if marked { MI_BOOKMARK } else { MI_BOOKMARK_BORDER }, tr!("notes.reader.bookmark"), move || r.toggle_bookmark())
        })
        .child(icon_btn(MI_SEARCH, tr!("notes.reader.search"), move || ctl.panel.set(Panel::Search)));
    let pages = match num {
        Some((n, t)) => format!("{n} / {t}"),
        None => String::new(),
    };
    let slider = {
        let r = r.clone();
        Slider::new().range(0.0, 1.0).value(progress).on_change(move |v| {
            let Some(book) = r.book() else { return };
            let loc = match &book.content {
                Content::Flow(d) => Location::flow(d.pos_at_fraction(v)),
                Content::Pdf(w) => Location::Pdf { page: ((v * w.page_count() as f32) as usize).min(w.page_count().saturating_sub(1)) },
            };
            r.go(loc);
        })
    };
    let bottom = Column::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Text::new(if pages.is_empty() { format!("{}%", (progress * 100.0).round() as i32) } else { format!("{pages} · {}%", (progress * 100.0).round() as i32) }).class("reader-menu-sub"))
                .child(DecoratedBox::new().class("grow").child(slider)),
        )
        .child(
            Row::new()
                .gap(4.0)
                .main_axis_alignment(MainAxisAlignment::SpaceAround)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(labeled(MI_TOC, tr!("notes.reader.toc"), move || ctl.panel.set(Panel::Toc)))
                .child(labeled(MI_BOOKMARK_BORDER, tr!("notes.reader.bookmarks"), move || ctl.panel.set(Panel::Bookmarks)))
                .child(labeled(MI_HIGHLIGHT, tr!("notes.reader.highlights"), move || ctl.panel.set(Panel::Highlights)))
                .child(labeled(MI_FORMAT_SIZE, tr!("notes.reader.text"), move || ctl.panel.set(Panel::Text)))
                .child(labeled(MI_DRAW, if ctl.annotate.get_untracked() { tr!("notes.reader.pen_off") } else { tr!("notes.reader.pen") }, move || {
                    ctl.annotate.update(|a| *a = !*a);
                    ctl.panel.set(Panel::None);
                }))
                .child(labeled(MI_AUTORENEW, tr!("notes.reader.refresh"), move || {
                    ctl.panel.set(Panel::None);
                    refresh_eink();
                })),
        );
    Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("grow")
        .child(DecoratedBox::new().class("reader-sheet reader-sheet-top").child(top))
        .child(DecoratedBox::new().class("grow").child(fill(GestureDetector::new().on_click(move || ctl.panel.set(Panel::None)).child(fill(DecoratedBox::new())))))
        .child(DecoratedBox::new().class("reader-sheet reader-sheet-bottom").child(bottom))
}

fn labeled(icon: &'static str, label: String, f: impl FnMut() + Send + 'static) -> impl Widget {
    GestureDetector::new()
        .cursor(CursorIcon::Pointer)
        .on_click(f)
        .child(
            Column::new()
                .gap(2.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .class("reader-action")
                .child(syngui::widgets::Icon::new(icon.to_string()).class("reader-action-icon"))
                .child(Text::new(label).class("reader-action-label")),
        )
}

/// Шапка выдвижной панели: заголовок и «закрыть».
fn sheet_header(title: String, ctl: Ctl) -> impl Widget {
    Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().class("grow").child(Text::new(title).class("reader-sheet-title")))
        .child(icon_btn(MI_CLOSE, tr!("notes.reader.close"), move || ctl.panel.set(Panel::None)))
}

/// Боковая панель поверх страницы, тап мимо — закрыть.
fn side_sheet(ctl: Ctl, body: impl Widget + 'static) -> impl Widget {
    Row::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("grow")
        .child(DecoratedBox::new().class("reader-side").child(body))
        .child(DecoratedBox::new().class("grow").child(fill(GestureDetector::new().on_click(move || ctl.panel.set(Panel::None)).child(fill(DecoratedBox::new().class("reader-scrim"))))))
}

fn list_row(title: String, sub: String, level: u8, current: bool, f: impl FnMut() + Send + 'static) -> W {
    let class = if current { "reader-row current" } else { "reader-row" };
    Box::new(
        GestureDetector::new().cursor(CursorIcon::Pointer).on_click(f).child(
            DecoratedBox::new().class(class).style("padding-left", StyleValue::px(12.0 + 16.0 * level.saturating_sub(1) as f32)).child(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(DecoratedBox::new().class("grow").child(Text::new(title).max_lines(2).elide(Elide::End).class("reader-row-title")))
                    .child(Text::new(sub).class("reader-row-sub")),
            ),
        ),
    )
}

/// Оглавление, закладки, выделения — вкладками одной панели.
fn lists_panel(r: Reader, panel: Panel) -> impl Widget {
    let ctl = r.ctl;
    let state = ctl.state.get_untracked();
    let tabs = Row::new()
        .gap(4.0)
        .children(
            [(Panel::Toc, tr!("notes.reader.toc")), (Panel::Bookmarks, tr!("notes.reader.bookmarks")), (Panel::Highlights, tr!("notes.reader.highlights"))]
                .into_iter()
                .map(|(p, t)| -> W {
                    Box::new(
                        GestureDetector::new()
                            .cursor(CursorIcon::Pointer)
                            .on_click(move || ctl.panel.set(p))
                            .child(DecoratedBox::new().class(if p == panel { "reader-tab active" } else { "reader-tab" }).child(Text::new(t))),
                    )
                })
                .collect::<Vec<_>>(),
        );
    let mut col = Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch);
    let book = r.book();
    let progress_of = |loc: Location| -> String {
        match (&book, loc) {
            (Some(b), Location::Flow { .. }) => match &b.content {
                Content::Flow(d) => format!("{}%", (d.progress(loc.pos()) * 100.0).round() as i32),
                _ => String::new(),
            },
            (_, Location::Pdf { page }) => format!("{}", page + 1),
            _ => String::new(),
        }
    };
    let mut count = 0;
    match panel {
        Panel::Toc => {
            let toc = book.as_ref().map(|b| b.toc()).unwrap_or_default();
            // Текущая глава — последняя, начало которой не дальше позиции.
            let cur = toc.iter().rposition(|t| match (t.loc, state.pos) {
                (Location::Pdf { page: a }, Location::Pdf { page: b }) => a <= b,
                (a, b) => a.pos() <= b.pos(),
            });
            for (i, t) in toc.into_iter().enumerate() {
                let r2 = r.clone();
                let loc = t.loc;
                col = col.child(list_row(t.title, progress_of(loc), t.level, Some(i) == cur, move || {
                    r2.go(loc);
                    r2.ctl.panel.set(Panel::None);
                }));
                count += 1;
            }
        }
        Panel::Bookmarks => {
            for b in state.bookmarks.iter() {
                let r2 = r.clone();
                let loc = b.loc;
                col = col.child(list_row(b.label.clone(), progress_of(loc), 1, loc == state.pos, move || {
                    r2.go(loc);
                    r2.ctl.panel.set(Panel::None);
                }));
                count += 1;
            }
        }
        _ => {
            for h in state.highlights.iter().rev() {
                let r2 = r.clone();
                let loc = h.start;
                let title = if h.note.is_empty() { format!("«{}»", h.text) } else { format!("«{}» — {}", h.text, h.note) };
                let id = h.id.clone();
                let rd = r.clone();
                col = col.child(
                    Row::new()
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(DecoratedBox::new().class("grow").child(list_row(title, progress_of(loc), 1, false, move || {
                            r2.go(loc);
                            r2.ctl.panel.set(Panel::None);
                        })))
                        .child(icon_btn(MI_EDIT_NOTE, tr!("notes.reader.note"), {
                            let id = id.clone();
                            move || ctl.note_edit.set(Some(id.clone()))
                        }))
                        .child(icon_btn(MI_DELETE, tr!("notes.reader.delete"), move || {
                            let id = id.clone();
                            rd.update_state(|s| s.highlights.retain(|x| x.id != id));
                            rd.bump();
                            ctl.panel.set(Panel::Highlights);
                        })),
                );
                count += 1;
            }
        }
    }
    let body: W = if count == 0 {
        Box::new(Text::new(match panel {
            Panel::Toc => tr!("notes.reader.no_toc"),
            Panel::Bookmarks => tr!("notes.reader.no_bookmarks"),
            _ => tr!("notes.reader.no_highlights"),
        }).class("reader-msg"))
    } else {
        Box::new(ScrollView::new().vertical().child(col))
    };
    side_sheet(
        ctl,
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("grow")
            .child(sheet_header(state.title.clone(), ctl))
            .child(tabs)
            .child(DecoratedBox::new().class("grow").clip(true).child(body)),
    )
}

fn search_panel(r: Reader) -> impl Widget {
    let ctl = r.ctl;
    let field = {
        let r = r.clone();
        TextField::with_text(ctl.query.get_untracked())
            .placeholder(tr!("notes.reader.search_hint"))
            .prefix_icon(MI_SEARCH)
            .autofocus(true)
            .on_submit(move |q| r.search(q))
    };
    let results = {
        let r = r.clone();
        Reactive::new(move || -> Vec<W> {
            let hits = ctl.hits.get();
            let q = ctl.query.get();
            if hits.is_empty() {
                let msg = if q.is_empty() { String::new() } else { tr!("notes.reader.not_found") };
                return vec![Box::new(Text::new(msg).class("reader-msg"))];
            }
            let mut col = Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch);
            col = col.child(Text::new(tr!("notes.reader.found", n = hits.len())).class("reader-row-sub"));
            for h in hits {
                let r2 = r.clone();
                let label = h.label.clone();
                col = col.child(list_row(label, String::new(), 1, false, move || {
                    r2.ctl.hit.set(Some(h.clone()));
                    r2.go(h.loc);
                    r2.ctl.panel.set(Panel::None);
                }));
            }
            vec![Box::new(ScrollView::new().vertical().child(col))]
        })
    };
    side_sheet(
        ctl,
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("grow")
            .child(sheet_header(tr!("notes.reader.search"), ctl))
            .child(field)
            .child(DecoratedBox::new().class("grow").clip(true).child(fill(results))),
    )
}

/// Ряд «название  [−] значение [+]».
fn stepper(label: String, value: String, minus: impl FnMut() + Send + 'static, plus: impl FnMut() + Send + 'static) -> W {
    Box::new(
        Row::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("reader-setting")
            .child(DecoratedBox::new().class("grow").child(Text::new(label)))
            .child(ToolButton::new(MI_REMOVE).on_click(minus).class("reader-btn"))
            .child(DecoratedBox::new().class("reader-value").child(Text::new(value)))
            .child(ToolButton::new(MI_ADD).on_click(plus).class("reader-btn")),
    )
}

fn switch(label: String, on: bool, mut f: impl FnMut(bool) + Send + 'static) -> W {
    Box::new(
        Row::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("reader-setting")
            .child(DecoratedBox::new().class("grow").child(Text::new(label)))
            .child(syngui::widgets::Toggle::with_state(on).on_change(move |v| f(v))),
    )
}

fn chips(options: Vec<(String, bool)>, f: impl Fn(usize) + Send + Sync + 'static) -> W {
    let f = Arc::new(f);
    Box::new(
        Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).children(
            options
                .into_iter()
                .enumerate()
                .map(|(i, (t, on))| -> W {
                    let f = f.clone();
                    Box::new(
                        GestureDetector::new()
                            .cursor(CursorIcon::Pointer)
                            .on_click(move || f(i))
                            .child(DecoratedBox::new().class(if on { "reader-chip active" } else { "reader-chip" }).child(Text::new(t))),
                    )
                })
                .collect::<Vec<_>>(),
        ),
    )
}

/// Гарнитуры на выбор (что есть в системе — решает шрифтовой движок).
pub const FONTS: &[(&str, Option<&str>)] = &[("", None), ("Noto Serif", Some("Noto Serif")), ("DejaVu Serif", Some("DejaVu Serif")), ("PT Serif", Some("PT Serif")), ("Literata", Some("Literata")), ("Noto Sans", Some("Noto Sans"))];

/// Гарнитура есть в системе (fontconfig; без него — считаем, что есть).
fn font_installed(family: &str) -> bool {
    static LIST: OnceLock<Option<String>> = OnceLock::new();
    let list = LIST.get_or_init(|| {
        let out = std::process::Command::new("fc-list").args([":", "family"]).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).to_lowercase())
    });
    match list {
        Some(l) => l.lines().any(|line| line.split(',').any(|f| f.trim() == family.to_lowercase())),
        None => true,
    }
}

fn text_panel(r: Reader) -> impl Widget {
    let ctl = r.ctl;
    let s = ctl.state.get_untracked();
    let is_pdf = r.book().is_some_and(|b| matches!(b.content, Content::Pdf(_)));
    let set = {
        let r = r.clone();
        move |f: Box<dyn FnOnce(&mut BookState)>| {
            // Позиция остаётся; страница перестроится вокруг неё.
            r.update_state(|s| f(s));
            r.bump();
            ctl.panel.set(Panel::Text);
        }
    };
    let set = Arc::new(set);
    let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    col = col.child(sheet_header(tr!("notes.reader.text"), ctl));
    if !is_pdf {
        let st = s.settings.clone();
        let (a, b) = (set.clone(), set.clone());
        col = col.child(stepper(tr!("notes.reader.font_size"), format!("{}", st.font_size.round() as i32), move || a(Box::new(|s| s.settings.font_size = (s.settings.font_size - 2.0).max(10.0))), move || b(Box::new(|s| s.settings.font_size = (s.settings.font_size + 2.0).min(64.0)))));
        let (a, b) = (set.clone(), set.clone());
        col = col.child(stepper(tr!("notes.reader.line_height"), format!("{:.2}", st.line_height), move || a(Box::new(|s| s.settings.line_height = (s.settings.line_height - 0.1).max(1.0))), move || b(Box::new(|s| s.settings.line_height = (s.settings.line_height + 0.1).min(2.5)))));
        let (a, b) = (set.clone(), set.clone());
        col = col.child(stepper(tr!("notes.reader.margins"), format!("{}", st.margin.round() as i32), move || a(Box::new(|s| s.settings.margin = (s.settings.margin - 8.0).max(4.0))), move || b(Box::new(|s| s.settings.margin = (s.settings.margin + 8.0).min(120.0)))));
        let avail: Vec<&(&str, Option<&str>)> = FONTS.iter().filter(|(_, f)| f.map_or(true, font_installed) || st.family.as_deref() == *f).collect();
        let fonts: Vec<(String, bool)> = avail.iter().map(|(n, f)| (if n.is_empty() { tr!("notes.reader.font_system") } else { n.to_string() }, st.family.as_deref() == *f)).collect();
        let fams: Vec<Option<String>> = avail.iter().map(|(_, f)| f.map(str::to_string)).collect();
        let a = set.clone();
        col = col.child(Text::new(tr!("notes.reader.font")).class("reader-row-sub"));
        col = col.child(chips(fonts, move |i| {
            let fam = fams[i].clone();
            a(Box::new(move |s| s.settings.family = fam));
        }));
        let a = set.clone();
        col = col.child(switch(tr!("notes.reader.justify"), st.justify, move |v| a(Box::new(move |s| s.settings.justify = v))));
        let a = set.clone();
        col = col.child(switch(tr!("notes.reader.hyphenate"), st.hyphenate, move |v| a(Box::new(move |s| s.settings.hyphenate = v))));
        let a = set.clone();
        col = col.child(switch(tr!("notes.reader.bold"), st.bold, move |v| a(Box::new(move |s| s.settings.bold = v))));
    } else {
        let a = set.clone();
        col = col.child(Text::new(tr!("notes.reader.fit")).class("reader-row-sub"));
        col = col.child(chips(
            vec![(tr!("notes.reader.fit_page"), s.pdf_fit == PdfFit::Page), (tr!("notes.reader.fit_width"), s.pdf_fit == PdfFit::Width)],
            move |i| a(Box::new(move |s| s.pdf_fit = if i == 0 { PdfFit::Page } else { PdfFit::Width })),
        ));
    }
    let a = set.clone();
    col = col.child(switch(tr!("notes.reader.spread"), s.spread, move |v| a(Box::new(move |s| s.spread = v))));
    let (a, b) = (set.clone(), set.clone());
    let every = if s.refresh_every == 0 { tr!("notes.reader.never") } else { s.refresh_every.to_string() };
    col = col.child(stepper(tr!("notes.reader.refresh_every"), every, move || a(Box::new(|s| s.refresh_every = s.refresh_every.saturating_sub(1))), move || b(Box::new(|s| s.refresh_every = (s.refresh_every + 1).min(50)))));
    Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("grow")
        .child(DecoratedBox::new().class("grow").child(fill(GestureDetector::new().on_click(move || ctl.panel.set(Panel::None)).child(fill(DecoratedBox::new())))))
        .child(DecoratedBox::new().class("reader-sheet reader-sheet-bottom").child(ScrollView::new().vertical().child(col)))
}

/// Панель над выделением: маркер, заметка, копировать, отмена.
fn selection_bar(r: Reader) -> impl Widget {
    let ctl = r.ctl;
    let (r1, r2, r3) = (r.clone(), r.clone(), r.clone());
    let bar = Row::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .class("reader-selbar")
        .child(labeled(MI_HIGHLIGHT, tr!("notes.reader.highlight"), move || {
            r1.highlight_selection("");
        }))
        .child(labeled(MI_EDIT_NOTE, tr!("notes.reader.note"), move || {
            if let Some(id) = r2.highlight_selection("") {
                r2.ctl.note_edit.set(Some(id));
            }
        }))
        .child(labeled(MI_CONTENT_COPY, tr!("notes.reader.copy"), move || {
            syngui::clipboard::copy(&r3.selection_text());
            r3.ctl.selection.set(None);
        }))
        .child(labeled(MI_CLOSE, tr!("notes.reader.cancel"), move || ctl.selection.set(None)));
    Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("grow")
        .child(DecoratedBox::new().class("grow"))
        .child(Row::new().main_axis_alignment(MainAxisAlignment::Center).child(bar))
}

/// Правка заметки к выделению.
fn note_dialog(r: Reader, id: String) -> impl Widget {
    let ctl = r.ctl;
    let h = ctl.state.get_untracked().highlights.into_iter().find(|h| h.id == id);
    let (quote, note) = h.map(|h| (h.text, h.note)).unwrap_or_default();
    let text = Arc::new(Mutex::new(note.clone()));
    let save = {
        let r = r.clone();
        let id = id.clone();
        let text = text.clone();
        move || {
            let t = text.lock().unwrap().clone();
            let id = id.clone();
            r.update_state(|s| {
                if let Some(h) = s.highlights.iter_mut().find(|h| h.id == id) {
                    h.note = t;
                }
            });
            r.ctl.note_edit.set(None);
            r.bump();
        }
    };
    let save2 = save.clone();
    let field = TextField::with_text(note)
        .placeholder(tr!("notes.reader.note_hint"))
        .autofocus(true)
        .on_change({
            let text = text.clone();
            move |t| *text.lock().unwrap() = t.to_string()
        })
        .on_submit(move |_| save2())
        .on_escape(move || ctl.note_edit.set(None));
    Center::new().child(
        DecoratedBox::new().class("reader-dialog").child(
            Column::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(Text::new(tr!("notes.reader.note")).class("reader-sheet-title"))
                .child(Text::new(format!("«{quote}»")).max_lines(4).elide(Elide::End).class("reader-quote"))
                .child(field)
                .child(
                    Row::new()
                        .gap(8.0)
                        .main_axis_alignment(MainAxisAlignment::End)
                        .child(Button::new(tr!("notes.reader.cancel")).on_click(move || ctl.note_edit.set(None)))
                        .child(Button::new(tr!("notes.reader.save")).on_click(save)),
                ),
        ),
    )
}

/// Кнопки читалки для шапки открытой страницы.
pub fn header_actions(r: Reader) -> impl Widget {
    let ctl = r.ctl;
    let bm = {
        let r = r.clone();
        Reactive::new(move || -> Vec<W> {
            let s = ctl.state.get();
            let r = r.clone();
            let marked = s.has_bookmark(s.pos);
            vec![Box::new(icon_btn(if marked { MI_BOOKMARK } else { MI_BOOKMARK_BORDER }, tr!("notes.reader.bookmark"), move || r.toggle_bookmark()))]
        })
    };
    let pen = Reactive::new(move || -> Vec<W> {
        let on = ctl.annotate.get();
        vec![Box::new(icon_btn(MI_DRAW, if on { tr!("notes.reader.pen_off") } else { tr!("notes.reader.pen") }, move || ctl.annotate.update(|a| *a = !*a)).class(if on { "reader-btn active" } else { "reader-btn" }))]
    });
    Row::new()
        .gap(2.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(icon_btn(MI_TOC, tr!("notes.reader.toc"), move || ctl.panel.set(Panel::Toc)))
        .child(icon_btn(MI_SEARCH, tr!("notes.reader.search"), move || ctl.panel.set(Panel::Search)))
        .child(icon_btn(MI_FORMAT_SIZE, tr!("notes.reader.text"), move || ctl.panel.set(Panel::Text)))
        .child(pen)
        .child(bm)
}
