//! Граф кадра клипа (D1): «Открыть как граф» превращает цепочку эффектов
//! изображения в ноды, дальше можно ветвить и сводить (Merge). Граф
//! считается на кадре источника до раскладки слоя (положение, масштаб,
//! прозрачность, маска и наложение слоя остаются свойствами клипа), одним и
//! тем же кодом в просмотре и экспорте — CPU-операциями исполнителя Fusion
//! (`fusion::img`) и нейрофильтрами synaptix-vfx.
//!
//! Ноды: `source` (кадр клипа), `output` (итог), эффекты `color`, `blur`,
//! `sharpen`, `vignette`, `lut`, `glow`, `upscale` (вход `in`) и `merge`
//! (входы `bg` и `fg`: режим наложения, непрозрачность).

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::doc::Effect;
use crate::fusion::img::{self, Image};

/// Эффекты изображения, которые при графе живут в нодах (а не в списке эффектов).
pub const IMAGE_FX: &[&str] = &["color", "blur", "sharpen", "vignette", "lut", "upscale"];

/// Виды нод: имя, подпись, входы, параметры (для справки агенту и инспектора).
pub const KINDS: &[(&str, &str, &[&str], &str)] = &[
    ("source", "Кадр клипа", &[], ""),
    ("output", "Выход", &["in"], ""),
    ("color", "Цвет", &["in"], "brightness, contrast, saturation, temperature (−1…1)"),
    ("blur", "Размытие", &["in"], "radius (пикс. кадра 1080p)"),
    ("sharpen", "Резкость", &["in"], "amount 0–2"),
    ("vignette", "Виньетка", &["in"], "amount 0–1"),
    ("lut", "LUT", &["in"], "path (.cube), strength 0–1"),
    ("glow", "Свечение", &["in"], "size (пикс.), strength 0–2"),
    ("upscale", "Нейроувеличение", &["in"], "model (plksr|span|compact|nanovsr)"),
    ("block", "Блок", &["in"], "block (имя), values {параметр: значение}; внутри — свой граф"),
    ("merge", "Слияние", &["bg", "fg"], "mode (normal|screen|add|multiply|overlay|darken|lighten|soft_light|hard_light|difference), opacity 0–1"),
];

/// Подпись и пределы числового параметра ноды графа кадра (холст, вынесенные параметры блоков).
pub fn param_spec(kind: &str, name: &str) -> Option<(&'static str, f32, f32)> {
    Some(match (kind, name) {
        ("color", "brightness") => ("Яркость", -1.0, 1.0),
        ("color", "contrast") => ("Контраст", -1.0, 1.0),
        ("color", "saturation") => ("Насыщенность", -1.0, 1.0),
        ("color", "temperature") => ("Температура", -1.0, 1.0),
        ("blur", "radius") => ("Радиус, пикс.", 0.0, 60.0),
        ("sharpen", "amount") => ("Сила", 0.0, 2.0),
        ("vignette", "amount") => ("Сила", 0.0, 1.0),
        ("lut", "strength") => ("Сила", 0.0, 1.0),
        ("glow", "size") => ("Размер, пикс.", 0.0, 100.0),
        ("glow", "strength") => ("Сила", 0.0, 2.0),
        ("merge", "opacity") => ("Непрозрачность", 0.0, 1.0),
        _ => return None,
    })
}

pub fn kind_title(k: &str) -> &'static str {
    KINDS.iter().find(|x| x.0 == k).map(|x| x.1).unwrap_or("Нода")
}

pub fn inputs_of(k: &str) -> &'static [&'static str] {
    KINDS.iter().find(|x| x.0 == k).map(|x| x.2).unwrap_or(&[])
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct FrameGraph {
    pub nodes: Vec<GNode>,
    #[serde(default)]
    pub links: Vec<GLink>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct GNode {
    pub id: u32,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub params: Map<String, Value>,
    /// Положение на холсте.
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct GLink {
    pub from: u32,
    pub to: u32,
    #[serde(default = "port_in")]
    pub port: String,
}

fn port_in() -> String {
    "in".into()
}

fn num(m: &Map<String, Value>, k: &str, d: f64) -> f32 {
    m.get(k).and_then(Value::as_f64).unwrap_or(d) as f32
}

impl FrameGraph {
    /// Цепочка из эффектов изображения клипа: кадр → эффекты по порядку → выход.
    pub fn chain(effects: &[Effect]) -> Self {
        let mut g = FrameGraph { nodes: vec![GNode { id: 1, kind: "source".into(), params: Map::new(), x: 0.0, y: 0.0 }], links: Vec::new() };
        let mut prev = 1;
        for (i, e) in effects.iter().filter(|e| IMAGE_FX.contains(&e.kind.as_str())).enumerate() {
            let id = i as u32 + 2;
            g.nodes.push(GNode { id, kind: e.kind.clone(), params: e.params.clone(), x: 220.0 * (i + 1) as f32, y: 0.0 });
            g.links.push(GLink { from: prev, to: id, port: "in".into() });
            prev = id;
        }
        let out = g.nodes.len() as u32 + 1;
        g.nodes.push(GNode { id: out, kind: "output".into(), params: Map::new(), x: 220.0 * (g.nodes.len()) as f32, y: 0.0 });
        g.links.push(GLink { from: prev, to: out, port: "in".into() });
        g
    }

    pub fn node(&self, id: u32) -> Option<&GNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn next_id(&self) -> u32 {
        self.nodes.iter().map(|n| n.id).max().unwrap_or(0) + 1
    }

    /// Что подано на вход `port` ноды `to`.
    pub fn input(&self, to: u32, port: &str) -> Option<u32> {
        self.links.iter().find(|l| l.to == to && l.port == port).map(|l| l.from)
    }

    /// Проверка: один источник и один выход, известные виды и входы, нет циклов,
    /// на каждый вход не больше одной связи.
    pub fn validate(&self) -> Result<(), String> {
        let mut ids = std::collections::HashSet::new();
        for n in &self.nodes {
            if !ids.insert(n.id) {
                return Err(format!("повтор id ноды {}", n.id));
            }
            if !KINDS.iter().any(|k| k.0 == n.kind) {
                return Err(format!("нет ноды «{}»; есть: {}", n.kind, KINDS.iter().map(|k| k.0).collect::<Vec<_>>().join(", ")));
            }
        }
        for k in ["source", "output"] {
            let c = self.nodes.iter().filter(|n| n.kind == k).count();
            if c != 1 {
                return Err(format!("нужна ровно одна нода {k} (есть {c})"));
            }
        }
        let mut seen = std::collections::HashSet::new();
        for l in &self.links {
            let (Some(_), Some(to)) = (self.node(l.from), self.node(l.to)) else {
                return Err(format!("связь {}→{}: нет такой ноды", l.from, l.to));
            };
            if self.node(l.from).is_some_and(|n| n.kind == "output") {
                return Err("у выхода нет выходов".into());
            }
            if !inputs_of(&to.kind).contains(&l.port.as_str()) {
                return Err(format!("у ноды {} ({}) нет входа «{}»; есть: {}", to.id, to.kind, l.port, inputs_of(&to.kind).join(", ")));
            }
            if !seen.insert((l.to, l.port.clone())) {
                return Err(format!("на вход {}.{} подано две связи", l.to, l.port));
            }
        }
        // циклы: обход от выхода
        fn visit(g: &FrameGraph, id: u32, stack: &mut Vec<u32>) -> Result<(), String> {
            if stack.contains(&id) {
                return Err(format!("цикл через ноду {id}"));
            }
            stack.push(id);
            for l in g.links.iter().filter(|l| l.to == id) {
                visit(g, l.from, stack)?;
            }
            stack.pop();
            Ok(())
        }
        let out = self.nodes.iter().find(|n| n.kind == "output").unwrap().id;
        visit(self, out, &mut Vec::new())
    }

    /// Ноды налезают друг на друга (созданы командами без координат).
    pub fn crowded(&self) -> bool {
        self.nodes.iter().enumerate().any(|(i, a)| self.nodes.iter().skip(i + 1).any(|b| (a.x - b.x).abs() < 200.0 && (a.y - b.y).abs() < 140.0))
    }

    /// Раскладка слоями: столбец — длина пути от источника, внутри — по порядку.
    pub fn layout(&mut self) {
        let mut depth: HashMap<u32, usize> = HashMap::new();
        fn d(g: &FrameGraph, id: u32, memo: &mut HashMap<u32, usize>, guard: usize) -> usize {
            if let Some(v) = memo.get(&id) {
                return *v;
            }
            let v = if guard > 64 { 0 } else { g.links.iter().filter(|l| l.to == id).map(|l| d(g, l.from, memo, guard + 1) + 1).max().unwrap_or(0) };
            memo.insert(id, v);
            v
        }
        for n in &self.nodes {
            d(self, n.id, &mut depth, 0);
        }
        let max = depth.values().copied().max().unwrap_or(0);
        let mut rows: HashMap<usize, f32> = HashMap::new();
        let ids: Vec<u32> = self.nodes.iter().map(|n| n.id).collect();
        for id in ids {
            let n = self.nodes.iter_mut().find(|n| n.id == id).unwrap();
            // выход — последним столбцом
            let col = if n.kind == "output" { max.max(1) } else { depth[&id] };
            let row = rows.entry(col).or_insert(0.0);
            n.x = 260.0 * col as f32;
            n.y = *row;
            *row += 200.0;
        }
    }

    /// Граф — простая цепочка кадр → … → выход: её эффекты по порядку.
    pub fn as_chain(&self) -> Option<Vec<&GNode>> {
        let out = self.nodes.iter().find(|n| n.kind == "output")?;
        let mut v = Vec::new();
        let mut cur = self.input(out.id, "in")?;
        loop {
            let n = self.node(cur)?;
            if n.kind == "source" {
                break;
            }
            if n.kind == "merge" {
                return None;
            }
            v.push(n);
            cur = self.input(n.id, "in")?;
        }
        // лишних нод (вне цепочки) нет
        (v.len() + 2 == self.nodes.len()).then(|| {
            v.reverse();
            v
        })
    }

    /// Текстом для агента и CLI.
    pub fn describe(&self) -> String {
        let mut lines = Vec::new();
        for n in &self.nodes {
            let ins: Vec<String> = inputs_of(&n.kind).iter().map(|p| format!("{p}←{}", self.input(n.id, p).map(|f| f.to_string()).unwrap_or_else(|| "—".into()))).collect();
            // у блока — только имя и вынесенные значения (вложенный граф длинный)
            let shown: Map<String, Value> = if n.kind == "block" { n.params.iter().filter(|(k, _)| matches!(k.as_str(), "block" | "values")).map(|(k, v)| (k.clone(), v.clone())).collect() } else { n.params.clone() };
            let params = if shown.is_empty() { String::new() } else { format!(" {}", Value::Object(shown)) };
            lines.push(format!("  {} {} «{}»{}{}", n.id, n.kind, kind_title(&n.kind), if ins.is_empty() { String::new() } else { format!(" [{}]", ins.join(", ")) }, params));
        }
        lines.join("\n")
    }

    /// Посчитать кадр. `src` — кадр источника (прямая альфа RGBA8); `scale` —
    /// во сколько раз кадр меньше 1080p (радиусы в пикселях 1080p);
    /// `neural` — нейрофильтр ноды (увеличение), `None` — пропустить.
    pub fn eval(&self, w: u32, h: u32, src: &[u8], scale: f32, neural: &mut dyn FnMut(&GNode, &Image) -> Option<Image>) -> Image {
        self.eval_image(Image::from_rgba8(w, h, src), scale, neural)
    }

    pub fn eval_image(&self, source: Image, scale: f32, neural: &mut dyn FnMut(&GNode, &Image) -> Option<Image>) -> Image {
        let Some(out) = self.nodes.iter().find(|n| n.kind == "output") else { return source };
        let mut memo: HashMap<u32, Image> = HashMap::new();
        let r = self.eval_node(out.id, &source, scale, neural, &mut memo, 0);
        r.unwrap_or(source)
    }

    fn eval_node(&self, id: u32, source: &Image, scale: f32, neural: &mut dyn FnMut(&GNode, &Image) -> Option<Image>, memo: &mut HashMap<u32, Image>, depth: usize) -> Option<Image> {
        if depth > 64 {
            return None;
        }
        if let Some(m) = memo.get(&id) {
            return Some(m.clone());
        }
        let n = self.node(id)?;
        let input = |port: &str, memo: &mut HashMap<u32, Image>, neural: &mut dyn FnMut(&GNode, &Image) -> Option<Image>| self.input(id, port).and_then(|f| self.eval_node(f, source, scale, neural, memo, depth + 1));
        let p = &n.params;
        let res = match n.kind.as_str() {
            "source" => source.clone(),
            "output" => input("in", memo, neural).unwrap_or_else(|| Image::empty(source.width, source.height)),
            "block" => {
                let i = input("in", memo, neural).unwrap_or_else(|| Image::empty(source.width, source.height));
                match crate::blocks::inner(p) {
                    Some(g) if depth < 8 => g.eval_image(i, scale, neural),
                    _ => i,
                }
            }
            "merge" => {
                let bg = input("bg", memo, neural).unwrap_or_else(|| Image::empty(source.width, source.height));
                match input("fg", memo, neural) {
                    Some(fg) => img::merge_images(&bg, &fg, (0.5, 0.5), 1.0, 0.0, num(p, "opacity", 1.0).clamp(0.0, 1.0), 1.0, mode_name(p.get("mode").and_then(Value::as_str).unwrap_or("normal")), "Over", None),
                    None => bg,
                }
            }
            kind => {
                let i = input("in", memo, neural).unwrap_or_else(|| Image::empty(source.width, source.height));
                match kind {
                    "color" => {
                        // как CSS-фильтры прежней цепочки: яркость множителем, контраст вокруг 0.5, насыщенность
                        let b = num(p, "brightness", 0.0);
                        let k = num(p, "contrast", 0.0);
                        let s = num(p, "saturation", 0.0);
                        let t = num(p, "temperature", 0.0);
                        let mut o = img::brightness_contrast(&i, 1.0 + b, 0.0, 1.0, k, 0.0, (1.0 + s).max(0.0));
                        if t.abs() > 1e-4 {
                            temperature(&mut o, t);
                        }
                        o
                    }
                    "blur" => {
                        let r = num(p, "radius", 6.0) * scale;
                        img::box_blur(&i, r, r)
                    }
                    "sharpen" => img::sharpen_image(&i, num(p, "amount", 0.5)),
                    "vignette" => {
                        let mut o = i;
                        vignette(&mut o, num(p, "amount", 0.5));
                        o
                    }
                    "glow" => img::glow_image(&i, num(p, "size", 20.0) * scale * 1920.0 / i.width.max(1) as f32, num(p, "strength", 0.6), 1.0),
                    "lut" => {
                        let path = p.get("path").and_then(Value::as_str).unwrap_or("");
                        match crate::render::lut::load(path) {
                            Ok(l) => {
                                let mut px = i.to_rgba8();
                                l.apply(&mut px, num(p, "strength", 1.0).clamp(0.0, 1.0));
                                Image::from_rgba8(i.width, i.height, &px)
                            }
                            Err(_) => i,
                        }
                    }
                    "upscale" => {
                        // нейросеть даёт кадр крупнее — возвращаем к размеру графа (детали остаются)
                        match neural(n, &i) {
                            Some(up) if up.width != i.width || up.height != i.height => resize(&up, i.width, i.height),
                            Some(up) => up,
                            None => i,
                        }
                    }
                    _ => i,
                }
            }
        };
        memo.insert(id, res.clone());
        Some(res)
    }
}

/// Кадр к размеру (Lanczos) — после нейроувеличения.
pub fn resize(im: &Image, w: u32, h: u32) -> Image {
    match image::RgbaImage::from_raw(im.width, im.height, im.to_rgba8()) {
        Some(src) => {
            let out = image::imageops::resize(&src, w, h, image::imageops::FilterType::Lanczos3);
            Image::from_rgba8(w, h, out.as_raw())
        }
        None => im.clone(),
    }
}

fn mode_name(m: &str) -> &'static str {
    match m {
        "screen" => "Screen",
        "add" => "Add",
        "multiply" => "Multiply",
        "overlay" => "Overlay",
        "darken" => "Darken",
        "lighten" => "Lighten",
        "soft_light" => "Soft Light",
        "hard_light" => "Hard Light",
        "difference" => "Difference",
        _ => "Normal",
    }
}

/// Тёплый (+) — к оранжевому, холодный (−) — к синему.
fn temperature(o: &mut Image, t: f32) {
    let k = t.clamp(-1.0, 1.0) * 0.12;
    for px in o.data.chunks_exact_mut(4) {
        let a = px[3];
        px[0] = (px[0] + k * a).clamp(0.0, a);
        px[2] = (px[2] - k * a).clamp(0.0, a);
    }
}

fn vignette(o: &mut Image, amount: f32) {
    let (w, h) = (o.width as f32, o.height as f32);
    let a = amount.clamp(0.0, 1.0);
    for (i, px) in o.data.chunks_exact_mut(4).enumerate() {
        let x = (i as u32 % o.width) as f32 / w - 0.5;
        let y = (i as u32 / o.width) as f32 / h - 0.5;
        let d = ((x * x + y * y).sqrt() / 0.7071).clamp(0.0, 1.0);
        let k = 1.0 - a * d * d;
        px[0] *= k;
        px[1] *= k;
        px[2] *= k;
    }
}

/// Ноды по порядку (для инспектора и справки).
pub fn order(g: &FrameGraph) -> BTreeMap<u32, &GNode> {
    g.nodes.iter().map(|n| (n.id, n)).collect()
}
