//! Вынесенные параметры шаблона Fusion (`MacroOperator.Inputs`) для инспектора,
//! команд (`template.params` / `template.set`), ключей анимации и `Overrides`.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Map, Value};

use super::anim::{eval_num, eval_point, eval_str};
use super::eval::Overrides;
use super::img::Color4;
use super::{Comp, ExposedInput, Lua};

/// Префикс ключей параметров шаблона в `Source::Title.style`.
pub const STYLE_PREFIX: &str = "p:";

/// Вариант выбора для перечисляемого параметра шаблона.
#[derive(Clone, Debug, PartialEq)]
pub struct ChoiceItem {
    pub value: String,
    pub label: String,
}

/// Тип и метаданные вынесенного параметра шаблона.
#[derive(Clone, Debug, PartialEq)]
pub enum ParamKind {
    Text {
        default: String,
    },
    Font {
        default: String,
    },
    Number {
        default: f64,
        min: f64,
        max: f64,
        step: f64,
    },
    Point {
        default: (f64, f64),
        min: f64,
        max: f64,
        step: f64,
    },
    /// Цвет `#rrggbb` / `#rrggbbaa`, собранный из каналов `(Red, Green, Blue, Option<Alpha>)`.
    Color {
        default: String,
        channels: [String; 3],
        alpha: Option<String>,
    },
    /// Цвет конкретной точки градиента узла `Background` (`Gradient.Colors[idx]`).
    GradStop {
        default: String,
        stop_index: usize,
    },
    Bool {
        default: bool,
    },
    Choice {
        default: String,
        items: Vec<ChoiceItem>,
    },
}

/// Готовый для инспектора и команд параметр шаблона Fusion.
#[derive(Clone, Debug, PartialEq)]
pub struct TemplateParam {
    /// Стабильный ключ параметра (например `Master.Size`, `TITLE_01.Color1`, `Master.Center`).
    pub key: String,
    /// Раздел / группа (имя страницы или узла, например `Master`, `TITLE_01`, `LABLE_COLOR_01`).
    pub group: String,
    /// Подпись для человека.
    pub label: String,
    /// Узел графа Fusion (`SourceOp`).
    pub source_op: String,
    /// Каноническое имя входа узла (`Source`).
    pub source: String,
    pub kind: ParamKind,
}

impl TemplateParam {
    /// Ключ в `Source::Title.style` (например `p:Master.Size`).
    pub fn style_key(&self) -> String {
        format!("{STYLE_PREFIX}{}", self.key)
    }

    pub fn style_key_sub(&self, suffix: &str) -> String {
        format!("{STYLE_PREFIX}{}.{suffix}", self.key)
    }

    /// Значение по умолчанию в JSON.
    pub fn default_json(&self) -> Value {
        match &self.kind {
            ParamKind::Text { default } | ParamKind::Font { default } | ParamKind::Color { default, .. } | ParamKind::GradStop { default, .. } | ParamKind::Choice { default, .. } => Value::String(default.clone()),
            ParamKind::Number { default, .. } => json!(round_clean(*default)),
            ParamKind::Point { default, .. } => json!([round_clean(default.0), round_clean(default.1)]),
            ParamKind::Bool { default } => Value::Bool(*default),
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self.kind {
            ParamKind::Text { .. } => "text",
            ParamKind::Font { .. } => "font",
            ParamKind::Number { .. } => "number",
            ParamKind::Point { .. } => "point",
            ParamKind::Color { .. } | ParamKind::GradStop { .. } => "color",
            ParamKind::Bool { .. } => "bool",
            ParamKind::Choice { .. } => "choice",
        }
    }
}

/// Нормализовать имена `*Clone` входы TextPlus (`Red1Clone` -> `Red1`,
/// `CharacterSpacingClone` -> `CharacterSpacing`, `LineSpacingClone` -> `LineSpacing`).
pub fn canonical_source(src: &str) -> &str {
    src.strip_suffix("Clone").unwrap_or(src)
}

fn round_clean(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

pub fn format_hex_rgba(r: f64, g: f64, b: f64, a: f64) -> String {
    let to = |x: f64| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    let (r, g, b, a) = (to(r), to(g), to(b), to(a));
    if a == 255 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}

pub fn parse_hex_rgba(s: &str) -> Option<[f64; 4]> {
    let s = s.trim().trim_start_matches('#');
    let b = |i: usize| u8::from_str_radix(s.get(i..i + 2)?, 16).ok().map(|x| x as f64 / 255.0);
    match s.len() {
        6 => Some([b(0)?, b(2)?, b(4)?, 1.0]),
        8 => Some([b(0)?, b(2)?, b(4)?, b(6)?]),
        _ => None,
    }
}

fn is_skipped_input(e: &ExposedInput) -> bool {
    if e.id.starts_with("MainInput") {
        return true;
    }
    let src = canonical_source(&e.source);
    if src.ends_with("Sep") || src.ends_with("Separator") || src.ends_with("Label") || src.ends_with("Nest") {
        return true;
    }
    matches!(
        src,
        "Input"
            | "Background"
            | "Foreground"
            | "EffectMask"
            | "Keyframes"
            | "MaterialInput"
            | "SceneInput"
            | "Image"
            | "MotionBlur"
            | "Quality"
            | "ShutterAngle"
            | "CenterBias"
            | "SampleSpread"
            | "Comments"
            | "Blank1"
            | "Blank2"
            | "Blank3"
            | "Blank4"
            | "Blank5"
            | "Blank6"
            | "VerticalJustificationTop"
            | "HorizontalJustificationLeft"
            | "UseFrameFormatSettings"
            | "Depth"
            | "ClippingMode"
            | "FilterMethod"
            | "SubPixel"
            | "ReferenceSize"
            | "WidthResolution"
            | "HeightResolution"
    )
}

/// Если `src` — красный канал цветового набора, возвращает `(ключ_цвета, [R, G, B], Option<A>, базовое_имя)`.
fn color_channel_spec(src: &str) -> Option<(&'static str, [&'static str; 3], Option<&'static str>, &'static str)> {
    match canonical_source(src) {
        "TopLeftRed" => Some(("TopLeftColor", ["TopLeftRed", "TopLeftGreen", "TopLeftBlue"], Some("TopLeftAlpha"), "Цвет")),
        "TopRightRed" => Some(("TopRightColor", ["TopRightRed", "TopRightGreen", "TopRightBlue"], Some("TopRightAlpha"), "Верхний правый")),
        "BottomLeftRed" => Some(("BottomLeftColor", ["BottomLeftRed", "BottomLeftGreen", "BottomLeftBlue"], Some("BottomLeftAlpha"), "Нижний левый")),
        "BottomRightRed" => Some(("BottomRightColor", ["BottomRightRed", "BottomRightGreen", "BottomRightBlue"], Some("BottomRightAlpha"), "Нижний правый")),
        "Red1" => Some(("Color1", ["Red1", "Green1", "Blue1"], Some("Alpha1"), "Цвет текста")),
        "Red2" => Some(("Color2", ["Red2", "Green2", "Blue2"], Some("Alpha2"), "Цвет 2")),
        "Red3" => Some(("Color3", ["Red3", "Green3", "Blue3"], Some("Alpha3"), "Цвет 3")),
        "Color1Red" => Some(("Color1", ["Color1Red", "Color1Green", "Color1Blue"], Some("Color1Alpha"), "Цвет 1")),
        "Color2Red" => Some(("Color2", ["Color2Red", "Color2Green", "Color2Blue"], Some("Color2Alpha"), "Цвет 2")),
        "Red" => Some(("Color", ["Red", "Green", "Blue"], Some("Alpha"), "Цвет")),
        _ => None,
    }
}

fn is_point_source(src: &str) -> bool {
    matches!(
        canonical_source(src),
        "Center" | "Pivot" | "Start" | "End" | "Offset1" | "Offset2" | "Point1" | "Point2" | "Point3" | "Axis" | "TopLeft" | "TopRight" | "BottomLeft" | "BottomRight"
    )
}

fn is_bool_source(src: &str) -> bool {
    matches!(
        canonical_source(src),
        "Invert" | "Solid" | "FlipHoriz" | "FlipVert" | "LockXY" | "UseSizeAndAspect" | "Enabled" | "ProcessRed" | "ProcessGreen" | "ProcessBlue" | "ProcessAlpha"
    )
}

fn is_text_source(tool_ty: &str, src: &str) -> bool {
    matches!(canonical_source(src), "StyledText") || (tool_ty == "StyledTextFollower" && canonical_source(src) == "Text")
}

fn choice_spec(tool_ty: &str, src: &str) -> Option<(String, Vec<ChoiceItem>)> {
    let items = |pairs: &[(&str, &str)]| -> Vec<ChoiceItem> {
        pairs.iter().map(|(v, l)| ChoiceItem { value: (*v).into(), label: (*l).into() }).collect()
    };
    match (tool_ty, canonical_source(src)) {
        ("Background", "Type") => Some((
            "Solid".into(),
            items(&[
                ("Solid", "Сплошной"),
                ("Horizontal", "Горизонтальный"),
                ("Vertical", "Вертикальный"),
                ("FourCorner", "4 угла"),
                ("Gradient", "Градиент"),
            ]),
        )),
        ("Background", "GradientType") => Some((
            "Linear".into(),
            items(&[
                ("Linear", "Линейный"),
                ("Radial", "Радиальный"),
                ("Reflect", "Отражённый"),
                ("Square", "Квадрат"),
                ("Angle", "Угловой"),
            ]),
        )),
        ("Merge" | "Fuse.Duplicate", "ApplyMode") => Some((
            "Normal".into(),
            items(&[
                ("Normal", "Обычный"),
                ("Screen", "Экран"),
                ("Multiply", "Умножение"),
                ("Overlay", "Перекрытие"),
                ("SoftLight", "Мягкий свет"),
                ("HardLight", "Жёсткий свет"),
                ("Darken", "Затемнение"),
                ("Lighten", "Осветление"),
                ("Difference", "Разница"),
            ]),
        )),
        ("Merge", "Operator") => Some((
            "Over".into(),
            items(&[
                ("Over", "Поверх"),
                ("In", "Внутри"),
                ("Held Out", "Вырез"),
                ("Atop", "Наверх"),
                ("XOr", "Исключающее ИЛИ"),
            ]),
        )),
        ("TextPlus", "Style") => Some((
            "Regular".into(),
            items(&[
                ("Regular", "Обычный"),
                ("Medium", "Средний"),
                ("SemiBold", "Полужирный"),
                ("Bold", "Жирный"),
                ("ExtraBold", "Сверхжирный"),
                ("Black", "Чёрный"),
                ("Light", "Тонкий"),
                ("Italic", "Курсив"),
                ("Bold Italic", "Жирный курсив"),
            ]),
        )),
        ("BitmapMask", "Channel") => Some((
            "Alpha".into(),
            items(&[
                ("Alpha", "Альфа"),
                ("Luminance", "Яркость"),
                ("Red", "Красный"),
                ("Green", "Зелёный"),
                ("Blue", "Синий"),
            ]),
        )),
        _ => None,
    }
}

fn numeric_range(tool_ty: &str, src: &str, def: f64, min_s: Option<f64>, max_s: Option<f64>) -> (f64, f64, f64) {
    let (mut lo, mut hi, step) = match (tool_ty, canonical_source(src)) {
        ("TextPlus", "Size") => (0.005, 0.5, 0.002),
        (_, "Size" | "XSize" | "YSize" | "Scale" | "LayoutSize" | "Aspect") => (0.0, 5.0, 0.01),
        (_, "Angle" | "AngleZ" | "AngleX" | "AngleY" | "dropAngle" | "BlurAngle") => (-360.0, 360.0, 0.5),
        (_, "Width" | "Height" | "CornerRadius" | "SoftEdge" | "BorderWidth") => (0.0, 2.0, 0.005),
        (_, "Blend" | "Opacity1" | "Alpha1" | "AlphaGain" | "Mix" | "Level" | "WriteLength" | "WritePosition" | "Low" | "High" | "shadowStrength") => (0.0, 1.0, 0.01),
        ("TextPlus", "Start" | "End") => (0.0, 1.0, 0.01),
        (_, "CharacterSpacing" | "LineSpacing") => (0.5, 3.0, 0.01),
        (_, "HorizontalLeftCenterRight" | "VerticalTopCenterBottom") => (-1.0, 1.0, 0.05),
        (_, "GlowSize" | "XBlurSize" | "YBlurSize" | "Blur") => (0.0, 100.0, 0.5),
        (_, "Glow" | "Gain" | "Gamma" | "Saturation" | "XAmount") => (0.0, 4.0, 0.02),
        (_, "Brightness" | "Contrast" | "Lift") => (-1.0, 1.0, 0.01),
        (_, "Copies") => (1.0, 32.0, 1.0),
        (_, "TimeOffset") => (-60.0, 60.0, 0.5),
        (_, "Length" | "SmoothStrength" | "BlurStrength" | "dropDistance") => (0.0, 1.0, 0.005),
        (_, "PixelFrequency") => (1.0, 200.0, 1.0),
        _ => {
            if let (Some(a), Some(b)) = (min_s, max_s) {
                let s = ((b - a) / 100.0).max(0.001);
                (a, b, s)
            } else if (0.0..=1.0).contains(&def) {
                (0.0, 2.0, 0.01)
            } else if def >= 0.0 {
                (0.0, (def * 3.0).max(10.0), 0.1)
            } else {
                (def * 2.0 - 1.0, def.abs() * 2.0 + 1.0, 0.05)
            }
        }
    };
    if let Some(a) = min_s {
        lo = lo.min(a);
    }
    if let Some(b) = max_s {
        hi = hi.max(b);
    }
    if def < lo {
        lo = (def * 1.2 - 0.1).floor();
    }
    if def > hi {
        hi = (def * 1.5 + 0.1).ceil();
    }
    (lo, hi, step)
}

fn pretty_label(raw_name: &str, src: &str, fallback: &str) -> String {
    let base = if !raw_name.is_empty() && raw_name != src { raw_name } else { fallback };
    match canonical_source(base) {
        "StyledText" | "Text" => "Текст".into(),
        "Font" => "Шрифт".into(),
        "Style" => "Начертание".into(),
        "Size" => "Размер".into(),
        "XSize" => "Масштаб X".into(),
        "YSize" => "Масштаб Y".into(),
        "Center" | "Position" => "Положение".into(),
        "Pivot" => "Опорная точка".into(),
        "Angle" | "AngleZ" => "Поворот, °".into(),
        "Width" => "Ширина".into(),
        "Height" => "Высота".into(),
        "CornerRadius" => "Скругление".into(),
        "SoftEdge" => "Мягкость края".into(),
        "BorderWidth" => "Толщина рамки".into(),
        "CharacterSpacing" | "Tracking" => "Межбуквенный интервал".into(),
        "LineSpacing" => "Межстрочный интервал".into(),
        "Color" | "Top Left" => "Цвет".into(),
        "Top Right" => "Цвет (верх-право)".into(),
        "Bottom Left" => "Цвет (низ-лево)".into(),
        "Bottom Right" => "Цвет (низ-право)".into(),
        "Type" => "Тип заливки".into(),
        "GradientType" => "Тип градиента".into(),
        "Start" => "Начало".into(),
        "End" => "Конец".into(),
        "Blend" => "Непрозрачность".into(),
        "Opacity1" => "Прозрачность".into(),
        "Invert" => "Инвертировать".into(),
        "Solid" => "Заливка".into(),
        "FlipHoriz" => "Отразить по гориз.".into(),
        "FlipVert" => "Отразить по верт.".into(),
        "GlowSize" => "Радиус свечения".into(),
        "Glow" => "Сила свечения".into(),
        "XBlurSize" => "Размытие".into(),
        "HorizontalLeftCenterRight" => "Выравнивание X".into(),
        "VerticalTopCenterBottom" => "Выравнивание Y".into(),
        "Copies" => "Копии".into(),
        "TimeOffset" => "Сдвиг времени".into(),
        other => other.replace('_', " "),
    }
}

fn pretty_group(op: &str) -> String {
    op.replace('_', " ")
}

impl Comp {
    /// Прочитать текст по умолчанию у `TextPlus` (включая подключённый `StyledTextFollower`).
    pub fn read_text_default(&self, tool_name: &str) -> String {
        if let Some(inp) = self.resolve_input(tool_name, "StyledText") {
            if let Some(s) = inp.get("Value").and_then(Lua::str) {
                return s.to_string();
            }
            if let Some(sop) = inp.get("SourceOp").and_then(Lua::str) {
                if let Some(t) = self.tools.get(sop) {
                    if let Some(s) = t.raw.input("Text").and_then(Lua::str) {
                        return s.to_string();
                    }
                    if let Some(s) = t.raw.input("Value").and_then(Lua::str) {
                        return s.to_string();
                    }
                }
            }
        }
        String::new()
    }

    /// Прочитать цвета точек градиента узла `Background`.
    fn gradient_stop_hexes(&self, tool_name: &str) -> Vec<String> {
        let Some(colors) = self.resolve_input(tool_name, "Gradient").and_then(|i| i.get("Value")).and_then(|g| g.get("Colors")) else {
            return Vec::new();
        };
        let mut stops: Vec<(f64, String)> = Vec::new();
        for (k, v) in colors.items() {
            let Some(pos) = k.as_deref().and_then(|s| s.parse::<f64>().ok()) else { continue };
            let ns: Vec<f64> = v.items().iter().filter_map(|(_, x)| x.num()).collect();
            let r = ns.first().copied().unwrap_or(0.0);
            let g = ns.get(1).copied().unwrap_or(0.0);
            let b = ns.get(2).copied().unwrap_or(0.0);
            let a = ns.get(3).copied().unwrap_or(1.0);
            stops.push((pos, format_hex_rgba(r, g, b, a)));
        }
        stops.sort_by(|a, b| a.0.total_cmp(&b.0));
        stops.into_iter().map(|(_, hex)| hex).collect()
    }

    /// Все вынесенные параметры `MacroOperator` (сгруппированные цвета, точки,
    /// тексты, шрифты, списки, числа и точки градиента).
    pub fn all_template_params(&self) -> Vec<TemplateParam> {
        let mut out = Vec::new();
        let mut seen_keys: HashSet<String> = HashSet::new();
        let mut consumed: HashSet<usize> = HashSet::new();

        for (idx, exp) in self.exposed.iter().enumerate() {
            if consumed.contains(&idx) || is_skipped_input(exp) {
                continue;
            }
            let tool_ty = self.tools.get(&exp.source_op).map(|t| t.ty.as_str()).unwrap_or("");
            let csrc = canonical_source(&exp.source);
            let group = exp.page.clone().filter(|p| !p.is_empty()).unwrap_or_else(|| pretty_group(&exp.source_op));

            // 1. Цветовая группа RGBA (TopLeftRed/Green/Blue/Alpha, Red1/Green1/Blue1/Alpha1 и т.д.)
            if let Some((ckey, [r_ch, g_ch, b_ch], a_ch, fallback_lbl)) = color_channel_spec(&exp.source) {
                // Помечаем парные каналы G/B/A того же узла (в той же ControlGroup или рядом)
                let mut has_alpha = false;
                for (j, other) in self.exposed.iter().enumerate().skip(idx + 1) {
                    if other.source_op != exp.source_op {
                        continue;
                    }
                    let same_cg = match (exp.control_group, other.control_group) {
                        (Some(a), Some(b)) => a == b,
                        _ => j <= idx + 4,
                    };
                    if !same_cg {
                        continue;
                    }
                    let oc = canonical_source(&other.source);
                    if oc == g_ch || oc == b_ch {
                        consumed.insert(j);
                    } else if a_ch.is_some_and(|a| oc == a) {
                        consumed.insert(j);
                        has_alpha = true;
                    }
                }
                let key = format!("{}.{ckey}", exp.source_op);
                if !seen_keys.insert(key.clone()) {
                    continue;
                }
                let def_ch = |ch: &str, d: f64| {
                    // Сначала фактическое значение инструмента в шаблоне, затем Default из InstanceInput
                    if self.resolve_input(&exp.source_op, ch).is_some() {
                        eval_num(self, &exp.source_op, ch, d, 0.0, self.source_frames, 0)
                    } else {
                        self.exposed
                            .iter()
                            .find(|x| x.source_op == exp.source_op && canonical_source(&x.source) == ch)
                            .and_then(|x| x.default)
                            .unwrap_or(d)
                    }
                };
                let (dr, dg, db, da) = if ckey == "Color1" && tool_ty == "TextPlus" { (1.0, 1.0, 1.0, 1.0) } else { (0.0, 0.0, 0.0, 1.0) };
                let r = def_ch(r_ch, dr);
                let g = def_ch(g_ch, dg);
                let b = def_ch(b_ch, db);
                let a = a_ch.map(|ac| def_ch(ac, da)).unwrap_or(1.0);
                let label = pretty_label(&exp.name, &exp.source, fallback_lbl);
                out.push(TemplateParam {
                    key,
                    group,
                    label,
                    source_op: exp.source_op.clone(),
                    source: ckey.into(),
                    kind: ParamKind::Color {
                        default: format_hex_rgba(r, g, b, a),
                        channels: [r_ch.into(), g_ch.into(), b_ch.into()],
                        alpha: if has_alpha { a_ch.map(str::to_string) } else { a_ch.map(str::to_string) },
                    },
                });
                continue;
            }

            // Пропускаем «осиротевшие» каналы Green/Blue/Alpha, если вдруг Red не был перед ними
            if matches!(
                csrc,
                "TopLeftGreen"
                    | "TopLeftBlue"
                    | "TopLeftAlpha"
                    | "TopRightGreen"
                    | "TopRightBlue"
                    | "TopRightAlpha"
                    | "BottomLeftGreen"
                    | "BottomLeftBlue"
                    | "BottomLeftAlpha"
                    | "BottomRightGreen"
                    | "BottomRightBlue"
                    | "BottomRightAlpha"
                    | "Green1"
                    | "Blue1"
                    | "Alpha1"
                    | "Green2"
                    | "Blue2"
                    | "Alpha2"
                    | "Green3"
                    | "Blue3"
                    | "Alpha3"
                    | "Color1Green"
                    | "Color1Blue"
                    | "Color1Alpha"
                    | "Color2Green"
                    | "Color2Blue"
                    | "Color2Alpha"
            ) {
                continue;
            }

            // 2. Градиент Background — выносим цвета точек градиента (0..N, до 4 штук)
            if csrc == "Gradient" {
                let stops = self.gradient_stop_hexes(&exp.source_op);
                for (si, hex) in stops.into_iter().take(4).enumerate() {
                    let key = format!("{}.GradColor{si}", exp.source_op);
                    if !seen_keys.insert(key.clone()) {
                        continue;
                    }
                    out.push(TemplateParam {
                        key,
                        group: group.clone(),
                        label: format!("Градиент {}", si + 1),
                        source_op: exp.source_op.clone(),
                        source: format!("GradColor{si}"),
                        kind: ParamKind::GradStop { default: hex, stop_index: si },
                    });
                }
                continue;
            }

            let key = format!("{}.{csrc}", exp.source_op);
            if !seen_keys.insert(key.clone()) {
                continue;
            }

            // 3. Текст
            if is_text_source(tool_ty, csrc) {
                let default = self.read_text_default(&exp.source_op);
                out.push(TemplateParam {
                    key,
                    group,
                    label: pretty_label(&exp.name, &exp.source, "Текст"),
                    source_op: exp.source_op.clone(),
                    source: csrc.into(),
                    kind: ParamKind::Text { default },
                });
                continue;
            }

            // 4. Шрифт
            if csrc == "Font" {
                let default = eval_str(self, &exp.source_op, "Font", "").to_string();
                out.push(TemplateParam {
                    key,
                    group,
                    label: pretty_label(&exp.name, &exp.source, "Шрифт"),
                    source_op: exp.source_op.clone(),
                    source: "Font".into(),
                    kind: ParamKind::Font { default },
                });
                continue;
            }

            // 5. Перечисление (Background.Type, GradientType, TextPlus.Style, ApplyMode…)
            if let Some((fallback_def, mut items)) = choice_spec(tool_ty, csrc) {
                let cur = eval_str(self, &exp.source_op, csrc, &fallback_def).to_string();
                let default = if cur.is_empty() { fallback_def } else { cur };
                if !items.iter().any(|i| i.value == default) {
                    items.insert(0, ChoiceItem { value: default.clone(), label: default.clone() });
                }
                out.push(TemplateParam {
                    key,
                    group,
                    label: pretty_label(&exp.name, &exp.source, csrc),
                    source_op: exp.source_op.clone(),
                    source: csrc.into(),
                    kind: ParamKind::Choice { default, items },
                });
                continue;
            }

            // 6. 2D точка (Center, Pivot, Start, End…) — но не TextPlus.Start/End (это WriteOn 0..1!)
            let is_pt = (is_point_source(csrc) && !(tool_ty == "TextPlus" && matches!(csrc, "Start" | "End")))
                || exp.default_xy.is_some()
                || self.resolve_input(&exp.source_op, csrc).and_then(|i| i.get("Value")).and_then(Lua::point).is_some();
            if is_pt {
                let fallback = exp.default_xy.unwrap_or((0.5, 0.5));
                let def = if self.resolve_input(&exp.source_op, csrc).is_some() {
                    eval_point(self, &exp.source_op, csrc, fallback, 0.0, self.source_frames, 0)
                } else {
                    fallback
                };
                let lo = (-0.5f64).min(def.0 - 0.5).min(def.1 - 0.5);
                let hi = (1.5f64).max(def.0 + 0.5).max(def.1 + 0.5);
                out.push(TemplateParam {
                    key,
                    group,
                    label: pretty_label(&exp.name, &exp.source, csrc),
                    source_op: exp.source_op.clone(),
                    source: csrc.into(),
                    kind: ParamKind::Point { default: def, min: lo, max: hi, step: 0.005 },
                });
                continue;
            }

            // 7. Флаг
            if is_bool_source(csrc) {
                let d0 = exp.default.unwrap_or(0.0);
                let val = if self.resolve_input(&exp.source_op, csrc).is_some() {
                    eval_num(self, &exp.source_op, csrc, d0, 0.0, self.source_frames, 0)
                } else {
                    d0
                };
                out.push(TemplateParam {
                    key,
                    group,
                    label: pretty_label(&exp.name, &exp.source, csrc),
                    source_op: exp.source_op.clone(),
                    source: csrc.into(),
                    kind: ParamKind::Bool { default: val > 0.5 },
                });
                continue;
            }

            // 8. Число / ползунок
            let d0 = exp.default.unwrap_or(0.0);
            let def = if self.resolve_input(&exp.source_op, csrc).is_some() {
                eval_num(self, &exp.source_op, csrc, d0, 0.0, self.source_frames, 0)
            } else {
                d0
            };
            let (min, max, step) = numeric_range(tool_ty, csrc, def, exp.min_scale, exp.max_scale);
            out.push(TemplateParam {
                key,
                group,
                label: pretty_label(&exp.name, &exp.source, csrc),
                source_op: exp.source_op.clone(),
                source: csrc.into(),
                kind: ParamKind::Number { default: def, min, max, step },
            });
        }

        out
    }

    /// Видимые в инспекторе параметры с учётом текущего `Background.Type`
    /// (для `Solid` скрывает лишние углы/градиент, для `Gradient` — 4 угла).
    pub fn visible_template_params(&self, style: &Map<String, Value>) -> Vec<TemplateParam> {
        let all = self.all_template_params();
        let mut bg_types: HashMap<String, String> = HashMap::new();
        for (name, t) in &self.tools {
            if t.ty == "Background" {
                let sk = format!("{STYLE_PREFIX}{name}.Type");
                let ty = style
                    .get(&sk)
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| eval_str(self, name, "Type", "Solid").to_string());
                bg_types.insert(name.clone(), ty);
            }
        }
        all.into_iter()
            .filter(|p| {
                let Some(bg_ty) = bg_types.get(&p.source_op) else { return true };
                match p.source.as_str() {
                    "TopLeftColor" => bg_ty != "Gradient",
                    "TopRightColor" => matches!(bg_ty.as_str(), "Horizontal" | "FourCorner"),
                    "BottomLeftColor" => matches!(bg_ty.as_str(), "Vertical" | "FourCorner"),
                    "BottomRightColor" => bg_ty == "FourCorner",
                    "GradientType" | "Start" | "End" => bg_ty == "Gradient",
                    s if s.starts_with("GradColor") => bg_ty == "Gradient",
                    _ => true,
                }
            })
            .collect()
    }

    /// Значение по умолчанию для ключа `p:...` (включая `...Center.x` / `...Center.y`).
    pub fn param_default(&self, param_key: &str) -> Option<Value> {
        let k = param_key.strip_prefix(STYLE_PREFIX).unwrap_or(param_key);
        let all = self.all_template_params();
        if let Some(p) = all.iter().find(|p| p.key == k) {
            return Some(p.default_json());
        }
        if let Some(base) = k.strip_suffix(".x") {
            if let Some(TemplateParam { kind: ParamKind::Point { default, .. }, .. }) = all.iter().find(|p| p.key == base) {
                return Some(json!(round_clean(default.0)));
            }
        }
        if let Some(base) = k.strip_suffix(".y") {
            if let Some(TemplateParam { kind: ParamKind::Point { default, .. }, .. }) = all.iter().find(|p| p.key == base) {
                return Some(json!(round_clean(default.1)));
            }
        }
        None
    }

    /// Человекочитаемая подпись параметра по ключу (для панели «Кривые» и команд).
    pub fn param_label(&self, param_key: &str) -> Option<String> {
        let k = param_key.strip_prefix(STYLE_PREFIX).unwrap_or(param_key);
        let all = self.all_template_params();
        if let Some(p) = all.iter().find(|p| p.key == k) {
            return Some(format!("{}: {}", p.group, p.label));
        }
        if let Some(base) = k.strip_suffix(".x") {
            if let Some(p) = all.iter().find(|p| p.key == base) {
                return Some(format!("{}: {} X", p.group, p.label));
            }
        }
        if let Some(base) = k.strip_suffix(".y") {
            if let Some(p) = all.iter().find(|p| p.key == base) {
                return Some(format!("{}: {} Y", p.group, p.label));
            }
        }
        None
    }
}

/// Собрать `Overrides` для исполнителя графа Fusion из `Source::Title { text, style }`.
pub fn overrides_from_title(comp: &Comp, text: &str, style: &Map<String, Value>) -> Overrides {
    let mut ov = Overrides {
        texts: if text.is_empty() { Vec::new() } else { text.split('\n').map(str::to_string).collect() },
        font: style.get("font").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string),
        ..Default::default()
    };

    let params = comp.all_template_params();
    for p in &params {
        let sk = p.style_key();
        match &p.kind {
            ParamKind::Text { .. } => {
                if let Some(s) = style.get(&sk).and_then(Value::as_str) {
                    ov.strs.insert((p.source_op.clone(), p.source.clone()), s.to_string());
                }
            }
            ParamKind::Font { .. } => {
                if let Some(s) = style.get(&sk).and_then(Value::as_str).filter(|s| !s.is_empty()) {
                    ov.strs.insert((p.source_op.clone(), "Font".into()), s.to_string());
                }
            }
            ParamKind::Choice { .. } => {
                if let Some(s) = style.get(&sk).and_then(Value::as_str) {
                    ov.strs.insert((p.source_op.clone(), p.source.clone()), s.to_string());
                }
            }
            ParamKind::Number { .. } => {
                if let Some(v) = style.get(&sk).and_then(Value::as_f64) {
                    ov.nums.insert((p.source_op.clone(), p.source.clone()), v);
                }
            }
            ParamKind::Bool { .. } => {
                if let Some(v) = style.get(&sk) {
                    let b = v.as_bool().or_else(|| v.as_f64().map(|n| n > 0.5));
                    if let Some(b) = b {
                        ov.nums.insert((p.source_op.clone(), p.source.clone()), if b { 1.0 } else { 0.0 });
                    }
                }
            }
            ParamKind::Point { default, .. } => {
                let x = style.get(&p.style_key_sub("x")).and_then(Value::as_f64);
                let y = style.get(&p.style_key_sub("y")).and_then(Value::as_f64);
                let pair = style.get(&sk).and_then(|v| {
                    v.as_array().and_then(|a| Some((a.first()?.as_f64()?, a.get(1)?.as_f64()?))).or_else(|| {
                        let o = v.as_object()?;
                        Some((o.get("x")?.as_f64()?, o.get("y")?.as_f64()?))
                    })
                });
                if x.is_some() || y.is_some() || pair.is_some() {
                    let (px, py) = pair.unwrap_or(*default);
                    ov.points.insert((p.source_op.clone(), p.source.clone()), (x.unwrap_or(px), y.unwrap_or(py)));
                }
            }
            ParamKind::Color { channels, alpha, .. } => {
                if let Some(hex) = style.get(&sk).and_then(Value::as_str) {
                    if let Some([r, g, b, a]) = parse_hex_rgba(hex) {
                        ov.nums.insert((p.source_op.clone(), channels[0].clone()), r);
                        ov.nums.insert((p.source_op.clone(), channels[1].clone()), g);
                        ov.nums.insert((p.source_op.clone(), channels[2].clone()), b);
                        if let Some(ach) = alpha {
                            // Если цвет задан в #rrggbb (6 знаков), альфу не затираем, если не указана явно
                            if hex.trim().trim_start_matches('#').len() == 8 {
                                ov.nums.insert((p.source_op.clone(), ach.clone()), a);
                            }
                        }
                    }
                }
            }
            ParamKind::GradStop { stop_index, .. } => {
                if let Some(hex) = style.get(&sk).and_then(Value::as_str) {
                    if let Some([r, g, b, a]) = parse_hex_rgba(hex) {
                        ov.grad_colors.insert((p.source_op.clone(), *stop_index), Color4::from_straight(r as f32, g as f32, b as f32, a as f32));
                    }
                }
            }
        }
    }

    // Fallback: прямые ключи `p:Tool.Input` даже если их не было в MacroOperator (полезно агенту/скриптам)
    for (k, v) in style {
        let Some(rest) = k.strip_prefix(STYLE_PREFIX) else { continue };
        let Some((op, src)) = rest.split_once('.') else { continue };
        if src.contains('.') || src.ends_with("Color") || src.starts_with("GradColor") {
            continue;
        }
        let csrc = canonical_source(src);
        let key = (op.to_string(), csrc.to_string());
        if let Some(n) = v.as_f64() {
            ov.nums.entry(key).or_insert(n);
        } else if let Some(b) = v.as_bool() {
            ov.nums.entry(key).or_insert(if b { 1.0 } else { 0.0 });
        } else if let Some(s) = v.as_str() {
            ov.strs.entry(key).or_insert_with(|| s.to_string());
        }
    }

    ov
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fusion::{eval, parse};

    #[test]
    fn groups_macro_inputs_and_overrides_render() {
        let setting = r#"{
            Tools = ordered() {
                MyMacro = MacroOperator {
                    Inputs = ordered() {
                        Input1 = InstanceInput { SourceOp = "Xf", Source = "Center", Name = "Position", },
                        Input2 = InstanceInput { SourceOp = "Xf", Source = "Size", Default = 1, },
                        Input3 = InstanceInput { SourceOp = "Bg", Source = "Type", },
                        Input4 = InstanceInput { SourceOp = "Bg", Source = "TopLeftRed", Name = "Color", ControlGroup = 3, Default = 0, },
                        Input5 = InstanceInput { SourceOp = "Bg", Source = "TopLeftGreen", ControlGroup = 3, Default = 0, },
                        Input6 = InstanceInput { SourceOp = "Bg", Source = "TopLeftBlue", ControlGroup = 3, Default = 1, },
                        Input7 = InstanceInput { SourceOp = "Bg", Source = "TopLeftAlpha", ControlGroup = 3, Default = 1, },
                        Input8 = InstanceInput { SourceOp = "Txt", Source = "StyledText", },
                        Input9 = InstanceInput { SourceOp = "Txt", Source = "Red1Clone", Name = "Color", ControlGroup = 5, Default = 1, },
                        Input10 = InstanceInput { SourceOp = "Txt", Source = "Green1Clone", ControlGroup = 5, Default = 1, },
                        Input11 = InstanceInput { SourceOp = "Txt", Source = "Blue1Clone", ControlGroup = 5, Default = 1, },
                    },
                    Outputs = { MainOutput1 = InstanceOutput { SourceOp = "M", Source = "Output", }, },
                    Tools = ordered() {
                        Bg = Background { Inputs = { TopLeftRed = Input { Value = 0, }, TopLeftGreen = Input { Value = 0, }, TopLeftBlue = Input { Value = 1, }, TopLeftAlpha = Input { Value = 1, }, }, },
                        Txt = TextPlus { Inputs = { StyledText = Input { Value = "HI", }, Size = Input { Value = 0.2, }, Red1 = Input { Value = 1, }, Green1 = Input { Value = 1, }, Blue1 = Input { Value = 1, }, }, },
                        Xf = Transform { Inputs = { Input = Input { SourceOp = "Txt", Source = "Output", }, Size = Input { Value = 1, }, Center = Input { Value = { 0.5, 0.5 }, }, }, },
                        M = Merge { Inputs = { Background = Input { SourceOp = "Bg", Source = "Output", }, Foreground = Input { SourceOp = "Xf", Source = "Output", }, }, },
                    },
                },
            },
        }"#;
        let root = parse(setting).unwrap();
        let comp = Comp::from_lua("TestMacro", &root);
        let params = comp.all_template_params();
        let keys: Vec<&str> = params.iter().map(|p| p.key.as_str()).collect();
        assert_eq!(keys, vec!["Xf.Center", "Xf.Size", "Bg.Type", "Bg.TopLeftColor", "Txt.StyledText", "Txt.Color1"]);

        // Базовый рендер: синий фон (0, 0, 255)
        let base = eval::render(&comp, 64, 64, 0.5, 2.0, 30.0, &[], &Overrides::default());
        assert!(base.rgba[2] > 200 && base.rgba[0] < 30, "expected blue bg, got {:?}", &base.rgba[0..4]);

        // Переопределение цвета фона на красный через style["p:Bg.TopLeftColor"]
        let mut style = Map::new();
        style.insert("p:Bg.TopLeftColor".into(), json!("#ff0000"));
        style.insert("p:Xf.Center.x".into(), json!(0.2));
        let ov = overrides_from_title(&comp, "", &style);
        let red = eval::render(&comp, 64, 64, 0.5, 2.0, 30.0, &[], &ov);
        assert!(red.rgba[0] > 200 && red.rgba[2] < 30, "expected red bg after override, got {:?}", &red.rgba[0..4]);
    }
}
