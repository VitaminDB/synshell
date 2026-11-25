//! Шаблоны Fusion (`.setting`, DaVinci Resolve):
//! - разбор таблиц Lua ([`parse`], [`Lua`]) и сборка графа композиции ([`Comp`]);
//! - исполнение графа на произвольный кадр/время ([`eval`], [`anim`], [`img`]):
//!   `Transform`, `Merge`, `Background` (+ `Gradient`), маски (`RectangleMask`,
//!   `EllipseMask`, `TriangleMask`, `PolylineMask`, `BSplineMask`, `BitmapMask`),
//!   `TextPlus` (+ `StyledTextFollower`), `BezierSpline`, `PolyPath`, `LUTLookup`/`LUTBezier`,
//!   `Dissolve`, `KeyStretcher`/`KeyStretcherMod`, `TimeStretcher`/`TimeSpeed`,
//!   `Blur`/`GaussianBlur`, `DirectionalBlur`/`RadialBlur`/`ZoomBlur`, `Glow`,
//!   `Displace`, `Fuse.Duplicate`, `BrightnessContrast`, `ColorCurves`,
//!   `DropShadow`, `FastNoise`, `CornerPositioner`, `BetterResize`, `MediaIn`/`MediaOut`.

pub mod anim;
pub mod eval;
pub mod img;
pub mod params;
pub mod report;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

pub use eval::{Overrides, RenderCtx};
pub use img::{Color4, Image};
pub use params::{overrides_from_title, ParamKind, TemplateParam, STYLE_PREFIX};

/// Инструменты, которые исполнитель графа Fusion поддерживает нативно.
pub const SUPPORTED_TOOLS: &[&str] = &[
    "MacroOperator",
    "GroupOperator",
    "MediaOut",
    "MediaIn",
    "AudioDisplay",
    "PipeRouter",
    "SetDomain",
    "ResolveParameter",
    "AlphaMultiply",
    "Transform",
    "Merge",
    "Background",
    "Gradient",
    "RectangleMask",
    "EllipseMask",
    "TriangleMask",
    "PolylineMask",
    "BSplineMask",
    "BitmapMask",
    "TextPlus",
    "StyledTextFollower",
    "BezierSpline",
    "PolyPath",
    "Vector",
    "Polyline",
    "BSplinePolyline",
    "LUTLookup",
    "LUTBezier",
    "Dissolve",
    "KeyStretcher",
    "KeyStretcherMod",
    "TimeStretcher",
    "TimeSpeed",
    "Blur",
    "DirectionalBlur",
    "Glow",
    "SoftGlow",
    "Sharpen",
    "Displace",
    "LensDistort",
    "LumaKeyer",
    "Fuse.Duplicate",
    "BrightnessContrast",
    "ColorCurves",
    "ColorCorrector",
    "FastNoise",
    "BetterResize",
    "Resize",
    "Crop",
    "CornerPositioner",
    "ChannelBooleans",
    "Shadow",
    "PublishNumber",
    "PublishText",
    "PublishPoint",
    "Shake",
    "ofx.com.blackmagicdesign.resolvefx.GaussianBlur",
    "ofx.com.blackmagicdesign.resolvefx.DirectionalBlur",
    "ofx.com.blackmagicdesign.resolvefx.RadialBlur",
    "ofx.com.blackmagicdesign.resolvefx.ZoomBlur",
    "ofx.com.blackmagicdesign.resolvefx.dropshadow",
    "ofx.com.blackmagicdesign.resolvefx.DropShadow",
    "ofx.com.blackmagicdesign.resolvefx.MosaicBlur",
    "ofx.com.blackmagicdesign.resolvefx.mosaicblur",
    "ofx.com.blackmagicdesign.resolvefx.Waviness",
    "ofx.com.blackmagicdesign.resolvefx.waviness",
];

pub fn is_supported_tool(ty: &str) -> bool {
    SUPPORTED_TOOLS.iter().any(|&s| s == ty)
}

/// Значение таблицы Lua.
#[derive(Clone, Debug, PartialEq)]
pub enum Lua {
    Str(String),
    Num(f64),
    Bool(bool),
    Nil,
    /// Таблица: тип перед `{` (`TextPlus`, `Input`, `ordered()`), поля по порядку
    /// (ключ — имя, `["строка"]`, `[0]` или `None` у элементов списка).
    Table { ty: String, items: Vec<(Option<String>, Lua)> },
}

impl Lua {
    pub fn get(&self, key: &str) -> Option<&Lua> {
        match self {
            Lua::Table { items, .. } => items.iter().find(|(k, _)| k.as_deref() == Some(key)).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn ty(&self) -> &str {
        match self {
            Lua::Table { ty, .. } => ty,
            _ => "",
        }
    }
    pub fn items(&self) -> &[(Option<String>, Lua)] {
        match self {
            Lua::Table { items, .. } => items,
            _ => &[],
        }
    }
    pub fn num(&self) -> Option<f64> {
        match self {
            Lua::Num(n) => Some(*n),
            Lua::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            Lua::Str(s) => s.trim().parse().ok(),
            _ => None,
        }
    }
    pub fn str(&self) -> Option<&str> {
        match self {
            Lua::Str(s) => Some(s),
            // FuID { "Gradient" } хранит строку первым безымянным элементом
            Lua::Table { ty, items } if ty == "FuID" || ty == "StyledText" => {
                if let Some(v) = self.get("Value").and_then(Lua::str) {
                    return Some(v);
                }
                items.first().and_then(|(_, v)| v.str())
            }
            _ => None,
        }
    }
    /// Пара чисел `{ x, y }` или `Point { X = x, Y = y }`.
    pub fn point(&self) -> Option<(f64, f64)> {
        if let (Some(x), Some(y)) = (self.get("X").and_then(Lua::num), self.get("Y").and_then(Lua::num)) {
            return Some((x, y));
        }
        let nums: Vec<f64> = self.items().iter().filter_map(|(_, v)| v.num()).collect();
        if nums.len() >= 2 {
            Some((nums[0], nums[1]))
        } else {
            None
        }
    }
    /// `Input { Value = … }` → значение.
    pub fn input(&self, name: &str) -> Option<&Lua> {
        self.get("Inputs")?.get(name)?.get("Value")
    }
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> P<'a> {
    fn ws(&mut self) {
        loop {
            while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
            if self.s[self.i..].starts_with(b"--[[") {
                self.i += 4;
                while self.i + 1 < self.s.len() && &self.s[self.i..self.i + 2] != b"]]" {
                    self.i += 1;
                }
                if self.i + 1 < self.s.len() {
                    self.i += 2;
                }
                continue;
            }
            if self.s[self.i..].starts_with(b"--") {
                while self.i < self.s.len() && self.s[self.i] != b'\n' {
                    self.i += 1;
                }
                continue;
            }
            break;
        }
    }
    fn peek(&self) -> u8 {
        self.s.get(self.i).copied().unwrap_or(0)
    }
    fn ident(&mut self) -> String {
        let st = self.i;
        while self.i < self.s.len() && (self.s[self.i].is_ascii_alphanumeric() || self.s[self.i] == b'_' || self.s[self.i] == b'.') {
            self.i += 1;
        }
        String::from_utf8_lossy(&self.s[st..self.i]).into_owned()
    }
    fn long_string(&mut self) -> Option<Result<String, String>> {
        if self.peek() != b'[' {
            return None;
        }
        let mut j = self.i + 1;
        let mut eqs = 0usize;
        while j < self.s.len() && self.s[j] == b'=' {
            eqs += 1;
            j += 1;
        }
        if j >= self.s.len() || self.s[j] != b'[' {
            return None;
        }
        self.i = j + 1;
        if self.peek() == b'\n' {
            self.i += 1;
        }
        let start = self.i;
        while self.i < self.s.len() {
            if self.s[self.i] == b']' {
                let mut k = self.i + 1;
                let mut n = 0;
                while k < self.s.len() && self.s[k] == b'=' {
                    n += 1;
                    k += 1;
                }
                if n == eqs && k < self.s.len() && self.s[k] == b']' {
                    let out = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();
                    self.i = k + 1;
                    return Some(Ok(out));
                }
            }
            self.i += 1;
        }
        Some(Err("незакрытая длинная строка".into()))
    }
    fn string(&mut self) -> Result<String, String> {
        let q = self.s[self.i];
        self.i += 1;
        let mut out = Vec::new();
        while self.i < self.s.len() && self.s[self.i] != q {
            if self.s[self.i] == b'\\' && self.i + 1 < self.s.len() {
                self.i += 1;
                out.push(match self.s[self.i] {
                    b'n' => b'\n',
                    b'r' => b'\r',
                    b't' => b'\t',
                    c => c,
                });
            } else {
                out.push(self.s[self.i]);
            }
            self.i += 1;
        }
        if self.i >= self.s.len() {
            return Err("незакрытая строка".into());
        }
        self.i += 1;
        Ok(String::from_utf8_lossy(&out).into_owned())
    }
    fn table(&mut self, ty: String) -> Result<Lua, String> {
        self.i += 1; // {
        let mut items = Vec::new();
        loop {
            self.ws();
            match self.peek() {
                b'}' => {
                    self.i += 1;
                    return Ok(Lua::Table { ty, items });
                }
                0 => return Err("незакрытая таблица".into()),
                b',' | b';' => {
                    self.i += 1;
                    continue;
                }
                _ => {}
            }
            // ключ: name = …, ["str"] = …, [1] = …, [-5] = …
            let save = self.i;
            let mut key = None;
            if self.peek() == b'[' && !self.s[self.i..].starts_with(b"[[") && !self.s[self.i..].starts_with(b"[=") {
                self.i += 1;
                self.ws();
                let k = if matches!(self.peek(), b'"' | b'\'') {
                    self.string()?
                } else {
                    let st = self.i;
                    while self.i < self.s.len() && self.s[self.i] != b']' {
                        self.i += 1;
                    }
                    String::from_utf8_lossy(&self.s[st..self.i]).trim().to_string()
                };
                self.ws();
                if self.peek() == b']' {
                    self.i += 1;
                    self.ws();
                    if self.peek() == b'=' {
                        self.i += 1;
                        key = Some(k);
                    } else {
                        self.i = save;
                    }
                } else {
                    self.i = save;
                }
            } else if self.peek().is_ascii_alphabetic() || self.peek() == b'_' {
                let k = self.ident();
                self.ws();
                if self.peek() == b'=' && self.s.get(self.i + 1) != Some(&b'=') {
                    self.i += 1;
                    key = Some(k);
                } else {
                    self.i = save;
                }
            }
            let v = self.value()?;
            items.push((key, v));
        }
    }
    fn value(&mut self) -> Result<Lua, String> {
        self.ws();
        if let Some(r) = self.long_string() {
            return r.map(Lua::Str);
        }
        match self.peek() {
            b'{' => self.table(String::new()),
            b'"' | b'\'' => Ok(Lua::Str(self.string()?)),
            b'-' | b'+' | b'0'..=b'9' | b'.' => {
                let st = self.i;
                self.i += 1;
                while self.i < self.s.len() && (self.s[self.i].is_ascii_alphanumeric() || matches!(self.s[self.i], b'.' | b'-' | b'+')) {
                    self.i += 1;
                }
                let t = String::from_utf8_lossy(&self.s[st..self.i]).into_owned();
                Ok(t.parse().map(Lua::Num).unwrap_or(Lua::Str(t)))
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let id = self.ident();
                match id.as_str() {
                    "true" => return Ok(Lua::Bool(true)),
                    "false" => return Ok(Lua::Bool(false)),
                    "nil" => return Ok(Lua::Nil),
                    _ => {}
                }
                self.ws();
                let mut ty = id;
                if self.peek() == b'(' {
                    let mut depth = 0i32;
                    while self.i < self.s.len() {
                        match self.s[self.i] {
                            b'(' => depth += 1,
                            b')' => {
                                depth -= 1;
                                if depth == 0 {
                                    self.i += 1;
                                    break;
                                }
                            }
                            _ => {}
                        }
                        self.i += 1;
                    }
                    ty.push_str("()");
                    self.ws();
                }
                match self.peek() {
                    b'{' => self.table(ty),
                    b'"' | b'\'' => Ok(Lua::Table { ty, items: vec![(None, Lua::Str(self.string()?))] }),
                    _ => Ok(Lua::Str(ty)),
                }
            }
            c => Err(format!("неожиданный символ «{}» на {}", c as char, self.i)),
        }
    }
}

pub fn parse(text: &str) -> Result<Lua, String> {
    let text = text.trim_start_matches('\u{feff}');
    let mut p = P { s: text.as_bytes(), i: 0 };
    p.value()
}

/// Текст шаблона: строка, шрифт, начертание, размер (доля высоты кадра), цвет, центр.
#[derive(Clone, Debug, PartialEq)]
pub struct TextLayer {
    pub name: String,
    pub text: String,
    pub font: String,
    pub style: String,
    pub size: f64,
    pub color: String,
    pub center: (f64, f64),
}

/// Вынесенный параметр `MacroOperator.Inputs` (C3).
#[derive(Clone, Debug, PartialEq)]
pub struct ExposedInput {
    pub id: String,
    pub source_op: String,
    pub source: String,
    pub name: String,
    pub control_group: Option<i64>,
    pub default: Option<f64>,
    pub default_xy: Option<(f64, f64)>,
    pub min_scale: Option<f64>,
    pub max_scale: Option<f64>,
    pub page: Option<String>,
}

/// Один узел (инструмент) графа Fusion.
#[derive(Clone, Debug)]
pub struct Tool {
    pub name: String,
    pub ty: String,
    /// Если инструмент является инстансом другого инструмента (`Instance_*`).
    pub source_op: Option<String>,
    pub raw: Lua,
}

/// Скомпилированный граф шаблона Fusion (кэшируется по пути к файлу).
#[derive(Clone, Debug)]
pub struct Comp {
    pub name: String,
    pub tools: HashMap<String, Tool>,
    pub order: Vec<String>,
    /// Главный выходной узел композиции.
    pub output: Option<String>,
    /// Внешние входы MacroOperator (`MainInput1` → 0, `MainInput2` → 1…): `(SourceOp, Source)`.
    pub main_inputs: Vec<(String, String)>,
    /// Вынесенные параметры MacroOperator.
    pub exposed: Vec<ExposedInput>,
    /// Исходная длительность в кадрах (оценка по сплайнам и KeyStretcher).
    pub source_frames: f64,
}

/// Все инструменты шаблона (с заходом в MacroOperator/GroupOperator).
fn collect_tools<'a>(root: &'a Lua, out: &mut Vec<(&'a str, &'a Lua)>) {
    if let Some(t) = root.get("Tools") {
        for (k, v) in t.items() {
            if let Some(k) = k {
                out.push((k.as_str(), v));
                collect_tools(v, out);
            }
        }
    }
}

impl Comp {
    pub fn from_lua(name: impl Into<String>, root: &Lua) -> Self {
        let mut raw_tools = Vec::new();
        collect_tools(root, &mut raw_tools);

        let mut tools = HashMap::new();
        let mut order = Vec::new();
        let mut main_inputs_indexed: Vec<(usize, (String, String))> = Vec::new();
        let mut exposed = Vec::new();
        let mut main_output: Option<String> = None;

        for (tname, node) in &raw_tools {
            let ty = node.ty().to_string();
            if matches!(ty.as_str(), "MacroOperator" | "GroupOperator") {
                if let Some(ins) = node.get("Inputs") {
                    for (ik, iv) in ins.items() {
                        if iv.ty() == "InstanceInput" {
                            let Some(sop) = iv.get("SourceOp").and_then(Lua::str) else { continue };
                            let Some(src) = iv.get("Source").and_then(Lua::str) else { continue };
                            if let Some(num) = ik.as_deref().and_then(|k| k.strip_prefix("MainInput")).and_then(|s| s.parse::<usize>().ok()) {
                                main_inputs_indexed.push((num, (sop.to_string(), src.to_string())));
                                continue;
                            }
                            let label = iv.get("Name").and_then(Lua::str).unwrap_or(src).to_string();
                            let default_xy = match (iv.get("DefaultX").and_then(Lua::num), iv.get("DefaultY").and_then(Lua::num)) {
                                (Some(x), Some(y)) => Some((x, y)),
                                _ => None,
                            };
                            exposed.push(ExposedInput {
                                id: ik.clone().unwrap_or_else(|| format!("{sop}.{src}")),
                                source_op: sop.to_string(),
                                source: src.to_string(),
                                name: label,
                                control_group: iv.get("ControlGroup").and_then(Lua::num).map(|n| n.round() as i64),
                                default: iv.get("Default").and_then(Lua::num),
                                default_xy,
                                min_scale: iv.get("MinScale").or_else(|| iv.get("MinAllowed")).and_then(Lua::num),
                                max_scale: iv.get("MaxScale").or_else(|| iv.get("MaxAllowed")).and_then(Lua::num),
                                page: iv.get("Page").and_then(Lua::str).map(str::to_string),
                            });
                        }
                    }
                }
                if main_output.is_none() {
                    if let Some(outs) = node.get("Outputs") {
                        // MainOutput1 приоритетнее; иначе любой выход со Source == "Output" или "Result"
                        if let Some(mo) = outs.get("MainOutput1").and_then(|o| o.get("SourceOp")).and_then(Lua::str) {
                            main_output = Some(mo.to_string());
                        } else {
                            for (_, ov) in outs.items() {
                                let src = ov.get("Source").and_then(Lua::str).unwrap_or("");
                                if matches!(src, "Output" | "Result" | "") {
                                    if let Some(sop) = ov.get("SourceOp").and_then(Lua::str) {
                                        main_output = Some(sop.to_string());
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                continue;
            }
            let source_op = node.get("SourceOp").and_then(Lua::str).map(str::to_string);
            order.push((*tname).to_string());
            tools.insert(
                (*tname).to_string(),
                Tool { name: (*tname).to_string(), ty, source_op, raw: (*node).clone() },
            );
        }

        // Определяем выход, если не нашли в MacroOperator.Outputs
        let output = main_output
            .filter(|n| tools.contains_key(n))
            .or_else(|| order.iter().find(|n| tools.get(*n).is_some_and(|t| t.ty == "MediaOut")).cloned())
            .or_else(|| {
                // Ищем последний инструмент-изображение, на который никто не ссылается
                let mut referenced = std::collections::HashSet::new();
                for t in tools.values() {
                    if let Some(ins) = t.raw.get("Inputs") {
                        for (_, iv) in ins.items() {
                            if let Some(sop) = iv.get("SourceOp").and_then(Lua::str) {
                                referenced.insert(sop.to_string());
                            }
                        }
                    }
                }
                order.iter().rev().find(|n| !referenced.contains(*n) && tools.get(*n).is_some_and(|t| is_image_tool(&t.ty))).cloned()
            });

        // Исходная длительность в кадрах: максимум из KeyStretcher.SourceEnd и ключевых кадров сплайнов
        let mut max_frame = 0.0f64;
        for t in tools.values() {
            if matches!(t.ty.as_str(), "KeyStretcher" | "KeyStretcherMod") {
                if let Some(se) = t.raw.input("SourceEnd").and_then(Lua::num) {
                    max_frame = max_frame.max(se);
                }
            }
            if t.ty == "BezierSpline" {
                if let Some(kfs) = t.raw.get("KeyFrames") {
                    for (k, _) in kfs.items() {
                        if let Some(f) = k.as_deref().and_then(|s| s.parse::<f64>().ok()) {
                            max_frame = max_frame.max(f);
                        }
                    }
                }
            }
        }
        let source_frames = if max_frame >= 1.0 { max_frame } else { 90.0 };
        main_inputs_indexed.sort_by_key(|(n, _)| *n);
        let main_inputs: Vec<(String, String)> = main_inputs_indexed.into_iter().map(|(_, pair)| pair).collect();

        Self { name: name.into(), tools, order, output, main_inputs, exposed, source_frames }
    }

    /// Является ли граф переходом между двумя клипами (`MainInput1`+`MainInput2` или `MediaIn1`+`MediaIn2`).
    pub fn is_transition(&self) -> bool {
        if self.main_inputs.len() >= 2 {
            return true;
        }
        self.tools.values().filter(|t| t.ty == "MediaIn").count() >= 2
    }

    /// Уникальные типы инструментов графа (без служебных операторов).
    pub fn tool_types(&self) -> Vec<String> {
        let mut s: Vec<String> = self.tools.values().map(|t| t.ty.clone()).collect();
        s.sort();
        s.dedup();
        s
    }

    /// Неподдерживаемые типы инструментов в этом шаблоне.
    pub fn unsupported_tools(&self) -> Vec<String> {
        self.tool_types().into_iter().filter(|t| !is_supported_tool(t)).collect()
    }

    /// Получить вход `input_name` инструмента `tool_name` с учётом наследования инстанса (`Tool.source_op`).
    pub fn resolve_input<'a>(&'a self, tool_name: &str, input_name: &str) -> Option<&'a Lua> {
        self.resolve_input_rec(tool_name, input_name, 0)
    }

    fn resolve_input_rec<'a>(&'a self, tool_name: &str, input_name: &str, depth: usize) -> Option<&'a Lua> {
        if depth > 8 {
            return None;
        }
        let tool = self.tools.get(tool_name)?;
        if let Some(inp) = tool.raw.get("Inputs").and_then(|ins| ins.get(input_name)) {
            let has_own = inp.get("Value").is_some() || inp.get("SourceOp").is_some() || inp.get("Expression").is_some();
            if has_own {
                return Some(inp);
            }
            // В Fusion пустой `EffectMask = Input {}` на инстансе означает «отвязано от родительского входа»
            if tool.source_op.is_some() && matches!(input_name, "EffectMask" | "Input" | "Background" | "Foreground") {
                return None;
            }
        }
        if let Some(base) = &tool.source_op {
            return self.resolve_input_rec(base, input_name, depth + 1);
        }
        None
    }

    /// Имена всех TextPlus-инструментов в порядке обхода (для подстановки текста пользователя).
    pub fn text_tool_names(&self) -> Vec<String> {
        self.order
            .iter()
            .filter(|n| self.tools.get(*n).is_some_and(|t| t.ty == "TextPlus"))
            .cloned()
            .collect()
    }
}

fn is_image_tool(ty: &str) -> bool {
    !matches!(
        ty,
        "BezierSpline"
            | "PolyPath"
            | "Vector"
            | "AudioDisplay"
            | "ResolveParameter"
            | "LUTLookup"
            | "LUTBezier"
            | "StyledTextFollower"
            | "KeyStretcherMod"
            | "PublishNumber"
            | "PublishText"
            | "PublishPoint"
            | "Shake"
    )
}

/// Извлечь текстовые слои (включая текст внутри подключённого `StyledTextFollower`!).
pub fn text_layers(root: &Lua) -> Vec<TextLayer> {
    let mut all = Vec::new();
    collect_tools(root, &mut all);
    let by_name: HashMap<&str, &Lua> = all.iter().copied().collect();
    let mut v = Vec::new();
    for (name, t) in all.iter().filter(|(_, t)| t.ty() == "TextPlus") {
        let mut text = match t.input("StyledText") {
            Some(Lua::Str(s)) => s.clone(),
            Some(tbl @ Lua::Table { .. }) => tbl.get("Value").and_then(Lua::str).unwrap_or_default().to_string(),
            _ => String::new(),
        };
        if text.is_empty() {
            // Текст может жить в StyledTextFollower (как TITLE_03 -> Follower1 в Minimal_Title_01)
            if let Some(sop) = t.get("Inputs").and_then(|i| i.get("StyledText")).and_then(|i| i.get("SourceOp")).and_then(Lua::str) {
                if let Some(f) = by_name.get(sop) {
                    if let Some(tv) = f.input("Text") {
                        text = tv.str().unwrap_or_default().to_string();
                    }
                }
            }
        }
        if text.trim().is_empty() {
            continue;
        }
        let c = |k: &str, d: f64| t.input(k).and_then(Lua::num).unwrap_or(d);
        let (r, g, b) = (c("Red1", 1.0), c("Green1", 1.0), c("Blue1", 1.0));
        let to = |x: f64| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
        let center = t.input("Center").and_then(Lua::point).unwrap_or((0.5, 0.5));
        v.push(TextLayer {
            name: name.to_string(),
            text,
            font: t.input("Font").and_then(Lua::str).unwrap_or("").to_string(),
            style: t.input("Style").and_then(Lua::str).unwrap_or("").to_string(),
            size: c("Size", 0.08),
            color: format!("#{:02x}{:02x}{:02x}", to(r), to(g), to(b)),
            center,
        });
    }
    v
}

/// Загрузить и скомпилировать `.setting` с кэшированием в памяти по пути.
pub fn load_cached(path: &Path) -> Result<Arc<Comp>, String> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<Comp>>>> = OnceLock::new();
    let map = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(hit) = map.lock().unwrap_or_else(|e| e.into_inner()).get(path) {
        return Ok(hit.clone());
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let root = parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let comp = Arc::new(Comp::from_lua(name, &root));
    map.lock().unwrap_or_else(|e| e.into_inner()).insert(path.to_path_buf(), comp.clone());
    Ok(comp)
}

/// Библиотека шаблонов: `~/Storage/Видеомонтаж/DaVinci Resolve` (или `SYN_VIDEO_EDITOR_TEMPLATES`).
pub fn library() -> PathBuf {
    if let Some(p) = std::env::var_os("SYN_VIDEO_EDITOR_TEMPLATES") {
        return PathBuf::from(p);
    }
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join("Storage/Видеомонтаж/DaVinci Resolve")
}

/// Все `.setting` в папке (рекурсивно): (категория — путь папки, имя, путь).
pub fn scan(root: &Path) -> Vec<(String, String, PathBuf)> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, String, PathBuf)>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, root, out);
            } else if p.extension().is_some_and(|x| x == "setting") {
                let cat = p.parent().and_then(|d| d.strip_prefix(root).ok()).map(|d| d.display().to_string()).unwrap_or_default();
                let name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                out.push((cat, name, p));
            }
        }
    }
    let mut v = Vec::new();
    walk(root, root, &mut v);
    v.sort();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_textplus() {
        let s = r#"{ Tools = ordered() { T = TextPlus { Inputs = { StyledText = Input { Value = "Привет", }, Font = Input { Value = "Uni Sans", }, Size = Input { Value = 0.07, }, Red1 = Input { Value = 1, }, Green1 = Input { Value = 0, }, Blue1 = Input { Value = 0, }, ["Gamut.X"] = Input { Value = FuID { "SLog2" }, }, Center = Input { Value = { 0.5, 0.3 }, }, }, ViewInfo = OperatorInfo { Pos = { 1, -2 } }, }, }, }"#;
        let root = parse(s).unwrap();
        let t = text_layers(&root);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].text, "Привет");
        assert_eq!(t[0].font, "Uni Sans");
        assert_eq!(t[0].color, "#ff0000");
        assert_eq!(t[0].center, (0.5, 0.3));
    }

    #[test]
    fn parses_and_renders_library_template() {
        let p = library().join("Титры/Заголовки/Minimal_Title_01.setting");
        let Ok(comp) = load_cached(&p) else { return };
        let layers = text_layers(&parse(&std::fs::read_to_string(&p).unwrap()).unwrap());
        assert!(layers.iter().any(|l| l.text == "DAVINCI"), "{layers:?}");
        assert!(layers.iter().any(|l| l.text == "TEMPLATE"), "StyledTextFollower text: {layers:?}");
        assert_eq!(comp.unsupported_tools(), Vec::<String>::new());

        // В начале (t=0.0) элементы ещё скрыты/въезжают; в середине (t=2.0) — видны и текст, и градиентные плашки!
        let mid = eval::render(&comp, 480, 270, 2.0, 4.0, 30.0, &[], &Overrides::default());
        let non_empty = mid.rgba.chunks_exact(4).filter(|px| px[3] > 20).count();
        assert!(non_empty > 2000, "Minimal_Title_01 mid frame should have visible pixels, got {non_empty}");
        // Есть цветные пиксели градиента (не только чёрно-белый)
        let colored = mid.rgba.chunks_exact(4).filter(|px| px[3] > 50 && (px[0].abs_diff(px[1]) > 30 || px[1].abs_diff(px[2]) > 30)).count();
        assert!(colored > 200, "Minimal_Title_01 should render colored gradient bars, got {colored}");
    }

    #[test]
    fn executes_core_and_tail_nodes() {
        // Граф: Background (Gradient) -> Mask (Ellipse) -> Transform (BezierSpline) -> Glow -> Displace -> Merge с TextPlus -> Dissolve -> Fuse.Duplicate
        let setting = r#"{
            Tools = ordered() {
                Mask1 = EllipseMask {
                    Inputs = {
                        Center = Input { Value = { 0.5, 0.5 }, },
                        Width = Input { Value = 0.5, },
                        Height = Input { Value = 0.5, },
                        SoftEdge = Input { Value = 0.02, },
                    },
                },
                Bg1 = Background {
                    Inputs = {
                        EffectMask = Input { SourceOp = "Mask1", Source = "Mask", },
                        Type = Input { Value = FuID { "Gradient" }, },
                        Start = Input { Value = { 0.2, 0.5 }, },
                        End = Input { Value = { 0.8, 0.5 }, },
                        Gradient = Input {
                            Value = Gradient {
                                Colors = {
                                    [0] = { 1, 0.2, 0, 1 },
                                    [1] = { 0, 0.4, 1, 1 },
                                }
                            },
                        },
                    },
                },
                Xf1 = Transform {
                    Inputs = {
                        Input = Input { SourceOp = "Bg1", Source = "Output", },
                        Size = Input { SourceOp = "Xf1Size", Source = "Value", },
                        Angle = Input { Value = 15, },
                    },
                },
                Xf1Size = BezierSpline {
                    KeyFrames = {
                        [0] = { 0.4, RH = { 10, 0.4 } },
                        [30] = { 1.0, LH = { 20, 1.0 } },
                    },
                },
                Glow1 = Glow {
                    Inputs = {
                        Input = Input { SourceOp = "Xf1", Source = "Output", },
                        GlowSize = Input { Value = 8, },
                        Glow = Input { Value = 0.6, },
                        Blend = Input { Value = 0.7, },
                    },
                },
                Txt1 = TextPlus {
                    Inputs = {
                        StyledText = Input { Value = "HELLO", },
                        Size = Input { Value = 0.14, },
                        Red1 = Input { Value = 1, },
                        Green1 = Input { Value = 1, },
                        Blue1 = Input { Value = 1, },
                    },
                },
                M1 = Merge {
                    Inputs = {
                        Background = Input { SourceOp = "Glow1", Source = "Output", },
                        Foreground = Input { SourceOp = "Txt1", Source = "Output", },
                    },
                },
                Dup1 = Fuse.Duplicate {
                    Inputs = {
                        Background = Input { SourceOp = "M1", Source = "Output", },
                        Copies = Input { Value = 3, },
                        Center = Input { Value = { 0.53, 0.5 }, },
                        Gain = Input { Value = 0.7, },
                    },
                },
                DisplaceMap = FastNoise {
                    Inputs = {
                        Detail = Input { Value = 2, },
                    },
                },
                Disp1 = Displace {
                    Inputs = {
                        Input = Input { SourceOp = "Dup1", Source = "Output", },
                        Foreground = Input { SourceOp = "DisplaceMap", Source = "Output", },
                        Type = Input { Value = 1, },
                        XRefraction = Input { Value = 0.02, },
                    },
                },
                Blur1 = Blur {
                    Inputs = {
                        Input = Input { SourceOp = "Disp1", Source = "Output", },
                        XBlurSize = Input { Value = 2, },
                    },
                },
                Diss1 = Dissolve {
                    Inputs = {
                        Background = Input { SourceOp = "Bg1", Source = "Output", },
                        Foreground = Input { SourceOp = "Blur1", Source = "Output", },
                        Mix = Input { Value = 0.8, },
                    },
                },
                KS1 = KeyStretcher {
                    Inputs = {
                        Keyframes = Input { SourceOp = "Diss1", Source = "Output", },
                        SourceEnd = Input { Value = 30, },
                        StretchStart = Input { Value = 15, },
                        StretchEnd = Input { Value = 30, },
                    },
                },
                MediaOut1 = MediaOut {
                    Inputs = {
                        Input = Input { SourceOp = "KS1", Source = "Result", },
                    },
                },
            },
        }"#;
        let root = parse(setting).unwrap();
        let comp = Comp::from_lua("test", &root);
        let f0 = eval::render(&comp, 240, 135, 0.0, 2.0, 30.0, &[], &Overrides::default());
        let f1 = eval::render(&comp, 240, 135, 1.5, 2.0, 30.0, &[], &Overrides::default());
        let a0: usize = f0.rgba.chunks_exact(4).filter(|p| p[3] > 20).count();
        let a1: usize = f1.rgba.chunks_exact(4).filter(|p| p[3] > 20).count();
        assert!(a0 > 500 && a1 > a0, "animation should grow ellipse: a0={a0}, a1={a1}");
    }
}
