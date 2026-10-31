//! Рукописный ввод и рисование: блокноты от руки, рисунки, пометки поверх
//! книг. Ориентир — Onyx BOOX «Заметки»: перо рисует, палец листает и
//! масштабирует, кнопка/обратный конец пера — ластик, лассо, распознавание
//! фигур удержанием пера. Всё рассчитано на E Ink: без анимаций, готовые
//! штрихи — в растре страницы, на каждое событие пера перерисовывается
//! только живой штрих.
//!
//! Части:
//! - [`model`] — документ (страницы, слои, штрихи, фигуры, картинки, текст),
//!   правки [`model::Op`] и история;
//! - [`codec`] — файл `.ink` (`notes/objects/<id>.ink`);
//! - [`brush`], [`raster`], [`mesh`] — кисти, растр (tiny-skia), сетка ГПУ;
//! - [`tools`], [`recognize`] — ластик, лассо, заливка, пипетка, фигуры;
//! - [`InkHandle`] — документ + сигналы для интерфейса;
//! - [`canvas::InkCanvas`] — холст (страница, бесконечный холст, оверлей);
//! - [`toolbar::InkToolbar`] — панель инструментов;
//! - [`export`] — PNG и SVG.
//!
//! ```ignore
//! let h = InkHandle::new(InkDoc::notebook());
//! h.on_change(|h| save(h.save_bytes()));
//! Column::new().child(InkToolbar::new(h.clone()).build()).child(InkCanvas::new(h))
//! ```

pub mod brush;
pub mod canvas;
pub mod codec;
pub mod export;
pub mod icons;
pub mod input;
pub mod mesh;
pub mod model;
pub mod raster;
pub mod recognize;
pub mod toolbar;
pub mod tools;
pub mod views;

#[cfg(test)]
mod tests;

use std::sync::{Arc, Mutex, MutexGuard};

use syngui::prelude::*;

pub use canvas::InkCanvas;
pub use model::{BBox, Brush, DocKind, InkDoc, InkPage, Item, Op, PageSize, Rgba, ShapeKind, Template, P2};
pub use toolbar::InkToolbar;
pub use views::{actions, details, flush_all, page_view, thumbnail, views_for_library};

/// Стили панели и вида (подключить в теме приложения после `library.mss`).
pub const INK_MSS: &str = include_str!("../../../styles/ink.mss");

/// Строки интерфейса (свои каталоги, регистрируются один раз).
pub fn register_i18n() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        syngui::i18n::register_catalogs(&[include_str!("i18n/en.lang"), include_str!("i18n/ru.lang")]);
    });
}

// ─────────────────────────── Инструменты ───────────────────────────

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    #[default]
    Pen,
    Eraser,
    Lasso,
    Shape,
    Text,
    /// Заливка области (рисовалка).
    Fill,
    Eyedropper,
    /// Только листать/двигать (перо не рисует).
    Hand,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EraserMode {
    /// Стирает штрих целиком.
    #[default]
    Stroke,
    /// Стирает под ластиком, режет штрих.
    Point,
    /// Обводка лассо — стереть всё внутри.
    Area,
}

/// Состояние панели инструментов.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolState {
    pub tool: Tool,
    pub brush: Brush,
    pub color: Rgba,
    /// Ступень толщины 0..5 для каждой кисти (индекс — `Brush::ALL`).
    pub width_steps: [usize; 6],
    pub eraser: EraserMode,
    /// Радиус ластика, единицы страницы.
    pub eraser_r: f32,
    pub shape: ShapeKind,
    /// Заливка новых фигур (рисовалка).
    pub shape_fill: Option<Rgba>,
    /// Рисовать пальцем (по умолчанию нет — палец листает и масштабирует).
    pub finger_draw: bool,
    /// Удержание пера в конце штриха делает фигуру.
    pub hold_to_shape: bool,
    /// Без сглаживания краёв (чётче на ч/б E Ink).
    pub crisp: bool,
}

impl Default for ToolState {
    fn default() -> Self {
        Self {
            tool: Tool::Pen,
            brush: Brush::Fountain,
            color: Rgba::BLACK,
            width_steps: [1; 6],
            eraser: EraserMode::Stroke,
            eraser_r: 10.0,
            shape: ShapeKind::Line,
            shape_fill: None,
            finger_draw: false,
            hold_to_shape: true,
            crisp: false,
        }
    }
}

impl ToolState {
    fn brush_index(&self) -> usize {
        Brush::ALL.iter().position(|b| *b == self.brush).unwrap_or(0)
    }
    pub fn width_step(&self) -> usize {
        self.width_steps[self.brush_index()].min(4)
    }
    pub fn set_width_step(&mut self, k: usize) {
        let i = self.brush_index();
        self.width_steps[i] = k.min(4);
    }
    /// Ширина текущей кисти.
    pub fn width(&self) -> f32 {
        self.brush.widths()[self.width_step()]
    }
    /// Ширина линии фигур (как у шариковой той же ступени).
    pub fn shape_width(&self) -> f32 {
        Brush::Ballpoint.widths()[self.width_step()]
    }
}

/// Палитра: ч/б оттенки для E Ink и цвета для Kaleido/экрана.
pub const PALETTE: [Rgba; 16] = [
    Rgba([0, 0, 0, 255]),
    Rgba([85, 85, 85, 255]),
    Rgba([150, 150, 150, 255]),
    Rgba([255, 255, 255, 255]),
    Rgba([200, 30, 40, 255]),
    Rgba([235, 110, 20, 255]),
    Rgba([240, 200, 0, 255]),
    Rgba([40, 150, 60, 255]),
    Rgba([20, 140, 160, 255]),
    Rgba([30, 80, 200, 255]),
    Rgba([110, 50, 170, 255]),
    Rgba([210, 60, 140, 255]),
    Rgba([130, 80, 40, 255]),
    Rgba([255, 235, 120, 255]),
    Rgba([150, 220, 160, 255]),
    Rgba([150, 190, 240, 255]),
];

// ─────────────────────────── Выделение ───────────────────────────

/// Выбранное лассо: страница, слой, id элементов.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    pub page: u64,
    pub layer: u64,
    pub ids: Vec<u64>,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

/// Буфер обмена элементов (общий на процесс: копировать между страницами).
fn clipboard() -> &'static Mutex<Vec<Item>> {
    static CB: std::sync::OnceLock<Mutex<Vec<Item>>> = std::sync::OnceLock::new();
    CB.get_or_init(|| Mutex::new(Vec::new()))
}

/// Изменения для перерисовки растра: (ревизия, страница, область; `None` —
/// вся страница).
#[derive(Default)]
struct DirtyLog {
    entries: Vec<(u64, u64, Option<BBox>)>,
}

/// Что менялось после ревизии `since` на странице `page`: `None` — всё.
pub(crate) enum Dirty {
    Nothing,
    Region(BBox),
    All,
}

type ChangeCb = Arc<dyn Fn(&InkHandle) + Send + Sync>;

// ─────────────────────────── Ручка ───────────────────────────

/// Документ и его состояние для интерфейса. Клонируется дёшево; все клоны —
/// один документ.
#[derive(Clone)]
pub struct InkHandle {
    doc: Arc<Mutex<InkDoc>>,
    history: Arc<Mutex<model::History>>,
    dirty: Arc<Mutex<DirtyLog>>,
    /// Число страниц без замка документа (`current()` зовут и под замком).
    pages: Arc<std::sync::atomic::AtomicUsize>,
    listeners: Arc<Mutex<Vec<ChangeCb>>>,
    /// Каждая правка (автосохранение, перерисовка).
    pub revision: RwSignal<u64>,
    /// Текущая страница (индекс).
    pub page: RwSignal<usize>,
    pub tool: RwSignal<ToolState>,
    pub selection: RwSignal<Selection>,
    /// Бамп — вписать страницу в окно.
    pub fit_request: RwSignal<u64>,
    /// Текст в правке на холсте: (страница, id).
    pub editing_text: RwSignal<Option<(u64, u64)>>,
}

impl InkHandle {
    pub fn new(doc: InkDoc) -> Self {
        register_i18n();
        let mut tool = ToolState::default();
        if doc.kind == DocKind::Drawing {
            tool.brush = Brush::Brush;
            tool.width_steps = [1, 1, 1, 1, 2, 1];
        }
        let n = doc.pages.len();
        Self {
            pages: Arc::new(std::sync::atomic::AtomicUsize::new(n)),
            doc: Arc::new(Mutex::new(doc)),
            history: Arc::new(Mutex::new(model::History::new())),
            dirty: Arc::new(Mutex::new(DirtyLog::default())),
            listeners: Arc::new(Mutex::new(Vec::new())),
            revision: use_signal(0),
            page: use_signal(0),
            tool: use_signal(tool),
            selection: use_signal(Selection::default()),
            fit_request: use_signal(0),
            editing_text: use_signal(None),
        }
    }

    /// Открыть из байтов файла (`.ink`); испорченный — ошибка.
    pub fn load(bytes: &[u8]) -> std::result::Result<Self, codec::CodecError> {
        Ok(Self::new(codec::load(bytes)?))
    }

    /// Байты файла для сохранения.
    pub fn save_bytes(&self) -> Vec<u8> {
        let mut d = self.lock().clone();
        d.gc_assets();
        codec::save(&d)
    }

    /// Сам документ (фоновое сохранение).
    pub(crate) fn doc_arc(&self) -> Arc<Mutex<InkDoc>> {
        self.doc.clone()
    }

    /// Страниц не меньше `n` (оверлей книги: страниц стало больше).
    pub fn ensure_pages(&self, n: usize) {
        let missing = n.saturating_sub(self.page_count());
        if missing == 0 {
            return;
        }
        let mut d = self.lock();
        for _ in 0..missing {
            let p = d.blank_page();
            d.pages.push(p);
        }
        drop(d);
        self.sync_count();
        self.revision.set(self.revision.get_untracked() + 1);
    }

    pub fn lock(&self) -> MutexGuard<'_, InkDoc> {
        self.doc.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn hist(&self) -> MutexGuard<'_, model::History> {
        self.history.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Подписаться на правки (автосохранение). Вызывается после каждой.
    pub fn on_change(&self, f: impl Fn(&InkHandle) + Send + Sync + 'static) {
        self.listeners.lock().unwrap_or_else(|e| e.into_inner()).push(Arc::new(f));
    }

    pub fn kind(&self) -> DocKind {
        self.lock().kind
    }

    pub fn page_count(&self) -> usize {
        self.pages.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn sync_count(&self) {
        let n = self.lock().pages.len();
        self.pages.store(n, std::sync::atomic::Ordering::Relaxed);
    }

    /// Индекс текущей страницы (в пределах документа).
    pub fn current(&self) -> usize {
        let n = self.page_count();
        self.page.get_untracked().min(n.saturating_sub(1))
    }

    pub fn current_page_id(&self) -> u64 {
        let i = self.current();
        self.lock().pages.get(i).map(|p| p.id).unwrap_or(0)
    }

    // ── Правки ─────────────────────────────────────────────

    fn mark(&self, op: &Op, region: Option<BBox>) {
        if !matches!(op, Op::Items { .. }) {
            self.sync_count();
        }
        let rev = self.revision.get_untracked() + 1;
        {
            let mut log = self.dirty.lock().unwrap_or_else(|e| e.into_inner());
            match op {
                Op::Items { page, .. } if region.is_some() => log.entries.push((rev, *page, region)),
                _ => {
                    let mut pages = Vec::new();
                    op.pages(&mut pages);
                    for p in pages {
                        log.entries.push((rev, p, None));
                    }
                }
            }
            if log.entries.len() > 4096 {
                let cut = log.entries.len() - 2048;
                log.entries.drain(..cut);
            }
        }
        self.revision.set(rev);
        let cbs: Vec<ChangeCb> = self.listeners.lock().unwrap_or_else(|e| e.into_inner()).clone();
        for cb in cbs {
            cb(self);
        }
    }

    /// Область, которую задевает правка элементов.
    fn op_region(op: &Op) -> Option<BBox> {
        match op {
            Op::Items { removed, added, .. } => {
                Some(removed.iter().chain(added.iter()).fold(BBox::EMPTY, |b, (_, i)| b.union(i.bbox())))
            }
            Op::Batch(v) => {
                let mut b = BBox::EMPTY;
                let mut page = None;
                for o in v {
                    match o {
                        Op::Items { page: p, .. } if page.is_none() || page == Some(*p) => {
                            page = Some(*p);
                            b = b.union(Self::op_region(o)?);
                        }
                        _ => return None,
                    }
                }
                Some(b)
            }
            _ => None,
        }
    }

    /// Применить правку с историей.
    pub fn commit(&self, op: Op) {
        if op.is_empty() {
            return;
        }
        {
            let mut d = self.lock();
            self.hist().commit(&mut d, op.clone());
        }
        self.mark_op(&op);
    }

    fn mark_op(&self, op: &Op) {
        match (op, Self::op_region(op)) {
            (Op::Batch(v), Some(r)) => {
                if let Some(Op::Items { page, layer, .. }) = v.first() {
                    let fake = Op::Items { page: *page, layer: *layer, removed: Vec::new(), added: Vec::new() };
                    self.mark(&fake, Some(r));
                }
            }
            (_, r) => self.mark(op, r),
        }
    }

    /// Запомнить уже применённую правку (ластик копит куски по ходу штриха).
    pub(crate) fn record(&self, op: Op) {
        self.hist().record(op);
    }

    /// Правка без истории, но с перерисовкой и автосохранением (применяется
    /// сразу — ластик по ходу; итог потом [`Self::record`]).
    pub(crate) fn apply_live(&self, op: &Op) {
        op.apply(&mut self.lock());
        self.mark_op(op);
    }

    pub fn can_undo(&self) -> bool {
        self.hist().can_undo()
    }
    pub fn can_redo(&self) -> bool {
        self.hist().can_redo()
    }

    pub fn undo(&self) {
        self.clear_selection();
        let op = {
            let mut d = self.lock();
            self.hist().undo(&mut d)
        };
        if let Some(op) = op {
            self.follow(&op);
            self.mark_op(&op);
        }
    }

    pub fn redo(&self) {
        self.clear_selection();
        let op = {
            let mut d = self.lock();
            self.hist().redo(&mut d)
        };
        if let Some(op) = op {
            self.follow(&op);
            self.mark_op(&op);
        }
    }

    /// После отмены/повтора — показать страницу, где была правка.
    fn follow(&self, op: &Op) {
        let mut pages = Vec::new();
        op.pages(&mut pages);
        let n = self.page_count();
        if let Some(pid) = pages.first() {
            let found = self.lock().page_index(*pid);
            if let Some(i) = found {
                if i != self.current() {
                    self.page.set(i);
                }
                return;
            }
        }
        if self.page.get_untracked() >= n {
            self.page.set(n.saturating_sub(1));
        }
    }

    /// Что менялось на странице после ревизии `since`.
    pub(crate) fn dirty_since(&self, since: u64, page: u64) -> Dirty {
        let log = self.dirty.lock().unwrap_or_else(|e| e.into_inner());
        if log.entries.first().is_some_and(|(r, ..)| *r > since + 1) && since + 1 < self.revision.get_untracked() {
            return Dirty::All;
        }
        let mut b = BBox::EMPTY;
        let mut any = false;
        for (r, p, reg) in log.entries.iter().rev() {
            if *r <= since {
                break;
            }
            if *p != page {
                continue;
            }
            any = true;
            match reg {
                Some(x) => b = b.union(*x),
                None => return Dirty::All,
            }
        }
        if !any {
            Dirty::Nothing
        } else if b.is_empty() {
            Dirty::Nothing
        } else {
            Dirty::Region(b)
        }
    }

    // ── Страницы ───────────────────────────────────────────

    pub fn go_to(&self, i: usize) {
        let n = self.page_count();
        let i = i.min(n.saturating_sub(1));
        if i != self.page.get_untracked() {
            self.clear_selection();
            self.page.set(i);
        }
    }

    pub fn prev_page(&self) {
        let c = self.current();
        if c > 0 {
            self.go_to(c - 1);
        }
    }

    /// Следующая страница; на последней — новая, если `create`.
    pub fn next_page(&self, create: bool) {
        let c = self.current();
        if c + 1 < self.page_count() {
            self.go_to(c + 1);
        } else if create && self.kind() != DocKind::Overlay {
            self.insert_page(c + 1);
        }
    }

    /// Новая страница по умолчаниям документа (шаблон — как у текущей).
    pub fn insert_page(&self, at: usize) {
        let (before, after) = {
            let mut d = self.lock();
            let before = d.pages.clone();
            let mut p = d.blank_page();
            if let Some(cur) = d.pages.get(self.page.get_untracked().min(d.pages.len().saturating_sub(1))) {
                p.template = cur.template.clone();
                p.size = cur.size;
                p.paper = cur.paper;
            }
            let mut after = before.clone();
            after.insert(at.min(after.len()), p);
            (before, after)
        };
        self.commit(Op::Pages { before, after });
        self.go_to(at);
    }

    /// Удалить страницу (последнюю не удаляет — очищает).
    pub fn delete_page(&self, i: usize) {
        let op = {
            let mut d = self.lock();
            if i >= d.pages.len() {
                return;
            }
            let before = d.pages.clone();
            let mut after = before.clone();
            if after.len() == 1 {
                let mut p = d.blank_page();
                p.template = after[0].template.clone();
                p.size = after[0].size;
                after[0] = p;
            } else {
                after.remove(i);
            }
            Op::Pages { before, after }
        };
        self.commit(op);
        let n = self.page_count();
        if self.page.get_untracked() >= n {
            self.page.set(n - 1);
        }
    }

    /// Переставить страницу.
    pub fn move_page(&self, from: usize, to: usize) {
        let op = {
            let d = self.lock();
            if from >= d.pages.len() || to >= d.pages.len() || from == to {
                return;
            }
            let before = d.pages.clone();
            let mut after = before.clone();
            let p = after.remove(from);
            after.insert(to, p);
            Op::Pages { before, after }
        };
        self.commit(op);
        self.go_to(to);
    }

    /// Правка текущей страницы целиком (шаблон, бумага, слои).
    pub fn edit_page(&self, f: impl FnOnce(&mut InkPage)) {
        let ci = self.current();
        let op = {
            let d = self.lock();
            let Some(p) = d.pages.get(ci) else { return };
            let before = p.clone();
            let mut after = before.clone();
            f(&mut after);
            Op::Page { before: Box::new(before), after: Box::new(after) }
        };
        self.commit(op);
    }

    /// Шаблон текущей страницы; `all` — всех страниц и новых.
    pub fn set_template(&self, t: Template, all: bool) {
        if all {
            let op = {
                let mut d = self.lock();
                d.template = t.clone();
                let before = d.pages.clone();
                let after: Vec<InkPage> = before.iter().cloned().map(|mut p| {
                    p.template = t.clone();
                    p
                }).collect();
                Op::Pages { before, after }
            };
            self.commit(op);
        } else {
            self.edit_page(|p| p.template = t);
        }
    }

    /// Фон-картинка страницы.
    pub fn set_background_image(&self, bytes: Vec<u8>) {
        let key = self.lock().add_asset(bytes);
        self.edit_page(|p| p.template = Template::Image { asset: key });
    }

    // ── Слои ───────────────────────────────────────────────

    pub fn add_layer(&self) {
        let id = self.lock().alloc_id();
        self.edit_page(|p| {
            let n = p.layers.len() + 1;
            p.layers.push(model::Layer::new(id, model::default_layer_name(n)));
            p.active_layer = p.layers.len() - 1;
        });
    }

    pub fn delete_layer(&self, i: usize) {
        self.edit_page(|p| {
            if p.layers.len() > 1 && i < p.layers.len() {
                p.layers.remove(i);
                p.active_layer = p.active_layer.min(p.layers.len() - 1);
            } else if let Some(l) = p.layers.get_mut(i) {
                l.items.clear();
            }
        });
    }

    pub fn set_active_layer(&self, i: usize) {
        self.edit_page(|p| p.active_layer = i.min(p.layers.len().saturating_sub(1)));
    }

    pub fn move_layer(&self, from: usize, to: usize) {
        self.edit_page(|p| {
            if from < p.layers.len() && to < p.layers.len() {
                let active_id = p.layers[p.active()].id;
                let l = p.layers.remove(from);
                p.layers.insert(to, l);
                p.active_layer = p.layer_index(active_id).unwrap_or(0);
            }
        });
    }

    pub fn edit_layer(&self, i: usize, f: impl FnOnce(&mut model::Layer)) {
        self.edit_page(|p| {
            if let Some(l) = p.layers.get_mut(i) {
                f(l);
            }
        });
    }

    // ── Добавление элементов ───────────────────────────────

    /// Добавить элемент в активный слой текущей страницы (сверху).
    pub fn add_item(&self, mut item: Item) -> Option<u64> {
        let op = {
            let mut d = self.lock();
            let id = d.alloc_id();
            item.set_id(id);
            let pi = self.page.get_untracked().min(d.pages.len().saturating_sub(1));
            let p = d.pages.get(pi)?;
            let li = p.active();
            let l = p.layers.get(li)?;
            if l.locked {
                return None;
            }
            Op::Items { page: p.id, layer: l.id, removed: Vec::new(), added: vec![(l.items.len(), item)] }
        };
        let id = match &op {
            Op::Items { added, .. } => added[0].1.id(),
            _ => 0,
        };
        self.commit(op);
        Some(id)
    }

    /// Вставить картинку (байты PNG/JPEG) в точку `at` шириной не больше
    /// `max_w`.
    pub fn insert_image(&self, bytes: Vec<u8>, at: P2, max_w: f32) -> Option<u64> {
        let (w, h) = image::load_from_memory(&bytes).ok().map(|i| (i.width() as f32, i.height() as f32))?;
        let k = (max_w / w).min(1.0);
        let key = self.lock().add_asset(bytes);
        self.add_item(Item::Image(model::ImageItem { id: 0, asset: key, x: at.x, y: at.y, w: w * k, h: h * k, angle: 0.0 }))
    }

    // ── Выделение ──────────────────────────────────────────

    pub fn clear_selection(&self) {
        if !self.selection.get_untracked().is_empty() {
            self.selection.set(Selection::default());
        }
    }

    pub fn select(&self, s: Selection) {
        self.selection.set(s);
    }

    /// Выбранные элементы с индексами.
    pub fn selected_items(&self) -> Vec<(usize, Item)> {
        let s = self.selection.get_untracked();
        let d = self.lock();
        let Some(pi) = d.page_index(s.page) else { return Vec::new() };
        let Some(li) = d.pages[pi].layer_index(s.layer) else { return Vec::new() };
        d.pages[pi].layers[li]
            .items
            .iter()
            .enumerate()
            .filter(|(_, i)| s.ids.contains(&i.id()))
            .map(|(k, i)| (k, i.clone()))
            .collect()
    }

    pub fn selection_bbox(&self) -> BBox {
        self.selected_items().iter().fold(BBox::EMPTY, |b, (_, i)| b.union(i.bbox()))
    }

    /// Заменить выбранные элементы их изменёнными копиями (те же id, места).
    pub fn edit_selection(&self, f: impl Fn(&mut Item)) {
        let s = self.selection.get_untracked();
        let removed = self.selected_items();
        if removed.is_empty() {
            return;
        }
        let added: Vec<(usize, Item)> = removed
            .iter()
            .map(|(k, i)| {
                let mut j = i.clone();
                f(&mut j);
                (*k, j)
            })
            .collect();
        self.commit(Op::Items { page: s.page, layer: s.layer, removed, added });
    }

    pub fn transform_selection(&self, m: &model::Affine) {
        self.edit_selection(|i| i.transform(m));
    }

    pub fn recolor_selection(&self, c: Rgba) {
        self.edit_selection(|i| i.set_color(c));
    }

    pub fn rewidth_selection(&self, w: f32) {
        self.edit_selection(|i| i.set_width(w));
    }

    pub fn delete_selection(&self) {
        let s = self.selection.get_untracked();
        let removed = self.selected_items();
        if removed.is_empty() {
            return;
        }
        self.commit(Op::Items { page: s.page, layer: s.layer, removed, added: Vec::new() });
        self.clear_selection();
    }

    pub fn copy_selection(&self) {
        let items: Vec<Item> = self.selected_items().into_iter().map(|(_, i)| i).collect();
        if !items.is_empty() {
            *clipboard().lock().unwrap_or_else(|e| e.into_inner()) = items;
        }
    }

    pub fn cut_selection(&self) {
        self.copy_selection();
        self.delete_selection();
    }

    pub fn has_clipboard(&self) -> bool {
        !clipboard().lock().unwrap_or_else(|e| e.into_inner()).is_empty()
    }

    /// Вставить из буфера в активный слой со сдвигом; вставленное выбрано.
    pub fn paste(&self, offset: P2) {
        let items = clipboard().lock().unwrap_or_else(|e| e.into_inner()).clone();
        if items.is_empty() {
            return;
        }
        let (op, sel) = {
            let mut d = self.lock();
            let pi = self.page.get_untracked().min(d.pages.len().saturating_sub(1));
            let ids: Vec<u64> = items.iter().map(|_| d.alloc_id()).collect();
            let p = &d.pages[pi];
            let l = &p.layers[p.active()];
            if l.locked {
                return;
            }
            let base = l.items.len();
            let m = model::Affine::translate(offset.x, offset.y);
            let added: Vec<(usize, Item)> = items
                .into_iter()
                .zip(&ids)
                .enumerate()
                .map(|(k, (mut it, id))| {
                    it.set_id(*id);
                    it.transform(&m);
                    (base + k, it)
                })
                .collect();
            (Op::Items { page: p.id, layer: l.id, removed: Vec::new(), added }, Selection { page: p.id, layer: l.id, ids })
        };
        self.commit(op);
        self.select(sel);
    }

    // ── Прочее ─────────────────────────────────────────────

    pub fn set_tool(&self, f: impl FnOnce(&mut ToolState)) {
        let mut t = self.tool.get_untracked();
        f(&mut t);
        if t.tool != Tool::Lasso && t != self.tool.get_untracked() {
            self.clear_selection();
        }
        self.tool.set(t);
    }

    pub fn fit(&self) {
        self.fit_request.set(self.fit_request.get_untracked() + 1);
    }

    /// Очистить текущую страницу (все слои).
    pub fn clear_page(&self) {
        self.edit_page(|p| {
            for l in &mut p.layers {
                l.items.clear();
            }
        });
    }

    /// PNG текущей страницы.
    pub fn export_png(&self, scale: f32) -> Option<Vec<u8>> {
        let ci = self.current();
        let d = self.lock();
        raster::export_png(&d, ci, scale, &raster::RasterOpts::default())
    }

    /// SVG текущей страницы.
    pub fn export_svg(&self) -> String {
        let ci = self.current();
        let d = self.lock();
        export::svg(&d, ci)
    }
}
