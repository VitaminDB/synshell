//! Инспектор — `PropertyGrid` syngui в стиле приложения: разделы (проект,
//! клип, титр, переход, эффекты), числа в пределах протягиваются мышью
//! (щелчок — ввод), выбор — списком, цвет — палитрой, действия — кнопками в
//! строке. Каждая правка — команда; правки одного поля подряд — один шаг
//! отмены. Сетка перестраивается только при смене выделения или состава
//! (`St::insp_key`), поэтому протяжка не обрывается.

use std::sync::Arc;

use serde_json::{json, Map, Value};
use syngui::prelude::*;

use super::{ed, St};
use crate::doc::{tc, Source};

/// Ключ состава инспектора: меняется, только когда сетку надо перестроить.
pub fn key() -> String {
    let e = ed();
    let p = &e.project;
    match e.selection.first().and_then(|id| p.clip(*id)) {
        Some(c) => {
            let src = match &c.source {
                Source::Media { media } => format!("m{media}"),
                Source::Title { style, .. } => format!("title:{}", style.get("font").and_then(Value::as_str).unwrap_or("")),
                Source::Color { .. } => "color".into(),
            };
            let fx: Vec<String> = c.effects.iter().map(|e| format!("{}{}{}", e.id, e.kind, e.params.get("path").and_then(Value::as_str).unwrap_or(""))).collect();
            format!("c{}:{src}:{}:{}:{:.4}:{:.4}", c.id, fx.join(","), c.transition.as_ref().map(|t| t.kind.as_str()).unwrap_or("-"), c.start, c.duration)
        }
        None => format!("p:{}x{}@{}:{}:{}:{:.4}", p.settings.width, p.settings.height, p.settings.fps, p.media.len(), p.tracks.len(), p.duration()),
    }
}

type Handler = Arc<dyn Fn(St, PropertyValue) + Send + Sync>;

/// Строки сетки и обработчики их правок.
#[derive(Default)]
struct Rows {
    props: Vec<Property>,
    on: Vec<Option<Handler>>,
}

impl Rows {
    fn header(&mut self, name: &str) {
        self.props.push(Property::header(name));
        self.on.push(None);
    }
    fn info(&mut self, name: &str, v: impl Into<String>) {
        self.props.push(Property::text(name, v));
        self.on.push(None);
    }
    fn add(&mut self, p: Property, f: impl Fn(St, PropertyValue) + Send + Sync + 'static) {
        self.props.push(p);
        self.on.push(Some(Arc::new(f)));
    }
    /// Число в пределах: значение ровно по шагу.
    fn range(&mut self, name: &str, v: f64, min: f64, max: f64, step: f64, f: impl Fn(St, f64) + Send + Sync + 'static) {
        self.add(Property::range(name, v, min, max, step), move |st, pv| {
            if let PropertyValue::Range { value, .. } = pv {
                let k = (1.0 / step).round().max(1.0);
                f(st, (value * k).round() / k);
            }
        });
    }
    fn choice(&mut self, name: &str, items: &[(&str, &str)], cur: &str, f: impl Fn(St, String) + Send + Sync + 'static) {
        let keys: Vec<String> = items.iter().map(|(k, _)| k.to_string()).collect();
        let sel = keys.iter().position(|k| k == cur).unwrap_or(0);
        self.add(Property::choice(name, items.iter().map(|(_, t)| t.to_string()).collect(), sel), move |st, pv| {
            if let PropertyValue::Choice(_, i) = pv {
                if let Some(k) = keys.get(i) {
                    f(st, k.clone());
                }
            }
        });
    }
    fn action(&mut self, name: &str, label: &str, f: impl Fn(St) + Send + Sync + 'static) {
        self.add(Property::action(name, label), move |st, _| f(st));
    }
}

fn num(m: &Map<String, Value>, k: &str, d: f64) -> f64 {
    m.get(k).and_then(Value::as_f64).unwrap_or(d)
}

/// Параметр эффекта: имя, подпись, мин, макс, шаг, по умолчанию.
type Param = (&'static str, &'static str, f64, f64, f64, f64);

fn effect_params(kind: &str) -> &'static [Param] {
    match kind {
        "fade_in" | "fade_out" | "audio_fade_in" | "audio_fade_out" => &[("duration", "Длительность, с", 0.05, 5.0, 0.05, 0.5)],
        "lut" => &[("strength", "Сила", 0.0, 1.0, 0.01, 1.0)],
        "color" => &[("brightness", "Яркость", -1.0, 1.0, 0.01, 0.0), ("contrast", "Контраст", -1.0, 1.0, 0.01, 0.0), ("saturation", "Насыщенность", -1.0, 1.0, 0.01, 0.0), ("temperature", "Температура", -1.0, 1.0, 0.01, 0.0)],
        "blur" => &[("radius", "Радиус, пикс.", 0.0, 60.0, 0.5, 6.0)],
        "sharpen" => &[("amount", "Сила", 0.0, 2.0, 0.01, 0.5)],
        "vignette" => &[("amount", "Сила", 0.0, 1.0, 0.01, 0.5)],
        "transform" => &[("x", "Сдвиг X", -1.0, 1.0, 0.005, 0.0), ("y", "Сдвиг Y", -1.0, 1.0, 0.005, 0.0), ("scale", "Масштаб", 0.1, 4.0, 0.01, 1.0), ("rotation", "Поворот, °", -180.0, 180.0, 0.5, 0.0), ("opacity", "Непрозрачность", 0.0, 1.0, 0.01, 1.0)],
        _ => &[],
    }
}

pub(super) fn effect_title(kind: &str) -> &'static str {
    match kind {
        "fade_in" => "Проявление",
        "fade_out" => "Затухание",
        "audio_fade_in" => "Нарастание звука",
        "audio_fade_out" => "Затухание звука",
        "lut" => "LUT",
        "color" => "Цвет",
        "blur" => "Размытие",
        "sharpen" => "Резкость",
        "vignette" => "Виньетка",
        "transform" => "Положение и масштаб",
        "upscale" => "Нейроувеличение",
        "interpolate" => "Плавность RIFE",
        _ => "Эффект",
    }
}

const ROW_H: f32 = 28.0;

pub fn view(st: St) -> impl Widget {
    // ключ пересчитывается по каждой правке, сетка — только при его смене
    create_effect(move || {
        let _ = st.rev.get();
        let k = key();
        if st.insp_key.get_untracked() != k {
            st.insp_key.set(k);
        }
    });
    ScrollView::new().vertical().child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.insp_key.get();
        let (proj, sel) = {
            let e = ed();
            (e.project.clone(), e.selection.first().copied())
        };
        let mut r = Rows::default();
        let clip_sel = sel.and_then(|id| proj.clip(id).cloned());
        match &clip_sel {
            None => project(&mut r, &proj),
            Some(c) => clip(&mut r, &proj, c),
        }
        let handlers = r.on;
        let h = r.props.len() as f32 * ROW_H + 2.0;
        let grid = PropertyGrid::new()
            .properties(r.props)
            .label_width(150.0)
            .row_height(ROW_H)
            .height(h)
            .on_change(move |i, v| {
                if let Some(Some(f)) = handlers.get(i) {
                    f(st, v);
                }
            })
            .class("insp-grid");
        let mut col = Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).class("inspector").child(grid);
        if clip_sel.is_none() {
            col = col.child(Text::new("Выделите клип на таймлайне, чтобы править его").max_lines(2).class("panel-hint"));
        }
        vec![Box::new(col)]
    }))
}

fn project(r: &mut Rows, p: &crate::doc::Project) {
    r.header("Проект");
    r.add(Property::text("Название", p.name.clone()), |st, v| {
        if let PropertyValue::Text(s) = v {
            st.exec("project.settings", json!({"name": s}));
        }
    });
    let frame = format!("{}x{}", p.settings.width, p.settings.height);
    let mut items: Vec<(String, String)> = [("1920x1080", "1920×1080 (16:9)"), ("1080x1920", "1080×1920 (9:16, Reels)"), ("3840x2160", "3840×2160 (4K)"), ("2160x3840", "2160×3840 (4K, 9:16)"), ("1080x1080", "1080×1080 (1:1)"), ("1080x1350", "1080×1350 (4:5)"), ("1280x720", "1280×720")].iter().map(|(k, t)| (k.to_string(), t.to_string())).collect();
    if !items.iter().any(|(k, _)| *k == frame) {
        items.insert(0, (frame.clone(), frame.replace('x', "×")));
    }
    let refs: Vec<(&str, &str)> = items.iter().map(|(k, t)| (k.as_str(), t.as_str())).collect();
    r.choice("Кадр", &refs, &frame, |st, k| {
        if let Some((w, h)) = k.split_once('x') {
            st.exec("project.settings", json!({"width": w.parse::<u32>().unwrap_or(1920), "height": h.parse::<u32>().unwrap_or(1080)}));
        }
    });
    r.choice("Частота", &[("24", "24 fps"), ("25", "25 fps"), ("30", "30 fps"), ("50", "50 fps"), ("60", "60 fps")], &format!("{}", p.settings.fps), |st, k| {
        st.exec("project.settings", json!({"fps": k.parse::<f64>().unwrap_or(30.0)}));
    });
    r.info("Длительность", tc(p.duration()));
    r.info("Медиа", p.media.len().to_string());
    r.info("Дорожки", p.tracks.len().to_string());
}

fn clip(r: &mut Rows, p: &crate::doc::Project, c: &crate::doc::Clip) {
    let id = c.id;
    r.header(&format!("Клип c{id}"));
    let src = match &c.source {
        Source::Media { media } => p.media(*media).map(|m| format!("m{media} {}", m.name())).unwrap_or_default(),
        Source::Title { .. } => "титр".into(),
        Source::Color { color } => format!("цвет {color}"),
    };
    r.info("Источник", src);
    r.info("Время", format!("{} — {} ({})", tc(c.start), tc(c.end()), tc(c.duration)));
    r.add(Property::text("Имя", c.name.clone()), move |st, v| {
        if let PropertyValue::Text(s) = v {
            st.exec("clip.set", json!({"id": id, "name": s}));
        }
    });
    if c.media().is_some() {
        r.range("Скорость", c.speed, 0.1, 4.0, 0.05, move |st, x| {
            st.exec("clip.set", json!({"id": id, "speed": x}));
        });
        r.range("Громкость", c.volume, 0.0, 2.0, 0.01, move |st, x| {
            st.exec("clip.set", json!({"id": id, "volume": x}));
        });
    }
    if let Source::Title { text, style } = &c.source {
        r.header("Титр");
        // переносы строк — «\n» в поле
        r.add(Property::text("Текст", text.replace('\n', "\\n")), move |st, v| {
            if let PropertyValue::Text(s) = v {
                st.exec("clip.set", json!({"id": id, "text": s.replace("\\n", "\n")}));
            }
        });
        let h = p.settings.width.min(p.settings.height) as f64;
        r.range("Размер", num(style, "size", h * 0.075), 12.0, 400.0, 1.0, move |st, x| {
            st.exec("clip.set", json!({"id": id, "style": {"size": x}}));
        });
        let font = style.get("font").and_then(Value::as_str).unwrap_or("По умолчанию").to_string();
        r.action("Шрифт", &font, move |st| st.font_pick.set(Some(id)));
        let col = style.get("color").and_then(Value::as_str).unwrap_or("#ffffff");
        r.add(Property::color("Цвет", Color::from_hex(col)), move |st, v| {
            if let PropertyValue::Color(c) = v {
                st.exec("clip.set", json!({"id": id, "style": {"color": c.to_hex()}}));
            }
        });
        let pos = style.get("position").and_then(Value::as_str).unwrap_or("bottom");
        r.choice("Положение", &[("top", "Сверху"), ("center", "По центру"), ("bottom", "Снизу")], pos, move |st, k| {
            st.exec("clip.set", json!({"id": id, "style": {"position": k}}));
        });
        let anims = [("fade", "Проявление"), ("slide", "Подъём"), ("pop", "Выпрыгивание"), ("none", "Без анимации")];
        for (k, label) in [("in", "Появление"), ("out", "Исчезание")] {
            let cur = style.get(k).and_then(Value::as_str).unwrap_or("fade");
            r.choice(label, &anims, cur, move |st, x| {
                st.exec("clip.set", json!({"id": id, "style": {k: x}}));
            });
        }
        r.range("Длит. анимации, с", num(style, "anim", 0.4), 0.0, 2.0, 0.05, move |st, x| {
            st.exec("clip.set", json!({"id": id, "style": {"anim": x}}));
        });
        r.add(Property::boolean("Подложка", style.get("box").and_then(Value::as_bool).unwrap_or(false)), move |st, v| {
            if let PropertyValue::Bool(on) = v {
                st.exec("clip.set", json!({"id": id, "style": {"box": on}}));
            }
        });
    }
    if let Some(t) = &c.transition {
        r.header("Переход в начале");
        let dur = t.duration;
        let kinds: Vec<(&str, &str)> = crate::render::compose::TRANSITIONS.to_vec();
        r.choice("Вид", &kinds, &t.kind, move |st, k| {
            st.exec("transition.set", json!({"id": id, "kind": k, "duration": dur}));
        });
        let kind = t.kind.clone();
        r.range("Длительность, с", t.duration, 0.1, 3.0, 0.05, move |st, x| {
            st.exec("transition.set", json!({"id": id, "kind": kind, "duration": x}));
        });
        r.action("", "Убрать переход", move |st| {
            st.exec("transition.remove", json!({"id": id}));
        });
    }
    for ef in &c.effects {
        let eid = ef.id;
        r.header(&format!("Эффект: {}", effect_title(&ef.kind)));
        for &(name, label, min, max, step, def) in effect_params(&ef.kind) {
            r.range(label, num(&ef.params, name, def), min, max, step, move |st, x| {
                st.exec("effect.set", json!({"id": eid, "params": {name: x}}));
            });
        }
        match ef.kind.as_str() {
            "lut" => {
                let path = ef.params.get("path").and_then(Value::as_str).unwrap_or("");
                let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Выбрать…".into());
                r.action("Файл LUT", &name, move |st| pick_lut(st, eid));
            }
            "upscale" => {
                let models: Vec<(String, String)> = crate::render::neural::available().into_iter().filter(|(_, a)| a != "rife").map(|(id, a)| (id.clone(), format!("{id} ({a})"))).collect();
                let cur = ef.params.get("model").and_then(Value::as_str).unwrap_or("");
                let mut items: Vec<(&str, &str)> = vec![("", "Любая")];
                items.extend(models.iter().map(|(k, t)| (k.as_str(), t.as_str())));
                r.choice("Модель", &items, cur, move |st, k| {
                    st.exec("effect.set", json!({"id": eid, "params": {"model": k}}));
                });
            }
            "interpolate" => r.info("Частота", format!("до {} fps при экспорте", p.settings.fps)),
            _ => {}
        }
        r.action("", "Убрать эффект", move |st| {
            st.exec("effect.remove", json!({"id": eid}));
        });
    }
}

/// Файл LUT через окно выбора; папка — LUT библиотеки.
fn pick_lut(st: St, eid: u64) {
    std::thread::spawn(move || {
        let dir = crate::fusion::library().join("LUT");
        let mut d = rfd::FileDialog::new().set_title("Файл LUT").add_filter("LUT", &["cube"]);
        if dir.is_dir() {
            d = d.set_directory(dir);
        }
        if let Some(p) = d.pick_file() {
            let path = p.display().to_string();
            syngui::async_runtime::run_on_main_thread(move || {
                st.exec("effect.set", json!({"id": eid, "params": {"path": path}}));
            });
        }
    });
}
