//! Состояние окна: вкладки, панели (по две на вкладку — разделённый вид),
//! загрузка папок в фоне и слежение за ними, буфер обмена, тема.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use synshell_common::Config;
use syngui::prelude::*;
use syngui::data::ItemSelection;
use syngui::widgets::MenuItem;

use crate::loc::Location;
use crate::model::{self, Entry, Sort, SortKey};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Details,
    List,
    Tiles,
    Icons,
}

impl ViewMode {
    pub fn parse(s: &str) -> Self {
        match s {
            "list" | "compact" => Self::List,
            "tiles" => Self::Tiles,
            "icons" => Self::Icons,
            _ => Self::Details,
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Self::Details => "details",
            Self::List => "list",
            Self::Tiles => "tiles",
            Self::Icons => "icons",
        }
    }
}

/// Панель: одна папка со своей историей, выделением и видом.
#[derive(Clone, Copy)]
pub struct Pane {
    pub id: u64,
    pub loc: RwSignal<Location>,
    /// Показываемые записи (отобранные и отсортированные).
    pub entries: RwSignal<Arc<Vec<Entry>>>,
    pub loading: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
    pub sel: RwSignal<ItemSelection>,
    pub back: RwSignal<Vec<Location>>,
    pub fwd: RwSignal<Vec<Location>>,
    pub view: RwSignal<ViewMode>,
    pub icon_size: RwSignal<u32>,
    pub sort: RwSignal<Sort>,
    /// Растёт при смене папки (прокрутка в начало).
    pub generation: RwSignal<u64>,
    pub scroll_to: RwSignal<Option<(usize, u64)>>,
    pub renaming: RwSignal<Option<PathBuf>>,
    /// Быстрый фильтр по имени в текущей папке.
    pub filter: RwSignal<String>,
}

#[derive(Clone, Copy)]
pub struct Tab {
    pub panes: [Pane; 2],
    pub split: RwSignal<bool>,
    pub active: RwSignal<usize>,
}

impl Tab {
    pub fn pane(&self) -> Pane {
        self.panes[if self.split.get_untracked() { self.active.get_untracked().min(1) } else { 0 }]
    }
    pub fn pane_tracked(&self) -> Pane {
        self.panes[if self.split.get() { self.active.get().min(1) } else { 0 }]
    }
}

/// Своё содержимое буфера (для приглушения вырезанных).
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub paths: Vec<PathBuf>,
    pub cut: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToastKind {
    Info,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub text: String,
    pub kind: ToastKind,
    /// Кнопка «Отменить» (Ctrl+Z).
    pub undo: bool,
    pub id: u64,
}

/// Модальные окна.
#[derive(Debug, Clone, PartialEq)]
pub enum Dialog {
    ConfirmDelete { paths: Vec<PathBuf> },
    ConfirmEmptyTrash,
    OpenWith { paths: Vec<PathBuf>, mime: String },
    Properties { paths: Vec<PathBuf> },
}

#[derive(Clone, Copy)]
pub struct Ctx {
    pub theme: RwSignal<String>,
    pub cfg: RwSignal<Arc<Config>>,
    pub tabs: RwSignal<Vec<Tab>>,
    pub cur: RwSignal<usize>,
    pub show_hidden: RwSignal<bool>,
    pub sidebar: RwSignal<bool>,
    pub clip: RwSignal<Option<Clip>>,
    pub jobs_rev: RwSignal<u64>,
    pub thumbs_rev: RwSignal<u64>,
    pub places_rev: RwSignal<u64>,
    pub toast: RwSignal<Option<Toast>>,
    pub menu: RwSignal<Vec<MenuItem>>,
    pub menu_open: RwSignal<bool>,
    pub menu_pos: RwSignal<Point>,
    pub dialog: RwSignal<Option<Dialog>>,
    /// Адресная строка в режиме ввода.
    pub address_edit: RwSignal<bool>,
    /// Поле поиска: текст.
    pub search: RwSignal<String>,
    /// Растёт, когда надо поставить фокус в поиск.
    pub search_focus: RwSignal<u64>,
    /// Растёт, когда размер значков меняют не ползунком (клавиши, меню) —
    /// ползунок строки состояния пересобирается только тогда, иначе
    /// перестройка посреди перетаскивания обрывала бы его.
    pub zoom_rev: RwSignal<u64>,
    /// Состояние окна (развёрнуто, в фокусе) — для кнопок заголовка.
    pub window: RwSignal<syngui::window::WindowState>,
    /// Телефон: выдвижная панель мест и вкладок.
    pub drawer: RwSignal<bool>,
    /// Телефон: нижний лист «Вид и сортировка».
    pub sheet: RwSignal<bool>,
    /// Телефон: строка поиска вместо заголовка.
    pub phone_search: RwSignal<bool>,
}

thread_local! {
    static PHONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static CTX: RefCell<Option<Ctx>> = const { RefCell::new(None) };
    /// Сырые записи панелей и наблюдатели — вне сигналов.
    static PANES: RefCell<HashMap<u64, PaneData>> = RefCell::new(HashMap::new());
    static MENU_HANDLER: RefCell<Option<Box<dyn FnMut(&str)>>> = RefCell::new(None);
}

#[derive(Default)]
struct PaneData {
    raw: Vec<Entry>,
    watcher: Option<notify::RecommendedWatcher>,
    watched: Option<PathBuf>,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
/// Метка загрузки для каждой панели: ответ старой загрузки отбрасывается.
static LOAD_TOKENS: std::sync::Mutex<Vec<(u64, u64)>> = std::sync::Mutex::new(Vec::new());

fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

pub fn ctx() -> Ctx {
    CTX.with(|c| c.borrow().expect("state::init не вызван"))
}

pub fn try_ctx() -> Option<Ctx> {
    CTX.with(|c| *c.borrow())
}

/// Телефонная раскладка (узкое окно): касания, одна панель, нижние панели.
/// Меняется вместе с пересборкой всего окна, поэтому не сигнал.
pub fn is_phone() -> bool {
    PHONE.with(|p| p.get())
}

pub fn set_phone(v: bool) {
    PHONE.with(|p| p.set(v));
}

pub fn default_sort(c: &Config) -> Sort {
    Sort { key: SortKey::parse(&c.files.sort_by), descending: c.files.sort_descending, folders_first: c.files.folders_first }
}

/// Создать состояние (главный поток, до построения интерфейса).
pub fn init(cfg: Config, start: Vec<Location>) -> Ctx {
    let cfg = Arc::new(cfg);
    let ctx = Ctx {
        theme: use_signal(theme_mss(&cfg)),
        show_hidden: use_signal(cfg.files.show_hidden),
        cfg: use_signal(cfg.clone()),
        tabs: use_signal(Vec::new()),
        cur: use_signal(0usize),
        sidebar: use_signal(true),
        clip: use_signal(None),
        jobs_rev: use_signal(0u64),
        thumbs_rev: use_signal(0u64),
        places_rev: use_signal(0u64),
        toast: use_signal(None),
        menu: use_signal(Vec::new()),
        menu_open: use_signal(false),
        menu_pos: use_signal(Point::zero()),
        dialog: use_signal(None),
        address_edit: use_signal(false),
        search: use_signal(String::new()),
        search_focus: use_signal(0u64),
        zoom_rev: use_signal(0u64),
        window: use_signal(syngui::window::WindowState::default()),
        drawer: use_signal(false),
        sheet: use_signal(false),
        phone_search: use_signal(false),
    };
    CTX.with(|c| *c.borrow_mut() = Some(ctx));
    crate::thumbs::set_max_mb(cfg.files.thumbnail_max_mb);
    let start = if start.is_empty() { vec![Location::Dir(synshell_common::paths::home())] } else { start };
    for loc in start {
        new_tab(loc, false);
    }
    ctx.cur.set(0);
    ctx
}

fn new_pane(cfg: &Config) -> Pane {
    Pane {
        id: next_id(),
        loc: use_signal(Location::Dir(PathBuf::new())),
        entries: use_signal(Arc::new(Vec::new())),
        loading: use_signal(false),
        error: use_signal(None),
        sel: use_signal(ItemSelection::default()),
        back: use_signal(Vec::new()),
        fwd: use_signal(Vec::new()),
        view: use_signal(ViewMode::parse(&cfg.files.view)),
        icon_size: use_signal(cfg.files.icon_size.clamp(32, 256)),
        sort: use_signal(default_sort(cfg)),
        generation: use_signal(0u64),
        scroll_to: use_signal(None),
        renaming: use_signal(None),
        filter: use_signal(String::new()),
    }
}

/// Открыть вкладку (и сделать текущей, если `focus`).
pub fn new_tab(loc: Location, focus: bool) -> Tab {
    let ctx = ctx();
    let cfg = ctx.cfg.get_untracked();
    let tab = Tab { panes: [new_pane(&cfg), new_pane(&cfg)], split: use_signal(false), active: use_signal(0usize) };
    navigate(tab.panes[0], loc.clone(), false);
    // Вторая панель откроется там же при включении разделения.
    tab.panes[1].loc.set(loc);
    ctx.tabs.update(|t| t.push(tab));
    if focus {
        ctx.cur.set(ctx.tabs.get_untracked().len() - 1);
    }
    tab
}

pub fn close_tab(i: usize) {
    let ctx = ctx();
    let tabs = ctx.tabs.get_untracked();
    if tabs.len() <= 1 {
        // Последняя вкладка — закрыть окно.
        save_session();
        std::process::exit(0);
    }
    if let Some(t) = tabs.get(i) {
        for p in t.panes {
            PANES.with(|m| m.borrow_mut().remove(&p.id));
        }
    }
    ctx.tabs.update(|t| {
        t.remove(i);
    });
    let n = ctx.tabs.get_untracked().len();
    let cur = ctx.cur.get_untracked();
    if cur >= n || cur > i {
        ctx.cur.set(cur.saturating_sub(1).min(n - 1));
    } else {
        // Тот же индекс — другая вкладка: всё равно перерисовать.
        ctx.cur.set_always(cur);
    }
}

pub fn tab() -> Tab {
    let ctx = ctx();
    let tabs = ctx.tabs.get_untracked();
    tabs[ctx.cur.get_untracked().min(tabs.len() - 1)]
}

pub fn tab_tracked() -> Tab {
    let ctx = ctx();
    let tabs = ctx.tabs.get();
    tabs[ctx.cur.get().min(tabs.len() - 1)]
}

/// Активная панель текущей вкладки.
pub fn pane() -> Pane {
    tab().pane()
}

pub fn toggle_split() {
    let t = tab();
    let on = !t.split.get_untracked();
    if on {
        // Вторая панель — туда же, где первая (как в Dolphin).
        let loc = t.panes[0].loc.get_untracked();
        navigate(t.panes[1], loc, false);
        t.active.set(1);
    } else {
        // Закрываем неактивную: активная остаётся слева.
        if t.active.get_untracked() == 1 {
            let loc = t.panes[1].loc.get_untracked();
            navigate(t.panes[0], loc, true);
        }
        t.active.set(0);
    }
    t.split.set(on);
}

// ---------------------------------------------------------------- навигация

pub fn navigate(p: Pane, loc: Location, push_history: bool) {
    let cur = p.loc.get_untracked();
    if push_history && cur != loc && !matches!(&cur, Location::Dir(d) if d.as_os_str().is_empty()) {
        p.back.update(|b| b.push(cur.clone()));
        p.fwd.set(Vec::new());
    }
    let changed = cur != loc;
    p.loc.set(loc.clone());
    if changed {
        p.filter.set(String::new());
        p.renaming.set(None);
        p.sel.set(ItemSelection::default());
        p.generation.update(|g| *g += 1);
        crate::thumbs::cancel_pending();
    }
    if let Some(ctx) = try_ctx() {
        if !matches!(loc, Location::Search { .. }) {
            ctx.search.set(String::new());
            ctx.phone_search.set(false);
        }
        ctx.address_edit.set(false);
        // Телефон: выбрали место в выдвижной панели — она закрывается.
        ctx.drawer.set(false);
    }
    load(p, None);
}

pub fn go_back(p: Pane) {
    let Some(prev) = p.back.with_untracked(|b| b.last().cloned()) else { return };
    p.back.update(|b| {
        b.pop();
    });
    let cur = p.loc.get_untracked();
    p.fwd.update(|f| f.push(cur.clone()));
    // Вернулись из папки — выделить её (как в Проводнике).
    let select = cur.path().map(Path::to_path_buf);
    navigate(p, prev, false);
    if let Some(s) = select {
        select_later(p, s);
    }
}

pub fn go_forward(p: Pane) {
    let Some(next) = p.fwd.with_untracked(|f| f.last().cloned()) else { return };
    p.fwd.update(|f| {
        f.pop();
    });
    let cur = p.loc.get_untracked();
    p.back.update(|b| b.push(cur));
    navigate(p, next, false);
}

pub fn go_up(p: Pane) {
    let cur = p.loc.get_untracked();
    if let Some(parent) = cur.parent() {
        let child = cur.path().map(Path::to_path_buf);
        navigate(p, parent, true);
        if let Some(c) = child {
            select_later(p, c);
        }
    }
}

thread_local! {
    /// Что выделить, когда папка загрузится.
    static PENDING_SELECT: RefCell<HashMap<u64, Vec<PathBuf>>> = RefCell::new(HashMap::new());
}

/// Выделить путь(и) после ближайшей загрузки панели.
pub fn select_later(p: Pane, path: PathBuf) {
    PENDING_SELECT.with(|m| m.borrow_mut().entry(p.id).or_default().push(path));
    apply_pending_select(p);
}

fn apply_pending_select(p: Pane) {
    let want = PENDING_SELECT.with(|m| m.borrow().get(&p.id).cloned()).unwrap_or_default();
    if want.is_empty() {
        return;
    }
    let entries = p.entries.get_untracked();
    let idx: Vec<usize> = want.iter().filter_map(|w| entries.iter().position(|e| &e.path == w)).collect();
    if idx.is_empty() {
        return;
    }
    PENDING_SELECT.with(|m| m.borrow_mut().remove(&p.id));
    let first = idx[0];
    if is_phone() {
        // На телефоне выделение — режим выбора: только показать.
        scroll_to(p, first);
        return;
    }
    p.sel.set(ItemSelection { selected: idx.into_iter().collect(), cursor: Some(first), anchor: Some(first) });
    scroll_to(p, first);
}

pub fn scroll_to(p: Pane, i: usize) {
    p.scroll_to.set(Some((i, next_id())));
}

/// Загрузить содержимое расположения в фоне. `keep` — выделение по путям.
pub fn load(p: Pane, keep: Option<Vec<PathBuf>>) {
    let loc = p.loc.get_untracked();
    let token = next_id();
    {
        let mut t = LOAD_TOKENS.lock().unwrap();
        t.retain(|(id, _)| *id != p.id);
        t.push((p.id, token));
    }
    let show_hidden = try_ctx().map(|c| c.show_hidden.get_untracked()).unwrap_or(false);
    if keep.is_none() {
        p.loading.set(true);
    }
    watch(p, &loc);
    let pid = p.id;
    std::thread::spawn(move || {
        let current = |t: u64| LOAD_TOKENS.lock().unwrap().iter().any(|(id, tok)| *id == pid && *tok == t);
        let result: std::result::Result<Vec<Entry>, String> = match &loc {
            Location::Dir(d) => model::read_dir(d).map_err(|e| io_message(&e, d)),
            Location::Trash => Ok(crate::trash::list()),
            Location::Search { root, query } => {
                // Результаты приходят порциями — показываем по мере нахождения.
                let mut batch = Vec::new();
                let mut last = std::time::Instant::now();
                let mut all = 0usize;
                crate::search::walk(root, query, show_hidden, &mut |e| {
                    if !current(token) {
                        return false;
                    }
                    batch.push(e);
                    all += 1;
                    if last.elapsed() > std::time::Duration::from_millis(150) {
                        let part = std::mem::take(&mut batch);
                        run_on_main_thread(move || append(pid, token, part));
                        last = std::time::Instant::now();
                    }
                    all < 20_000
                });
                let part = batch;
                run_on_main_thread(move || append(pid, token, part));
                run_on_main_thread(move || finish_search(pid, token));
                return;
            }
        };
        if !current(token) {
            return;
        }
        run_on_main_thread(move || apply(pid, token, result, keep));
    });
}

fn io_message(e: &std::io::Error, p: &Path) -> String {
    match e.kind() {
        std::io::ErrorKind::PermissionDenied => format!("Нет доступа к «{}»", p.display()),
        std::io::ErrorKind::NotFound => format!("Папка «{}» не найдена", p.display()),
        _ => format!("{}: {e}", p.display()),
    }
}

fn pane_by_id(id: u64) -> Option<Pane> {
    let ctx = try_ctx()?;
    ctx.tabs.get_untracked().iter().flat_map(|t| t.panes).find(|p| p.id == id)
}

fn token_ok(pid: u64, token: u64) -> bool {
    LOAD_TOKENS.lock().unwrap().iter().any(|(id, t)| *id == pid && *t == token)
}

fn apply(pid: u64, token: u64, result: std::result::Result<Vec<Entry>, String>, keep: Option<Vec<PathBuf>>) {
    if !token_ok(pid, token) {
        return;
    }
    let Some(p) = pane_by_id(pid) else { return };
    p.loading.set(false);
    match result {
        Ok(raw) => {
            p.error.set(None);
            let keep = keep.unwrap_or_else(|| selected_paths(p));
            PANES.with(|m| m.borrow_mut().entry(pid).or_default().raw = raw);
            refilter(p);
            restore_selection(p, &keep);
            apply_pending_select(p);
        }
        Err(e) => {
            PANES.with(|m| m.borrow_mut().entry(pid).or_default().raw.clear());
            p.entries.set_always(Arc::new(Vec::new()));
            p.error.set(Some(e));
        }
    }
}

fn append(pid: u64, token: u64, part: Vec<Entry>) {
    if !token_ok(pid, token) || part.is_empty() {
        return;
    }
    let Some(p) = pane_by_id(pid) else { return };
    PANES.with(|m| m.borrow_mut().entry(pid).or_default().raw.extend(part));
    refilter(p);
}

fn finish_search(pid: u64, token: u64) {
    if !token_ok(pid, token) {
        return;
    }
    if let Some(p) = pane_by_id(pid) {
        p.loading.set(false);
        p.error.set(None);
    }
}

/// Пересобрать показываемые записи (сортировка, скрытые, фильтр).
pub fn refilter(p: Pane) {
    let keep = selected_paths(p);
    let show_hidden = try_ctx().map(|c| c.show_hidden.get_untracked()).unwrap_or(false);
    let sort = p.sort.get_untracked();
    let filter = p.filter.get_untracked();
    let v = PANES.with(|m| {
        let m = m.borrow();
        let raw = m.get(&p.id).map(|d| d.raw.as_slice()).unwrap_or(&[]);
        // В корзине и поиске скрытые не прячем — их туда положили явно.
        let hidden = show_hidden || !matches!(p.loc.get_untracked(), Location::Dir(_));
        model::visible(raw, sort, hidden, &filter)
    });
    p.entries.set_always(v);
    restore_selection(p, &keep);
}

pub fn refilter_all() {
    let ctx = ctx();
    for t in ctx.tabs.get_untracked() {
        for p in t.panes {
            refilter(p);
        }
    }
}

fn restore_selection(p: Pane, keep: &[PathBuf]) {
    let entries = p.entries.get_untracked();
    let old = p.sel.get_untracked();
    // Курсор — там же, где был (по пути), иначе на первом выделенном.
    let old_cursor = old.cursor.and_then(|_| keep.first().cloned());
    let selected: std::collections::BTreeSet<usize> =
        keep.iter().filter_map(|k| entries.iter().position(|e| &e.path == k)).collect();
    let cursor = old_cursor
        .and_then(|c| entries.iter().position(|e| e.path == c))
        .or(selected.iter().next().copied());
    p.sel.set(ItemSelection { selected, cursor, anchor: cursor });
}

pub fn selected_paths(p: Pane) -> Vec<PathBuf> {
    let sel = p.sel.get_untracked();
    let entries = p.entries.get_untracked();
    sel.selected.iter().filter_map(|&i| entries.get(i).map(|e| e.path.clone())).collect()
}

pub fn selected_entries(p: Pane) -> Vec<Entry> {
    let sel = p.sel.get_untracked();
    let entries = p.entries.get_untracked();
    sel.selected.iter().filter_map(|&i| entries.get(i).cloned()).collect()
}

/// Перечитать все панели, показывающие этот каталог (после операций).
pub fn reload_dir(dir: &Path) {
    let Some(ctx) = try_ctx() else { return };
    for t in ctx.tabs.get_untracked() {
        for p in t.panes {
            if p.loc.get_untracked().dir() == Some(dir) {
                load(p, Some(selected_paths(p)));
            }
        }
    }
}

pub fn reload_all() {
    let Some(ctx) = try_ctx() else { return };
    for t in ctx.tabs.get_untracked() {
        for p in t.panes {
            load(p, Some(selected_paths(p)));
        }
    }
}

// ---------------------------------------------------------------- слежение

fn watch(p: Pane, loc: &Location) {
    let dir = match loc {
        Location::Dir(d) => Some(d.clone()),
        Location::Trash => Some(synshell_common::paths::data_home().join("Trash/files")),
        Location::Search { .. } => None,
    };
    let already = PANES.with(|m| m.borrow().get(&p.id).map(|d| d.watched == dir).unwrap_or(false));
    if already {
        return;
    }
    let pid = p.id;
    let watcher = dir.as_ref().and_then(|d| {
        use notify::Watcher;
        let pending = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(ev) = res else { return };
            if matches!(ev.kind, notify::EventKind::Access(_)) {
                return;
            }
            // Пачку событий (копирование сотни файлов) — одной перезагрузкой.
            if pending.swap(true, Ordering::SeqCst) {
                return;
            }
            let pending = pending.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(250));
                pending.store(false, Ordering::SeqCst);
                run_on_main_thread(move || {
                    if let Some(p) = pane_by_id(pid) {
                        load(p, Some(selected_paths(p)));
                    }
                });
            });
        })
        .ok()?;
        w.watch(d, notify::RecursiveMode::NonRecursive).ok()?;
        Some(w)
    });
    PANES.with(|m| {
        let mut m = m.borrow_mut();
        let d = m.entry(pid).or_default();
        d.watcher = watcher;
        d.watched = dir;
    });
}

// ---------------------------------------------------------------- меню

/// Показать контекстное меню в точке окна; `on` получает id пункта.
pub fn show_menu(items: Vec<MenuItem>, at: Point, on: impl FnMut(&str) + 'static) {
    let ctx = ctx();
    MENU_HANDLER.with(|h| *h.borrow_mut() = Some(Box::new(on)));
    ctx.menu.set_always(items);
    ctx.menu_pos.set(at);
    ctx.menu_open.set(true);
}

pub fn menu_selected(id: &str) {
    let h = MENU_HANDLER.with(|h| h.borrow_mut().take());
    if let Some(mut f) = h {
        f(id);
    }
}

// ---------------------------------------------------------------- сообщения

pub fn toast(text: impl Into<String>) {
    show_toast(text.into(), ToastKind::Info, false);
}

pub fn toast_error(text: impl Into<String>) {
    show_toast(text.into(), ToastKind::Error, false);
}

pub fn toast_undo(text: impl Into<String>) {
    show_toast(text.into(), ToastKind::Info, true);
}

fn show_toast(text: String, kind: ToastKind, undo: bool) {
    let Some(ctx) = try_ctx() else { return };
    let id = next_id();
    ctx.toast.set(Some(Toast { text, kind, undo, id }));
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(5));
        run_on_main_thread(move || {
            if let Some(ctx) = try_ctx() {
                if ctx.toast.get_untracked().map(|t| t.id) == Some(id) {
                    ctx.toast.set(None);
                }
            }
        });
    });
}

// ---------------------------------------------------------------- тема

pub const STYLES: &str = include_str!("../styles/files.mss");

/// Таблица стилей окна: палитра `[appearance]`, производные цвета, свой MSS,
/// `files.mss` темы и пользовательский `theme.mss`.
pub fn theme_mss(c: &Config) -> String {
    let a = &c.appearance;
    let mut s = a.mss_variables();
    let p = a.palette();
    let dark = a.is_dark();
    // Слои как в Проводнике Windows 11: заголовок с вкладками темнее,
    // активная вкладка сливается с панелью инструментов, содержимое — светлее.
    let titlebar = if dark { p.bg.mix(Rgba(0, 0, 0), 0.25) } else { p.surface_alt.mix(p.bg, 0.5) };
    let chrome = if dark { p.bg.mix(p.surface, 0.55) } else { p.surface };
    let content = if dark { p.bg.mix(p.surface, 0.2) } else { p.bg.mix(p.surface, 0.35) };
    let sidebar = if dark { p.bg.mix(p.surface, 0.2) } else { p.bg.mix(p.surface, 0.35) };
    s.push_str(&format!(
        ":root {{\n  --titlebar: {};\n  --chrome: {};\n  --content: {};\n  --sidebar: {};\n  --row-hover: {};\n  --row-selected: {};\n  --row-selected-hover: {};\n  --field: {};\n  --shadow: {};\n  --divider: {};\n}}\n",
        titlebar.hex(),
        chrome.hex(),
        content.hex(),
        sidebar.hex(),
        p.fg.with_alpha(0.06).hex(),
        p.accent.with_alpha(if dark { 0.28 } else { 0.18 }).hex(),
        p.accent.with_alpha(if dark { 0.36 } else { 0.26 }).hex(),
        if dark { p.bg.mix(p.surface, 0.8).hex() } else { p.surface.hex() },
        if dark { "#00000080" } else { "#1a203024" },
        p.border.with_alpha(0.7).hex(),
    ));
    s.push_str(&a.theme_mss_variables());
    s.push_str(STYLES);
    s.push_str(a.theme_files_mss());
    if let Ok(user) = std::fs::read_to_string(synshell_common::paths::user_theme_file()) {
        s.push('\n');
        s.push_str(&user);
    }
    s
}

#[allow(non_snake_case)]
fn Rgba(r: u8, g: u8, b: u8) -> synshell_common::config::Rgba {
    synshell_common::config::Rgba::rgb(r, g, b)
}

/// Конфиг изменился снаружи — применить.
pub fn config_changed(cfg: Config) {
    let Some(ctx) = try_ctx() else { return };
    let mss = theme_mss(&cfg);
    if ctx.theme.get_untracked() != mss {
        ctx.theme.set(mss);
    }
    synshell_common::xdg::set_icon_theme(&cfg.appearance.icon_theme);
    crate::thumbs::set_max_mb(cfg.files.thumbnail_max_mb);
    let pinned_changed = ctx.cfg.get_untracked().files.pinned != cfg.files.pinned;
    ctx.cfg.set_always(Arc::new(cfg));
    if pinned_changed {
        ctx.places_rev.update(|r| *r += 1);
    }
}

// ---------------------------------------------------------------- сеанс

fn session_file() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| synshell_common::paths::home().join(".local/state"))
        .join("synshell/files-session.toml")
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Session {
    tabs: Vec<String>,
    current: usize,
}

pub fn save_session() {
    let Some(ctx) = try_ctx() else { return };
    let tabs: Vec<String> = ctx
        .tabs
        .get_untracked()
        .iter()
        .filter_map(|t| match t.panes[0].loc.get_untracked() {
            Location::Dir(d) => Some(d.display().to_string()),
            Location::Trash => Some("trash:".into()),
            Location::Search { root, .. } => Some(root.display().to_string()),
        })
        .collect();
    let s = Session { tabs, current: ctx.cur.get_untracked() };
    let f = session_file();
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(t) = toml::to_string(&s) {
        let _ = std::fs::write(f, t);
    }
}

/// Вкладки прошлого сеанса и индекс текущей.
pub fn load_session() -> Option<(Vec<Location>, usize)> {
    let t = std::fs::read_to_string(session_file()).ok()?;
    let s: Session = toml::from_str(&t).ok()?;
    let locs: Vec<Location> = s
        .tabs
        .iter()
        .filter_map(|x| Location::parse(x))
        .filter(|l| l.dir().map(Path::is_dir).unwrap_or(true))
        .collect();
    (!locs.is_empty()).then(|| {
        let cur = s.current.min(locs.len() - 1);
        (locs, cur)
    })
}

// ---------------------------------------------------------------- модификаторы

static MODS: std::sync::Mutex<Modifiers> =
    std::sync::Mutex::new(Modifiers { shift: false, ctrl: false, alt: false, meta: false });

/// Последние известные модификаторы (для переносов на боковую панель).
pub fn set_modifiers(m: Modifiers) {
    *MODS.lock().unwrap() = m;
}

pub fn ctx_modifiers() -> Modifiers {
    *MODS.lock().unwrap()
}

/// Связать фоновые потоки (задания, миниатюры) с перерисовкой.
pub fn connect_background() {
    crate::ops::set_notify(|| {
        run_on_main_thread(|| {
            if let Some(ctx) = try_ctx() {
                ctx.jobs_rev.update(|r| *r += 1);
            }
        })
    });
    let pending = Arc::new(std::sync::atomic::AtomicBool::new(false));
    crate::thumbs::set_notify(move || {
        // Готовые миниатюры — одной перерисовкой раз в 120 мс.
        if pending.swap(true, Ordering::SeqCst) {
            return;
        }
        let pending = pending.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(120));
            pending.store(false, Ordering::SeqCst);
            run_on_main_thread(|| {
                if let Some(ctx) = try_ctx() {
                    ctx.thumbs_rev.update(|r| *r += 1);
                }
            });
        });
    });
}

/// Идёт загрузка папок или миниатюр (снимок ждёт, пока всё отрисуется).
pub fn busy() -> bool {
    let Some(ctx) = try_ctx() else { return false };
    let loading = ctx.tabs.get_untracked().iter().flat_map(|t| t.panes).any(|p| p.loading.get_untracked());
    loading || crate::thumbs::pending() > 0
}
