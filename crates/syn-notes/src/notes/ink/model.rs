//! Модель рукописного документа: страницы → слои → элементы (штрихи,
//! фигуры, картинки, текст). Координаты — в единицах страницы (логические
//! px при 96 dpi: A4 = 794×1123); у бесконечного холста начало в (0, 0) и
//! границ нет.
//!
//! Правки идут через [`Op`] — их же хранит история отмены
//! ([`History`]): применение и откат симметричны, элементы находятся по id,
//! а не по индексу, поэтому история переживает перестановки.

use serde::{Deserialize, Serialize};

// ─────────────────────────── Основа ───────────────────────────

/// Цвет sRGB с альфой (как в файле и в палитре).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rgba(pub [u8; 4]);

impl Rgba {
    pub const BLACK: Rgba = Rgba([0, 0, 0, 255]);
    pub const WHITE: Rgba = Rgba([255, 255, 255, 255]);
    pub const TRANSPARENT: Rgba = Rgba([0, 0, 0, 0]);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Rgba([r, g, b, 255])
    }

    pub fn with_alpha(self, a: u8) -> Self {
        Rgba([self.0[0], self.0[1], self.0[2], a])
    }

    pub fn alpha(self) -> f32 {
        self.0[3] as f32 / 255.0
    }

    /// `#rrggbb` / `#rrggbbaa`.
    pub fn parse(s: &str) -> Option<Self> {
        let h = s.trim().trim_start_matches('#');
        let byte = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
        match h.len() {
            6 => Some(Rgba([byte(0)?, byte(2)?, byte(4)?, 255])),
            8 => Some(Rgba([byte(0)?, byte(2)?, byte(4)?, byte(6)?])),
            _ => None,
        }
    }

    pub fn to_hex(self) -> String {
        let [r, g, b, a] = self.0;
        if a == 255 {
            format!("#{r:02x}{g:02x}{b:02x}")
        } else {
            format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
        }
    }

    /// Смешать с белым: `t` = 0 — свой цвет, 1 — белый (тон карандаша).
    pub fn toward_white(self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let m = |c: u8| (c as f32 + (255.0 - c as f32) * t).round() as u8;
        Rgba([m(self.0[0]), m(self.0[1]), m(self.0[2]), self.0[3]])
    }

    /// Яркость 0..1 (для ч/б: какие цвета на E Ink сливаются).
    pub fn luma(self) -> f32 {
        (0.299 * self.0[0] as f32 + 0.587 * self.0[1] as f32 + 0.114 * self.0[2] as f32) / 255.0
    }
}

/// Точка в единицах страницы.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct P2 {
    pub x: f32,
    pub y: f32,
}

impl P2 {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
    pub fn dist(self, o: P2) -> f32 {
        ((self.x - o.x).powi(2) + (self.y - o.y).powi(2)).sqrt()
    }
    pub fn sub(self, o: P2) -> P2 {
        P2::new(self.x - o.x, self.y - o.y)
    }
    pub fn add(self, o: P2) -> P2 {
        P2::new(self.x + o.x, self.y + o.y)
    }
    pub fn scale(self, k: f32) -> P2 {
        P2::new(self.x * k, self.y * k)
    }
    pub fn len(self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
    pub fn lerp(self, o: P2, t: f32) -> P2 {
        P2::new(self.x + (o.x - self.x) * t, self.y + (o.y - self.y) * t)
    }
}

/// Ограничивающий прямоугольник.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BBox {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl BBox {
    pub const EMPTY: BBox = BBox { x0: f32::INFINITY, y0: f32::INFINITY, x1: f32::NEG_INFINITY, y1: f32::NEG_INFINITY };

    pub fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        Self { x0: x0.min(x1), y0: y0.min(y1), x1: x0.max(x1), y1: y0.max(y1) }
    }
    pub fn is_empty(&self) -> bool {
        self.x1 < self.x0 || self.y1 < self.y0
    }
    pub fn add(&mut self, p: P2) {
        self.x0 = self.x0.min(p.x);
        self.y0 = self.y0.min(p.y);
        self.x1 = self.x1.max(p.x);
        self.y1 = self.y1.max(p.y);
    }
    pub fn union(mut self, o: BBox) -> BBox {
        if o.is_empty() {
            return self;
        }
        self.x0 = self.x0.min(o.x0);
        self.y0 = self.y0.min(o.y0);
        self.x1 = self.x1.max(o.x1);
        self.y1 = self.y1.max(o.y1);
        self
    }
    pub fn inflate(self, d: f32) -> BBox {
        if self.is_empty() {
            return self;
        }
        BBox { x0: self.x0 - d, y0: self.y0 - d, x1: self.x1 + d, y1: self.y1 + d }
    }
    pub fn intersects(&self, o: &BBox) -> bool {
        !(self.is_empty() || o.is_empty() || o.x0 > self.x1 || o.x1 < self.x0 || o.y0 > self.y1 || o.y1 < self.y0)
    }
    pub fn contains(&self, p: P2) -> bool {
        p.x >= self.x0 && p.x <= self.x1 && p.y >= self.y0 && p.y <= self.y1
    }
    pub fn w(&self) -> f32 {
        (self.x1 - self.x0).max(0.0)
    }
    pub fn h(&self) -> f32 {
        (self.y1 - self.y0).max(0.0)
    }
    pub fn center(&self) -> P2 {
        P2::new((self.x0 + self.x1) / 2.0, (self.y0 + self.y1) / 2.0)
    }
    pub fn of_points(pts: impl IntoIterator<Item = P2>) -> BBox {
        let mut b = BBox::EMPTY;
        for p in pts {
            b.add(p);
        }
        b
    }
}

/// Аффинное преобразование `x' = a·x + c·y + e`, `y' = b·x + d·y + f`
/// (лассо: перенос, масштаб, поворот).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Affine {
    pub const IDENTITY: Affine = Affine { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    pub fn translate(dx: f32, dy: f32) -> Self {
        Affine { e: dx, f: dy, ..Self::IDENTITY }
    }
    /// Масштаб относительно точки.
    pub fn scale_about(c: P2, sx: f32, sy: f32) -> Self {
        Affine { a: sx, b: 0.0, c: 0.0, d: sy, e: c.x - sx * c.x, f: c.y - sy * c.y }
    }
    /// Поворот на `rad` относительно точки.
    pub fn rotate_about(c: P2, rad: f32) -> Self {
        let (s, co) = rad.sin_cos();
        Affine { a: co, b: s, c: -s, d: co, e: c.x - co * c.x + s * c.y, f: c.y - s * c.x - co * c.y }
    }
    pub fn apply(&self, p: P2) -> P2 {
        P2::new(self.a * p.x + self.c * p.y + self.e, self.b * p.x + self.d * p.y + self.f)
    }
    /// Сначала `self`, потом `o`.
    pub fn then(&self, o: &Affine) -> Affine {
        Affine {
            a: o.a * self.a + o.c * self.b,
            b: o.b * self.a + o.d * self.b,
            c: o.a * self.c + o.c * self.d,
            d: o.b * self.c + o.d * self.d,
            e: o.a * self.e + o.c * self.f + o.e,
            f: o.b * self.e + o.d * self.f + o.f,
        }
    }
    /// Средний линейный масштаб (толщина линий при масштабе лассо).
    pub fn mean_scale(&self) -> f32 {
        ((self.a * self.d - self.b * self.c).abs()).sqrt()
    }
    /// Угол поворота.
    pub fn rotation(&self) -> f32 {
        self.b.atan2(self.a)
    }
}

// ─────────────────────────── Страница ───────────────────────────

/// Размер страницы.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageSize {
    #[default]
    A4,
    A5,
    Letter,
    /// Своя (например, пропорция экрана), единицы страницы.
    Custom { w: f32, h: f32 },
    /// Бесконечный холст («Рисунок»).
    Infinite,
}

impl PageSize {
    pub const PRESETS: [PageSize; 3] = [PageSize::A4, PageSize::A5, PageSize::Letter];

    /// Ширина и высота; `None` у бесконечного холста.
    pub fn dims(&self) -> Option<(f32, f32)> {
        match *self {
            PageSize::A4 => Some((794.0, 1123.0)),
            PageSize::A5 => Some((559.0, 794.0)),
            PageSize::Letter => Some((816.0, 1056.0)),
            PageSize::Custom { w, h } => Some((w.max(16.0), h.max(16.0))),
            PageSize::Infinite => None,
        }
    }

    pub fn is_infinite(&self) -> bool {
        matches!(self, PageSize::Infinite)
    }

    pub fn key(&self) -> &'static str {
        match self {
            PageSize::A4 => "a4",
            PageSize::A5 => "a5",
            PageSize::Letter => "letter",
            PageSize::Custom { .. } => "custom",
            PageSize::Infinite => "infinite",
        }
    }
}

/// Фон-шаблон страницы.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Template {
    #[default]
    Blank,
    /// Линейка с полем слева.
    Lined { step: f32 },
    Grid { step: f32 },
    Dots { step: f32 },
    /// Корнелл: поле вопросов слева, итог внизу, линейка.
    Cornell { step: f32 },
    /// Нотный стан: группы по 5 линий.
    Music { step: f32 },
    /// Своя картинка-фон (байты картинки — в [`InkDoc::assets`]).
    Image { asset: String },
}

impl Template {
    pub fn key(&self) -> &'static str {
        match self {
            Template::Blank => "blank",
            Template::Lined { .. } => "lined",
            Template::Grid { .. } => "grid",
            Template::Dots { .. } => "dots",
            Template::Cornell { .. } => "cornell",
            Template::Music { .. } => "music",
            Template::Image { .. } => "image",
        }
    }

    /// Шаблоны для выбора (шаг — умолчание).
    pub fn presets() -> Vec<Template> {
        vec![
            Template::Blank,
            Template::Lined { step: 32.0 },
            Template::Grid { step: 24.0 },
            Template::Dots { step: 24.0 },
            Template::Cornell { step: 32.0 },
            Template::Music { step: 8.0 },
        ]
    }
}

/// Слой страницы.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub id: u64,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    pub opacity: f32,
    pub items: Vec<Item>,
}

impl Layer {
    pub fn new(id: u64, name: impl Into<String>) -> Self {
        Self { id, name: name.into(), visible: true, locked: false, opacity: 1.0, items: Vec::new() }
    }

    pub fn find(&self, id: u64) -> Option<usize> {
        self.items.iter().position(|i| i.id() == id)
    }

    pub fn bbox(&self) -> BBox {
        self.items.iter().fold(BBox::EMPTY, |b, i| b.union(i.bbox()))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InkPage {
    pub id: u64,
    pub size: PageSize,
    pub template: Template,
    /// Цвет бумаги; `None` — белая (в оверлее — прозрачная).
    #[serde(default)]
    pub paper: Option<Rgba>,
    pub layers: Vec<Layer>,
    /// Слой, в который рисуем.
    pub active_layer: usize,
}

impl InkPage {
    pub fn dims(&self) -> Option<(f32, f32)> {
        self.size.dims()
    }

    pub fn layer_index(&self, id: u64) -> Option<usize> {
        self.layers.iter().position(|l| l.id == id)
    }

    pub fn active(&self) -> usize {
        self.active_layer.min(self.layers.len().saturating_sub(1))
    }

    /// Все элементы видимых слоёв (для экспорта/поиска).
    pub fn bbox(&self) -> BBox {
        self.layers.iter().filter(|l| l.visible).fold(BBox::EMPTY, |b, l| b.union(l.bbox()))
    }

    pub fn is_blank(&self) -> bool {
        self.layers.iter().all(|l| l.items.is_empty())
    }
}

/// Назначение документа (влияет на интерфейс, не на формат).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocKind {
    /// Блокнот: страницы, листание.
    #[default]
    Notebook,
    /// Рисунок: бесконечный холст или лист, слои, кисти, заливка.
    Drawing,
    /// Пометки поверх чужого содержимого (страницы книги).
    Overlay,
}

/// Картинка, на которую ссылаются элементы (вставки, фон).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    pub key: String,
    /// Закодированные байты (PNG/JPEG).
    #[serde(with = "serde_bytes_vec")]
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InkDoc {
    pub kind: DocKind,
    /// Размер и шаблон новых страниц.
    pub page_size: PageSize,
    pub template: Template,
    pub pages: Vec<InkPage>,
    #[serde(default)]
    pub assets: Vec<Asset>,
    /// Следующий id (элементы, слои, страницы — общий счётчик).
    pub next_id: u64,
}

impl Default for InkDoc {
    fn default() -> Self {
        InkDoc::new(DocKind::Notebook, PageSize::A4, Template::Lined { step: 32.0 })
    }
}

impl InkDoc {
    /// Документ с одной пустой страницей.
    pub fn new(kind: DocKind, page_size: PageSize, template: Template) -> Self {
        let mut d = InkDoc { kind, page_size, template, pages: Vec::new(), assets: Vec::new(), next_id: 1 };
        let p = d.blank_page();
        d.pages.push(p);
        d
    }

    /// Рукописный блокнот (A4, линейка).
    pub fn notebook() -> Self {
        Self::new(DocKind::Notebook, PageSize::A4, Template::Lined { step: 32.0 })
    }

    /// Рисунок на бесконечном холсте.
    pub fn drawing() -> Self {
        Self::new(DocKind::Drawing, PageSize::Infinite, Template::Blank)
    }

    /// Пометки поверх `pages` страниц размером `w`×`h` (книга).
    pub fn overlay(pages: usize, w: f32, h: f32) -> Self {
        let mut d = Self::new(DocKind::Overlay, PageSize::Custom { w, h }, Template::Blank);
        for _ in 1..pages.max(1) {
            let p = d.blank_page();
            d.pages.push(p);
        }
        d
    }

    pub fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Новая страница по умолчаниям документа (в документ не вставляется).
    pub fn blank_page(&mut self) -> InkPage {
        let id = self.alloc_id();
        let lid = self.alloc_id();
        InkPage {
            id,
            size: self.page_size,
            template: self.template.clone(),
            paper: None,
            layers: vec![Layer::new(lid, default_layer_name(1))],
            active_layer: 0,
        }
    }

    pub fn page_index(&self, id: u64) -> Option<usize> {
        self.pages.iter().position(|p| p.id == id)
    }

    pub fn asset(&self, key: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.key == key)
    }

    /// Положить картинку (ключ — по содержимому, повтор не дублируется).
    pub fn add_asset(&mut self, bytes: Vec<u8>) -> String {
        let key = asset_key(&bytes);
        if self.asset(&key).is_none() {
            self.assets.push(Asset { key: key.clone(), bytes });
        }
        key
    }

    /// Убрать картинки, на которые никто не ссылается.
    pub fn gc_assets(&mut self) {
        let mut used = std::collections::HashSet::new();
        for p in &self.pages {
            if let Template::Image { asset } = &p.template {
                used.insert(asset.clone());
            }
            for l in &p.layers {
                for i in &l.items {
                    if let Item::Image(im) = i {
                        used.insert(im.asset.clone());
                    }
                }
            }
        }
        if let Template::Image { asset } = &self.template {
            used.insert(asset.clone());
        }
        self.assets.retain(|a| used.contains(&a.key));
    }

    /// Применить правку (без истории).
    pub fn apply(&mut self, op: &Op) {
        op.apply(self);
    }
}

pub fn default_layer_name(n: usize) -> String {
    format!("{} {n}", syngui::i18n::tr("notes.ink.layer"))
}

fn asset_key(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let h = Sha256::digest(bytes);
    h.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

// ─────────────────────────── Элементы ───────────────────────────

/// Кисть штриха — как ширина и тон зависят от давления, скорости, наклона.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Brush {
    /// Перьевая ручка: ширина от давления и скорости.
    #[default]
    Fountain,
    /// Шариковая: почти ровная.
    Ballpoint,
    /// Карандаш: тон от давления.
    Pencil,
    /// Маркер-выделитель: полупрозрачный, под остальными штрихами слоя.
    Highlighter,
    /// Кисть: сильная зависимость от давления, сужение на концах.
    Brush,
    /// Каллиграфическое перо: ширина от направления (и наклона пера).
    Calligraphy,
}

impl Brush {
    pub const ALL: [Brush; 6] = [Brush::Fountain, Brush::Ballpoint, Brush::Pencil, Brush::Highlighter, Brush::Brush, Brush::Calligraphy];

    pub fn key(&self) -> &'static str {
        match self {
            Brush::Fountain => "fountain",
            Brush::Ballpoint => "ballpoint",
            Brush::Pencil => "pencil",
            Brush::Highlighter => "highlighter",
            Brush::Brush => "brush",
            Brush::Calligraphy => "calligraphy",
        }
    }

    /// Ступени толщины (5, как у Onyx), единицы страницы.
    pub fn widths(&self) -> [f32; 5] {
        match self {
            Brush::Highlighter => [8.0, 12.0, 18.0, 26.0, 36.0],
            Brush::Brush => [3.0, 6.0, 10.0, 16.0, 26.0],
            Brush::Calligraphy => [3.0, 5.0, 8.0, 12.0, 18.0],
            _ => [1.0, 1.8, 2.8, 4.2, 6.5],
        }
    }
}

/// Точка штриха: положение, давление 0..1, наклон пера (градусы по X/Y),
/// время от начала штриха (мс).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InkPoint {
    pub x: f32,
    pub y: f32,
    pub p: f32,
    pub tx: f32,
    pub ty: f32,
    pub t: u32,
}

impl InkPoint {
    pub fn new(x: f32, y: f32, p: f32) -> Self {
        Self { x, y, p, tx: 0.0, ty: 0.0, t: 0 }
    }
    pub fn pos(&self) -> P2 {
        P2::new(self.x, self.y)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub id: u64,
    pub brush: Brush,
    pub color: Rgba,
    /// Базовая ширина (единицы страницы).
    pub width: f32,
    #[serde(with = "crate::notes::ink::codec::packed_points")]
    pub points: Vec<InkPoint>,
}

impl Stroke {
    pub fn bbox(&self) -> BBox {
        BBox::of_points(self.points.iter().map(|p| p.pos())).inflate(self.width * 2.0 + 1.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShapeKind {
    Line,
    Arrow,
    /// Прямоугольник: 4 угла (после поворота — параллелограмм).
    Rect,
    /// Эллипс, вписанный в параллелограмм из 4 точек — переживает любое
    /// аффинное преобразование.
    Ellipse,
    Triangle,
    /// Многоугольник по точкам (замкнутый).
    Polygon,
    /// Ломаная (незамкнутая).
    Polyline,
}

impl ShapeKind {
    pub fn key(&self) -> &'static str {
        match self {
            ShapeKind::Line => "line",
            ShapeKind::Arrow => "arrow",
            ShapeKind::Rect => "rect",
            ShapeKind::Ellipse => "ellipse",
            ShapeKind::Triangle => "triangle",
            ShapeKind::Polygon => "polygon",
            ShapeKind::Polyline => "polyline",
        }
    }
    pub fn closed(&self) -> bool {
        matches!(self, ShapeKind::Rect | ShapeKind::Ellipse | ShapeKind::Triangle | ShapeKind::Polygon)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shape {
    pub id: u64,
    pub kind: ShapeKind,
    pub pts: Vec<P2>,
    pub color: Rgba,
    pub width: f32,
    #[serde(default)]
    pub fill: Option<Rgba>,
}

impl Shape {
    /// Прямоугольник по двум углам.
    pub fn rect(id: u64, a: P2, b: P2, color: Rgba, width: f32) -> Self {
        let pts = vec![a, P2::new(b.x, a.y), b, P2::new(a.x, b.y)];
        Shape { id, kind: ShapeKind::Rect, pts, color, width, fill: None }
    }

    /// Эллипс, вписанный в прямоугольник по двум углам.
    pub fn ellipse(id: u64, a: P2, b: P2, color: Rgba, width: f32) -> Self {
        Shape { kind: ShapeKind::Ellipse, ..Shape::rect(id, a, b, color, width) }
    }

    pub fn line(id: u64, a: P2, b: P2, color: Rgba, width: f32, arrow: bool) -> Self {
        Shape { id, kind: if arrow { ShapeKind::Arrow } else { ShapeKind::Line }, pts: vec![a, b], color, width, fill: None }
    }

    /// Контур фигуры ломаной (эллипс — 64 точки).
    pub fn outline(&self) -> Vec<P2> {
        match self.kind {
            ShapeKind::Ellipse if self.pts.len() == 4 => {
                // Параллелограмм p0..p3: центр + полуоси u, v.
                let c = self.pts[0].lerp(self.pts[2], 0.5);
                let u = self.pts[1].lerp(self.pts[2], 0.5).sub(c);
                let v = self.pts[3].lerp(self.pts[2], 0.5).sub(c);
                (0..64)
                    .map(|i| {
                        let a = i as f32 / 64.0 * std::f32::consts::TAU;
                        c.add(u.scale(a.cos())).add(v.scale(a.sin()))
                    })
                    .collect()
            }
            _ => self.pts.clone(),
        }
    }

    pub fn bbox(&self) -> BBox {
        BBox::of_points(self.outline()).inflate(self.width + 6.0)
    }
}

/// Вставленная картинка: прямоугольник `w`×`h` в точке `(x, y)`, повёрнутый
/// на `angle` вокруг своего центра.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageItem {
    pub id: u64,
    pub asset: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    #[serde(default)]
    pub angle: f32,
}

impl ImageItem {
    pub fn corners(&self) -> [P2; 4] {
        rotated_rect(self.x, self.y, self.w, self.h, self.angle)
    }
    pub fn bbox(&self) -> BBox {
        BBox::of_points(self.corners())
    }
}

/// Текстовый блок (печатный текст на рукописной странице).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextItem {
    pub id: u64,
    pub text: String,
    pub x: f32,
    pub y: f32,
    /// Ширина блока (перенос строк).
    pub w: f32,
    pub size: f32,
    pub color: Rgba,
    #[serde(default)]
    pub bold: bool,
}

impl TextItem {
    /// Оценка высоты без измерителя текста (строки × интервал).
    pub fn approx_height(&self) -> f32 {
        let per_line = (self.w / (self.size * 0.55)).max(1.0) as usize;
        let lines: usize = self.text.split('\n').map(|l| l.chars().count().div_ceil(per_line).max(1)).sum();
        lines.max(1) as f32 * self.size * 1.35
    }
    pub fn bbox(&self) -> BBox {
        BBox::new(self.x, self.y, self.x + self.w, self.y + self.approx_height())
    }
}

pub(crate) fn rotated_rect(x: f32, y: f32, w: f32, h: f32, angle: f32) -> [P2; 4] {
    let c = P2::new(x + w / 2.0, y + h / 2.0);
    let r = Affine::rotate_about(c, angle);
    [r.apply(P2::new(x, y)), r.apply(P2::new(x + w, y)), r.apply(P2::new(x + w, y + h)), r.apply(P2::new(x, y + h))]
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Item {
    Stroke(Stroke),
    Shape(Shape),
    Image(ImageItem),
    Text(TextItem),
}

impl Item {
    pub fn id(&self) -> u64 {
        match self {
            Item::Stroke(s) => s.id,
            Item::Shape(s) => s.id,
            Item::Image(i) => i.id,
            Item::Text(t) => t.id,
        }
    }

    pub fn set_id(&mut self, id: u64) {
        match self {
            Item::Stroke(s) => s.id = id,
            Item::Shape(s) => s.id = id,
            Item::Image(i) => i.id = id,
            Item::Text(t) => t.id = id,
        }
    }

    pub fn bbox(&self) -> BBox {
        match self {
            Item::Stroke(s) => s.bbox(),
            Item::Shape(s) => s.bbox(),
            Item::Image(i) => i.bbox(),
            Item::Text(t) => t.bbox(),
        }
    }

    pub fn color(&self) -> Option<Rgba> {
        match self {
            Item::Stroke(s) => Some(s.color),
            Item::Shape(s) => Some(s.color),
            Item::Text(t) => Some(t.color),
            Item::Image(_) => None,
        }
    }

    /// Сменить цвет (сохраняя прозрачность выделителя).
    pub fn set_color(&mut self, c: Rgba) {
        match self {
            Item::Stroke(s) => {
                let a = s.color.0[3];
                s.color = c.with_alpha(a);
            }
            Item::Shape(s) => s.color = c,
            Item::Text(t) => t.color = c,
            Item::Image(_) => {}
        }
    }

    /// Сменить толщину линии (у текста — кегль не трогаем).
    pub fn set_width(&mut self, w: f32) {
        match self {
            Item::Stroke(s) => s.width = w,
            Item::Shape(s) => s.width = w,
            _ => {}
        }
    }

    /// Применить аффинное преобразование.
    pub fn transform(&mut self, m: &Affine) {
        let k = m.mean_scale();
        match self {
            Item::Stroke(s) => {
                for p in &mut s.points {
                    let q = m.apply(p.pos());
                    p.x = q.x;
                    p.y = q.y;
                }
                s.width *= k;
            }
            Item::Shape(s) => {
                for p in &mut s.pts {
                    *p = m.apply(*p);
                }
                s.width *= k;
            }
            Item::Image(i) => {
                let c = m.apply(P2::new(i.x + i.w / 2.0, i.y + i.h / 2.0));
                // Масштаб по осям картинки.
                let (s, co) = i.angle.sin_cos();
                let ux = m.apply(P2::new(co, s)).sub(m.apply(P2::new(0.0, 0.0))).len();
                let vy = m.apply(P2::new(-s, co)).sub(m.apply(P2::new(0.0, 0.0))).len();
                i.w *= ux;
                i.h *= vy;
                i.x = c.x - i.w / 2.0;
                i.y = c.y - i.h / 2.0;
                i.angle += m.rotation();
            }
            Item::Text(t) => {
                let o = m.apply(P2::new(t.x, t.y));
                t.x = o.x;
                t.y = o.y;
                t.w *= k;
                t.size = (t.size * k).max(4.0);
            }
        }
    }
}

// ─────────────────────────── Правки и история ───────────────────────────

/// Правка документа. Применение и откат симметричны.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// Замена элементов слоя: `removed` уходят (по id), `added` встают на
    /// свои индексы (индексы — в итоговом списке, по возрастанию).
    Items { page: u64, layer: u64, removed: Vec<(usize, Item)>, added: Vec<(usize, Item)> },
    /// Замена страницы целиком (слои, шаблон, бумага).
    Page { before: Box<InkPage>, after: Box<InkPage> },
    /// Вставка/удаление/перестановка страниц: весь список страниц.
    Pages { before: Vec<InkPage>, after: Vec<InkPage> },
    Batch(Vec<Op>),
}

impl Op {
    /// Обратная правка.
    pub fn inverse(&self) -> Op {
        match self {
            Op::Items { page, layer, removed, added } => {
                Op::Items { page: *page, layer: *layer, removed: added.clone(), added: removed.clone() }
            }
            Op::Page { before, after } => Op::Page { before: after.clone(), after: before.clone() },
            Op::Pages { before, after } => Op::Pages { before: after.clone(), after: before.clone() },
            Op::Batch(v) => Op::Batch(v.iter().rev().map(Op::inverse).collect()),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Op::Items { removed, added, .. } => removed.is_empty() && added.is_empty(),
            Op::Batch(v) => v.iter().all(Op::is_empty),
            Op::Page { before, after } => before == after,
            Op::Pages { before, after } => before == after,
        }
    }

    pub fn apply(&self, doc: &mut InkDoc) {
        match self {
            Op::Items { page, layer, removed, added } => {
                let Some(pi) = doc.page_index(*page) else { return };
                let pg = &mut doc.pages[pi];
                let Some(li) = pg.layer_index(*layer) else { return };
                let items = &mut pg.layers[li].items;
                if !removed.is_empty() {
                    let ids: std::collections::HashSet<u64> = removed.iter().map(|(_, i)| i.id()).collect();
                    items.retain(|i| !ids.contains(&i.id()));
                }
                let mut add: Vec<&(usize, Item)> = added.iter().collect();
                add.sort_by_key(|(i, _)| *i);
                for (idx, it) in add {
                    let at = (*idx).min(items.len());
                    items.insert(at, it.clone());
                }
            }
            Op::Page { after, .. } => {
                if let Some(pi) = doc.page_index(after.id) {
                    doc.pages[pi] = (**after).clone();
                }
            }
            Op::Pages { after, .. } => doc.pages = after.clone(),
            Op::Batch(v) => {
                for o in v {
                    o.apply(doc);
                }
            }
        }
    }

    /// Страницы, которых коснулась правка (для перерисовки).
    pub fn pages(&self, out: &mut Vec<u64>) {
        match self {
            Op::Items { page, .. } => out.push(*page),
            Op::Page { after, .. } => out.push(after.id),
            Op::Pages { after, before } => {
                out.extend(after.iter().map(|p| p.id));
                out.extend(before.iter().map(|p| p.id));
            }
            Op::Batch(v) => v.iter().for_each(|o| o.pages(out)),
        }
    }
}

/// История отмены: стек правок и стек повторов.
#[derive(Clone, Debug, Default)]
pub struct History {
    undo: Vec<Op>,
    redo: Vec<Op>,
    pub limit: usize,
}

impl History {
    pub fn new() -> Self {
        Self { undo: Vec::new(), redo: Vec::new(), limit: 500 }
    }

    /// Применить правку и запомнить.
    pub fn commit(&mut self, doc: &mut InkDoc, op: Op) {
        if op.is_empty() {
            return;
        }
        op.apply(doc);
        self.record(op);
    }

    /// Запомнить уже применённую правку.
    pub fn record(&mut self, op: Op) {
        if op.is_empty() {
            return;
        }
        self.undo.push(op);
        self.redo.clear();
        let limit = if self.limit == 0 { 500 } else { self.limit };
        if self.undo.len() > limit {
            let extra = self.undo.len() - limit;
            self.undo.drain(..extra);
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Отменить; возвращает отменённую правку (её страницы — перерисовать).
    pub fn undo(&mut self, doc: &mut InkDoc) -> Option<Op> {
        let op = self.undo.pop()?;
        op.inverse().apply(doc);
        self.redo.push(op.clone());
        Some(op)
    }

    pub fn redo(&mut self, doc: &mut InkDoc) -> Option<Op> {
        let op = self.redo.pop()?;
        op.apply(doc);
        self.undo.push(op.clone());
        Some(op)
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

/// Байты в serde как один блок (postcard пишет их без накладных расходов).
mod serde_bytes_vec {
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(v)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Vec<u8>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("bytes")
            }
            fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<Vec<u8>, E> {
                Ok(v.to_vec())
            }
            fn visit_byte_buf<E: serde::de::Error>(self, v: Vec<u8>) -> Result<Vec<u8>, E> {
                Ok(v)
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut a: A) -> Result<Vec<u8>, A::Error> {
                let mut out = Vec::new();
                while let Some(b) = a.next_element::<u8>()? {
                    out.push(b);
                }
                Ok(out)
            }
        }
        d.deserialize_bytes(V)
    }
}
