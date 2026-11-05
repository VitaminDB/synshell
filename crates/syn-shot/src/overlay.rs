//! Оверлей вывода поверх застывшего кадра: затемнение вне выделения,
//! рамка с ручками, размер в пикселях, лупа у указателя. Сам принимает
//! мышь и переводит её в глобальные координаты для [`crate::sel`].

use std::any::Any;

use syngui::core::{Color, Point, Rect, Size};
use syngui::input::{Event, EventResult, MouseButton};
use syngui::layout::Constraints;
use syngui::mss::{ComputedStyle, MssFields};
use syngui::render::{Border, DisplayList};
use syngui::widget::context::EventContext;
use syngui::widget::{DirtyFlags, Element, ElementId, ElementTree, StyledElement, UpdateContext, Widget};

use crate::sel;
use crate::shot::R;

const DIM: Color = Color::rgba(0.0, 0.0, 0.0, 0.68);
const HANDLE: f32 = 9.0;
/// Лупа: клеток по стороне (нечётно — центр на пикселе под указателем) и
/// размер клетки.
const LOUPE_N: i64 = 15;
const LOUPE_CELL: f32 = 9.0;

/// Оверлей вывода `frame`; `rev` меняется при каждом изменении выделения —
/// элемент перерисовывается.
pub struct Overlay {
    pub frame: usize,
    pub rev: u64,
    classes: Vec<String>,
}

impl Overlay {
    pub fn new(frame: usize, rev: u64) -> Self {
        Self { frame, rev, classes: vec!["shot-overlay".into()] }
    }
}

impl Widget for Overlay {
    fn create_element(&self) -> Box<dyn Element> {
        let geo = sel::shot().frames[self.frame].geo;
        Box::new(OverlayElement {
            id: ElementId::new(),
            bounds: Rect::zero(),
            origin: (geo.x, geo.y),
            rev: self.rev,
            classes: self.classes.clone(),
            dirty: DirtyFlags::LAYOUT | DirtyFlags::RENDER,
            mss: MssFields::new(),
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
    fn mount(&self, _tree: &mut ElementTree, _parent: ElementId) {}
    fn widget_classes(&self) -> &[String] {
        &self.classes
    }
}

struct OverlayElement {
    id: ElementId,
    bounds: Rect,
    /// Логическое положение вывода: локальные точки + origin = глобальные.
    origin: (f64, f64),
    rev: u64,
    classes: Vec<String>,
    dirty: DirtyFlags,
    mss: MssFields,
}

impl OverlayElement {
    fn global(&self, p: Point) -> (f64, f64) {
        (p.x as f64 + self.origin.0, p.y as f64 + self.origin.1)
    }

    fn local(&self, r: &R) -> Rect {
        Rect::new(
            Point::new((r.x - self.origin.0) as f32, (r.y - self.origin.1) as f32),
            Size::new(r.w as f32, r.h as f32),
        )
    }

    fn accent(&self) -> Color {
        self.mss.accent_color.unwrap_or(Color::rgba(0.24, 0.55, 0.98, 1.0))
    }

    /// Затемнить всё, кроме `hole` (локальный прямоугольник).
    fn dim_around(&self, list: &mut DisplayList, hole: Option<Rect>) {
        let (w, h) = (self.bounds.size.width, self.bounds.size.height);
        let Some(hole) = hole else {
            list.push_rect(Rect::new(Point::zero(), Size::new(w, h)), DIM, [0.0; 4]);
            return;
        };
        let x0 = hole.origin.x.clamp(0.0, w);
        let y0 = hole.origin.y.clamp(0.0, h);
        let x1 = (hole.origin.x + hole.size.width).clamp(0.0, w);
        let y1 = (hole.origin.y + hole.size.height).clamp(0.0, h);
        let mut rect = |x: f32, y: f32, rw: f32, rh: f32| {
            if rw > 0.0 && rh > 0.0 {
                list.push_rect(Rect::new(Point::new(x, y), Size::new(rw, rh)), DIM, [0.0; 4]);
            }
        };
        rect(0.0, 0.0, w, y0);
        rect(0.0, y1, w, h - y1);
        rect(0.0, y0, x0, y1 - y0);
        rect(x1, y0, w - x1, y1 - y0);
    }

    fn frame_rect(&self, list: &mut DisplayList, r: Rect, width: f32, color: Color) {
        list.push_rect_bordered(
            Rect::new(Point::new(r.origin.x - width, r.origin.y - width), Size::new(r.size.width + 2.0 * width, r.size.height + 2.0 * width)),
            Color::rgba(0.0, 0.0, 0.0, 0.0),
            [0.0; 4],
            Border::new(width, color),
        );
    }

    fn handles(&self, list: &mut DisplayList, r: Rect) {
        let accent = self.accent();
        let (x0, y0) = (r.origin.x, r.origin.y);
        let (x1, y1) = (x0 + r.size.width, y0 + r.size.height);
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let mut pts = vec![(x0, y0), (x1, y0), (x0, y1), (x1, y1)];
        if r.size.width > 4.0 * HANDLE {
            pts.extend([(cx, y0), (cx, y1)]);
        }
        if r.size.height > 4.0 * HANDLE {
            pts.extend([(x0, cy), (x1, cy)]);
        }
        for (x, y) in pts {
            list.push_rect_bordered(
                Rect::new(Point::new(x - HANDLE / 2.0, y - HANDLE / 2.0), Size::new(HANDLE, HANDLE)),
                Color::rgba(1.0, 1.0, 1.0, 1.0),
                [HANDLE / 2.0; 4],
                Border::new(1.5, accent),
            );
        }
    }

    /// Подпись размера над рамкой (или внутри, если сверху нет места);
    /// у окна под указателем — ещё и его заголовок.
    fn size_label(&self, list: &mut DisplayList, global: &R, r: Rect, title: Option<&str>) {
        let (pw, ph) = sel::shot().pixel_size(global);
        let text = match title.map(str::trim).filter(|t| !t.is_empty()) {
            Some(t) => {
                let t: String = if t.chars().count() > 60 { t.chars().take(59).chain(['…']).collect() } else { t.to_string() };
                format!("{t}  ·  {pw} × {ph}")
            }
            None => format!("{pw} × {ph}"),
        };
        let w = text.chars().count() as f32 * 7.6 + 18.0;
        let h = 24.0;
        let x = r.origin.x.clamp(4.0, (self.bounds.size.width - w - 4.0).max(4.0));
        let y = if r.origin.y - h - 6.0 >= 0.0 { r.origin.y - h - 6.0 } else { r.origin.y + 6.0 };
        let pill = Rect::new(Point::new(x, y), Size::new(w, h));
        list.push_rect(pill, Color::rgba(0.07, 0.08, 0.10, 0.86), [6.0; 4]);
        list.push_text_centered(&text, Rect::new(Point::new(x, y + 4.0), Size::new(w, h - 4.0)), Color::rgba(1.0, 1.0, 1.0, 1.0), 12.5);
    }

    /// Лупа: пиксели вокруг указателя крупно, центр обведён, под ней —
    /// координаты пикселя.
    fn loupe(&self, list: &mut DisplayList, frame: usize, p: (f64, f64)) {
        let shot = sel::shot();
        let f = &shot.frames[frame];
        let (px, py) = f.pixel_pos(p);
        let side = LOUPE_N as f32 * LOUPE_CELL;
        let local = Point::new((p.0 - self.origin.0) as f32, (p.1 - self.origin.1) as f32);
        let (w, h) = (self.bounds.size.width, self.bounds.size.height);
        let label_h = 22.0;
        let mut x = local.x + 24.0;
        let mut y = local.y + 24.0;
        if x + side > w - 4.0 {
            x = local.x - 24.0 - side;
        }
        if y + side + label_h > h - 4.0 {
            y = local.y - 24.0 - side - label_h;
        }
        let box_r = Rect::new(Point::new(x, y), Size::new(side, side));
        list.push_shadow(box_r, Color::rgba(0.0, 0.0, 0.0, 0.45), 16.0, (0.0, 4.0), [8.0; 4]);
        list.push_clip_rounded(box_r, [8.0; 4]);
        let half = LOUPE_N / 2;
        for j in 0..LOUPE_N {
            for i in 0..LOUPE_N {
                let c = f.pixel(px + i - half, py + j - half).unwrap_or([0, 0, 0, 255]);
                list.push_rect(
                    Rect::new(Point::new(x + i as f32 * LOUPE_CELL, y + j as f32 * LOUPE_CELL), Size::new(LOUPE_CELL, LOUPE_CELL)),
                    Color::from_srgb(c[0], c[1], c[2], 1.0),
                    [0.0; 4],
                );
            }
        }
        let c = Rect::new(Point::new(x + half as f32 * LOUPE_CELL, y + half as f32 * LOUPE_CELL), Size::new(LOUPE_CELL, LOUPE_CELL));
        list.push_rect_bordered(c, Color::rgba(0.0, 0.0, 0.0, 0.0), [0.0; 4], Border::new(1.0, Color::rgba(1.0, 1.0, 1.0, 0.95)));
        self.frame_rect(list, Rect::new(Point::new(c.origin.x + 1.0, c.origin.y + 1.0), Size::new(LOUPE_CELL - 2.0, LOUPE_CELL - 2.0)), 1.0, Color::rgba(0.0, 0.0, 0.0, 0.8));
        list.pop_clip();
        list.push_rect_bordered(box_r, Color::rgba(0.0, 0.0, 0.0, 0.0), [8.0; 4], Border::new(1.0, Color::rgba(1.0, 1.0, 1.0, 0.55)));
        let text = format!("{px}, {py}");
        let lw = text.chars().count() as f32 * 7.4 + 16.0;
        let lr = Rect::new(Point::new(x + (side - lw) / 2.0, y + side + 4.0), Size::new(lw, label_h - 4.0));
        list.push_rect(lr, Color::rgba(0.07, 0.08, 0.10, 0.86), [5.0; 4]);
        list.push_text_centered(&text, Rect::new(Point::new(lr.origin.x, lr.origin.y + 2.0), lr.size), Color::rgba(1.0, 1.0, 1.0, 1.0), 11.5);
    }
}

impl Element for OverlayElement {
    fn update(&mut self, widget: &dyn Widget, _ctx: &mut UpdateContext) {
        if let Some(w) = widget.as_any().downcast_ref::<Overlay>() {
            if w.rev != self.rev {
                self.rev = w.rev;
                self.mark_dirty(DirtyFlags::RENDER);
            }
        }
    }

    fn layout(&mut self, c: Constraints) -> Size {
        let w = if c.max_width.is_finite() { c.max_width } else { 0.0 };
        let h = if c.max_height.is_finite() { c.max_height } else { 0.0 };
        self.bounds = Rect::new(self.bounds.origin, Size::new(w, h));
        self.bounds.size
    }

    fn build_display_list(&self, list: &mut DisplayList, _clip: Rect) {
        let accent = self.accent();
        match sel::current_rect() {
            Some(g) => {
                let r = self.local(&g);
                self.dim_around(list, Some(r));
                self.frame_rect(list, r, 1.5, accent);
                if !sel::is_creating() {
                    self.handles(list, r);
                }
                // Подпись — на выводе, где левый верхний угол рамки.
                let shot = sel::shot();
                if shot.nearest_frame((g.x, g.y)) == self.frame_index() {
                    self.size_label(list, &g, r, None);
                }
            }
            None => match sel::hover_candidate() {
                Some(c) => {
                    let r = self.local(&c);
                    self.dim_around(list, Some(r));
                    self.frame_rect(list, r, 2.0, accent.with_alpha(0.85));
                    let shot = sel::shot();
                    if shot.nearest_frame((c.x, c.y)) == self.frame_index() {
                        let title = shot.window_at(sel::with(|s| s.pointer)).map(|w| w.title.clone());
                        self.size_label(list, &c, r, title.as_deref());
                    }
                }
                None => self.dim_around(list, None),
            },
        }
        if sel::loupe_visible() {
            let p = sel::with(|s| s.pointer);
            if sel::shot().frame_index_at(p) == Some(self.frame_index()) {
                self.loupe(list, self.frame_index(), p);
            }
        }
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut EventContext) -> EventResult {
        match event {
            Event::MouseMove(pos) => {
                // «Уход» за экран syngui шлёт точкой (-1, -1) — не движение.
                if pos.x < 0.0 && pos.y < 0.0 && sel::with(|s| s.drag == sel::Drag::None) {
                    return EventResult::Ignored;
                }
                let g = self.global(*pos);
                sel::motion(g);
                ctx.set_cursor(sel::cursor_at(g));
                EventResult::Handled
            }
            Event::MouseDown { button: MouseButton::Left, position } => {
                sel::press(self.global(*position));
                EventResult::Handled
            }
            Event::DoubleClick { button: MouseButton::Left, position } => {
                sel::double_click(self.global(*position));
                EventResult::Handled
            }
            Event::MouseUp { button: MouseButton::Left, position } => {
                let g = self.global(*position);
                sel::release(g);
                ctx.set_cursor(sel::cursor_at(g));
                EventResult::Handled
            }
            Event::MouseDown { button: MouseButton::Right, .. } => {
                sel::secondary();
                EventResult::Handled
            }
            _ => EventResult::Ignored,
        }
    }

    fn children(&self) -> &[ElementId] {
        &[]
    }
    fn bounds(&self) -> Rect {
        self.bounds
    }
    fn set_position(&mut self, pos: Point) {
        self.bounds.origin = pos;
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
    fn id(&self) -> ElementId {
        self.id
    }
    fn set_id(&mut self, id: ElementId) {
        self.id = id;
    }
    fn mount(&mut self, _tree: &mut ElementTree) {}
    fn set_classes(&mut self, classes: Vec<String>) {
        self.classes = classes;
    }
    fn get_classes(&self) -> &[String] {
        &self.classes
    }
    fn element_type_name(&self) -> &str {
        "ShotOverlay"
    }
    fn reset_mss_styles(&mut self) {
        self.mss.reset();
    }
    fn mss(&self) -> Option<&MssFields> {
        Some(&self.mss)
    }
    fn apply_computed_style(&mut self, style: &ComputedStyle) {
        self.mss.apply(style);
        self.mark_dirty(DirtyFlags::RENDER);
    }
}

impl OverlayElement {
    fn frame_index(&self) -> usize {
        sel::shot().frames.iter().position(|f| (f.geo.x, f.geo.y) == self.origin).unwrap_or(0)
    }
}

impl StyledElement for OverlayElement {
    fn apply_style(&mut self, _style: &ComputedStyle) {}
    fn classes(&self) -> &[String] {
        &self.classes
    }
    fn set_classes(&mut self, classes: Vec<String>) {
        self.classes = classes;
    }
}
