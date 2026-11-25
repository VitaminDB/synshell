//! Документ проекта: настройки, медиатека, дорожки и клипы. Время — секунды
//! (f64) на шкале проекта, команды выравнивают его по кадрам.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub type Id = u64;

pub const FORMAT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Project {
    pub version: u32,
    pub name: String,
    pub settings: Settings,
    #[serde(default)]
    pub media: Vec<Media>,
    #[serde(default)]
    pub tracks: Vec<Track>,
    #[serde(default = "first_id")]
    pub next_id: Id,
}

fn first_id() -> Id {
    1
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Settings {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    #[serde(default = "default_rate")]
    pub sample_rate: u32,
    /// Цвет фона кадра (`#rrggbb` или `transparent`).
    #[serde(default = "default_bg")]
    pub bg: String,
}

fn default_rate() -> u32 {
    48_000
}

fn default_bg() -> String {
    "#000000".into()
}

impl Default for Settings {
    fn default() -> Self {
        Self { width: 1920, height: 1080, fps: 30.0, sample_rate: 48_000, bg: default_bg() }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Video,
    Audio,
    Image,
    /// Анимация Lottie (.json / .lottie): кадры рисует `syngui::lottie`,
    /// клип длиннее анимации проигрывает её по кругу.
    Lottie,
}

impl MediaKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MediaKind::Video => "video",
            MediaKind::Audio => "audio",
            MediaKind::Image => "image",
            MediaKind::Lottie => "lottie",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Media {
    pub id: Id,
    pub path: PathBuf,
    pub kind: MediaKind,
    /// Длительность, с (у картинки — 0: длина клипа задаётся на таймлайне).
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub fps: f64,
    #[serde(default)]
    pub has_audio: bool,
    /// Расшифровка речи (media.transcribe).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript: Option<Transcript>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Transcript {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default)]
    pub words: Vec<TranscriptWord>,
}

/// Слово расшифровки: время источника, с.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TranscriptWord {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

impl Media {
    /// Предел источника для обрезки: картинка и Lottie (по кругу) — без предела.
    pub fn src_limit(&self) -> Option<f64> {
        (self.duration > 0.0 && !matches!(self.kind, MediaKind::Image | MediaKind::Lottie)).then_some(self.duration)
    }

    pub fn name(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| self.path.display().to_string())
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TrackKind {
    /// Видео, картинки, титры (слои: верхняя дорожка — поверх).
    Video,
    Audio,
}

impl TrackKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TrackKind::Video => "video",
            TrackKind::Audio => "audio",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Track {
    pub id: Id,
    pub kind: TrackKind,
    pub name: String,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub hidden: bool,
    /// Клипы по возрастанию `start`, без наложений.
    #[serde(default)]
    pub clips: Vec<Clip>,
}

/// Виды векторных фигур: (код, подпись).
pub const SHAPE_KINDS: &[(&str, &str)] = &[
    ("rect", "Прямоугольник"),
    ("ellipse", "Круг / овал"),
    ("triangle", "Треугольник"),
    ("star", "Звезда"),
    ("line", "Линия"),
    ("arrow", "Стрелка"),
];

/// Режимы наложения слоя: (код, подпись).
pub const BLEND_MODES: &[(&str, &str)] = &[
    ("normal", "Обычный"),
    ("multiply", "Умножение"),
    ("screen", "Экран"),
    ("overlay", "Перекрытие"),
    ("add", "Сложение"),
    ("darken", "Затемнение"),
    ("lighten", "Осветление"),
    ("soft_light", "Мягкий свет"),
    ("hard_light", "Жёсткий свет"),
    ("difference", "Разница"),
];

pub fn shape_title(kind: &str) -> &'static str {
    SHAPE_KINDS.iter().find(|(k, _)| *k == kind).map(|(_, l)| *l).unwrap_or("Фигура")
}

/// Откуда берётся содержимое клипа.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Source {
    Media { media: Id },
    /// Титр: текст и стиль (позже — блок из шаблона).
    Title {
        text: String,
        #[serde(default, skip_serializing_if = "Map::is_empty")]
        style: Map<String, Value>,
    },
    /// Заливка цветом (`#rrggbb`).
    Color { color: String },
    /// Векторная фигура (`rect`, `ellipse`, `triangle`, `star`, `line`, `arrow`) и стиль.
    Shape {
        kind: String,
        #[serde(default, skip_serializing_if = "Map::is_empty")]
        style: Map<String, Value>,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Clip {
    pub id: Id,
    pub source: Source,
    /// Начало на таймлайне, с.
    pub start: f64,
    /// Длительность на таймлайне, с.
    pub duration: f64,
    /// Смещение в источнике, с (откуда в файле начинается клип).
    #[serde(default)]
    pub offset: f64,
    #[serde(default = "one")]
    pub speed: f64,
    #[serde(default = "one")]
    pub volume: f64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,
    /// Переход в начале клипа от предыдущего на дорожке (по центру склейки).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition: Option<Transition>,
    /// Ключевые кадры: путь параметра (`e<эффект>.<параметр>`, `style.<поле>`
    /// титра или фигуры) → ключи по времени клипа.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub keys: std::collections::BTreeMap<String, Vec<Key>>,
    /// Режим наложения слоя (`normal`, `multiply`, `screen`, `overlay`, `add`, …).
    #[serde(default, skip_serializing_if = "is_normal_blend")]
    pub blend: String,
    /// Родительский клип: потомок наследует его сдвиг, масштаб, поворот и прозрачность.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<Id>,
    /// Связанный клип (видео ↔ его звук на аудиодорожке): правки идут синхронно, пока связь не разорвана.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<Id>,
    /// Граф кадра (D1): эффекты изображения нодами с ветвлением и слиянием;
    /// при графе `color/blur/sharpen/vignette/lut/upscale` берутся только из него.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph: Option<crate::framegraph::FrameGraph>,
}

fn is_normal_blend(s: &String) -> bool {
    s.is_empty() || s == "normal"
}

fn one() -> f64 {
    1.0
}

impl Clip {
    pub fn end(&self) -> f64 {
        self.start + self.duration
    }
    pub fn media(&self) -> Option<Id> {
        match self.source {
            Source::Media { media } => Some(media),
            _ => None,
        }
    }

    /// Текущее значение параметра по пути (без ключей).
    pub fn base(&self, path: &str) -> Option<Value> {
        let (head, name) = path.split_once('.')?;
        if head == "style" {
            return match &self.source {
                Source::Title { style, .. } | Source::Shape { style, .. } => style.get(name).cloned(),
                _ => None,
            };
        }
        let id: Id = head.strip_prefix('e')?.parse().ok()?;
        self.effects.iter().find(|e| e.id == id)?.params.get(name).cloned()
    }

    /// Значение параметра в момент `lt` (время клипа): по ключам, иначе базовое.
    pub fn value_at(&self, path: &str, lt: f64) -> Option<Value> {
        match self.keys.get(path) {
            Some(k) if !k.is_empty() => Some(interp(k, lt)),
            _ => self.base(path),
        }
    }

    /// Новые id эффектов (копия, разрез): пути ключей переименовываются вместе с ними.
    pub fn renew_effect_ids(&mut self, mut alloc: impl FnMut() -> Id) {
        let mut map = std::collections::HashMap::new();
        for e in &mut self.effects {
            let n = alloc();
            map.insert(e.id, n);
            e.id = n;
        }
        let keys = std::mem::take(&mut self.keys);
        for (path, k) in keys {
            let renamed = path.split_once('.').and_then(|(h, name)| h.strip_prefix('e').and_then(|s| s.parse::<Id>().ok()).map(|old| (old, name))).map(|(old, name)| match map.get(&old) {
                Some(n) => Some(format!("e{n}.{name}")),
                None => None,
            });
            match renamed {
                Some(Some(p)) => {
                    self.keys.insert(p, k);
                }
                Some(None) => {} // ключи эффекта, которого больше нет
                None => {
                    self.keys.insert(path, k);
                }
            }
        }
    }

    /// Сдвинуть ключи на `dt` по времени клипа (правая часть разреза).
    pub fn shift_keys(&mut self, dt: f64) {
        for k in self.keys.values_mut() {
            for key in k.iter_mut() {
                key.t += dt;
            }
        }
    }

    /// Клип в момент `lt`: параметры с ключами заменены значениями на этот
    /// момент (композитор читает обычные параметры и о ключах не знает).
    pub fn at(&self, lt: f64) -> std::borrow::Cow<'_, Clip> {
        if self.keys.is_empty() {
            return std::borrow::Cow::Borrowed(self);
        }
        let mut c = self.clone();
        for (path, keys) in &self.keys {
            if keys.is_empty() {
                continue;
            }
            let v = interp(keys, lt);
            let Some((head, name)) = path.split_once('.') else { continue };
            if head == "style" {
                match &mut c.source {
                    Source::Title { style, .. } | Source::Shape { style, .. } => {
                        style.insert(name.to_string(), v);
                    }
                    _ => {}
                }
            } else if let Some(id) = head.strip_prefix('e').and_then(|s| s.parse::<Id>().ok()) {
                if let Some(e) = c.effects.iter_mut().find(|e| e.id == id) {
                    e.params.insert(name.to_string(), v);
                }
            }
        }
        std::borrow::Cow::Owned(c)
    }
}

/// Ключевой кадр: время клипа, значение (число или цвет `#rrggbb`) и
/// сглаживание участка до следующего ключа.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Key {
    pub t: f64,
    pub v: Value,
    /// linear, ease (плавно), in (разгон), out (торможение), hold (скачком).
    #[serde(default = "linear", skip_serializing_if = "is_linear")]
    pub ease: String,
}

fn linear() -> String {
    "linear".into()
}

fn is_linear(s: &String) -> bool {
    s == "linear"
}

/// Виды сглаживания (кроме `bezier(x1,y1,x2,y2)`).
pub const EASES: &[(&str, &str)] = &[("linear", "Ровно"), ("ease", "Плавно"), ("in", "Разгон"), ("out", "Торможение"), ("back", "С перелётом"), ("elastic", "Пружина"), ("bounce", "Отскок"), ("hold", "Скачком")];

/// Кривая Безье участка: `bezier(x1,y1,x2,y2)` как cubic-bezier в CSS.
pub fn parse_bezier(s: &str) -> Option<[f64; 4]> {
    let inner = s.trim().strip_prefix("bezier(")?.strip_suffix(')')?;
    let v: Vec<f64> = inner.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    (v.len() == 4).then(|| [v[0].clamp(0.0, 1.0), v[1], v[2].clamp(0.0, 1.0), v[3]])
}

/// Сглаживание допустимо.
pub fn valid_ease(s: &str) -> bool {
    EASES.iter().any(|(k, _)| *k == s) || parse_bezier(s).is_some()
}

fn bezier(b: [f64; 4], x: f64) -> f64 {
    let [x1, y1, x2, y2] = b;
    let f = |t: f64, a: f64, c: f64| 3.0 * (1.0 - t) * (1.0 - t) * t * a + 3.0 * (1.0 - t) * t * t * c + t * t * t;
    let df = |t: f64, a: f64, c: f64| 3.0 * (1.0 - t) * (1.0 - t) * a + 6.0 * (1.0 - t) * t * (c - a) + 3.0 * t * t * (1.0 - c);
    // t по x: Ньютон, затем деление пополам, если производная мала
    let mut t = x;
    for _ in 0..8 {
        let d = df(t, x1, x2);
        if d.abs() < 1e-6 {
            break;
        }
        t = (t - (f(t, x1, x2) - x) / d).clamp(0.0, 1.0);
    }
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..20 {
        if (f(t, x1, x2) - x).abs() < 1e-6 {
            break;
        }
        if f(t, x1, x2) < x {
            lo = t;
        } else {
            hi = t;
        }
        t = (lo + hi) / 2.0;
    }
    f(t, y1, y2)
}

fn bounce(p: f64) -> f64 {
    let (n, d) = (7.5625, 2.75);
    if p < 1.0 / d {
        n * p * p
    } else if p < 2.0 / d {
        let q = p - 1.5 / d;
        n * q * q + 0.75
    } else if p < 2.5 / d {
        let q = p - 2.25 / d;
        n * q * q + 0.9375
    } else {
        let q = p - 2.625 / d;
        n * q * q + 0.984375
    }
}

/// Доля пути `p` (0–1) с учётом сглаживания (back и elastic выходят за 0–1).
pub fn ease(kind: &str, p: f64) -> f64 {
    let p = p.clamp(0.0, 1.0);
    match kind {
        "ease" => p * p * (3.0 - 2.0 * p),
        "in" => p * p * p,
        "out" => 1.0 - (1.0 - p).powi(3),
        "back" => {
            let (c1, c3) = (1.70158, 2.70158);
            1.0 + c3 * (p - 1.0).powi(3) + c1 * (p - 1.0).powi(2)
        }
        "elastic" => {
            if p <= 0.0 || p >= 1.0 {
                p
            } else {
                2f64.powf(-10.0 * p) * ((p * 10.0 - 0.75) * (2.0 * std::f64::consts::PI / 3.0)).sin() + 1.0
            }
        }
        "bounce" => bounce(p),
        "hold" => 0.0,
        k => match parse_bezier(k) {
            Some(b) => bezier(b, p),
            None => p,
        },
    }
}

fn hex(v: &Value) -> Option<[f64; 4]> {
    let s = v.as_str()?.trim().trim_start_matches('#');
    let b = |i: usize| u8::from_str_radix(s.get(i..i + 2)?, 16).ok().map(|x| x as f64);
    match s.len() {
        6 => Some([b(0)?, b(2)?, b(4)?, 255.0]),
        8 => Some([b(0)?, b(2)?, b(4)?, b(6)?]),
        _ => None,
    }
}

/// Значение ключей в момент `t`: числа и цвета плавно, остальное — скачком.
pub fn interp(keys: &[Key], t: f64) -> Value {
    let first = &keys[0];
    if t <= first.t || keys.len() == 1 {
        return first.v.clone();
    }
    let last = &keys[keys.len() - 1];
    if t >= last.t {
        return last.v.clone();
    }
    let i = keys.windows(2).position(|w| t >= w[0].t && t < w[1].t).unwrap_or(0);
    let (a, b) = (&keys[i], &keys[i + 1]);
    let p = ease(&a.ease, (t - a.t) / (b.t - a.t).max(1e-9));
    if let (Some(x), Some(y)) = (a.v.as_f64(), b.v.as_f64()) {
        return serde_json::json!(x + (y - x) * p);
    }
    if let (Some(x), Some(y)) = (hex(&a.v), hex(&b.v)) {
        let c: Vec<u8> = (0..4).map(|i| (x[i] + (y[i] - x[i]) * p).round() as u8).collect();
        return Value::String(if c[3] == 255 { format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]) } else { format!("#{:02x}{:02x}{:02x}{:02x}", c[0], c[1], c[2], c[3]) });
    }
    if p >= 1.0 { b.v.clone() } else { a.v.clone() }
}

/// Переход: вид (dissolve, dip_black, slide_left, slide_right, slide_up, slide_down,
/// wipe_left, wipe_right, zoom, blur или `fusion` с шаблоном `.setting`) и длительность, с.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Transition {
    pub kind: String,
    pub duration: f64,
    /// Путь к `.setting` шаблону Fusion (когда `kind == "fusion"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    /// Переопределения параметров MacroOperator перехода (`p:*`).
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub params: Map<String, Value>,
}

impl Transition {
    pub fn builtin(kind: impl Into<String>, duration: f64) -> Self {
        Self { kind: kind.into(), duration, template: None, params: Map::new() }
    }

    pub fn fusion(template: impl Into<String>, duration: f64) -> Self {
        Self { kind: "fusion".into(), duration, template: Some(template.into()), params: Map::new() }
    }

    pub fn is_fusion(&self) -> bool {
        self.kind == "fusion" && self.template.as_deref().is_some_and(|s| !s.is_empty())
    }

    /// Короткое имя перехода для плашки на таймлайне и инспектора.
    pub fn title(&self) -> String {
        if let Some(p) = self.template.as_deref().filter(|s| self.kind == "fusion" && !s.is_empty()) {
            return std::path::Path::new(p)
                .file_stem()
                .map(|s| s.to_string_lossy().replace('_', " "))
                .unwrap_or_else(|| "Fusion".into());
        }
        crate::render::compose::TRANSITIONS
            .iter()
            .find(|(k, _)| *k == self.kind)
            .map(|(_, l)| (*l).to_string())
            .unwrap_or_else(|| self.kind.clone())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Effect {
    pub id: Id,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub params: Map<String, Value>,
}

impl Default for Project {
    fn default() -> Self {
        Self::new("Без названия")
    }
}

impl Project {
    pub fn new(name: &str) -> Self {
        let mut p = Self { version: FORMAT_VERSION, name: name.to_string(), settings: Settings::default(), media: Vec::new(), tracks: Vec::new(), next_id: 1 };
        let v = p.alloc();
        p.tracks.push(Track { id: v, kind: TrackKind::Video, name: "V1".into(), muted: false, hidden: false, clips: Vec::new() });
        let a = p.alloc();
        p.tracks.push(Track { id: a, kind: TrackKind::Audio, name: "A1".into(), muted: false, hidden: false, clips: Vec::new() });
        p
    }

    pub fn alloc(&mut self) -> Id {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn media(&self, id: Id) -> Option<&Media> {
        self.media.iter().find(|m| m.id == id)
    }

    pub fn track(&self, id: Id) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    pub fn track_mut(&mut self, id: Id) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.id == id)
    }

    /// Дорожка и индекс клипа.
    pub fn find_clip(&self, id: Id) -> Option<(usize, usize)> {
        self.tracks.iter().enumerate().find_map(|(ti, t)| t.clips.iter().position(|c| c.id == id).map(|ci| (ti, ci)))
    }

    pub fn clip(&self, id: Id) -> Option<&Clip> {
        self.find_clip(id).map(|(t, c)| &self.tracks[t].clips[c])
    }

    pub fn clip_mut(&mut self, id: Id) -> Option<&mut Clip> {
        self.find_clip(id).map(move |(t, c)| &mut self.tracks[t].clips[c])
    }

    /// Является ли `ancestor` предком `node` (или самим `node`) по цепочке `parent`.
    pub fn is_ancestor(&self, ancestor: Id, mut node: Id) -> bool {
        for _ in 0..32 {
            if node == ancestor {
                return true;
            }
            match self.clip(node).and_then(|c| c.parent) {
                Some(p) => node = p,
                None => return false,
            }
        }
        false
    }

    /// Отвязать потомков удалённого клипа.
    pub fn clear_parent(&mut self, removed: Id) {
        for t in &mut self.tracks {
            for c in &mut t.clips {
                if c.parent == Some(removed) {
                    c.parent = None;
                }
            }
        }
    }

    /// Связанный парный клип (видео ↔ звук), если он существует в проекте.
    pub fn linked_peer(&self, id: Id) -> Option<Id> {
        let peer = self.clip(id)?.link?;
        (peer != id && self.clip(peer).is_some()).then_some(peer)
    }

    /// Разорвать связь `id` с парным клипом с обеих сторон.
    pub fn unlink(&mut self, id: Id) {
        if let Some(peer) = self.clip(id).and_then(|c| c.link) {
            if let Some(p) = self.clip_mut(peer) {
                if p.link == Some(id) {
                    p.link = None;
                }
            }
        }
        if let Some(c) = self.clip_mut(id) {
            c.link = None;
        }
    }

    /// Убрать ссылки `link` на удалённый клип.
    pub fn clear_link(&mut self, removed: Id) {
        for t in &mut self.tracks {
            for c in &mut t.clips {
                if c.link == Some(removed) {
                    c.link = None;
                }
            }
        }
    }

    /// Длительность всего таймлайна, с.
    pub fn duration(&self) -> f64 {
        self.tracks.iter().flat_map(|t| t.clips.iter().map(Clip::end)).fold(0.0, f64::max)
    }

    /// Время, выровненное по кадру проекта.
    pub fn snap(&self, t: f64) -> f64 {
        let fps = self.settings.fps.max(1.0);
        ((t.max(0.0) * fps).round() / fps * 1e6).round() / 1e6
    }

    /// Компактный текст таймлайна — для агента, CLI и журнала.
    pub fn dump(&self) -> String {
        use std::fmt::Write;
        let s = &self.settings;
        let mut o = String::new();
        let _ = writeln!(o, "Проект «{}»: {}x{} @ {} fps, длительность {}", self.name, s.width, s.height, fmt_fps(s.fps), tc(self.duration()));
        if self.media.is_empty() {
            let _ = writeln!(o, "Медиатека пуста");
        } else {
            let _ = writeln!(o, "Медиатека:");
            for m in &self.media {
                let mut info = vec![m.kind.as_str().to_string()];
                if m.duration > 0.0 {
                    info.push(tc(m.duration));
                }
                if m.width > 0 {
                    info.push(format!("{}x{}", m.width, m.height));
                }
                if m.fps > 0.0 {
                    info.push(format!("{} fps", fmt_fps(m.fps)));
                }
                if m.kind == MediaKind::Video && m.has_audio {
                    info.push("со звуком".into());
                }
                if let Some(t) = &m.transcript {
                    info.push(format!("расшифровка: {} слов", t.words.len()));
                }
                let _ = writeln!(o, "  m{} {} ({})", m.id, m.name(), info.join(", "));
            }
        }
        for t in &self.tracks {
            let mut flags = String::new();
            if t.muted {
                flags.push_str(" [без звука]");
            }
            if t.hidden {
                flags.push_str(" [скрыта]");
            }
            let _ = writeln!(o, "Дорожка t{} {} ({}){}: {} клип(ов)", t.id, t.name, t.kind.as_str(), flags, t.clips.len());
            for c in &t.clips {
                let what = match &c.source {
                    Source::Media { media } => format!("m{} {}", media, self.media(*media).map(|m| m.name()).unwrap_or_default()),
                    Source::Title { text, .. } => format!("титр «{text}»"),
                    Source::Color { color } => format!("цвет {color}"),
                    Source::Shape { kind, .. } => format!("фигура {}", shape_title(kind)),
                };
                let mut extra = Vec::new();
                if c.offset > 0.0 {
                    extra.push(format!("из источника с {}", tc(c.offset)));
                }
                if (c.speed - 1.0).abs() > 1e-9 {
                    extra.push(format!("скорость {}", c.speed));
                }
                if (c.volume - 1.0).abs() > 1e-9 {
                    extra.push(format!("громкость {}", c.volume));
                }
                if !c.blend.is_empty() && c.blend != "normal" {
                    extra.push(format!("наложение {}", c.blend));
                }
                if let Some(pid) = c.parent {
                    extra.push(format!("родитель c{pid}"));
                }
                if let Some(lid) = c.link {
                    extra.push(format!("связан с c{lid}"));
                }
                for e in &c.effects {
                    extra.push(format!("эффект e{} {}{}", e.id, e.kind, if e.params.is_empty() { String::new() } else { format!(" {}", Value::Object(e.params.clone())) }));
                }
                if let Some(t) = &c.transition {
                    extra.push(format!("переход {} {:.2} с", t.kind, t.duration));
                }
                let extra = if extra.is_empty() { String::new() } else { format!(" — {}", extra.join("; ")) };
                let _ = writeln!(o, "  c{} {}–{} {}{}", c.id, tc(c.start), tc(c.end()), what, extra);
            }
        }
        o
    }
}

fn fmt_fps(f: f64) -> String {
    if (f - f.round()).abs() < 1e-6 {
        format!("{}", f.round() as i64)
    } else {
        format!("{f:.3}")
    }
}

/// Таймкод `м:сс.ммм` (часы — если есть).
pub fn tc(t: f64) -> String {
    let ms = (t.max(0.0) * 1000.0).round() as u64;
    let (h, m, s, r) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}.{r:03}")
    } else {
        format!("{m}:{s:02}.{r:03}")
    }
}

/// Разбор времени: секунды (`12.5`), `м:сс(.ммм)`, `ч:мм:сс(.ммм)`.
pub fn parse_time(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => {
            let s = s.trim().trim_end_matches('s').trim_end_matches('с');
            let parts: Vec<&str> = s.split(':').collect();
            let mut t = 0.0;
            for p in &parts {
                t = t * 60.0 + p.trim().replace(',', ".").parse::<f64>().ok()?;
            }
            (parts.len() <= 3).then_some(t)
        }
        _ => None,
    }
}

/// Строка-путь в `PathBuf` с раскрытием `~`.
pub fn expand(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(p)
}
