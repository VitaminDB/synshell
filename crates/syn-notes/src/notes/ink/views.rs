//! Рукописные страницы в библиотеке заметок: блокнот (`PageKind::Notebook`)
//! и рисунок (`PageKind::Drawing`).
//!
//! Документ страницы — `notes/objects/<id>.ink`, миниатюра первой страницы —
//! `notes/objects/<id>.ink.png` (карточка библиотеки не грузит документ).
//! Открытые документы живут в памяти до конца сеанса; сохранение — в фоне,
//! через 1 с после последней правки (сериализация не на главном потоке);
//! [`flush_all`] — записать всё сразу (выход).
//!
//! Вид блокнота в духе Onyx: панель инструментов сверху, страница по
//! центру, внизу «‹ 3 / 12 › +»; тап по номеру — обзор страниц сеткой.
//! Рисунок — плавающая вертикальная панель слева и бесконечный холст.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use syngui::core::{Point, Rect};
use syngui::prelude::*;
use syngui::widgets::overlay::menu::MenuItem;
use syngui::widgets::{GestureDetector, Image, ImageFit, PopupMenu, ToolButton};

use super::icons as ic;
use super::model::{DocKind, InkDoc};
use super::raster::{self, RasterOpts};
use super::{codec, InkCanvas, InkHandle, InkToolbar};
use crate::notes::library::{KindViews, Library};
use crate::notes::project::{self, PageKind};
use crate::notes::{autosave, NotesCtx};

type W = Box<dyn Widget>;
type Key = (PathBuf, String);

/// Ширина миниатюры, px.
const THUMB_W: f32 = 300.0;
/// Сохранение — через столько после последней правки.
const SAVE_DELAY: Duration = Duration::from_millis(1000);

pub fn ink_path(page_id: &str) -> String {
    format!("{}/{page_id}.ink", project::OBJECTS_DIR)
}

pub fn thumb_path(page_id: &str) -> String {
    format!("{}/{page_id}.ink.png", project::OBJECTS_DIR)
}

/// Для `library::set_kind_views`.
pub fn views_for_library() -> KindViews {
    KindViews { thumbnail, details, view: page_view, actions }
}

// ─────────────────────────── Документы и сохранение ───────────────────────────

thread_local! {
    static DOCS: RefCell<HashMap<Key, InkHandle>> = RefCell::new(HashMap::new());
}

struct Pending {
    doc: Arc<Mutex<InkDoc>>,
    changed: Instant,
}

fn pending() -> &'static Mutex<HashMap<Key, Pending>> {
    static P: OnceLock<Mutex<HashMap<Key, Pending>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Clone)]
struct Thumb {
    png: Arc<Vec<u8>>,
    pages: usize,
    gen: u64,
}

fn thumbs() -> &'static Mutex<HashMap<Key, Thumb>> {
    static T: OnceLock<Mutex<HashMap<Key, Thumb>>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_gen() -> u64 {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Фоновый поток сохранения (один на процесс).
fn ensure_saver() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new().name("ink-save".into()).spawn(|| loop {
            std::thread::sleep(Duration::from_millis(300));
            let due: Vec<(Key, Arc<Mutex<InkDoc>>)> = {
                let mut p = lock(pending());
                let keys: Vec<Key> = p.iter().filter(|(_, v)| v.changed.elapsed() >= SAVE_DELAY).map(|(k, _)| k.clone()).collect();
                keys.into_iter().filter_map(|k| p.remove(&k).map(|v| (k, v.doc))).collect()
            };
            for (k, doc) in due {
                save_now(&k, &doc);
            }
        });
    });
}

/// Сериализовать и поставить в очередь записи бандла; миниатюру — тоже.
fn save_now(key: &Key, doc: &Arc<Mutex<InkDoc>>) {
    let mut d = lock(doc).clone();
    d.gc_assets();
    let bytes = codec::save(&d);
    autosave::queue_bytes_to(&key.0, &ink_path(&key.1), bytes);
    if let Some(png) = render_thumb(&d) {
        autosave::queue_bytes_to(&key.0, &thumb_path(&key.1), png.clone());
        lock(thumbs()).insert(key.clone(), Thumb { png: Arc::new(png), pages: d.pages.len(), gen: next_gen() });
    }
}

fn render_thumb(d: &InkDoc) -> Option<Vec<u8>> {
    let pg = d.pages.first()?;
    let ext = raster::page_extent(pg);
    let scale = THUMB_W / ext.w().max(1.0);
    raster::export_png(d, 0, scale, &RasterOpts::default())
}

/// Записать все несохранённые рукописные документы сейчас (перед выходом —
/// до `autosave::flush_all`).
pub fn flush_all() {
    let all: Vec<(Key, Arc<Mutex<InkDoc>>)> = lock(pending()).drain().map(|(k, v)| (k, v.doc)).collect();
    for (k, doc) in all {
        save_now(&k, &doc);
    }
}

/// Документ страницы: из памяти, из бандла или новый по типу.
pub fn open_doc(project: &Path, page_id: &str, kind: PageKind) -> InkHandle {
    let key: Key = (project.to_path_buf(), page_id.to_string());
    if let Some(h) = DOCS.with(|d| d.borrow().get(&key).cloned()) {
        return h;
    }
    let doc = project::read_bytes(project, &ink_path(page_id))
        .and_then(|b| match codec::load(&b) {
            Ok(d) => Some(d),
            Err(e) => {
                log::warn!("ink: {page_id}: {e}");
                None
            }
        })
        .unwrap_or_else(|| match kind {
            PageKind::Drawing => InkDoc::drawing(),
            _ => InkDoc::notebook(),
        });
    let h = InkHandle::new(doc);
    ensure_saver();
    let k2 = key.clone();
    let arc = h.doc_arc();
    h.on_change(move |_| {
        let mut p = lock(pending());
        p.insert(k2.clone(), Pending { doc: arc.clone(), changed: Instant::now() });
    });
    DOCS.with(|d| d.borrow_mut().insert(key, h.clone()));
    h
}

/// Документ, если он уже открыт в этом сеансе.
pub fn opened(project: &Path, page_id: &str) -> Option<InkHandle> {
    DOCS.with(|d| d.borrow().get(&(project.to_path_buf(), page_id.to_string())).cloned())
}

// ─────────────────────────── Карточка библиотеки ───────────────────────────

fn is_ink(kind: PageKind) -> bool {
    matches!(kind, PageKind::Notebook | PageKind::Drawing)
}

/// Миниатюра первой страницы (PNG) и число страниц: из памяти, из бандла
/// (`.ink.png`), иначе — нарисовать из документа.
fn thumb_of(project: &Path, page_id: &str) -> Option<Thumb> {
    let key: Key = (project.to_path_buf(), page_id.to_string());
    // Открытый документ с несохранёнными правками — рисуем его сейчас.
    if lock(pending()).contains_key(&key) {
        if let Some(h) = opened(project, page_id) {
            let d = h.lock().clone();
            if let Some(png) = render_thumb(&d) {
                let t = Thumb { png: Arc::new(png), pages: d.pages.len(), gen: next_gen() };
                lock(thumbs()).insert(key.clone(), t.clone());
                return Some(t);
            }
        }
    }
    if let Some(t) = lock(thumbs()).get(&key).cloned() {
        return Some(t);
    }
    let pages = opened(project, page_id).map(|h| h.page_count());
    let t = match project::read_bytes(project, &thumb_path(page_id)) {
        Some(png) => {
            let pages = pages.or_else(|| project::read_bytes(project, &ink_path(page_id)).and_then(|b| codec::load(&b).ok()).map(|d| d.pages.len()));
            Thumb { png: Arc::new(png), pages: pages.unwrap_or(1), gen: next_gen() }
        }
        None => {
            let d = match opened(project, page_id) {
                Some(h) => h.lock().clone(),
                None => codec::load(&project::read_bytes(project, &ink_path(page_id))?).ok()?,
            };
            Thumb { png: Arc::new(render_thumb(&d)?), pages: d.pages.len(), gen: next_gen() }
        }
    };
    lock(thumbs()).insert(key, t.clone());
    Some(t)
}

pub fn thumbnail(_ctx: NotesCtx, page_id: &str, kind: PageKind) -> Option<W> {
    if !is_ink(kind) {
        return None;
    }
    let t = thumb_of(&autosave::project_path(), page_id)?;
    Some(Box::new(
        Image::from_bytes(format!("ink-thumb:{page_id}:{}", t.gen), (*t.png).clone()).fit(ImageFit::Contain).placeholder(false).class("ink-thumb"),
    ))
}

pub fn details(_ctx: NotesCtx, page_id: &str, kind: PageKind) -> Option<String> {
    if kind != PageKind::Notebook {
        return None;
    }
    let project = autosave::project_path();
    let n = match opened(&project, page_id) {
        Some(h) => h.page_count(),
        None => thumb_of(&project, page_id)?.pages,
    };
    super::register_i18n();
    Some(syngui::i18n::tr("notes.ink.pages_count").replace("{n}", &n.to_string()))
}

// ─────────────────────────── Открытая страница ───────────────────────────

pub fn page_view(_ctx: NotesCtx, _lib: Library, page_id: &str, kind: PageKind) -> Option<W> {
    if !is_ink(kind) {
        return None;
    }
    let h = open_doc(&autosave::project_path(), page_id, kind);
    Some(match h.kind() {
        DocKind::Drawing => drawing_view(h),
        _ => notebook_view(h),
    })
}

fn fill<M>(w: impl syngui::widgets::containers::IntoWidget<M>) -> Stack {
    Stack::new().fit(StackFit::Expand).child(w)
}

/// Блокнот: панель сверху, страница, листание снизу; обзор страниц.
pub fn notebook_view(h: InkHandle) -> W {
    let overview = use_signal(false);
    let toolbar = InkToolbar::new(h.clone()).build();
    let body = {
        let h = h.clone();
        Reactive::new(move || -> Vec<W> {
            if overview.get() {
                vec![Box::new(fill(pages_overview(h.clone(), overview)))]
            } else {
                vec![Box::new(fill(InkCanvas::new(h.clone())))]
            }
        })
    };
    Box::new(
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("grow ink-view")
            .child(DecoratedBox::new().class("ink-topbar").child(toolbar))
            .child(DecoratedBox::new().class("grow ink-body").clip(true).child(fill(body)))
            .child(page_bar(h, overview)),
    )
}

/// Рисунок: вертикальная панель слева, холст.
pub fn drawing_view(h: InkHandle) -> W {
    Box::new(
        Row::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("grow ink-view")
            .child(DecoratedBox::new().class("ink-sidebar").child(InkToolbar::new(h.clone()).vertical(true).build()))
            .child(DecoratedBox::new().class("grow ink-body").clip(true).child(fill(InkCanvas::new(h)))),
    )
}

/// «‹  3 / 12  ›  +»: тап по номеру — обзор страниц.
fn page_bar(h: InkHandle, overview: RwSignal<bool>) -> impl Widget {
    Reactive::new(move || -> Vec<W> {
        let _ = h.revision.get();
        let cur = h.page.get();
        let n = h.page_count();
        let cur = cur.min(n.saturating_sub(1));
        let ov = overview.get();
        let (a, b, c) = (h.clone(), h.clone(), h.clone());
        let label = syngui::i18n::tr("notes.ink.page_of").replace("{n}", &(cur + 1).to_string()).replace("{total}", &n.to_string());
        vec![Box::new(
            DecoratedBox::new().class("ink-pagebar").child(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(DecoratedBox::new().class("grow"))
                    .child(ToolButton::new(ic::PREV).tooltip(syngui::i18n::tr("notes.ink.page.prev")).disabled(cur == 0 || ov).on_click(move || a.prev_page()).class("ink-tool"))
                    .child(
                        GestureDetector::new()
                            .on_click(move || overview.set(!overview.get_untracked()))
                            .child(
                                Row::new()
                                    .gap(6.0)
                                    .cross_axis_alignment(CrossAxisAlignment::Center)
                                    .child(Icon::new(ic::PAGES).class("ink-pagebar-icon"))
                                    .child(Text::new(label).class("ink-pagebar-num")),
                            )
                            .class(if ov { "ink-pagebar-btn active" } else { "ink-pagebar-btn" }),
                    )
                    .child(ToolButton::new(ic::NEXT).tooltip(syngui::i18n::tr("notes.ink.page.next")).disabled(cur + 1 >= n || ov).on_click(move || b.next_page(false)).class("ink-tool"))
                    .child(DecoratedBox::new().class("grow"))
                    .child(
                        ToolButton::new(ic::ADD)
                            .tooltip(syngui::i18n::tr("notes.ink.page.add"))
                            .on_click(move || {
                                let at = c.current() + 1;
                                c.insert_page(at);
                                overview.set(false);
                            })
                            .class("ink-tool"),
                    ),
            ),
        )]
    })
}

thread_local! {
    /// Миниатюры страниц обзора: (страница, ревизия) → PNG.
    static PAGE_THUMBS: RefCell<HashMap<u64, (u64, Arc<Vec<u8>>)>> = RefCell::new(HashMap::new());
}

fn page_thumb(h: &InkHandle, i: usize) -> Option<(u64, Arc<Vec<u8>>)> {
    let rev = h.revision.get_untracked();
    let d = h.lock();
    let pg = d.pages.get(i)?;
    let id = pg.id;
    if let Some(t) = PAGE_THUMBS.with(|m| m.borrow().get(&id).cloned()) {
        // Правки этой страницы после миниатюры — перерисовать.
        if t.0 == rev || matches!(h.dirty_since(t.0, id), super::Dirty::Nothing) {
            return Some((id, t.1));
        }
    }
    let ext = raster::page_extent(pg);
    let png = raster::export_png(&d, i, 200.0 / ext.w().max(1.0), &RasterOpts::default())?;
    let png = Arc::new(png);
    PAGE_THUMBS.with(|m| m.borrow_mut().insert(id, (rev, png.clone())));
    Some((id, png))
}

/// Обзор страниц сеткой: тап — открыть, «×» — удалить, «‹›» — переставить.
fn pages_overview(h: InkHandle, overview: RwSignal<bool>) -> impl Widget {
    let vp = syngui::viewport::viewport_size();
    Reactive::new(move || -> Vec<W> {
        let _ = h.revision.get();
        let cur = h.page.get();
        let cols = ((vp.get().width - 32.0) / 180.0).floor().clamp(2.0, 8.0) as usize;
        let n = h.page_count();
        let mut cells: Vec<W> = Vec::new();
        for i in 0..n {
            let (id, png) = match page_thumb(&h, i) {
                Some(x) => x,
                None => continue,
            };
            let rev = h.revision.get_untracked();
            let (go, del, left, right) = (h.clone(), h.clone(), h.clone(), h.clone());
            let img = Image::from_bytes(format!("ink-page:{id}:{rev}"), (*png).clone()).fit(ImageFit::Contain).placeholder(false);
            cells.push(Box::new(
                Column::new()
                    .gap(4.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .class(if i == cur { "ink-page-cell current" } else { "ink-page-cell" })
                    .child(
                        GestureDetector::new()
                            .on_click(move || {
                                go.go_to(i);
                                overview.set(false);
                            })
                            .child(DecoratedBox::new().class("ink-page-thumb").child(fill(img))),
                    )
                    .child(
                        Row::new()
                            .gap(2.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(ToolButton::new(ic::PREV).disabled(i == 0).on_click(move || left.move_page(i, i - 1)))
                            .child(DecoratedBox::new().class("grow").child(Center::new().child(Text::new(format!("{}", i + 1)).class("ink-page-num"))))
                            .child(ToolButton::new(ic::NEXT).disabled(i + 1 >= n).on_click(move || right.move_page(i, i + 1)))
                            .child(ToolButton::new(ic::DELETE).tooltip(syngui::i18n::tr("notes.ink.page.delete")).on_click(move || del.delete_page(i))),
                    ),
            ));
        }
        let add = h.clone();
        cells.push(Box::new(
            GestureDetector::new()
                .on_click(move || {
                    let n = add.page_count();
                    add.insert_page(n);
                    overview.set(false);
                })
                .child(DecoratedBox::new().class("ink-page-cell ink-page-add").child(Center::new().child(Icon::new(ic::ADD).class("ink-page-add-icon")))),
        ));
        vec![Box::new(ScrollView::new().vertical().child(DecoratedBox::new().class("ink-pages").child(Grid::new(cols).gap(16.0).children(cells))))]
    })
}

// ─────────────────────────── Действия в шапке ───────────────────────────

pub fn actions(ctx: NotesCtx, _lib: Library, page_id: &str, kind: PageKind) -> Option<W> {
    if !is_ink(kind) {
        return None;
    }
    let h = open_doc(&autosave::project_path(), page_id, kind);
    let open = use_signal(false);
    let pos = use_signal(Point::zero());
    let id = page_id.to_string();
    let items = vec![
        MenuItem::new("image", syngui::i18n::tr("notes.ink.insert_image")).icon(ic::IMAGE),
        MenuItem::new("png", syngui::i18n::tr("notes.ink.export.png")).icon(ic::EXPORT),
        MenuItem::new("svg", syngui::i18n::tr("notes.ink.export.svg")).icon(ic::EXPORT),
    ];
    let hm = h.clone();
    let menu = PopupMenu::new().items(items).is_open(open).position(pos).on_select(move |k: &str| {
        let title = ctx.tree.get_untracked().title_of(&id).unwrap_or_else(|| "ink".into());
        match k {
            "image" => insert_image_dialog(hm.clone()),
            "png" => export(&hm, &title, "png"),
            "svg" => export(&hm, &title, "svg"),
            _ => {}
        }
    });
    let undo_redo = {
        let h = h.clone();
        Reactive::new(move || -> Vec<W> {
            let _ = h.revision.get();
            let (u, r) = (h.clone(), h.clone());
            vec![Box::new(
                Row::new()
                    .gap(2.0)
                    .child(ToolButton::new(ic::UNDO).tooltip(syngui::i18n::tr("notes.ink.undo")).disabled(!h.can_undo()).on_click(move || u.undo()).class("lib-btn"))
                    .child(ToolButton::new(ic::REDO).tooltip(syngui::i18n::tr("notes.ink.redo")).disabled(!h.can_redo()).on_click(move || r.redo()).class("lib-btn")),
            )]
        })
    };
    Some(Box::new(
        Row::new()
            .gap(2.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(undo_redo)
            .child(
                Stack::new().clip(false).child(
                    ToolButton::new(ic::EXPORT)
                        .tooltip(syngui::i18n::tr("notes.ink.export.png"))
                        .on_click_with_bounds(move |_, b: Rect| {
                            pos.set(Point::new(b.origin.x, b.origin.y + b.size.height + 4.0));
                            open.set(true);
                        })
                        .class("lib-btn"),
                ).child(menu),
            ),
    ))
}

/// Экспорт текущей страницы в «Изображения» (или домашнюю папку).
fn export(h: &InkHandle, title: &str, ext: &str) {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    let dir = [home.join("Pictures"), home.join("Изображения"), home.clone()].into_iter().find(|d| d.is_dir()).unwrap_or(home);
    let safe: String = title.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
    let path = dir.join(format!("{} — {}.{ext}", safe.trim(), h.current() + 1));
    let res = match ext {
        "svg" => std::fs::write(&path, h.export_svg()),
        _ => match h.export_png(2.0) {
            Some(b) => std::fs::write(&path, b),
            None => Err(std::io::Error::other("render")),
        },
    };
    let app = use_context::<crate::context::AppCtx>();
    match res {
        Ok(()) => app.notifications.info(syngui::i18n::tr("notes.ink.export.done").replace("{path}", &path.display().to_string())),
        Err(e) => app.notifications.error(format!("{}: {e}", path.display())),
    }
}

/// Вставить картинку с диска в центр видимой страницы.
fn insert_image_dialog(h: InkHandle) {
    let dlg = rfd::AsyncFileDialog::new()
        .set_title(syngui::i18n::tr("notes.ink.insert_image"))
        .add_filter("Images", &["png", "jpg", "jpeg"]);
    syngui::async_runtime::spawn(async move {
        let Some(f) = dlg.pick_file().await else { return };
        let path = f.path().to_path_buf();
        syngui::async_runtime::run_on_main_thread(move || {
            let Ok(bytes) = std::fs::read(&path) else { return };
            let cur = h.current();
            let dims = h.lock().pages.get(cur).and_then(|p| p.dims());
            let (w, top) = match dims {
                Some((pw, _)) => (pw * 0.6, super::model::P2::new(pw * 0.2, 120.0)),
                None => (400.0, super::model::P2::new(40.0, 40.0)),
            };
            h.insert_image(bytes, top, w);
        });
    });
}
