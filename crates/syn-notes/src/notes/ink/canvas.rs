//! Холст: страница блокнота, бесконечный холст рисунка или прозрачный
//! слой пометок поверх чужого содержимого (книги).
//!
//! Готовое лежит в растре видимой области (tiny-skia → текстура), который
//! дорисовывается только в изменённых местах; живой штрих, предпросмотр
//! фигуры и перенос лассо — сеткой ГПУ поверх. На E Ink так меняется ровно
//! область под пером.
//!
//! Ввод (как у Onyx): перо рисует выбранным инструментом, обратный конец
//! или кнопка пера — ластик; пальцы — листать (свайп), двигать и
//! масштабировать (два пальца), тап двумя — отменить, тремя — повторить;
//! пока перо у экрана, касания ладони не считаются. Рисовать пальцем —
//! настройка. Удержание пера в конце штриха — ровная фигура.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use syngui::containers::Positioned;
use syngui::core::{Color, Point, Rect, Size, Transform};
use syngui::gpu::image_store::{ImageHandle, ImageStore};
use syngui::input::{Event, EventResult, Key, MouseButton};
use syngui::layout::Constraints;
use syngui::mss::{ComputedStyle, MssFields, TextAlign, TextDecoration};
use syngui::prelude::*;
use syngui::render::{DisplayList, TextureId};
use syngui::widget::context::{EventContext, EventContextExt, UpdateContext};
use syngui::widget::{DirtyFlags, Element, ElementId, ElementTree, LayoutHint};

use super::input::{pen_event, PenPhase, PenSample, PenTool};
use super::mesh::{self, Mesh};
use super::model::{Affine, BBox, DocKind, InkPoint, Item, Op, Rgba, Shape, ShapeKind, Stroke, TextItem, P2};
use super::raster::{self, RasterOpts};
use super::{recognize, tools, EraserMode, InkHandle, Selection, Tool};

pub const ZOOM_MIN: f32 = 0.2;
pub const ZOOM_MAX: f32 = 8.0;
/// Сколько держать перо на месте, чтобы штрих стал фигурой.
const HOLD: Duration = Duration::from_millis(600);
/// Перо считается «у экрана» ещё столько после последнего события.
const PEN_NEAR: Duration = Duration::from_millis(700);
/// Размер ручек выделения, px экрана.
const HANDLE_PX: f32 = 16.0;

/// Холст рукописного документа.
pub struct InkCanvas {
    handle: InkHandle,
    overlay: bool,
    page: Option<usize>,
    readonly: bool,
    classes: Vec<String>,
}

impl InkCanvas {
    pub fn new(handle: InkHandle) -> Self {
        Self { handle, overlay: false, page: None, readonly: false, classes: vec!["ink-canvas".into()] }
    }

    /// Прозрачный слой пометок: страница растягивается на весь размер
    /// виджета (по ширине), бумаги и шаблона нет, пальцы проходят насквозь
    /// (листает и прокручивает содержимое под слоем), рисует только перо.
    pub fn overlay(mut self) -> Self {
        self.overlay = true;
        self
    }

    /// Показывать страницу `i` (иначе — `handle.page`).
    pub fn page(mut self, i: usize) -> Self {
        self.page = Some(i);
        self
    }

    /// Только просмотр.
    pub fn readonly(mut self, r: bool) -> Self {
        self.readonly = r;
        self
    }

    pub fn class(mut self, c: &str) -> Self {
        self.classes.push(c.to_string());
        self
    }
}

impl Widget for InkCanvas {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(InkElement {
            id: ElementId::new(),
            bounds: Rect::zero(),
            dirty: DirtyFlags::LAYOUT | DirtyFlags::RENDER,
            classes: self.classes.clone(),
            mss: MssFields::new(),
            handle: self.handle.clone(),
            overlay: self.overlay,
            page_override: self.page,
            readonly: self.readonly,
            view: View::default(),
            fitted: None,
            fit_seen: self.handle.fit_request.get_untracked(),
            raster: RefCell::new(None),
            store: None,
            images: RefCell::new(raster::ImageCache::default()),
            local_dirty: RefCell::new(BBox::EMPTY),
            live: None,
            mouse_btn: None,
            pen_seen: None,
            fingers: HashMap::new(),
            gesture: None,
            finger_pen: None,
            editing: self.handle.editing_text.get_untracked(),
            rebuild: false,
            focused: false,
            focus_request: false,
            subscribed: false,
            dpr: Arc::new(Mutex::new(1.0)),
            stroke_t0: 0,
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
    fn widget_classes(&self) -> &[String] {
        &self.classes
    }
}

/// Вид: точка страницы `p` на экране — `p·zoom + (ox, oy)` от угла холста.
#[derive(Clone, Copy, Debug, PartialEq)]
struct View {
    zoom: f32,
    ox: f32,
    oy: f32,
}

impl Default for View {
    fn default() -> Self {
        Self { zoom: 1.0, ox: 0.0, oy: 0.0 }
    }
}

impl View {
    fn to_page(&self, local: Point) -> P2 {
        P2::new((local.x - self.ox) / self.zoom, (local.y - self.oy) / self.zoom)
    }
    fn to_local(&self, p: P2) -> Point {
        Point::new(p.x * self.zoom + self.ox, p.y * self.zoom + self.oy)
    }
}

/// Растр видимой области страницы.
struct RasterCache {
    page: u64,
    /// Область страницы, что в растре.
    region: BBox,
    /// px на единицу страницы.
    scale: f32,
    crisp: bool,
    pm: tiny_skia::Pixmap,
    rgba: Vec<u8>,
    tex: Option<ImageHandle>,
    rev: u64,
    /// Номер для ключа текстуры.
    key: u64,
}

/// Перенос выделения.
#[derive(Clone, Copy, Debug, PartialEq)]
enum DragKind {
    Move,
    /// Масштаб от противоположного угла (индекс угла 0..4).
    Scale(usize),
    Rotate,
}

enum Live {
    Stroke {
        points: Vec<InkPoint>,
        brush: super::Brush,
        color: Rgba,
        width: f32,
        mesh: Mesh,
        /// Где и когда перо встало (удержание → фигура).
        anchor: (P2, Instant),
        shape: Option<Shape>,
    },
    Erase {
        mode: EraserMode,
        last: P2,
        ops: Vec<Op>,
        path: Vec<P2>,
    },
    Lasso {
        path: Vec<P2>,
    },
    Shape {
        kind: ShapeKind,
        a: P2,
        b: P2,
    },
    SelDrag {
        kind: DragKind,
        start: P2,
        m: Affine,
        bbox: BBox,
        items: Vec<(usize, Item)>,
        mesh: Mesh,
    },
    Pan {
        start: Point,
        view: View,
    },
}

/// Жест пальцами.
struct Gesture {
    start_view: View,
    /// Центр и расстояние двух пальцев в начале.
    c0: Point,
    d0: f32,
    /// Один палец: начало (свайп/перенос).
    one: Option<Point>,
    max_fingers: usize,
    moved: bool,
    started: Instant,
}

pub struct InkElement {
    id: ElementId,
    bounds: Rect,
    dirty: DirtyFlags,
    classes: Vec<String>,
    mss: MssFields,
    handle: InkHandle,
    overlay: bool,
    page_override: Option<usize>,
    readonly: bool,
    view: View,
    /// Для какой страницы и размера вид вписан (смена — вписать заново).
    fitted: Option<(u64, (i32, i32))>,
    fit_seen: u64,
    raster: RefCell<Option<RasterCache>>,
    store: Option<Arc<Mutex<ImageStore>>>,
    images: RefCell<raster::ImageCache>,
    /// Область, которую надо перерисовать в растре помимо правок
    /// документа (начало/конец переноса выделения).
    local_dirty: RefCell<BBox>,
    live: Option<Live>,
    mouse_btn: Option<MouseButton>,
    pen_seen: Option<Instant>,
    fingers: HashMap<u64, Point>,
    gesture: Option<Gesture>,
    /// Палец, который рисует (настройка «рисовать пальцем»).
    finger_pen: Option<u64>,
    editing: Option<(u64, u64)>,
    rebuild: bool,
    focused: bool,
    focus_request: bool,
    subscribed: bool,
    /// Масштаб экрана (из списка отрисовки) — для толщины бахромы.
    dpr: Arc<Mutex<f32>>,
    /// Время пера в начале штриха (точки хранят время от него).
    stroke_t0: u32,
}

fn next_key() -> u64 {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl InkElement {
    /// Индекс страницы без замка документа (его часто держат вызывающие);
    /// вне диапазона — `pages.get` даст `None`, ручка сама держит `page`
    /// в пределах.
    fn page_index(&self) -> usize {
        self.page_override.unwrap_or_else(|| self.handle.page.get_untracked())
    }

    fn page_info(&self) -> (u64, Option<(f32, f32)>, bool) {
        let d = self.handle.lock();
        let i = self.page_index();
        match d.pages.get(i) {
            Some(p) => (p.id, p.dims(), p.size.is_infinite()),
            None => (0, None, true),
        }
    }

    fn local(&self, p: Point) -> Point {
        Point::new(p.x - self.bounds.origin.x, p.y - self.bounds.origin.y)
    }

    fn to_page(&self, p: Point) -> P2 {
        self.view.to_page(self.local(p))
    }

    fn dpr(&self) -> f32 {
        *self.dpr.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Пиксель экрана в единицах страницы (бахрома, ручки).
    fn px(&self) -> f32 {
        1.0 / (self.view.zoom * self.dpr()).max(0.01)
    }

    // ── Вид ────────────────────────────────────────────────

    /// Вписать страницу: блокнот — целиком по центру, оверлей — по ширине,
    /// холст — масштаб 1 от начала.
    fn fit(&mut self) {
        let (w, h) = (self.bounds.size.width, self.bounds.size.height);
        if w < 2.0 || h < 2.0 {
            return;
        }
        let (pid, dims, _) = self.page_info();
        self.fitted = Some((pid, (w as i32, h as i32)));
        self.view = match dims {
            Some((pw, ph)) if self.overlay => View { zoom: w / pw, ox: 0.0, oy: 0.0 }.clamp_overlay(h, ph),
            Some((pw, ph)) => {
                let margin = 8.0;
                let z = ((w - margin * 2.0) / pw).min((h - margin * 2.0) / ph).max(0.05);
                View { zoom: z, ox: (w - pw * z) / 2.0, oy: (h - ph * z) / 2.0 }
            }
            None => {
                // Холст: всё нарисованное в окне, не крупнее 1:1.
                let b = self.handle.lock().pages.get(self.page_index()).map(|p| p.bbox()).unwrap_or(BBox::EMPTY);
                if b.is_empty() {
                    View::default()
                } else {
                    let b = b.inflate(24.0);
                    let z = (w / b.w()).min(h / b.h()).clamp(ZOOM_MIN, 1.0);
                    View { zoom: z, ox: (w - b.w() * z) / 2.0 - b.x0 * z, oy: (h - b.h() * z) / 2.0 - b.y0 * z }
                }
            }
        };
        self.invalidate_raster();
    }

    fn invalidate_raster(&mut self) {
        // Растр остаётся (рисуется со сдвигом), но будет пересчитан.
        if let Some(r) = self.raster.borrow_mut().as_mut() {
            r.scale = -1.0;
        }
        self.mark_dirty(DirtyFlags::RENDER);
    }

    /// Не дать странице уехать: при масштабе «вписано» и меньше — по центру.
    fn clamp_view(&mut self) {
        let (_, dims, inf) = self.page_info();
        if inf || self.overlay {
            return;
        }
        let Some((pw, ph)) = dims else { return };
        let (w, h) = (self.bounds.size.width, self.bounds.size.height);
        let z = self.view.zoom;
        let (sw, sh) = (pw * z, ph * z);
        self.view.ox = if sw <= w { (w - sw) / 2.0 } else { self.view.ox.clamp(w - sw - 40.0, 40.0) };
        self.view.oy = if sh <= h { (h - sh) / 2.0 } else { self.view.oy.clamp(h - sh - 40.0, 40.0) };
    }

    /// Масштаб вокруг точки экрана (локальной).
    fn zoom_at(&mut self, local: Point, z: f32) {
        let z = z.clamp(ZOOM_MIN, ZOOM_MAX);
        let p = self.view.to_page(local);
        self.view.zoom = z;
        self.view.ox = local.x - p.x * z;
        self.view.oy = local.y - p.y * z;
        self.clamp_view();
    }

    /// Страница целиком в окне (листать свайпом, а не двигать).
    fn page_fits(&self) -> bool {
        let (_, dims, inf) = self.page_info();
        if inf {
            return false;
        }
        let Some((pw, ph)) = dims else { return false };
        pw * self.view.zoom <= self.bounds.size.width + 1.0 && ph * self.view.zoom <= self.bounds.size.height + 1.0
    }

    // ── Растр ──────────────────────────────────────────────

    /// Видимая область страницы (единицы страницы).
    fn visible_region(&self) -> BBox {
        let a = self.view.to_page(Point::new(0.0, 0.0));
        let b = self.view.to_page(Point::new(self.bounds.size.width, self.bounds.size.height));
        BBox::new(a.x, a.y, b.x, b.y)
    }

    /// Обновить растр: пересчитать целиком при смене страницы/масштаба,
    /// иначе — только изменённые области. Возвращает (текстура, область).
    fn sync_raster(&self, dpr: f32) -> Option<(ImageHandle, BBox)> {
        let store = self.store.clone()?;
        let (pid, dims, _) = self.page_info();
        let tool = self.handle.tool.get_untracked();
        let scale = self.view.zoom * dpr;
        let visible = self.visible_region();
        let max_side = syngui::gpu::image_store::max_bitmap_side().clamp(1024, 8192) as f32;
        let gesture = self.gesture.is_some() || matches!(self.live, Some(Live::Pan { .. }));
        let rev = self.handle.revision.get_untracked();
        let mut slot = self.raster.borrow_mut();
        let stale = match slot.as_ref() {
            None => true,
            Some(r) => {
                r.page != pid
                    || r.crisp != tool.crisp
                    || ((r.scale - scale).abs() > 0.005 * scale || !covers(&r.region, &visible, dims)) && !gesture
            }
        };
        // Во время жеста — старый растр со сдвигом (быстро; пересчёт после).
        if stale && (slot.is_none() || slot.as_ref().is_some_and(|r| r.page != pid) || !gesture) {
            // Область: страница целиком, если влезает, иначе видимое с запасом.
            let page_rect = dims.map(|(w, h)| BBox::new(0.0, 0.0, w, h));
            let mut region = match page_rect {
                Some(p) if p.w() * scale <= max_side && p.h() * scale <= max_side => p,
                _ => {
                    let m = visible.w().max(visible.h()) * 0.25;
                    let mut r = visible.inflate(m);
                    if let Some(p) = page_rect {
                        r = BBox::new(r.x0.max(p.x0), r.y0.max(p.y0), r.x1.min(p.x1), r.y1.min(p.y1));
                    }
                    r
                }
            };
            // Не больше предела текстуры.
            let k = (max_side / (region.w() * scale)).min(max_side / (region.h() * scale)).min(1.0);
            if k < 1.0 {
                region = visible;
                if let Some(p) = page_rect {
                    region = BBox::new(region.x0.max(p.x0), region.y0.max(p.y0), region.x1.min(p.x1), region.y1.min(p.y1));
                }
            }
            if region.is_empty() || region.w() < 0.5 || region.h() < 0.5 {
                return slot.as_ref().and_then(|r| r.tex.map(|t| (t, r.region)));
            }
            let s = scale.min(max_side / region.w().max(1.0)).min(max_side / region.h().max(1.0));
            let w = (region.w() * s).ceil().max(1.0) as u32;
            let h = (region.h() * s).ceil().max(1.0) as u32;
            let mut pm = tiny_skia::Pixmap::new(w, h)?;
            let opts = self.raster_opts();
            {
                let d = self.handle.lock();
                let page = d.pages.get(self.page_index())?;
                let skip = self.skip_ids();
                raster::render_region(&mut pm, &d, page, region, s, &opts, &mut self.images.borrow_mut(), &|id| skip.contains(&id));
            }
            let mut rgba = Vec::new();
            raster::demultiply_into(&pm, &mut rgba);
            let key = next_key();
            let tex = {
                let mut st = store.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(old) = slot.as_ref().and_then(|r| r.tex) {
                    st.release(old);
                }
                let (hdl, _) = st.request_rgba(&format!("ink-raster-{key}"), w, h, rgba.clone());
                hdl
            };
            *self.local_dirty.borrow_mut() = BBox::EMPTY;
            *slot = Some(RasterCache { page: pid, region, scale: s, crisp: tool.crisp, pm, rgba, tex: Some(tex), rev, key });
            return Some((tex, region));
        }
        let r = slot.as_mut()?;
        // Частичная перерисовка.
        let mut dirty = std::mem::replace(&mut *self.local_dirty.borrow_mut(), BBox::EMPTY);
        if rev != r.rev {
            match self.handle.dirty_since(r.rev, pid) {
                super::Dirty::Nothing => {}
                super::Dirty::Region(b) => dirty = dirty.union(b),
                super::Dirty::All => dirty = dirty.union(r.region),
            }
            r.rev = rev;
        }
        if !dirty.is_empty() && dirty.intersects(&r.region) {
            let d0 = dirty.inflate(2.0);
            let sub = BBox::new(d0.x0.max(r.region.x0), d0.y0.max(r.region.y0), d0.x1.min(r.region.x1), d0.y1.min(r.region.y1));
            // В пиксели, с выравниванием.
            let px0 = (((sub.x0 - r.region.x0) * r.scale).floor() as i32).max(0);
            let py0 = (((sub.y0 - r.region.y0) * r.scale).floor() as i32).max(0);
            let px1 = (((sub.x1 - r.region.x0) * r.scale).ceil() as i32).min(r.pm.width() as i32);
            let py1 = (((sub.y1 - r.region.y0) * r.scale).ceil() as i32).min(r.pm.height() as i32);
            if px1 > px0 && py1 > py0 {
                let (pw, ph) = ((px1 - px0) as u32, (py1 - py0) as u32);
                if let Some(mut tmp) = tiny_skia::Pixmap::new(pw, ph) {
                    let sub_region = BBox::new(
                        r.region.x0 + px0 as f32 / r.scale,
                        r.region.y0 + py0 as f32 / r.scale,
                        r.region.x0 + px1 as f32 / r.scale,
                        r.region.y0 + py1 as f32 / r.scale,
                    );
                    let opts = self.raster_opts();
                    {
                        let d = self.handle.lock();
                        if let Some(page) = d.pages.get(self.page_index()) {
                            let skip = self.skip_ids();
                            raster::render_region(&mut tmp, &d, page, sub_region, r.scale, &opts, &mut self.images.borrow_mut(), &|id| skip.contains(&id));
                        }
                    }
                    raster::blit(&mut r.pm, &tmp, px0, py0);
                    // Обычный RGBA — только изменённые строки.
                    let w = r.pm.width() as usize;
                    for (y, row) in tmp.pixels().chunks(pw as usize).enumerate() {
                        let off = ((py0 as usize + y) * w + px0 as usize) * 4;
                        for (x, px) in row.iter().enumerate() {
                            let c = px.demultiply();
                            let o = off + x * 4;
                            r.rgba[o..o + 4].copy_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
                        }
                    }
                    if let Some(t) = r.tex {
                        let mut st = store.lock().unwrap_or_else(|e| e.into_inner());
                        st.update_rgba_regions(t, r.pm.width(), r.pm.height(), r.rgba.clone(), vec![[px0 as u32, py0 as u32, pw, ph]]);
                    }
                }
            }
        }
        let _ = r.key;
        r.tex.map(|t| (t, r.region))
    }

    fn raster_opts(&self) -> RasterOpts {
        let tool = self.handle.tool.get_untracked();
        RasterOpts { anti_alias: !tool.crisp, paper: !self.overlay, ..RasterOpts::default() }
    }

    /// Элементы, которые сейчас рисует сетка (перенос выделения).
    fn skip_ids(&self) -> Vec<u64> {
        match &self.live {
            Some(Live::SelDrag { items, .. }) => items.iter().map(|(_, i)| i.id()).collect(),
            _ => Vec::new(),
        }
    }

    // ── Перо ───────────────────────────────────────────────

    fn pen_near(&self) -> bool {
        self.pen_seen.is_some_and(|t| t.elapsed() < PEN_NEAR) || matches!(self.live, Some(Live::Stroke { .. } | Live::Erase { .. }))
    }

    fn active_layer(&self) -> Option<(u64, u64, bool)> {
        let d = self.handle.lock();
        let p = d.pages.get(self.page_index())?;
        let l = p.layers.get(p.active())?;
        Some((p.id, l.id, l.locked || !l.visible))
    }

    fn pen_down(&mut self, s: &PenSample, ctx: &mut EventContext) -> EventResult {
        if self.readonly {
            return EventResult::Ignored;
        }
        if !s.from_mouse {
            self.pen_seen = Some(Instant::now());
        }
        // Пальцы, что успели лечь, — ладонь: отменить их жест.
        self.gesture = None;
        let p = self.to_page(s.position);
        let ts = self.handle.tool.get_untracked();
        let eraser = s.tool == PenTool::Eraser || s.barrel;
        let tool = if eraser { Tool::Eraser } else { ts.tool };
        if self.editing.is_some() {
            self.finish_text();
        }
        // Выделение: тащим за рамку/ручки, иначе снимаем.
        let sel = self.handle.selection.get_untracked();
        if !sel.is_empty() && !eraser {
            if let Some(kind) = self.hit_selection(p) {
                let items = self.handle.selected_items();
                let bbox = items.iter().fold(BBox::EMPTY, |b, (_, i)| b.union(i.bbox()));
                *self.local_dirty.borrow_mut() = bbox;
                self.live = Some(Live::SelDrag { kind, start: p, m: Affine::IDENTITY, bbox, items, mesh: Mesh::default() });
                self.rebuild_drag_mesh();
                self.mark_dirty(DirtyFlags::RENDER);
                return EventResult::Handled;
            }
            self.handle.clear_selection();
        }
        let (_, _, locked) = self.active_layer().unwrap_or((0, 0, true));
        let t = s.time_ms;
        match tool {
            Tool::Pen => {
                if locked {
                    return EventResult::Handled;
                }
                let pt = InkPoint { x: p.x, y: p.y, p: s.pressure, tx: s.tilt_x, ty: s.tilt_y, t: 0 };
                let mut live = Live::Stroke {
                    points: vec![pt],
                    brush: ts.brush,
                    color: ts.color,
                    width: ts.width(),
                    mesh: Mesh::default(),
                    anchor: (p, Instant::now()),
                    shape: None,
                };
                if let Live::Stroke { points, .. } = &mut live {
                    // Время точек — от начала штриха.
                    points[0].t = 0;
                }
                self.stroke_t0 = t;
                self.live = Some(live);
                self.rebuild_stroke_mesh();
            }
            Tool::Eraser => {
                let mode = if eraser && ts.tool != Tool::Eraser { ts.eraser } else { ts.eraser };
                self.live = Some(Live::Erase { mode, last: p, ops: Vec::new(), path: vec![p] });
                if mode != EraserMode::Area {
                    self.erase_to(p);
                }
            }
            Tool::Lasso => self.live = Some(Live::Lasso { path: vec![p] }),
            Tool::Shape => {
                if locked {
                    return EventResult::Handled;
                }
                self.live = Some(Live::Shape { kind: ts.shape, a: p, b: p });
            }
            Tool::Text => {
                self.text_at(p);
                return EventResult::Handled;
            }
            Tool::Fill => {
                self.fill_at(p, ts.color);
                return EventResult::Handled;
            }
            Tool::Eyedropper => {
                let page = self.handle.lock().pages.get(self.page_index()).cloned();
                if let Some(c) = page.and_then(|pg| tools::pick_color(&pg, p, 4.0 * self.px().max(0.5))) {
                    self.handle.set_tool(|t| {
                        t.color = c;
                        t.tool = Tool::Pen;
                    });
                }
                return EventResult::Handled;
            }
            Tool::Hand => {
                self.live = Some(Live::Pan { start: self.local(s.position), view: self.view });
            }
        }
        let _ = ctx;
        self.mark_dirty(DirtyFlags::RENDER);
        EventResult::Handled
    }

    fn pen_move(&mut self, s: &PenSample) -> EventResult {
        if !s.from_mouse {
            self.pen_seen = Some(Instant::now());
        }
        let p = self.to_page(s.position);
        let t0 = self.stroke_t0;
        let hold_on = self.handle.tool.get_untracked().hold_to_shape;
        let l = self.local(s.position);
        let zoom = self.view.zoom;
        match &mut self.live {
            Some(Live::Stroke { points, anchor, shape, .. }) => {
                if shape.is_some() {
                    return EventResult::Handled;
                }
                points.push(InkPoint { x: p.x, y: p.y, p: s.pressure, tx: s.tilt_x, ty: s.tilt_y, t: s.time_ms.saturating_sub(t0) });
                if anchor.0.dist(p) > 3.0 * (1.0 / zoom).max(0.5) {
                    *anchor = (p, Instant::now());
                }
                let held = hold_on && anchor.1.elapsed() >= HOLD;
                self.rebuild_stroke_mesh();
                if held {
                    self.try_recognize();
                }
            }
            Some(Live::Erase { mode, path, .. }) => {
                path.push(p);
                if *mode != EraserMode::Area {
                    self.erase_to(p);
                }
            }
            Some(Live::Lasso { path }) => {
                if path.last().is_none_or(|l| l.dist(p) > 1.0) {
                    path.push(p);
                }
            }
            Some(Live::Shape { b, .. }) => *b = p,
            Some(Live::SelDrag { .. }) => {
                self.update_drag(p);
            }
            Some(Live::Pan { start, view }) => {
                self.view.ox = view.ox + l.x - start.x;
                self.view.oy = view.oy + l.y - start.y;
                self.clamp_view();
            }
            None => return EventResult::Ignored,
        }
        self.mark_dirty(DirtyFlags::RENDER);
        EventResult::Handled
    }

    fn pen_up(&mut self, s: &PenSample) -> EventResult {
        if !s.from_mouse {
            self.pen_seen = Some(Instant::now());
        }
        let Some(live) = self.live.take() else { return EventResult::Ignored };
        match live {
            Live::Stroke { points, brush, color, width, shape, .. } => {
                let item = match shape {
                    Some(sh) => Item::Shape(sh),
                    None => Item::Stroke(Stroke { id: 0, brush, color, width, points }),
                };
                self.handle.add_item(item);
            }
            Live::Erase { mode, ops, path, .. } => match mode {
                EraserMode::Area => self.erase_area(&path),
                _ => {
                    if !ops.is_empty() {
                        self.handle.record(Op::Batch(ops));
                    }
                }
            },
            Live::Lasso { path } => {
                if let Some((pid, lid, _)) = self.active_layer() {
                    let ids = {
                        let d = self.handle.lock();
                        d.pages.get(self.page_index()).and_then(|pg| pg.layers.get(pg.active())).map(|l| tools::lasso_select(l, &path)).unwrap_or_default()
                    };
                    if !ids.is_empty() {
                        self.handle.select(Selection { page: pid, layer: lid, ids });
                    }
                }
            }
            Live::Shape { kind, a, b } => {
                if a.dist(b) > 2.0 {
                    let ts = self.handle.tool.get_untracked();
                    let mut sh = shape_between(kind, a, b, ts.color, ts.shape_width());
                    sh.fill = ts.shape_fill.filter(|_| kind.closed());
                    self.handle.add_item(Item::Shape(sh));
                }
            }
            Live::SelDrag { m, bbox, .. } => {
                *self.local_dirty.borrow_mut() = bbox;
                if m != Affine::IDENTITY {
                    self.handle.transform_selection(&m);
                }
            }
            Live::Pan { .. } => self.invalidate_raster(),
        }
        self.mark_dirty(DirtyFlags::RENDER);
        EventResult::Handled
    }

    fn rebuild_stroke_mesh(&mut self) {
        let px = self.px();
        if let Some(Live::Stroke { points, brush, color, width, mesh, shape, .. }) = &mut self.live {
            mesh.clear();
            match shape {
                Some(sh) => mesh::shape_mesh(mesh, sh, px, 1.0 / px),
                None => {
                    let dabs = super::brush::dabs_of(*brush, *color, *width, points);
                    mesh::dabs_mesh(mesh, &dabs, px, 1.0 / px);
                }
            }
        }
    }

    /// Перо задержали: штрих → фигура (если похож).
    fn try_recognize(&mut self) {
        let mut done = false;
        if let Some(Live::Stroke { points, color, shape, .. }) = &mut self.live {
            if shape.is_none() && points.len() > 3 {
                let pts: Vec<P2> = points.iter().map(|p| p.pos()).collect();
                let w = self.handle.tool.get_untracked().shape_width();
                if let Some(sh) = recognize::recognize(&pts, 0, *color, w) {
                    *shape = Some(sh);
                    done = true;
                }
            }
        }
        if done {
            self.rebuild_stroke_mesh();
            self.mark_dirty(DirtyFlags::RENDER);
        }
    }

    /// Ластик: отрезок от прошлой точки до `p`.
    fn erase_to(&mut self, p: P2) {
        let r = {
            let ts = self.handle.tool.get_untracked();
            // Не меньше 6 px экрана — по мелкому масштабу всё равно попадать.
            ts.eraser_r.max(6.0 / self.view.zoom)
        };
        let Some(Live::Erase { mode, last, ops, .. }) = &mut self.live else { return };
        let a = *last;
        *last = p;
        let op = {
            let mut d = self.handle.lock();
            let pi = self.page_override.unwrap_or_else(|| self.handle.page.get_untracked()).min(d.pages.len().saturating_sub(1));
            let Some(page) = d.pages.get(pi) else { return };
            let li = page.active();
            let (pid, layer) = (page.id, page.layers[li].clone());
            match mode {
                EraserMode::Stroke => {
                    let ids = tools::erase_whole(&layer, a, p, r);
                    if ids.is_empty() {
                        return;
                    }
                    let removed: Vec<(usize, Item)> =
                        layer.items.iter().enumerate().filter(|(_, i)| ids.contains(&i.id())).map(|(k, i)| (k, i.clone())).collect();
                    Op::Items { page: pid, layer: layer.id, removed, added: Vec::new() }
                }
                _ => {
                    let mut alloc = || d.alloc_id();
                    let (removed, added) = tools::erase_points(&layer, a, p, r, &mut alloc);
                    if removed.is_empty() {
                        return;
                    }
                    Op::Items { page: pid, layer: layer.id, removed, added }
                }
            }
        };
        self.handle.apply_live(&op);
        ops.push(op);
    }

    /// Ластик-лассо: стереть всё внутри обводки (незаблокированные слои).
    fn erase_area(&mut self, path: &[P2]) {
        let op = {
            let d = self.handle.lock();
            let Some(page) = d.pages.get(self.page_index()) else { return };
            let mut ops = Vec::new();
            for l in &page.layers {
                let ids = tools::lasso_select(l, path);
                if ids.is_empty() {
                    continue;
                }
                let removed = l.items.iter().enumerate().filter(|(_, i)| ids.contains(&i.id())).map(|(k, i)| (k, i.clone())).collect();
                ops.push(Op::Items { page: page.id, layer: l.id, removed, added: Vec::new() });
            }
            Op::Batch(ops)
        };
        self.handle.commit(op);
    }

    fn fill_at(&mut self, p: P2, color: Rgba) {
        let (pid, lid, locked) = match self.active_layer() {
            Some(x) => x,
            None => return,
        };
        if locked {
            return;
        }
        let res = {
            let d = self.handle.lock();
            let Some(page) = d.pages.get(self.page_index()) else { return };
            let region = match page.dims() {
                Some((w, h)) => BBox::new(0.0, 0.0, w, h),
                None => self.visible_region(),
            };
            let scale = (1.5 * self.view.zoom).clamp(0.5, 3.0);
            tools::flood_fill(&d, page, region, p, color, scale)
        };
        let Some((png, bb)) = res else { return };
        let op = {
            let mut d = self.handle.lock();
            let key = d.add_asset(png);
            let id = d.alloc_id();
            // Заливка — под штрихами слоя.
            let item = Item::Image(super::model::ImageItem { id, asset: key, x: bb.x0, y: bb.y0, w: bb.w(), h: bb.h(), angle: 0.0 });
            Op::Items { page: pid, layer: lid, removed: Vec::new(), added: vec![(0, item)] }
        };
        self.handle.commit(op);
    }

    // ── Текст ──────────────────────────────────────────────

    fn text_at(&mut self, p: P2) {
        let pid = self.page_info().0;
        let hit = {
            let d = self.handle.lock();
            d.pages.get(self.page_index()).and_then(|pg| {
                pg.layers.iter().flat_map(|l| l.items.iter()).rev().find_map(|i| match i {
                    Item::Text(t) if t.bbox().contains(p) => Some(t.id),
                    _ => None,
                })
            })
        };
        let id = match hit {
            Some(id) => Some(id),
            None => {
                let ts = self.handle.tool.get_untracked();
                let w = match self.page_info().1 {
                    Some((pw, _)) => (pw - p.x - 24.0).clamp(80.0, 480.0),
                    None => 320.0,
                };
                let size = 18.0 + ts.width_step() as f32 * 4.0;
                self.handle.add_item(Item::Text(TextItem { id: 0, text: String::new(), x: p.x, y: p.y - size * 0.6, w, size, color: ts.color, bold: false }))
            }
        };
        if let Some(id) = id {
            self.handle.editing_text.set(Some((pid, id)));
        }
    }

    /// Закончить правку текста (пустой — удалить).
    fn finish_text(&mut self) {
        self.handle.editing_text.set(None);
    }

    fn text_item(&self, page: u64, id: u64) -> Option<TextItem> {
        let d = self.handle.lock();
        let pi = d.page_index(page)?;
        d.pages[pi].layers.iter().flat_map(|l| l.items.iter()).find_map(|i| match i {
            Item::Text(t) if t.id == id => Some(t.clone()),
            _ => None,
        })
    }

    // ── Выделение ──────────────────────────────────────────

    fn selection_corners(bbox: BBox, m: &Affine) -> [P2; 4] {
        [
            m.apply(P2::new(bbox.x0, bbox.y0)),
            m.apply(P2::new(bbox.x1, bbox.y0)),
            m.apply(P2::new(bbox.x1, bbox.y1)),
            m.apply(P2::new(bbox.x0, bbox.y1)),
        ]
    }

    fn rotate_handle(corners: &[P2; 4], px: f32) -> P2 {
        let top = corners[0].lerp(corners[1], 0.5);
        let c = corners[0].lerp(corners[2], 0.5);
        let d = top.sub(c);
        let l = d.len().max(1e-3);
        top.add(d.scale(28.0 * px / l))
    }

    fn hit_selection(&self, p: P2) -> Option<DragKind> {
        let bbox = self.handle.selection_bbox();
        if bbox.is_empty() {
            return None;
        }
        let px = self.px() * self.dpr();
        let corners = Self::selection_corners(bbox, &Affine::IDENTITY);
        let tol = HANDLE_PX * 1.4 * px;
        if Self::rotate_handle(&corners, px).dist(p) <= tol {
            return Some(DragKind::Rotate);
        }
        for (i, c) in corners.iter().enumerate() {
            if c.dist(p) <= tol {
                return Some(DragKind::Scale(i));
            }
        }
        bbox.inflate(4.0 * px).contains(p).then_some(DragKind::Move)
    }

    fn update_drag(&mut self, p: P2) {
        if let Some(Live::SelDrag { kind, start, m, bbox, .. }) = &mut self.live {
            let c = bbox.center();
            *m = match *kind {
                DragKind::Move => Affine::translate(p.x - start.x, p.y - start.y),
                DragKind::Scale(i) => {
                    let corners = Self::selection_corners(*bbox, &Affine::IDENTITY);
                    let anchor = corners[(i + 2) % 4];
                    let k = (p.dist(anchor) / start.dist(anchor).max(1e-3)).clamp(0.05, 20.0);
                    Affine::scale_about(anchor, k, k)
                }
                DragKind::Rotate => {
                    let a0 = (start.y - c.y).atan2(start.x - c.x);
                    let a1 = (p.y - c.y).atan2(p.x - c.x);
                    Affine::rotate_about(c, a1 - a0)
                }
            };
        }
        self.rebuild_drag_mesh();
    }

    fn rebuild_drag_mesh(&mut self) {
        let px = self.px();
        if let Some(Live::SelDrag { m, items, mesh, .. }) = &mut self.live {
            mesh.clear();
            for (_, it) in items.iter() {
                let mut j = it.clone();
                j.transform(m);
                mesh::item_mesh(mesh, &j, px, 1.0 / px);
                if let Item::Image(im) = &j {
                    let c = im.corners();
                    mesh.polyline(&c, true, px * 1.5, mesh::color_of(Rgba([90, 90, 90, 255])), px, 1.0 / px);
                }
            }
        }
    }

    // ── Пальцы ─────────────────────────────────────────────

    fn touch_start(&mut self, id: u64, pos: Point) -> EventResult {
        if self.overlay {
            return EventResult::Ignored;
        }
        // Ладонь при пере — проглотить.
        if self.pen_near() && self.finger_pen.is_none() {
            return EventResult::Handled;
        }
        self.fingers.insert(id, pos);
        let ts = self.handle.tool.get_untracked();
        if self.fingers.len() == 1 && ts.finger_draw && !self.readonly && ts.tool != Tool::Hand {
            self.finger_pen = Some(id);
            let s = finger_sample(pos);
            self.gesture = Some(self.new_gesture());
            let mut dummy = EventContext::new(self.id);
            return self.pen_down(&s, &mut dummy);
        }
        if self.fingers.len() >= 2 {
            // Второй палец отменяет рисование пальцем.
            if self.finger_pen.take().is_some() {
                self.live = None;
            }
        }
        self.gesture = Some(self.new_gesture());
        EventResult::Handled
    }

    fn new_gesture(&self) -> Gesture {
        let pts: Vec<Point> = self.fingers.values().copied().collect();
        let (c0, d0) = centroid_spread(&pts);
        let max = self.gesture.as_ref().map(|g| g.max_fingers).unwrap_or(0).max(pts.len());
        Gesture {
            start_view: self.view,
            c0,
            d0,
            one: (pts.len() == 1).then(|| pts[0]),
            max_fingers: max,
            moved: self.gesture.as_ref().is_some_and(|g| g.moved),
            started: self.gesture.as_ref().map(|g| g.started).unwrap_or_else(Instant::now),
        }
    }

    fn touch_move(&mut self, id: u64, pos: Point) -> EventResult {
        if self.finger_pen == Some(id) {
            let s = finger_sample(pos);
            return self.pen_move(&s);
        }
        if !self.fingers.contains_key(&id) {
            return if self.pen_near() { EventResult::Handled } else { EventResult::Ignored };
        }
        self.fingers.insert(id, pos);
        let pts: Vec<Point> = self.fingers.values().copied().collect();
        let (_, dims, inf) = self.page_info();
        let (bw, bh) = (self.bounds.size.width, self.bounds.size.height);
        let bounds = self.bounds;
        let Some(g) = &mut self.gesture else { return EventResult::Handled };
        if pts.len() >= 2 {
            let (c, d) = centroid_spread(&pts);
            if (c.x - g.c0.x).abs() + (c.y - g.c0.y).abs() > 6.0 || (d - g.d0).abs() > 6.0 {
                g.moved = true;
            }
            let k = if g.d0 > 10.0 { d / g.d0 } else { 1.0 };
            let z = (g.start_view.zoom * k).clamp(ZOOM_MIN, ZOOM_MAX);
            let c0l = self_local(bounds, g.c0);
            let cl = self_local(bounds, c);
            // Точка страницы под начальным центром идёт за текущим центром.
            let pp = g.start_view.to_page(c0l);
            self.view = View { zoom: z, ox: cl.x - pp.x * z, oy: cl.y - pp.y * z };
            self.clamp_view();
        } else if let Some(start) = g.one {
            let (dx, dy) = (pos.x - start.x, pos.y - start.y);
            if dx.abs() + dy.abs() > 8.0 {
                g.moved = true;
            }
            let z = g.start_view.zoom;
            let fits = !inf && dims.is_some_and(|(pw, ph)| pw * z <= bw + 1.0 && ph * z <= bh + 1.0);
            if !fits {
                let sv = g.start_view;
                self.view.ox = sv.ox + dx;
                self.view.oy = sv.oy + dy;
                self.clamp_view();
            }
        }
        self.mark_dirty(DirtyFlags::RENDER);
        EventResult::Handled
    }

    fn touch_end(&mut self, id: u64, pos: Point) -> EventResult {
        if self.finger_pen == Some(id) {
            self.finger_pen = None;
            self.fingers.remove(&id);
            self.gesture = None;
            let s = finger_sample(pos);
            return self.pen_up(&s);
        }
        if self.fingers.remove(&id).is_none() {
            return if self.pen_near() { EventResult::Handled } else { EventResult::Ignored };
        }
        if !self.fingers.is_empty() {
            // Остальные пальцы продолжают с нового начала.
            self.gesture = Some(self.new_gesture());
            return EventResult::Handled;
        }
        let Some(g) = self.gesture.take() else { return EventResult::Handled };
        let quick = g.started.elapsed() < Duration::from_millis(350);
        if !g.moved && quick && g.max_fingers == 2 {
            self.handle.undo();
        } else if !g.moved && quick && g.max_fingers == 3 {
            self.handle.redo();
        } else if g.max_fingers == 1 {
            if let Some(start) = g.one {
                let (dx, dy) = (pos.x - start.x, pos.y - start.y);
                if self.page_fits() && dx.abs() > 60.0 && dx.abs() > dy.abs() * 1.5 && !self.overlay {
                    // Свайп влево — следующая (на последней — новая страница
                    // у блокнота), вправо — предыдущая.
                    if dx < 0.0 {
                        let create = self.handle.kind() == DocKind::Notebook;
                        self.handle.next_page(create);
                    } else {
                        self.handle.prev_page();
                    }
                }
            }
        }
        self.invalidate_raster();
        EventResult::Handled
    }

    // ── Отрисовка ──────────────────────────────────────────

    fn draw_overlay_items(&self, list: &mut DisplayList) {
        let px = self.px();
        let mut m = Mesh::default();
        match &self.live {
            Some(Live::Stroke { mesh, .. }) => {
                if !mesh.is_empty() {
                    list.push_canvas(mesh.vertices.clone(), mesh.indices.clone());
                }
            }
            Some(Live::SelDrag { mesh, m: mm, bbox, .. }) => {
                if !mesh.is_empty() {
                    list.push_canvas(mesh.vertices.clone(), mesh.indices.clone());
                }
                let corners = Self::selection_corners(*bbox, mm);
                m.dashed(&corners, true, px * 1.5, 6.0 * px, mesh::color_of(Rgba([60, 60, 60, 255])), px);
            }
            Some(Live::Lasso { path }) => {
                m.dashed(path, false, px * 1.5, 6.0 * px, mesh::color_of(Rgba([40, 40, 40, 255])), px);
            }
            Some(Live::Erase { mode: EraserMode::Area, path, .. }) => {
                m.dashed(path, false, px * 1.5, 6.0 * px, mesh::color_of(Rgba([160, 40, 40, 255])), px);
            }
            Some(Live::Erase { last, .. }) => {
                let r = self.handle.tool.get_untracked().eraser_r.max(6.0 / self.view.zoom);
                let ring = super::brush::circle(*last, r, 32);
                m.polyline(&ring, true, px * 1.5, mesh::color_of(Rgba([90, 90, 90, 255])), px, 1.0 / px);
            }
            Some(Live::Shape { kind, a, b }) => {
                let ts = self.handle.tool.get_untracked();
                let mut sh = shape_between(*kind, *a, *b, ts.color, ts.shape_width());
                sh.fill = ts.shape_fill.filter(|_| kind.closed());
                mesh::shape_mesh(&mut m, &sh, px, 1.0 / px);
            }
            _ => {}
        }
        // Рамка выделения с ручками (когда не тащим).
        if !matches!(self.live, Some(Live::SelDrag { .. })) {
            let bbox = self.handle.selection_bbox();
            if !bbox.is_empty() {
                let corners = Self::selection_corners(bbox, &Affine::IDENTITY);
                let ink = mesh::color_of(Rgba([30, 30, 30, 255]));
                m.dashed(&corners, true, px * 1.5, 6.0 * px, ink, px);
                let hs = HANDLE_PX * px * self.dpr() / 2.0;
                for c in &corners {
                    let sq = [P2::new(c.x - hs, c.y - hs), P2::new(c.x + hs, c.y - hs), P2::new(c.x + hs, c.y + hs), P2::new(c.x - hs, c.y + hs)];
                    m.fill_fan(&sq, mesh::color_of(Rgba::WHITE));
                    m.polyline(&sq, true, px * 2.0, ink, px, 1.0 / px);
                }
                let rh = Self::rotate_handle(&corners, px * self.dpr());
                let top = corners[0].lerp(corners[1], 0.5);
                m.polyline(&[top, rh], false, px * 1.5, ink, px, 1.0 / px);
                m.disc(rh, hs * 0.8, mesh::color_of(Rgba::WHITE), px, 1.0 / px);
                m.polyline(&super::brush::circle(rh, hs * 0.8, 20), true, px * 2.0, ink, px, 1.0 / px);
            }
        }
        if !m.is_empty() {
            list.push_canvas(m.vertices, m.indices);
        }
    }

    fn draw_texts(&self, list: &mut DisplayList) {
        let d = self.handle.lock();
        let Some(page) = d.pages.get(self.page_index()) else { return };
        let skip = self.skip_ids();
        let editing = self.editing.map(|(_, id)| id);
        let vis = self.visible_region().inflate(50.0);
        for l in page.layers.iter().filter(|l| l.visible) {
            for it in &l.items {
                if let Item::Text(t) = it {
                    if skip.contains(&t.id) || editing == Some(t.id) || !t.bbox().intersects(&vis) {
                        continue;
                    }
                    let c = Color::from_srgb(t.color.0[0], t.color.0[1], t.color.0[2], t.color.alpha() * l.opacity);
                    list.push_text_styled(
                        &t.text,
                        Rect::new(Point::new(t.x, t.y), Size::new(t.w, t.approx_height() + t.size)),
                        c,
                        t.size,
                        TextAlign::LEFT,
                        TextDecoration::None,
                        if t.bold { 700 } else { 400 },
                        None,
                    );
                }
            }
        }
        // Текст, который тащим лассо.
        if let Some(Live::SelDrag { items, m, .. }) = &self.live {
            for (_, it) in items {
                if let Item::Text(t) = it {
                    let mut j = Item::Text(t.clone());
                    j.transform(m);
                    if let Item::Text(t) = j {
                        let c = Color::from_srgb(t.color.0[0], t.color.0[1], t.color.0[2], t.color.alpha());
                        list.push_text_styled(&t.text, Rect::new(Point::new(t.x, t.y), Size::new(t.w, t.approx_height() + t.size)), c, t.size, TextAlign::LEFT, TextDecoration::None, if t.bold { 700 } else { 400 }, None);
                    }
                }
            }
        }
    }
}

fn self_local(b: Rect, p: Point) -> Point {
    Point::new(p.x - b.origin.x, p.y - b.origin.y)
}

fn finger_sample(pos: Point) -> PenSample {
    PenSample { position: pos, pressure: 0.5, tilt_x: 0.0, tilt_y: 0.0, tool: PenTool::Pen, barrel: false, time_ms: 0, from_mouse: true }
}

fn centroid_spread(pts: &[Point]) -> (Point, f32) {
    if pts.is_empty() {
        return (Point::zero(), 0.0);
    }
    let n = pts.len() as f32;
    let c = Point::new(pts.iter().map(|p| p.x).sum::<f32>() / n, pts.iter().map(|p| p.y).sum::<f32>() / n);
    let d = pts.iter().map(|p| ((p.x - c.x).powi(2) + (p.y - c.y).powi(2)).sqrt()).sum::<f32>() / n;
    (c, d * 2.0)
}

/// Покрывает ли растр видимую часть страницы.
fn covers(region: &BBox, visible: &BBox, dims: Option<(f32, f32)>) -> bool {
    let v = match dims {
        Some((w, h)) => BBox::new(visible.x0.max(0.0), visible.y0.max(0.0), visible.x1.min(w), visible.y1.min(h)),
        None => *visible,
    };
    if v.is_empty() {
        return true;
    }
    v.x0 >= region.x0 - 0.5 && v.y0 >= region.y0 - 0.5 && v.x1 <= region.x1 + 0.5 && v.y1 <= region.y1 + 0.5
}

/// Фигура по двум точкам (угол-угол или начало-конец).
pub fn shape_between(kind: ShapeKind, a: P2, b: P2, color: Rgba, width: f32) -> Shape {
    match kind {
        ShapeKind::Line => Shape::line(0, a, b, color, width, false),
        ShapeKind::Arrow => Shape::line(0, a, b, color, width, true),
        ShapeKind::Ellipse => Shape::ellipse(0, a, b, color, width),
        ShapeKind::Triangle => {
            let bb = BBox::new(a.x, a.y, b.x, b.y);
            Shape { id: 0, kind, pts: vec![P2::new(bb.center().x, bb.y0), P2::new(bb.x1, bb.y1), P2::new(bb.x0, bb.y1)], color, width, fill: None }
        }
        _ => Shape::rect(0, a, b, color, width),
    }
}

impl View {
    fn clamp_overlay(self, _h: f32, _ph: f32) -> View {
        self
    }
}

impl Element for InkElement {
    fn update(&mut self, widget: &dyn Widget, ctx: &mut UpdateContext) {
        let Some(w) = widget.as_any().downcast_ref::<InkCanvas>() else { return };
        self.handle = w.handle.clone();
        self.overlay = w.overlay;
        self.readonly = w.readonly;
        if self.page_override != w.page {
            self.page_override = w.page;
            self.fitted = None;
        }
        self.classes = w.classes.clone();
        self.sync_signals();
        self.mark_dirty(DirtyFlags::LAYOUT | DirtyFlags::RENDER);
        ctx.mark_layout_dirty();
    }

    fn on_signal_change(&mut self) -> bool {
        self.sync_signals();
        self.dirty |= DirtyFlags::RENDER;
        true
    }

    fn mount(&mut self, tree: &mut ElementTree) {
        self.store = tree.image_store.clone();
        if !self.subscribed {
            self.subscribed = true;
            let id = self.id;
            self.handle.revision.subscribe_element(id);
            self.handle.page.subscribe_element(id);
            self.handle.tool.subscribe_element(id);
            self.handle.selection.subscribe_element(id);
            self.handle.fit_request.subscribe_element(id);
            self.handle.editing_text.subscribe_element(id);
        }
    }

    fn layout(&mut self, c: Constraints) -> Size {
        let w = if c.max_width.is_finite() { c.max_width } else { 600.0 };
        let h = if c.max_height.is_finite() { c.max_height } else { 800.0 };
        let size = Size::new(w, h);
        self.bounds.size = size;
        let pid = self.page_info().0;
        if self.fitted.map(|(p, s)| p != pid || s != (w as i32, h as i32)).unwrap_or(true) {
            self.fit();
        }
        size
    }

    fn layout_hint(&self) -> LayoutHint {
        LayoutHint::Stack { expand: true }
    }

    fn fills_available(&self) -> bool {
        true
    }

    fn manages_own_children(&self) -> bool {
        true
    }

    fn needs_rebuild(&self) -> bool {
        self.rebuild
    }

    fn clear_rebuild(&mut self) {
        self.rebuild = false;
    }

    fn build_children(&self) -> Vec<Box<dyn Widget>> {
        let Some((pid, id)) = self.editing else { return Vec::new() };
        if pid != self.page_info().0 {
            return Vec::new();
        }
        let Some(t) = self.text_item(pid, id) else { return Vec::new() };
        let tl = self.view.to_local(P2::new(t.x, t.y));
        let w = (t.w * self.view.zoom).max(120.0);
        let h = ((t.approx_height() + t.size) * self.view.zoom).max(36.0);
        let h_submit = self.handle.clone();
        let h_esc = self.handle.clone();
        let commit = move |h: &InkHandle, text: Option<String>| {
            let page = h.lock().page_index(pid);
            if let (Some(text), Some(_)) = (text, page) {
                let op = {
                    let d = h.lock();
                    let pi = d.page_index(pid).unwrap_or(0);
                    d.pages[pi].layers.iter().find_map(|l| {
                        l.find(id).map(|k| {
                            let old = l.items[k].clone();
                            let added = if text.trim().is_empty() {
                                Vec::new()
                            } else {
                                let mut new = old.clone();
                                if let Item::Text(t) = &mut new {
                                    t.text = text.clone();
                                }
                                vec![(k, new)]
                            };
                            Op::Items { page: pid, layer: l.id, removed: vec![(k, old)], added }
                        })
                    })
                };
                if let Some(op) = op {
                    h.commit(op);
                }
            }
            h.editing_text.set(None);
        };
        let c1 = commit.clone();
        vec![Box::new(
            Positioned::new(
                TextField::with_text(t.text.clone())
                    .autofocus(true)
                    .submit_on_focus_lost(true)
                    .width(w)
                    .on_submit(move |s| c1(&h_submit, Some(s.to_string())))
                    .on_escape(move || commit(&h_esc, None))
                    .class("ink-text-editor"),
            )
            .at(tl.x, tl.y)
            .size(Size::new(w, h)),
        )]
    }

    fn take_focus_request(&mut self) -> bool {
        std::mem::take(&mut self.focus_request)
    }

    fn accessibility_info(&self) -> Option<syngui::a11y::AccessibilityInfo> {
        // Роль поля ввода — чтобы щелчок отдавал холсту фокус клавиатуры.
        Some(syngui::a11y::AccessibilityInfo {
            role: syngui::a11y::Role::TextField,
            state: syngui::a11y::NodeState { focused: self.focused, ..Default::default() },
            properties: syngui::a11y::NodeProperties::default(),
        })
    }

    fn needs_repaint(&self) -> bool {
        // Перо стоит в конце штриха — ждём удержания (мышь событий не шлёт).
        matches!(&self.live, Some(Live::Stroke { shape: None, .. })) && self.handle.tool.get_untracked().hold_to_shape
    }

    fn animate(&mut self, _dt: Duration) -> bool {
        let held = matches!(&self.live, Some(Live::Stroke { anchor, shape: None, points, .. }) if anchor.1.elapsed() >= HOLD && points.len() > 3);
        if held {
            self.try_recognize();
            return true;
        }
        false
    }

    fn build_display_list(&self, list: &mut DisplayList, _clip: Rect) {
        let dpr = list.scale_factor().max(0.5);
        *self.dpr.lock().unwrap_or_else(|e| e.into_inner()) = dpr;
        list.push_clip(self.bounds);
        if !self.overlay {
            let desk = self.mss.background_color.unwrap_or_else(|| Color::from_srgb(232, 232, 232, 1.0));
            list.push_rect(self.bounds, desk, [0.0; 4]);
        }
        let o = self.bounds.origin;
        if let Some((tex, region)) = self.sync_raster(dpr) {
            let a = self.view.to_local(P2::new(region.x0, region.y0));
            let r = Rect::new(Point::new(o.x + a.x, o.y + a.y), Size::new(region.w() * self.view.zoom, region.h() * self.view.zoom));
            list.push_image(r, TextureId(tex.0), Rect::new(Point::new(0.0, 0.0), Size::new(1.0, 1.0)), Color::WHITE);
        }
        // Остальное — в координатах страницы.
        list.push_transform(Transform::new(self.view.zoom, 0.0, 0.0, self.view.zoom, o.x + self.view.ox, o.y + self.view.oy));
        self.draw_texts(list);
        self.draw_overlay_items(list);
        list.pop_transform();
        list.pop_clip();
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut EventContext) -> EventResult {
        // Перо и мышь.
        if let Some((phase, s)) = pen_event(event, self.mouse_btn) {
            let inside = self.bounds.contains(s.position);
            return match phase {
                PenPhase::Down => {
                    if !inside {
                        return EventResult::Ignored;
                    }
                    if s.from_mouse {
                        self.mouse_btn = Some(if s.barrel { MouseButton::Right } else { MouseButton::Left });
                        ctx.capture();
                    }
                    self.focus_request = true;
                    let r = self.pen_down(&s, ctx);
                    ctx.request_paint();
                    r
                }
                PenPhase::Move => {
                    let r = self.pen_move(&s);
                    ctx.request_paint();
                    r
                }
                PenPhase::Up => {
                    self.mouse_btn = None;
                    let r = self.pen_up(&s);
                    ctx.request_paint();
                    r
                }
                PenPhase::Hover => {
                    if !s.from_mouse {
                        self.pen_seen = Some(Instant::now());
                        return if inside { EventResult::Handled } else { EventResult::Ignored };
                    }
                    EventResult::Ignored
                }
                PenPhase::Leave => {
                    if !s.from_mouse && self.live.is_some() {
                        // Перо ушло, не оторвав кончик — закончить как отрыв.
                        let r = self.pen_up(&s);
                        ctx.request_paint();
                        return r;
                    }
                    EventResult::Ignored
                }
            };
        }
        match event {
            Event::TouchStart { id, position } => {
                if !self.bounds.contains(*position) {
                    return EventResult::Ignored;
                }
                let r = self.touch_start(*id, *position);
                ctx.request_paint();
                r
            }
            Event::TouchMove { id, position } => {
                let r = self.touch_move(*id, *position);
                ctx.request_paint();
                r
            }
            Event::TouchEnd { id, position } => {
                let r = self.touch_end(*id, *position);
                ctx.request_paint();
                r
            }
            // Синтезированные из касаний щелчки — пальцы уже разобраны.
            Event::MouseDown { .. } | Event::MouseUp { .. } | Event::DoubleClick { .. }
                if syngui::input::is_synthesized_mouse() && !self.overlay =>
            {
                EventResult::Handled
            }
            Event::LongPress { .. } if !self.overlay => EventResult::Handled,
            Event::MouseWheel { delta, delta_x, position } => {
                if !self.bounds.contains(*position) || self.overlay {
                    return EventResult::Ignored;
                }
                let local = self.local(*position);
                if ctx.modifiers.ctrl {
                    let z = self.view.zoom * (1.0 + delta * 0.1).clamp(0.5, 1.5);
                    self.zoom_at(local, z);
                } else if self.page_fits() && delta_x.abs() < 0.01 {
                    if *delta < 0.0 {
                        self.handle.next_page(false);
                    } else if *delta > 0.0 {
                        self.handle.prev_page();
                    }
                } else {
                    self.view.ox += delta_x * 40.0;
                    self.view.oy += delta * 40.0;
                    self.clamp_view();
                }
                self.invalidate_raster();
                ctx.request_paint();
                EventResult::Handled
            }
            Event::FocusGained => {
                self.focused = true;
                EventResult::Handled
            }
            Event::FocusLost => {
                self.focused = false;
                EventResult::Handled
            }
            Event::KeyDown(key) if self.focused && self.editing.is_none() => {
                let m = ctx.modifiers;
                let h = self.handle.clone();
                let handled = match key {
                    Key::Z if m.ctrl && m.shift => {
                        h.redo();
                        true
                    }
                    Key::Z if m.ctrl => {
                        h.undo();
                        true
                    }
                    Key::Y if m.ctrl => {
                        h.redo();
                        true
                    }
                    Key::C if m.ctrl => {
                        h.copy_selection();
                        true
                    }
                    Key::X if m.ctrl => {
                        h.cut_selection();
                        true
                    }
                    Key::V if m.ctrl => {
                        h.paste(P2::new(20.0, 20.0));
                        true
                    }
                    Key::Delete | Key::Backspace => {
                        h.delete_selection();
                        true
                    }
                    Key::Escape => {
                        h.clear_selection();
                        true
                    }
                    Key::PageDown | Key::Right if !self.overlay => {
                        h.next_page(false);
                        true
                    }
                    Key::PageUp | Key::Left if !self.overlay => {
                        h.prev_page();
                        true
                    }
                    Key::P => {
                        h.set_tool(|t| t.tool = Tool::Pen);
                        true
                    }
                    Key::E => {
                        h.set_tool(|t| t.tool = Tool::Eraser);
                        true
                    }
                    Key::L => {
                        h.set_tool(|t| t.tool = Tool::Lasso);
                        true
                    }
                    _ => false,
                };
                if handled {
                    ctx.request_paint();
                    EventResult::Handled
                } else {
                    EventResult::Ignored
                }
            }
            _ => EventResult::Ignored,
        }
    }

    fn element_type_name(&self) -> &str {
        "ink-canvas"
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

impl InkElement {
    /// Подхватить сигналы ручки: страница, вписать, текст в правке.
    fn sync_signals(&mut self) {
        let fit = self.handle.fit_request.get_untracked();
        if fit != self.fit_seen {
            self.fit_seen = fit;
            self.fitted = None;
            if self.bounds.size.width > 1.0 {
                self.fit();
            }
        }
        let pid = self.page_info().0;
        if self.fitted.is_some_and(|(p, _)| p != pid) && self.bounds.size.width > 1.0 {
            // Новая страница: тот же масштаб, если страница того же размера
            // и вид был изменён пользователем, — проще вписать.
            self.fit();
            self.live = None;
        }
        let ed = self.handle.editing_text.get_untracked();
        if ed != self.editing {
            self.editing = ed;
            self.rebuild = true;
            self.dirty |= DirtyFlags::LAYOUT;
        }
    }
}
