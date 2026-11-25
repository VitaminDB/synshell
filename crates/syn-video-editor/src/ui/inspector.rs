//! Инспектор — `PropertyGrid` syngui в стиле приложения: разделы (проект,
//! клип, титр, переход, эффекты), числа в пределах протягиваются мышью
//! (щелчок — ввод), выбор — списком, цвет — палитрой, действия — кнопками в
//! строке. Каждая правка — команда; правки одного поля подряд — один шаг
//! отмены. Сетка перестраивается только при смене выделения или состава
//! (`St::insp_key`), поэтому протяжка не обрывается.

use std::sync::Arc;

use serde_json::{json, Map, Value};
use syngui::prelude::*;
use syngui::PropertyMark;

use super::{ed, St};
use crate::doc::{tc, Source};

/// Несколько клипов в выделении (связанная пара видео+звук — один клип):
/// все выделенные id, если «клипов» больше одного.
fn multi(sel: &[crate::doc::Id], p: &crate::doc::Project) -> Option<Vec<crate::doc::Id>> {
    let ids: Vec<_> = sel.iter().copied().filter(|id| p.clip(*id).is_some()).collect();
    let mut groups: Vec<crate::doc::Id> = ids.iter().map(|id| p.linked_peer(*id).map_or(*id, |q| q.min(*id))).collect();
    groups.sort_unstable();
    groups.dedup();
    (groups.len() > 1).then_some(ids)
}

/// Значения поля у клипов: одно общее или «разные».
fn common<T: PartialEq + Copy>(vals: &[T]) -> Option<T> {
    let f = *vals.first()?;
    vals.iter().all(|v| *v == f).then_some(f)
}

/// Ключ состава инспектора: меняется, только когда сетку надо перестроить.
pub fn key() -> String {
    let e = ed();
    let p = &e.project;
    if let Some(ids) = multi(&e.selection, p) {
        // состав и «одинаковы ли значения» (числа не включаем — протяжка не обрывается)
        let cl: Vec<&crate::doc::Clip> = ids.iter().filter_map(|i| p.clip(*i)).collect();
        let sp: Vec<i64> = cl.iter().filter(|c| c.media().is_some()).map(|c| (c.speed * 1000.0).round() as i64).collect();
        let bl: Vec<&str> = cl.iter().map(|c| c.blend.as_str()).collect();
        return format!("multi:{ids:?}:{}:{}", common(&sp).is_some(), bl.join(","));
    }
    match e.selection.first().and_then(|id| p.clip(*id)) {
        Some(c) => {
            let src = match &c.source {
                Source::Media { media } => format!("m{media}"),
                Source::Title { style, .. } => {
                    if let Some(tpl) = style.get("template").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                        // Перестраиваем сетку шаблона при смене списков/шрифтов/наличия правок,
                        // но не во время протяжки числовых ползунков (числа здесь не включаем).
                        let has_ov = style.keys().any(|k| k.starts_with(crate::fusion::STYLE_PREFIX));
                        let mut meta: Vec<String> = style
                            .iter()
                            .filter(|(k, v)| k.starts_with(crate::fusion::STYLE_PREFIX) && (k.ends_with(".Type") || k.ends_with(".Font") || k.ends_with(".Style") || v.is_boolean()))
                            .map(|(k, v)| format!("{k}={v}"))
                            .collect();
                        meta.sort();
                        format!("tpl:{tpl}:{has_ov}:{}:{}", style.get("font").and_then(Value::as_str).unwrap_or(""), meta.join(";"))
                    } else {
                        format!(
                            "title:{}:{}:{}:{}",
                            style.get("font").and_then(Value::as_str).unwrap_or(""),
                            style.get("by").and_then(Value::as_str).unwrap_or("all"),
                            style.get("in").and_then(Value::as_str).unwrap_or("fade"),
                            style.get("out").and_then(Value::as_str).unwrap_or("fade"),
                        )
                    }
                }
                Source::Color { .. } => "color".into(),
                Source::Shape { kind, .. } => format!("shape:{kind}"),
            };
            let fx: Vec<String> = c
                .effects
                .iter()
                .map(|e| format!("{}{}{}{}", e.id, e.kind, e.params.get("path").and_then(Value::as_str).unwrap_or(""), e.params.get("shape").and_then(Value::as_str).unwrap_or("")))
                .collect();
            // с ключами — значения зависят от позиции: перестройка по ней (кадр)
            let anim = if c.keys.is_empty() { String::new() } else { format!(":k{}:{:.0}", c.keys.values().map(Vec::len).sum::<usize>(), (e.playhead - c.start).clamp(0.0, c.duration) * p.settings.fps) };
            let par = c.parent.map(|i| i.to_string()).unwrap_or_default();
            let lnk = p.linked_peer(c.id).map(|i| format!("{i}:{:.2}", p.clip(i).map(|x| x.volume).unwrap_or(0.0))).unwrap_or_default();
            let tr_key = match &c.transition {
                Some(t) => {
                    let has_ov = !t.params.is_empty();
                    let mut meta: Vec<String> = t
                        .params
                        .iter()
                        .filter(|(k, v)| k.ends_with(".Type") || k.ends_with(".Font") || k.ends_with(".Style") || v.is_boolean())
                        .map(|(k, v)| format!("{k}={v}"))
                        .collect();
                    meta.sort();
                    format!("{}:{}:{has_ov}:{}", t.kind, t.template.as_deref().unwrap_or(""), meta.join(";"))
                }
                None => "-".into(),
            };
            let gk = c.graph.as_ref().map(|g| g.nodes.iter().map(|n| if n.kind == "block" { format!("b{}", n.id) } else { "n".into() }).collect::<String>()).unwrap_or_default();
            format!("c{}:{src}:{}:{tr_key}:{:.4}:{:.4}:{}:{par}:{lnk}{anim}:g{gk}", c.id, fx.join(","), c.start, c.duration, c.blend)
        }
        None => format!("p:{}x{}@{}:{}:{}:{}:{:.4}", p.settings.width, p.settings.height, p.settings.fps, p.settings.bg, p.media.len(), p.tracks.len(), p.duration()),
    }
}

type Handler = Arc<dyn Fn(St, PropertyValue) + Send + Sync>;
type MarkHandler = Arc<dyn Fn(St) + Send + Sync>;

/// Строки сетки и обработчики их правок.
#[derive(Default)]
struct Rows {
    props: Vec<Property>,
    on: Vec<Option<Handler>>,
    marks: Vec<Option<MarkHandler>>,
}

impl Rows {
    fn header(&mut self, name: &str) {
        self.props.push(Property::header(name));
        self.on.push(None);
        self.marks.push(None);
    }
    fn info(&mut self, name: &str, v: impl Into<String>) {
        self.props.push(Property::text(name, v));
        self.on.push(None);
        self.marks.push(None);
    }
    fn add(&mut self, p: Property, f: impl Fn(St, PropertyValue) + Send + Sync + 'static) {
        self.props.push(p);
        self.on.push(Some(Arc::new(f)));
        self.marks.push(None);
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
    /// Ключ анимации у последней строки: ◇ нет ключей, ◈ есть, но не в
    /// позиции, ◆ ключ в позиции; щелчок — поставить или убрать ключ.
    fn key_mark(&mut self, base: &crate::doc::Clip, lt: f64, fps: f64, path: String) {
        let half = 0.5 / fps.max(1.0);
        let m = match base.keys.get(&path) {
            None => PropertyMark::Empty,
            Some(k) if k.iter().any(|k| (k.t - lt).abs() <= half) => PropertyMark::Full,
            Some(_) => PropertyMark::Partial,
        };
        let cid = base.id;
        if let Some(p) = self.props.last_mut() {
            p.mark = Some(m);
        }
        if let Some(slot) = self.marks.last_mut() {
            *slot = Some(Arc::new(move |st: St| {
                if m == PropertyMark::Full {
                    st.exec("key.remove", json!({"id": cid, "param": path}));
                } else {
                    st.exec("key.set", json!({"id": cid, "param": path}));
                }
            }));
        }
    }
}

fn num(m: &Map<String, Value>, k: &str, d: f64) -> f64 {
    m.get(k).and_then(Value::as_f64).unwrap_or(d)
}

/// Параметр эффекта: имя, подпись, мин, макс, шаг, по умолчанию.
pub(super) type Param = (&'static str, &'static str, f64, f64, f64, f64);

pub(super) fn effect_params(kind: &str) -> &'static [Param] {
    match kind {
        "fade_in" | "fade_out" | "audio_fade_in" | "audio_fade_out" => &[("duration", "Длительность, с", 0.05, 5.0, 0.05, 0.5)],
        "lut" => &[("strength", "Сила", 0.0, 1.0, 0.01, 1.0)],
        "color" => &[("brightness", "Яркость", -1.0, 1.0, 0.01, 0.0), ("contrast", "Контраст", -1.0, 1.0, 0.01, 0.0), ("saturation", "Насыщенность", -1.0, 1.0, 0.01, 0.0), ("temperature", "Температура", -1.0, 1.0, 0.01, 0.0)],
        "blur" => &[("radius", "Радиус, пикс.", 0.0, 60.0, 0.5, 6.0)],
        "sharpen" => &[("amount", "Сила", 0.0, 2.0, 0.01, 0.5)],
        "vignette" => &[("amount", "Сила", 0.0, 1.0, 0.01, 0.5)],
        "transform" => &[("x", "Сдвиг X", -1.0, 1.0, 0.005, 0.0), ("y", "Сдвиг Y", -1.0, 1.0, 0.005, 0.0), ("scale", "Масштаб", 0.1, 4.0, 0.01, 1.0), ("rotation", "Поворот, °", -180.0, 180.0, 0.5, 0.0), ("opacity", "Непрозрачность", 0.0, 1.0, 0.01, 1.0)],
        "mask" => &[("x", "Центр X", -1.0, 1.0, 0.01, 0.0), ("y", "Центр Y", -1.0, 1.0, 0.01, 0.0), ("width", "Ширина", 0.05, 1.5, 0.01, 0.6), ("height", "Высота", 0.05, 1.5, 0.01, 0.6), ("radius", "Скругление", 0.0, 0.5, 0.01, 0.0), ("feather", "Мягкость края", 0.0, 0.3, 0.005, 0.0)],
        "glass" => &[
            ("blur", "Размытие фона, пикс.", 0.0, 60.0, 0.5, 18.0),
            ("tint_opacity", "Тонировка", 0.0, 1.0, 0.01, 0.18),
            ("border_opacity", "Кромка", 0.0, 1.0, 0.01, 0.48),
            ("highlight", "Блик", 0.0, 1.0, 0.01, 0.30),
            ("shadow", "Тень", 0.0, 1.0, 0.01, 0.28),
            ("saturation", "Насыщенность фона", 0.0, 2.0, 0.02, 1.20),
            ("brightness", "Светлота фона", -0.5, 0.5, 0.01, 0.04),
        ],
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
        "grade" => "Цветокоррекция",
        "color" => "Цвет",
        "blur" => "Размытие",
        "sharpen" => "Резкость",
        "vignette" => "Виньетка",
        "transform" => "Положение и масштаб",
        "mask" => "Маска",
        "glass" => "Матовое стекло",
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
        let _ = st.ph.get();
        let k = key();
        if st.insp_key.get_untracked() != k {
            st.insp_key.set(k);
        }
    });
    ScrollView::new().vertical().child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.insp_key.get();
        let (proj, sel, many) = {
            let e = ed();
            (e.project.clone(), e.selection.first().copied(), multi(&e.selection, &e.project))
        };
        let mut r = Rows::default();
        let clip_sel = sel.and_then(|id| proj.clip(id).cloned());
        match &clip_sel {
            None => project(&mut r, &proj),
            Some(_) if many.is_some() => clips(&mut r, &proj, many.as_deref().unwrap_or_default()),
            Some(c) => {
                // значения — на момент позиции (ключевые кадры)
                let lt = (ed().playhead - c.start).clamp(0.0, c.duration);
                clip(&mut r, &proj, &c.at(lt), c, lt)
            }
        }
        let handlers = r.on;
        let marks = r.marks;
        let grid = PropertyGrid::new()
            .properties(r.props)
            .label_width(150.0)
            .row_height(ROW_H)
            .auto_height(true)
            .on_change(move |i, v| {
                if let Some(Some(f)) = handlers.get(i) {
                    f(st, v);
                }
            })
            .on_mark(move |i| {
                if let Some(Some(f)) = marks.get(i) {
                    f(st);
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
    r.range("Ширина, пикс.", p.settings.width as f64, 16.0, 7680.0, 2.0, |st, w| {
        st.exec("project.settings", json!({"width": w.round() as u32}));
    });
    r.range("Высота, пикс.", p.settings.height as f64, 16.0, 7680.0, 2.0, |st, h| {
        st.exec("project.settings", json!({"height": h.round() as u32}));
    });
    r.choice("Частота", &[("24", "24 fps"), ("25", "25 fps"), ("30", "30 fps"), ("50", "50 fps"), ("60", "60 fps")], &format!("{}", p.settings.fps), |st, k| {
        st.exec("project.settings", json!({"fps": k.parse::<f64>().unwrap_or(30.0)}));
    });
    let bg_is_trans = p.settings.bg.eq_ignore_ascii_case("transparent") || p.settings.bg.eq_ignore_ascii_case("none");
    let bg_hex = if bg_is_trans || p.settings.bg.is_empty() { "#000000" } else { p.settings.bg.as_str() };
    r.add(Property::color("Фон кадра", Color::from_hex(bg_hex)), |st, v| {
        if let PropertyValue::Color(col) = v {
            st.exec("project.settings", json!({"bg": col.to_hex()}));
        }
    });
    r.add(Property::boolean("Прозрачный фон", bg_is_trans), |st, v| {
        if let PropertyValue::Bool(on) = v {
            st.exec("project.settings", json!({"bg": if on { "transparent" } else { "#000000" }}));
        }
    });
    r.info("Длительность", tc(p.duration()));
    r.info("Медиа", p.media.len().to_string());
    r.info("Дорожки", p.tracks.len().to_string());
}

/// Несколько выделенных клипов: общие поля, правка — всем одной командой.
fn clips(r: &mut Rows, p: &crate::doc::Project, ids: &[crate::doc::Id]) {
    let cl: Vec<&crate::doc::Clip> = ids.iter().filter_map(|i| p.clip(*i)).collect();
    let n = cl.iter().map(|c| p.linked_peer(c.id).map_or(c.id, |q| q.min(c.id))).collect::<std::collections::BTreeSet<_>>().len();
    r.header(&format!("Выделено клипов: {n}"));
    let start = cl.iter().map(|c| c.start).fold(f64::INFINITY, f64::min);
    let end = cl.iter().map(|c| c.end()).fold(0.0, f64::max);
    r.info("Время", format!("{} — {} ({})", tc(start), tc(end), tc(end - start)));
    let is_video = |c: &crate::doc::Clip| p.find_clip(c.id).is_some_and(|(ti, _)| p.tracks[ti].kind == crate::doc::TrackKind::Video);
    let all: Vec<crate::doc::Id> = cl.iter().map(|c| c.id).collect();

    // скорость — у клипов медиа (у связанной пары меняется вместе)
    let media: Vec<&crate::doc::Clip> = cl.iter().copied().filter(|c| c.media().is_some()).collect();
    if !media.is_empty() {
        let sp: Vec<i64> = media.iter().map(|c| (c.speed * 1000.0).round() as i64).collect();
        let same = common(&sp).is_some();
        let ids: Vec<crate::doc::Id> = media.iter().map(|c| c.id).collect();
        r.range(if same { "Скорость" } else { "Скорость (разная)" }, media[0].speed, 0.1, 4.0, 0.05, move |st, x| {
            st.exec("clip.set", json!({"ids": ids, "speed": x}));
        });
        // громкость: у видео со связанным звуком она у звука (clip.set перенаправит)
        let vol = |c: &crate::doc::Clip| match p.linked_peer(c.id).and_then(|q| p.clip(q)) {
            Some(a) if is_video(c) => a.volume,
            _ => c.volume,
        };
        let vols: Vec<i64> = media.iter().map(|c| (vol(c) * 1000.0).round() as i64).collect();
        let same = common(&vols).is_some();
        let ids: Vec<crate::doc::Id> = media.iter().map(|c| c.id).collect();
        r.range(if same { "Громкость" } else { "Громкость (разная)" }, vol(media[0]), 0.0, 2.0, 0.01, move |st, x| {
            st.exec("clip.set", json!({"ids": ids, "volume": x}));
        });
    }
    // наложение — у слоёв видеодорожек
    let vids: Vec<&crate::doc::Clip> = cl.iter().copied().filter(|c| is_video(c)).collect();
    if !vids.is_empty() {
        let bl: Vec<&str> = vids.iter().map(|c| if c.blend.is_empty() { "normal" } else { c.blend.as_str() }).collect();
        let ids: Vec<crate::doc::Id> = vids.iter().map(|c| c.id).collect();
        let mut items: Vec<(&str, &str)> = crate::doc::BLEND_MODES.to_vec();
        let cur = match common(&bl) {
            Some(b) => b,
            None => {
                items.insert(0, ("", "Разные"));
                ""
            }
        };
        r.choice("Наложение", &items, cur, move |st, b| {
            if !b.is_empty() {
                st.exec("clip.set", json!({"ids": ids, "blend": b}));
            }
        });
    }
    r.header("Действия");
    let ids = all.clone();
    r.action("Удалить", "Удалить", move |st| {
        st.exec("clip.remove", json!({"ids": ids}));
    });
    let ids = all;
    r.action("Удалить со сдвигом", "Удалить", move |st| {
        st.exec("clip.remove", json!({"ids": ids, "ripple": true}));
    });
}

/// Правка параметра: есть ключи — ключ в позиции, иначе постоянное значение.
fn set_or_key(st: St, cid: u64, path: &str, v: Value, fallback: impl FnOnce()) {
    let keyed = ed().project.clip(cid).is_some_and(|c| c.keys.contains_key(path));
    if keyed {
        st.exec("key.set", json!({"id": cid, "param": path, "value": v}));
    } else {
        fallback();
    }
}

/// `c` — клип со значениями на момент позиции, `base` — как в проекте (ключи).
fn clip(r: &mut Rows, p: &crate::doc::Project, c: &crate::doc::Clip, base: &crate::doc::Clip, lt: f64) {
    let id = c.id;
    let fps = p.settings.fps;
    r.header(&format!("Клип c{id}"));
    let src = match &c.source {
        Source::Media { media } => p.media(*media).map(|m| format!("m{media} {}", m.name())).unwrap_or_default(),
        Source::Title { .. } => "титр".into(),
        Source::Color { color } => format!("цвет {color}"),
        Source::Shape { kind, .. } => format!("фигура: {}", crate::doc::shape_title(kind)),
    };
    r.info("Источник", src);
    r.info("Время", format!("{} — {} ({})", tc(c.start), tc(c.end()), tc(c.duration)));
    if p.find_clip(id).is_some_and(|(ti, _)| p.tracks[ti].kind == crate::doc::TrackKind::Video) {
        let label = match &base.graph {
            Some(g) => format!("Открыть ({} нод)", g.nodes.len()),
            None => "Открыть как граф".to_string(),
        };
        r.action("Граф кадра", &label, move |st| super::framegraph_ui::open(st, id));
        // вынесенные параметры блоков графа
        for n in base.graph.iter().flat_map(|g| g.nodes.iter()).filter(|n| n.kind == "block") {
            let title = n.params.get("block").and_then(Value::as_str).unwrap_or("Блок");
            r.header(&format!("Блок «{title}»"));
            let vals = n.params.get("values").and_then(Value::as_object).cloned().unwrap_or_default();
            for e in crate::blocks::exposes(&n.params) {
                let nid = n.id;
                let key = e.key.clone();
                let v = vals.get(&e.key).and_then(Value::as_f64).unwrap_or(e.default);
                let step = ((e.max - e.min) / 100.0).max(0.001);
                let step = if step >= 1.0 { 1.0 } else if step >= 0.1 { 0.1 } else if step >= 0.01 { 0.01 } else { 0.001 };
                r.range(&e.label, v, e.min, e.max, step, move |st, x| {
                    st.exec("block.set", json!({"id": id, "node": nid, "values": {key.clone(): x}}));
                });
            }
        }
    }
    r.add(Property::text("Имя", c.name.clone()), move |st, v| {
        if let PropertyValue::Text(s) = v {
            st.exec("clip.set", json!({"id": id, "name": s}));
        }
    });
    let is_video = p.find_clip(id).is_some_and(|(ti, _)| p.tracks[ti].kind == crate::doc::TrackKind::Video);
    if is_video {
        let cur_blend = if c.blend.is_empty() { "normal" } else { c.blend.as_str() };
        r.choice("Наложение", crate::doc::BLEND_MODES, cur_blend, move |st, b| {
            st.exec("clip.set", json!({"id": id, "blend": b}));
        });
        let mut parents: Vec<(String, String)> = vec![("0".into(), "Нет".into())];
        for tr in &p.tracks {
            if tr.kind != crate::doc::TrackKind::Video {
                continue;
            }
            for o in &tr.clips {
                if o.id == id || p.is_ancestor(id, o.id) {
                    continue;
                }
                let lbl = if !o.name.is_empty() {
                    format!("c{} {}", o.id, o.name)
                } else {
                    match &o.source {
                        Source::Media { media } => format!("c{} {}", o.id, p.media(*media).map(|m| m.name()).unwrap_or_default()),
                        Source::Title { text, .. } => format!("c{} «{}»", o.id, text.lines().next().unwrap_or("")),
                        Source::Color { color } => format!("c{} {color}", o.id),
                        Source::Shape { kind, .. } => format!("c{} {}", o.id, crate::doc::shape_title(kind)),
                    }
                };
                parents.push((o.id.to_string(), lbl));
            }
        }
        let pref: Vec<(&str, &str)> = parents.iter().map(|(k, t)| (k.as_str(), t.as_str())).collect();
        let cur_par = c.parent.map(|i| i.to_string()).unwrap_or_else(|| "0".into());
        r.choice("Родитель", &pref, &cur_par, move |st, k| {
            st.exec("clip.set", json!({"id": id, "parent": k}));
        });
    }
    if let Some(mid) = c.media() {
        let linked = p.linked_peer(id);
        let mk = p.media(mid).map(|m| (m.kind, m.duration));
        if let Some((crate::doc::MediaKind::Lottie, d)) = mk {
            let loops = c.duration * c.speed / d.max(1e-3);
            r.info("Анимация", format!("Lottie, {} {}", tc(d), if loops > 1.01 { format!("— по кругу ×{loops:.1}") } else { String::new() }));
        }
        r.range("Скорость", c.speed, 0.1, 4.0, 0.05, move |st, x| {
            st.exec("clip.set", json!({"id": id, "speed": x}));
        });
        let silent = matches!(mk, Some((crate::doc::MediaKind::Lottie | crate::doc::MediaKind::Image, _)));
        let (vol_target, vol_val) = if is_video {
            match linked.and_then(|lid| p.clip(lid)) {
                Some(ac) => (ac.id, ac.volume),
                None => (id, c.volume),
            }
        } else {
            (id, c.volume)
        };
        if !silent {
            r.range("Громкость", vol_val, 0.0, 2.0, 0.01, move |st, x| {
                st.exec("clip.set", json!({"id": vol_target, "volume": x}));
            });
        }
        if linked.is_some() {
            r.action(if is_video { "Звук" } else { "Связь с видео" }, "Отвязать", move |st| {
                st.exec("clip.detach_audio", json!({"ids": [id]}));
            });
        } else if is_video && p.media(mid).is_some_and(|m| m.kind == crate::doc::MediaKind::Video && m.has_audio) && c.volume > 0.0 {
            r.action("Звук", "Отделить на дорожку", move |st| {
                st.exec("clip.detach_audio", json!({"ids": [id]}));
            });
        }
    }
    if let Source::Title { text, style } = &c.source {
        if let Some(tpl_path) = style.get("template").and_then(Value::as_str).filter(|s| !s.is_empty()) {
            if let Ok(comp) = crate::fusion::load_cached(std::path::Path::new(tpl_path)) {
                template_inspector(r, id, &comp, text, style, base, lt, fps);
            } else {
                plain_title_inspector(r, id, p, text, style, base, lt, fps);
            }
        } else {
            plain_title_inspector(r, id, p, text, style, base, lt, fps);
        }
    }
    if let Source::Shape { kind, style } = &c.source {
        let is_line = matches!(kind.as_str(), "line" | "arrow");
        r.header("Фигура");
        r.choice("Вид", crate::doc::SHAPE_KINDS, kind, move |st, k| {
            st.exec("clip.set", json!({"id": id, "shape": k}));
        });
        r.range("Ширина", num(style, "width", if is_line { 0.45 } else { 0.28 }), 0.02, 1.5, 0.01, move |st, x| {
            set_or_key(st, id, "style.width", json!(x), || {
                st.exec("clip.set", json!({"id": id, "style": {"width": x}}));
            });
        });
        r.key_mark(base, lt, fps, "style.width".into());
        r.range("Высота", num(style, "height", if kind == "line" { 0.06 } else if kind == "arrow" { 0.14 } else { 0.28 }), 0.02, 1.5, 0.01, move |st, x| {
            set_or_key(st, id, "style.height", json!(x), || {
                st.exec("clip.set", json!({"id": id, "style": {"height": x}}));
            });
        });
        r.key_mark(base, lt, fps, "style.height".into());
        if !is_line {
            let fill = style.get("fill").and_then(Value::as_str).unwrap_or("#3b82f6");
            r.add(Property::color("Заливка", Color::from_hex(fill)), move |st, v| {
                if let PropertyValue::Color(c) = v {
                    let hex = c.to_hex();
                    set_or_key(st, id, "style.fill", json!(hex), || {
                        st.exec("clip.set", json!({"id": id, "style": {"fill": hex}}));
                    });
                }
            });
            r.key_mark(base, lt, fps, "style.fill".into());
        }
        let stroke = style.get("stroke").and_then(Value::as_str).filter(|s| *s != "none").unwrap_or("#ffffff");
        r.add(Property::color(if is_line { "Цвет" } else { "Обводка" }, Color::from_hex(stroke)), move |st, v| {
            if let PropertyValue::Color(c) = v {
                let hex = c.to_hex();
                set_or_key(st, id, "style.stroke", json!(hex), || {
                    st.exec("clip.set", json!({"id": id, "style": {"stroke": hex}}));
                });
            }
        });
        r.key_mark(base, lt, fps, "style.stroke".into());
        r.range(if is_line { "Толщина, пикс." } else { "Обводка, пикс." }, num(style, "stroke_width", if is_line { 10.0 } else { 0.0 }), 0.0, 80.0, 1.0, move |st, x| {
            set_or_key(st, id, "style.stroke_width", json!(x), || {
                st.exec("clip.set", json!({"id": id, "style": {"stroke_width": x}}));
            });
        });
        r.key_mark(base, lt, fps, "style.stroke_width".into());
        if kind == "rect" {
            r.range("Скругление", num(style, "radius", 0.08), 0.0, 0.5, 0.01, move |st, x| {
                set_or_key(st, id, "style.radius", json!(x), || {
                    st.exec("clip.set", json!({"id": id, "style": {"radius": x}}));
                });
            });
            r.key_mark(base, lt, fps, "style.radius".into());
        }
        if kind == "star" {
            r.range("Лучи", num(style, "points", 5.0), 3.0, 12.0, 1.0, move |st, x| {
                st.exec("clip.set", json!({"id": id, "style": {"points": x}}));
            });
        }
    }
    if let Some(t) = &c.transition {
        r.header("Переход в начале");
        let dur = t.duration;
        if t.is_fusion() {
            let fusion_label = format!("Fusion: {}", t.title());
            let mut kinds: Vec<(&str, &str)> = vec![("fusion", fusion_label.as_str())];
            kinds.extend(crate::render::compose::TRANSITIONS.iter().copied());
            r.choice("Вид", &kinds, "fusion", move |st, k| {
                if k != "fusion" {
                    st.exec("transition.set", json!({"id": id, "kind": k, "duration": dur}));
                }
            });
            r.range("Длительность, с", t.duration, 0.1, 4.0, 0.05, move |st, x| {
                st.exec("transition.set", json!({"id": id, "kind": "fusion", "duration": x}));
            });
            r.action("Шаблон", "Выбрать другой…", move |st| super::templates::open_transitions(st));
            if let Some(tpl) = t.template.as_deref() {
                if let Ok(comp) = crate::fusion::load_cached(std::path::Path::new(tpl)) {
                    transition_fusion_params(r, id, &comp, &t.params);
                }
            }
        } else {
            let kinds: Vec<(&str, &str)> = crate::render::compose::TRANSITIONS.to_vec();
            r.choice("Вид", &kinds, &t.kind, move |st, k| {
                st.exec("transition.set", json!({"id": id, "kind": k, "duration": dur}));
            });
            let kind = t.kind.clone();
            r.range("Длительность, с", t.duration, 0.1, 3.0, 0.05, move |st, x| {
                st.exec("transition.set", json!({"id": id, "kind": kind, "duration": x}));
            });
            r.action("Fusion", "Выбрать из галереи…", move |st| super::templates::open_transitions(st));
        }
        r.action("", "Убрать переход", move |st| {
            st.exec("transition.remove", json!({"id": id}));
        });
    }
    for ef in &c.effects {
        let eid = ef.id;
        r.header(&format!("Эффект: {}", effect_title(&ef.kind)));
        // длительности затуханий — не анимируются
        let animatable = !ef.kind.contains("fade");
        for &(name, label, min, max, step, def) in effect_params(&ef.kind) {
            let path = format!("e{eid}.{name}");
            let pk = path.clone();
            r.range(label, num(&ef.params, name, def), min, max, step, move |st, x| {
                set_or_key(st, id, &pk, json!(x), || {
                    st.exec("effect.set", json!({"id": eid, "params": {name: x}}));
                });
            });
            if animatable {
                r.key_mark(base, lt, fps, path);
            }
        }
        match ef.kind.as_str() {
            "mask" => {
                let sh = ef.params.get("shape").and_then(Value::as_str).unwrap_or("rect");
                r.choice("Форма", &[("rect", "Прямоугольник"), ("ellipse", "Овал / круг")], sh, move |st, k| {
                    st.exec("effect.set", json!({"id": eid, "params": {"shape": k}}));
                });
                let inv = ef.params.get("invert").and_then(Value::as_bool).unwrap_or(false);
                r.add(Property::boolean("Инвертировать", inv), move |st, v| {
                    if let PropertyValue::Bool(on) = v {
                        st.exec("effect.set", json!({"id": eid, "params": {"invert": on}}));
                    }
                });
            }
            "glass" => {
                let tint = ef.params.get("tint").and_then(Value::as_str).unwrap_or("#ffffff");
                r.add(Property::color("Оттенок", Color::from_hex(tint)), move |st, v| {
                    if let PropertyValue::Color(col) = v {
                        st.exec("effect.set", json!({"id": eid, "params": {"tint": col.to_hex()}}));
                    }
                });
            }
            "grade" => {
                r.action("Колёса и кривые", "Открыть панель «Цвет»", move |st| st.open_panel("color"));
            }
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

/// Параметры обычного титра (без Fusion-шаблона).
fn plain_title_inspector(
    r: &mut Rows,
    id: u64,
    p: &crate::doc::Project,
    text: &str,
    style: &Map<String, Value>,
    base: &crate::doc::Clip,
    lt: f64,
    fps: f64,
) {
    r.header("Титр");
    r.add(Property::multiline("Текст", text.to_string()), move |st, v| {
        if let PropertyValue::Multiline(s) = v {
            st.exec("clip.set", json!({"id": id, "text": s}));
        }
    });
    let h = p.settings.width.min(p.settings.height) as f64;
    r.range("Размер", num(style, "size", h * 0.075), 12.0, 400.0, 1.0, move |st, x| {
        set_or_key(st, id, "style.size", json!(x), || {
            st.exec("clip.set", json!({"id": id, "style": {"size": x}}));
        });
    });
    r.key_mark(base, lt, fps, "style.size".into());
    let font = style.get("font").and_then(Value::as_str).unwrap_or("По умолчанию").to_string();
    r.action("Шрифт", &font, move |st| st.font_pick.set(Some((id, None))));
    let col = style.get("color").and_then(Value::as_str).unwrap_or("#ffffff");
    r.add(Property::color("Цвет", Color::from_hex(col)), move |st, v| {
        if let PropertyValue::Color(c) = v {
            let hex = c.to_hex();
            set_or_key(st, id, "style.color", json!(hex), || {
                st.exec("clip.set", json!({"id": id, "style": {"color": hex}}));
            });
        }
    });
    r.key_mark(base, lt, fps, "style.color".into());
    let pos = style.get("position").and_then(Value::as_str).unwrap_or("bottom");
    r.choice("Положение", &[("top", "Сверху"), ("center", "По центру"), ("bottom", "Снизу")], pos, move |st, k| {
        st.exec("clip.set", json!({"id": id, "style": {"position": k}}));
    });
    let cur_by = style.get("by").and_then(Value::as_str).unwrap_or("all");
    let cur_in = style.get("in").and_then(Value::as_str).unwrap_or("fade");
    let cur_out = style.get("out").and_then(Value::as_str).unwrap_or("fade");
    r.choice("Анимация по", crate::render::compose::TITLE_UNITS, cur_by, move |st, x| {
        st.exec("clip.set", json!({"id": id, "style": {"by": x}}));
    });
    for (k, label, cur) in [("in", "Появление", cur_in), ("out", "Исчезание", cur_out)] {
        r.choice(label, crate::render::compose::TITLE_ANIMS, cur, move |st, x| {
            st.exec("clip.set", json!({"id": id, "style": {k: x}}));
        });
    }
    r.range("Длит. анимации, с", num(style, "anim", 0.4), 0.0, 4.0, 0.05, move |st, x| {
        st.exec("clip.set", json!({"id": id, "style": {"anim": x}}));
    });
    let eff_unit = crate::render::compose::title_unit(cur_by, cur_in, cur_out);
    if eff_unit != "all" && (cur_in != "none" || cur_out != "none") {
        r.range("Разбег", num(style, "stagger", 0.65), 0.0, 1.0, 0.05, move |st, x| {
            st.exec("clip.set", json!({"id": id, "style": {"stagger": x}}));
        });
    }
    if cur_in != "none" || cur_out != "none" {
        let cur_ease = style.get("ease").and_then(Value::as_str).unwrap_or("out");
        r.choice("Сглаживание", crate::doc::EASES, cur_ease, move |st, x| {
            st.exec("clip.set", json!({"id": id, "style": {"ease": x}}));
        });
    }
    r.add(Property::boolean("Подложка", style.get("box").and_then(Value::as_bool).unwrap_or(false)), move |st, v| {
        if let PropertyValue::Bool(on) = v {
            st.exec("clip.set", json!({"id": id, "style": {"box": on}}));
        }
    });
}

/// Параметры шаблона Fusion (`MacroOperator.Inputs`, C3).
fn template_inspector(
    r: &mut Rows,
    id: u64,
    comp: &crate::fusion::Comp,
    text: &str,
    style: &Map<String, Value>,
    base: &crate::doc::Clip,
    lt: f64,
    fps: f64,
) {
    use crate::fusion::ParamKind;

    r.header(&format!("Шаблон: {}", comp.name.replace('_', " ")));
    let unsup = comp.unsupported_tools();
    if !unsup.is_empty() {
        r.info("Не поддержано", unsup.join(", "));
    }
    let has_overrides = style.keys().any(|k| k.starts_with(crate::fusion::STYLE_PREFIX))
        || base.keys.keys().any(|k| k.starts_with("style.p:"));
    if has_overrides {
        r.action("Параметры", "Сбросить", move |st| {
            st.exec("template.set", json!({"id": id, "reset": true}));
        });
    }

    let params = comp.visible_template_params(style);
    if params.is_empty() {
        r.add(Property::multiline("Текст", text.to_string()), move |st, v| {
            if let PropertyValue::Multiline(s) = v {
                st.exec("clip.set", json!({"id": id, "text": s}));
            }
        });
        let font = style.get("font").and_then(Value::as_str).unwrap_or("По умолчанию").to_string();
        r.action("Шрифт", &font, move |st| st.font_pick.set(Some((id, None))));
        return;
    }

    let text_order = comp.text_tool_names();
    let fallback_lines: Vec<&str> = if text.is_empty() { Vec::new() } else { text.split('\n').collect() };
    let mut last_group = String::new();

    for tp in params {
        if tp.group != last_group {
            r.header(&tp.group);
            last_group = tp.group.clone();
        }
        let sk = tp.style_key();
        let pkey = tp.key.clone();
        match tp.kind {
            ParamKind::Text { default } => {
                let fallback = text_order
                    .iter()
                    .position(|n| n == &tp.source_op)
                    .and_then(|i| fallback_lines.get(i).copied())
                    .unwrap_or(default.as_str());
                let cur = style.get(&sk).and_then(Value::as_str).unwrap_or(fallback).to_string();
                r.add(Property::multiline(&tp.label, cur), move |st, v| {
                    if let PropertyValue::Multiline(s) = v {
                        st.exec("template.set", json!({"id": id, "param": pkey, "value": s}));
                    }
                });
            }
            ParamKind::Font { default } => {
                let cur = style
                    .get(&sk)
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .or_else(|| style.get("font").and_then(Value::as_str).filter(|s| !s.is_empty()))
                    .unwrap_or(if default.is_empty() { "По умолчанию" } else { default.as_str() })
                    .to_string();
                r.action(&tp.label, &cur, move |st| st.font_pick.set(Some((id, Some(pkey.clone())))));
            }
            ParamKind::Choice { default, items } => {
                let cur = style.get(&sk).and_then(Value::as_str).unwrap_or(&default);
                let pairs: Vec<(&str, &str)> = items.iter().map(|i| (i.value.as_str(), i.label.as_str())).collect();
                r.choice(&tp.label, &pairs, cur, move |st, v| {
                    st.exec("template.set", json!({"id": id, "param": pkey, "value": v}));
                });
            }
            ParamKind::Bool { default } => {
                let cur = style
                    .get(&sk)
                    .and_then(|v| v.as_bool().or_else(|| v.as_f64().map(|n| n > 0.5)))
                    .unwrap_or(default);
                r.add(Property::boolean(&tp.label, cur), move |st, v| {
                    if let PropertyValue::Bool(on) = v {
                        st.exec("template.set", json!({"id": id, "param": pkey, "value": on}));
                    }
                });
            }
            ParamKind::Color { default, .. } | ParamKind::GradStop { default, .. } => {
                let cur = style.get(&sk).and_then(Value::as_str).unwrap_or(&default);
                let kpath = format!("style.{sk}");
                let kp = kpath.clone();
                r.add(Property::color(&tp.label, Color::from_hex(cur)), move |st, v| {
                    if let PropertyValue::Color(col) = v {
                        let hex = col.to_hex();
                        set_or_key(st, id, &kp, json!(hex), || {
                            st.exec("template.set", json!({"id": id, "param": pkey, "value": hex}));
                        });
                    }
                });
                r.key_mark(base, lt, fps, kpath);
            }
            ParamKind::Number { default, min, max, step } => {
                let cur = style.get(&sk).and_then(Value::as_f64).unwrap_or(default);
                let kpath = format!("style.{sk}");
                let kp = kpath.clone();
                r.range(&tp.label, cur, min, max, step, move |st, x| {
                    set_or_key(st, id, &kp, json!(x), || {
                        st.exec("template.set", json!({"id": id, "param": pkey, "value": x}));
                    });
                });
                r.key_mark(base, lt, fps, kpath);
            }
            ParamKind::Point { default, min, max, step } => {
                let sx = tp.style_key_sub("x");
                let sy = tp.style_key_sub("y");
                let cx = style.get(&sx).and_then(Value::as_f64).unwrap_or(default.0);
                let cy = style.get(&sy).and_then(Value::as_f64).unwrap_or(default.1);
                let kpx = format!("style.{sx}");
                let kpy = format!("style.{sy}");
                let pkey_x = format!("{}.x", tp.key);
                let pkey_y = format!("{}.y", tp.key);
                let kpx_c = kpx.clone();
                r.range(&format!("{} X", tp.label), cx, min, max, step, move |st, x| {
                    set_or_key(st, id, &kpx_c, json!(x), || {
                        st.exec("template.set", json!({"id": id, "param": pkey_x, "value": x}));
                    });
                });
                r.key_mark(base, lt, fps, kpx);
                let kpy_c = kpy.clone();
                r.range(&format!("{} Y", tp.label), cy, min, max, step, move |st, y| {
                    set_or_key(st, id, &kpy_c, json!(y), || {
                        st.exec("template.set", json!({"id": id, "param": pkey_y, "value": y}));
                    });
                });
                r.key_mark(base, lt, fps, kpy);
            }
        }
    }
}

/// Вынесенные параметры MacroOperator перехода Fusion (`Transition.params`).
fn transition_fusion_params(r: &mut Rows, id: u64, comp: &crate::fusion::Comp, params_map: &Map<String, Value>) {
    use crate::fusion::ParamKind;

    let unsup = comp.unsupported_tools();
    if !unsup.is_empty() {
        r.info("Не поддержано", unsup.join(", "));
    }
    if !params_map.is_empty() {
        r.action("Параметры", "Сбросить", move |st| {
            st.exec("transition.set", json!({"id": id, "kind": "fusion", "reset": true}));
        });
    }
    let params = comp.visible_template_params(params_map);
    for tp in params {
        let sk = tp.style_key();
        let pkey = tp.key.clone();
        match tp.kind {
            ParamKind::Text { default } => {
                let cur = params_map.get(&sk).and_then(Value::as_str).unwrap_or(&default).to_string();
                r.add(Property::multiline(&tp.label, cur), move |st, v| {
                    if let PropertyValue::Multiline(s) = v {
                        st.exec("transition.set", json!({"id": id, "kind": "fusion", "params": {pkey.clone(): s}}));
                    }
                });
            }
            ParamKind::Font { default } => {
                let cur = params_map.get(&sk).and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or(&default).to_string();
                r.add(Property::text(&tp.label, cur), move |st, v| {
                    if let PropertyValue::Text(s) = v {
                        st.exec("transition.set", json!({"id": id, "kind": "fusion", "params": {pkey.clone(): s}}));
                    }
                });
            }
            ParamKind::Choice { default, items } => {
                let cur = params_map.get(&sk).and_then(Value::as_str).unwrap_or(&default);
                let pairs: Vec<(&str, &str)> = items.iter().map(|i| (i.value.as_str(), i.label.as_str())).collect();
                r.choice(&tp.label, &pairs, cur, move |st, v| {
                    st.exec("transition.set", json!({"id": id, "kind": "fusion", "params": {pkey.clone(): v}}));
                });
            }
            ParamKind::Bool { default } => {
                let cur = params_map
                    .get(&sk)
                    .and_then(|v| v.as_bool().or_else(|| v.as_f64().map(|n| n > 0.5)))
                    .unwrap_or(default);
                r.add(Property::boolean(&tp.label, cur), move |st, v| {
                    if let PropertyValue::Bool(on) = v {
                        st.exec("transition.set", json!({"id": id, "kind": "fusion", "params": {pkey.clone(): on}}));
                    }
                });
            }
            ParamKind::Color { default, .. } | ParamKind::GradStop { default, .. } => {
                let cur = params_map.get(&sk).and_then(Value::as_str).unwrap_or(&default);
                r.add(Property::color(&tp.label, Color::from_hex(cur)), move |st, v| {
                    if let PropertyValue::Color(col) = v {
                        st.exec("transition.set", json!({"id": id, "kind": "fusion", "params": {pkey.clone(): col.to_hex()}}));
                    }
                });
            }
            ParamKind::Number { default, min, max, step } => {
                let cur = params_map.get(&sk).and_then(Value::as_f64).unwrap_or(default);
                r.range(&tp.label, cur, min, max, step, move |st, x| {
                    st.exec("transition.set", json!({"id": id, "kind": "fusion", "params": {pkey.clone(): x}}));
                });
            }
            ParamKind::Point { default, min, max, step } => {
                let sx = tp.style_key_sub("x");
                let sy = tp.style_key_sub("y");
                let cx = params_map.get(&sx).and_then(Value::as_f64).unwrap_or(default.0);
                let cy = params_map.get(&sy).and_then(Value::as_f64).unwrap_or(default.1);
                let pkey_x = format!("{}.x", tp.key);
                let pkey_y = format!("{}.y", tp.key);
                r.range(&format!("{} X", tp.label), cx, min, max, step, move |st, x| {
                    st.exec("transition.set", json!({"id": id, "kind": "fusion", "params": {pkey_x.clone(): x}}));
                });
                r.range(&format!("{} Y", tp.label), cy, min, max, step, move |st, y| {
                    st.exec("transition.set", json!({"id": id, "kind": "fusion", "params": {pkey_y.clone(): y}}));
                });
            }
        }
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
