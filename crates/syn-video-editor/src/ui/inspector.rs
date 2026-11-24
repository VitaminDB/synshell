//! Инспектор с правкой: клип (имя, скорость, громкость), титр (текст, размер,
//! цвет, положение, появление и исчезание, подложка), эффекты (параметры
//! ползунками, файл LUT, модель увеличения), переход (вид, длительность),
//! проект (название, кадр, частота). Каждое поле — команда; правки одного поля
//! подряд — один шаг отмены. Панель перестраивается только при смене
//! выделения или состава (ключ `St::insp_key`), поэтому ползунок не
//! пересоздаётся посреди перетаскивания.

use serde_json::{json, Map, Value};
use syngui::prelude::*;
use syngui::widgets::*;

use super::{ed, St};
use crate::doc::{tc, Source};

/// Ключ состава инспектора: меняется, только когда панель надо перестроить.
pub fn key() -> String {
    let e = ed();
    let p = &e.project;
    match e.selection.first().and_then(|id| p.clip(*id)) {
        Some(c) => {
            let src = match &c.source {
                Source::Media { media } => format!("m{media}"),
                Source::Title { .. } => "title".into(),
                Source::Color { .. } => "color".into(),
            };
            let fx: Vec<String> = c.effects.iter().map(|e| format!("{}{}", e.id, e.kind)).collect();
            format!("c{}:{src}:{}:{}", c.id, fx.join(","), c.transition.as_ref().map(|t| t.kind.as_str()).unwrap_or("-"))
        }
        None => format!("p:{}x{}@{}:{}", p.settings.width, p.settings.height, p.settings.fps, p.media.len()),
    }
}

fn row(label: &str, field: impl Widget + 'static) -> Box<dyn Widget> {
    Box::new(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).class("kv-row").child(Text::new(label).class("kv-key")).child(DecoratedBox::new().class("grow").child(field)))
}

fn info(label: &str, v: impl Into<String>) -> Box<dyn Widget> {
    row(label, Text::new(v.into()).max_lines(2).class("kv-value"))
}

fn section(t: &str) -> Box<dyn Widget> {
    Box::new(Text::new(t).class("section-title"))
}

fn slider(st: St, v: f32, min: f32, max: f32, step: f32, decimals: u8, f: impl Fn(St, f64) + Send + Sync + 'static) -> impl Widget {
    Slider::new().range(min, max).step(step).value(v.clamp(min, max)).show_value(decimals).value_width(52.0).on_change(move |x| {
        // ровно по шагу: в проекте 0.42, а не 0.41999998
        let k = (1.0 / step as f64).round().max(1.0);
        f(st, (x as f64 * k).round() / k)
    })
}

fn choice(cur: &str, items: &[(&str, &str)], f: impl Fn(&str) + Send + 'static) -> impl Widget {
    Dropdown::new().items(items.iter().map(|(k, t)| DropdownItem::new(*k, *t)).collect()).selected(cur.to_string()).on_change(move |k: &str| f(k))
}

fn num(m: &Map<String, Value>, k: &str, d: f64) -> f32 {
    m.get(k).and_then(Value::as_f64).unwrap_or(d) as f32
}

/// Параметр эффекта: имя, подпись, мин, макс, шаг, по умолчанию.
type Param = (&'static str, &'static str, f32, f32, f32, f32);

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

fn effect_title(kind: &str) -> &'static str {
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

pub fn view(st: St) -> impl Widget {
    // ключ пересчитывается по каждой правке, панель — только при его смене
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
        let mut col = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("inspector");
        match sel.and_then(|id| proj.clip(id).cloned()) {
            None => {
                for w in project(st, &proj) {
                    col = col.child(w);
                }
            }
            Some(c) => {
                for w in clip(st, &proj, &c) {
                    col = col.child(w);
                }
            }
        }
        vec![Box::new(col)]
    }))
}

fn project(st: St, p: &crate::doc::Project) -> Vec<Box<dyn Widget>> {
    let mut v = vec![section("ПРОЕКТ")];
    v.push(row("Название", TextField::new().text(p.name.clone()).submit_on_focus_lost(true).on_submit(move |s: &str| {
        st.exec("project.settings", json!({"name": s}));
    })));
    let frame = format!("{}x{}", p.settings.width, p.settings.height);
    let presets = [("1920x1080", "1920×1080 (16:9)"), ("1080x1920", "1080×1920 (9:16, Reels)"), ("3840x2160", "3840×2160 (4K)"), ("2160x3840", "2160×3840 (4K, 9:16)"), ("1080x1080", "1080×1080 (1:1)"), ("1080x1350", "1080×1350 (4:5)"), ("1280x720", "1280×720")];
    let mut items: Vec<(String, String)> = presets.iter().map(|(k, t)| (k.to_string(), t.to_string())).collect();
    if !items.iter().any(|(k, _)| *k == frame) {
        items.insert(0, (frame.clone(), frame.replace('x', "×")));
    }
    let refs: Vec<(&str, &str)> = items.iter().map(|(k, t)| (k.as_str(), t.as_str())).collect();
    v.push(row("Кадр", choice(&frame, &refs, move |k| {
        if let Some((w, h)) = k.split_once('x') {
            st.exec("project.settings", json!({"width": w.parse::<u32>().unwrap_or(1920), "height": h.parse::<u32>().unwrap_or(1080)}));
        }
    })));
    let fps = format!("{}", p.settings.fps);
    v.push(row("Частота", choice(&fps, &[("24", "24 fps"), ("25", "25 fps"), ("30", "30 fps"), ("50", "50 fps"), ("60", "60 fps")], move |k| {
        st.exec("project.settings", json!({"fps": k.parse::<f64>().unwrap_or(30.0)}));
    })));
    v.push(info("Длительность", tc(p.duration())));
    v.push(info("Медиа", p.media.len().to_string()));
    v.push(info("Дорожки", p.tracks.len().to_string()));
    v.push(Box::new(Text::new("Выделите клип на таймлайне, чтобы править его").max_lines(2).class("panel-hint")));
    v
}

fn clip(st: St, p: &crate::doc::Project, c: &crate::doc::Clip) -> Vec<Box<dyn Widget>> {
    let id = c.id;
    let mut v = vec![section(&format!("КЛИП C{id}"))];
    let src = match &c.source {
        Source::Media { media } => p.media(*media).map(|m| format!("m{media} {}", m.name())).unwrap_or_default(),
        Source::Title { .. } => "титр".into(),
        Source::Color { color } => format!("цвет {color}"),
    };
    v.push(info("Источник", src));
    v.push(info("Время", format!("{} — {} ({})", tc(c.start), tc(c.end()), tc(c.duration))));
    v.push(row("Имя", TextField::new().text(c.name.clone()).placeholder("как у файла").submit_on_focus_lost(true).on_submit(move |s: &str| {
        st.exec("clip.set", json!({"id": id, "name": s}));
    })));
    if c.media().is_some() {
        v.push(row("Скорость", slider(st, c.speed as f32, 0.1, 4.0, 0.05, 2, move |st, x| {
            st.exec("clip.set", json!({"id": id, "speed": x}));
        })));
        v.push(row("Громкость", slider(st, c.volume as f32, 0.0, 2.0, 0.01, 2, move |st, x| {
            st.exec("clip.set", json!({"id": id, "volume": x}));
        })));
    }
    if let Source::Title { text, style } = &c.source {
        v.push(section("ТИТР"));
        v.push(row("Текст", MultilineTextEdit::new().text(text.clone()).rows(3).auto_height(true).on_change(move |s: &str| {
            st.exec("clip.set", json!({"id": id, "text": s}));
        })));
        let set = move |k: &'static str| move |st: St, x: f64| {
            st.exec("clip.set", json!({"id": id, "style": {k: x}}));
        };
        let h = p.settings.width.min(p.settings.height) as f64;
        v.push(row("Размер", slider(st, num(style, "size", h * 0.075), 12.0, 400.0, 1.0, 0, set("size"))));
        let col = style.get("color").and_then(Value::as_str).unwrap_or("#ffffff").to_string();
        v.push(row("Цвет", ColorPicker::new().color(ColorValue::from_hex(&col).unwrap_or(ColorValue::new(255, 255, 255))).on_change(move |cv: ColorValue| {
            st.exec("clip.set", json!({"id": id, "style": {"color": cv.to_hex()}}));
        })));
        let pos = style.get("position").and_then(Value::as_str).unwrap_or("bottom").to_string();
        v.push(row("Положение", choice(&pos, &[("top", "Сверху"), ("center", "По центру"), ("bottom", "Снизу")], move |k| {
            st.exec("clip.set", json!({"id": id, "style": {"position": k}}));
        })));
        let anims = [("fade", "Проявление"), ("slide", "Подъём"), ("pop", "Выпрыгивание"), ("none", "Без анимации")];
        for (k, label) in [("in", "Появление"), ("out", "Исчезание")] {
            let cur = style.get(k).and_then(Value::as_str).unwrap_or("fade").to_string();
            v.push(row(label, choice(&cur, &anims, move |x| {
                st.exec("clip.set", json!({"id": id, "style": {k: x}}));
            })));
        }
        v.push(row("Длит. анимации, с", slider(st, num(style, "anim", 0.4), 0.0, 2.0, 0.05, 2, set("anim"))));
        let boxed = style.get("box").and_then(Value::as_bool).unwrap_or(false);
        v.push(row("Подложка", Toggle::with_state(boxed).on_change(move |on| {
            st.exec("clip.set", json!({"id": id, "style": {"box": on}}));
        })));
    }
    if let Some(t) = &c.transition {
        v.push(section("ПЕРЕХОД В НАЧАЛЕ"));
        let kinds: Vec<(&str, &str)> = crate::render::compose::TRANSITIONS.to_vec();
        let dur = t.duration;
        v.push(row("Вид", choice(&t.kind, &kinds, move |k| {
            st.exec("transition.set", json!({"id": id, "kind": k, "duration": dur}));
        })));
        let kind = t.kind.clone();
        v.push(row("Длительность, с", slider(st, t.duration as f32, 0.1, 3.0, 0.05, 2, move |st, x| {
            st.exec("transition.set", json!({"id": id, "kind": kind, "duration": x}));
        })));
        v.push(Box::new(Button::new("Убрать переход").on_click(move || {
            st.exec("transition.remove", json!({"id": id}));
        }).class("head-btn")));
    }
    if !c.effects.is_empty() {
        v.push(section("ЭФФЕКТЫ"));
    }
    for ef in &c.effects {
        let eid = ef.id;
        v.push(Box::new(
            Row::new()
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .class("fx-head")
                .child(DecoratedBox::new().class("grow").child(Text::new(effect_title(&ef.kind)).class("fx-title")))
                .child(ToolButton::new("\u{E5CD}").tooltip("Убрать эффект").on_click(move || {
                    st.exec("effect.remove", json!({"id": eid}));
                })),
        ));
        for &(name, label, min, max, step, def) in effect_params(&ef.kind) {
            let decimals = if step >= 1.0 { 0 } else if step >= 0.1 { 1 } else { 2 };
            v.push(row(label, slider(st, num(&ef.params, name, def as f64), min, max, step, decimals, move |st, x| {
                st.exec("effect.set", json!({"id": eid, "params": {name: x}}));
            })));
        }
        match ef.kind.as_str() {
            "lut" => {
                let path = ef.params.get("path").and_then(Value::as_str).unwrap_or("").to_string();
                let name = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                v.push(row("Файл", Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class("grow").child(Text::new(name).max_lines(1).elide(Elide::End).class("kv-value"))).child(Button::new("Выбрать…").on_click(move || pick_lut(st, eid)).class("head-btn"))));
            }
            "upscale" => {
                let models: Vec<(String, String)> = crate::render::neural::available().into_iter().filter(|(_, a)| a != "rife").map(|(id, a)| (id.clone(), format!("{id} ({a})"))).collect();
                let cur = ef.params.get("model").and_then(Value::as_str).unwrap_or("").to_string();
                let mut items: Vec<(&str, &str)> = vec![("", "Любая")];
                items.extend(models.iter().map(|(k, t)| (k.as_str(), t.as_str())));
                v.push(row("Модель", choice(&cur, &items, move |k| {
                    st.exec("effect.set", json!({"id": eid, "params": {"model": k}}));
                })));
            }
            "interpolate" => v.push(Box::new(Text::new(format!("Промежуточные кадры до {} fps при экспорте", p.settings.fps)).max_lines(2).class("panel-hint"))),
            _ => {}
        }
    }
    v
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
