//! Ключевые кадры: анимация параметров эффектов (положение, масштаб, поворот,
//! прозрачность, цвет, размытие…) и титра (размер, цвет). Время ключа — время
//! таймлайна в команде, внутри клипа хранится относительно его начала.

use serde_json::{json, Map, Value};

use super::{arg, req, Args, Command, Out, Res, Ty};
use crate::doc::{tc, Clip, Effect, Id, Key, Source};
use crate::editor::Editor;

pub static COMMANDS: &[&Command] = &[&SET, &MOVE, &REMOVE, &CLEAR, &LIST, &MOTION_LIST, &MOTION_APPLY];


/// Значение по умолчанию параметра эффекта `kind` или поля титра.
pub fn default_value(kind: &str, name: &str, p: &crate::doc::Project) -> Option<Value> {
    let n = |v: f64| Some(json!(v));
    match (kind, name) {
        ("transform", "x" | "y" | "rotation") => n(0.0),
        ("transform", "scale" | "opacity") => n(1.0),
        ("color", _) => n(0.0),
        ("blur", "radius") => n(6.0),
        ("sharpen" | "vignette", "amount") => n(0.5),
        ("lut", "strength") => n(1.0),
        ("mask", "x" | "y" | "radius" | "feather") => n(0.0),
        ("mask", "width" | "height") => n(0.6),
        ("glass", "blur") => n(18.0),
        ("glass", "tint_opacity") => n(0.18),
        ("glass", "border_opacity") => n(0.48),
        ("glass", "highlight") => n(0.30),
        ("glass", "shadow") => n(0.28),
        ("glass", "saturation") => n(1.20),
        ("glass", "brightness") => n(0.04),
        ("style", "size") => n(p.settings.width.min(p.settings.height) as f64 * 0.075),
        ("style", "color") => Some(json!("#ffffff")),
        ("style", "anim") => n(0.4),
        ("style", "fill") => Some(json!("#3b82f6")),
        ("style", "stroke") => Some(json!("#ffffff")),
        ("style", "width" | "height") => n(0.28),
        ("style", "radius") => n(0.08),
        ("style", "stroke_width") => n(0.0),
        _ => None,
    }
}

/// Параметры, которые можно анимировать: эффект → поля.
const ANIMATABLE: &[(&str, &[&str])] = &[
    ("transform", &["x", "y", "scale", "rotation", "opacity"]),
    ("mask", &["x", "y", "width", "height", "radius", "feather"]),
    ("glass", &["blur", "tint_opacity", "border_opacity", "highlight", "shadow", "saturation", "brightness"]),
    ("color", &["brightness", "contrast", "saturation", "temperature"]),
    ("blur", &["radius"]),
    ("sharpen", &["amount"]),
    ("vignette", &["amount"]),
    ("lut", &["strength"]),
    ("style", &["size", "color", "fill", "stroke", "width", "height", "radius", "stroke_width"]),
];

/// Куда ставить ключ: путь уже есть (`e<id>.<поле>`, `style.<поле>`) или
/// эффект вида `kind` надо создать.
enum Target {
    Path(String, String),
    Create(String, String),
}

/// Разобрать `e12.x`, `transform.x` (эффект по виду), `style.size` /
/// `title.size` — без правок проекта.
fn resolve(p: &crate::doc::Project, cid: Id, param: &str) -> Result<Target, String> {
    let (head, name) = param.split_once('.').ok_or_else(|| format!("параметр «{param}»: нужно вид.поле, например transform.scale"))?;
    let c = p.clip(cid).ok_or_else(|| format!("нет клипа c{cid}"))?;
    if head == "tpl" || head == "template" {
        let Source::Title { style, .. } = &c.source else {
            return Err(format!("c{cid} не шаблон Fusion"));
        };
        if style.get("template").and_then(Value::as_str).is_none_or(str::is_empty) {
            return Err(format!("у c{cid} нет шаблона Fusion"));
        }
        let key = name.strip_prefix(crate::fusion::STYLE_PREFIX).unwrap_or(name);
        return Ok(Target::Path(format!("style.{}{key}", crate::fusion::STYLE_PREFIX), "template".into()));
    }
    if head == "style" || head == "title" || head == "shape" {
        match &c.source {
            Source::Title { style, .. } => {
                if name.starts_with(crate::fusion::STYLE_PREFIX) {
                    if style.get("template").and_then(Value::as_str).is_none_or(str::is_empty) {
                        return Err(format!("у c{cid} нет шаблона Fusion для {name}"));
                    }
                    return Ok(Target::Path(format!("style.{name}"), "template".into()));
                }
                if !matches!(name, "size" | "color") {
                    return Err(format!("у титра анимируются size и color, а не {name}"));
                }
                return Ok(Target::Path(format!("style.{name}"), "style".into()));
            }
            Source::Shape { .. } => {
                if !matches!(name, "fill" | "stroke" | "width" | "height" | "radius" | "stroke_width") {
                    return Err(format!("у фигуры анимируются fill, stroke, width, height, radius, stroke_width, а не {name}"));
                }
                return Ok(Target::Path(format!("style.{name}"), "style".into()));
            }
            _ => return Err(format!("c{cid} не титр и не фигура — у него нет style.{name}")),
        }
    }
    if let Some(id) = head.strip_prefix('e').and_then(|s| s.parse::<Id>().ok()) {
        let e = c.effects.iter().find(|e| e.id == id).ok_or_else(|| format!("у c{cid} нет эффекта e{id}"))?;
        return Ok(Target::Path(format!("e{id}.{name}"), e.kind.clone()));
    }
    let fields = ANIMATABLE.iter().find(|(k, _)| *k == head).map(|(_, f)| *f).ok_or_else(|| format!("«{head}» не анимируется; можно: transform, color, blur, sharpen, vignette, lut, style"))?;
    if !fields.contains(&name) {
        return Err(format!("у {head} анимируются: {}", fields.join(", ")));
    }
    Ok(match c.effects.iter().find(|e| e.kind == head) {
        Some(e) => Target::Path(format!("e{}.{name}", e.id), head.to_string()),
        None => Target::Create(head.to_string(), name.to_string()),
    })
}

fn local(ed: &Editor, c: &Clip, at: Option<f64>) -> f64 {
    let fps = ed.project.settings.fps.max(1.0);
    let lt = (at.unwrap_or(ed.playhead) - c.start).clamp(0.0, c.duration);
    (lt * fps).round() / fps
}

static SET: Command = Command {
    id: "key.set",
    category: "Анимация",
    title: "Ключевой кадр",
    shortcut: "",
    keywords: "key keyframe ключ анимация движение масштаб поворот прозрачность цвет",
    help: "Поставить ключевой кадр параметра клипа. param: transform.x|y|scale|rotation|opacity (x, y — доли кадра от центра), color.brightness|contrast|saturation|temperature, blur.radius, vignette.amount, sharpen.amount, lut.strength, style.size|color (титр) или e<id>.<поле>; нет эффекта — добавится. at — время таймлайна (по умолчанию позиция), value — значение (по умолчанию текущее), ease — сглаживание до следующего ключа: linear, ease (плавно), in (разгон), out (торможение), back (с перелётом), elastic (пружина), bounce (отскок), hold (скачком), bezier(x1,y1,x2,y2) — своя кривая; без value у существующего ключа меняется только сглаживание. Между ключами значение меняется плавно.",
    args: &[req("id", Ty::Id, "клип (c…)"), req("param", Ty::Str, "параметр"), arg("at", Ty::Time, "момент"), arg("value", Ty::Any, "значение (число или #цвет)"), arg("ease", Ty::Str, "сглаживание: linear, ease, in, out, back, elastic, bounce, hold или bezier(x1,y1,x2,y2)")],
    danger: false,
    run: set,
};

fn set(ed: &mut Editor, a: &Args) -> Res {
    let cid = a.need_id("id")?;
    let param = a.need_str("param")?;
    let ease = a.str("ease")?;
    if let Some(e) = &ease {
        if !crate::doc::valid_ease(e) {
            return Err(format!("сглаживание «{e}»: linear, ease, in, out, back, elastic, bounce, hold или bezier(x1,y1,x2,y2)"));
        }
    }
    // всё проверяется до правки: ошибка не оставляет шага в истории
    let target = resolve(&ed.project, cid, &param)?;
    let c = ed.project.clip(cid).unwrap().clone();
    let lt = local(ed, &c, a.time("at")?);
    let (kind, name, cur) = match &target {
        Target::Path(p, k) => (k.clone(), p.split_once('.').map(|(_, n)| n.to_string()).unwrap_or_default(), c.value_at(p, lt)),
        Target::Create(k, n) => (k.clone(), n.clone(), None),
    };
    let v = match a.value("value").filter(|v| !v.is_null()) {
        Some(v) => v,
        None => cur
            .or_else(|| {
                if kind == "template" {
                    if let Source::Title { style, .. } = &c.source {
                        if let Some(tp) = style.get("template").and_then(Value::as_str) {
                            if let Ok(comp) = crate::fusion::load_cached(std::path::Path::new(tp)) {
                                return comp.param_default(&name);
                            }
                        }
                    }
                }
                default_value(&kind, &name, &ed.project)
            })
            .ok_or_else(|| format!("{param}: укажите value"))?,
    };
    if !(v.is_number() || v.as_str().is_some_and(|s| s.starts_with('#'))) {
        return Err(format!("value — число или цвет #rrggbb, а не {v}"));
    }
    ed.begin("Ключевой кадр");
    let path = match target {
        Target::Path(p, _) => p,
        Target::Create(k, n) => {
            let eid = ed.project.alloc();
            ed.project.clip_mut(cid).unwrap().effects.push(Effect { id: eid, kind: k, params: Map::new() });
            format!("e{eid}.{n}")
        }
    };
    let half = 0.5 / ed.project.settings.fps.max(1.0);
    let cm = ed.project.clip_mut(cid).unwrap();
    let keys = cm.keys.entry(path.clone()).or_default();
    let old = keys.iter().find(|k| (k.t - lt).abs() <= half).map(|k| k.ease.clone());
    keys.retain(|k| (k.t - lt).abs() > half);
    let ease = ease.or(old).unwrap_or_else(|| "linear".into());
    keys.push(Key { t: lt, v: v.clone(), ease });
    keys.sort_by(|a, b| a.t.total_cmp(&b.t));
    let n = keys.len();
    Ok(Out::with(format!("c{cid} {path} = {v} в {} (ключей: {n})", tc(c.start + lt)), json!({"path": path, "t": lt})))
}

static REMOVE: Command = Command {
    id: "key.remove",
    category: "Анимация",
    title: "Убрать ключ",
    shortcut: "",
    keywords: "key remove убрать ключ",
    help: "Убрать ключевой кадр параметра в момент at (по умолчанию позиция). Последний ключ убран — параметр снова постоянный (значение последнего ключа).",
    args: &[req("id", Ty::Id, "клип (c…)"), req("param", Ty::Str, "параметр (как в key.set)"), arg("at", Ty::Time, "момент")],
    danger: false,
    run: remove,
};

/// Путь существующих ключей без создания эффекта.
fn existing(c: &Clip, param: &str) -> Option<String> {
    if c.keys.contains_key(param) {
        return Some(param.to_string());
    }
    let (head, name) = param.split_once('.')?;
    if head == "tpl" || head == "template" {
        let key = name.strip_prefix(crate::fusion::STYLE_PREFIX).unwrap_or(name);
        return Some(format!("style.{}{key}", crate::fusion::STYLE_PREFIX));
    }
    if head == "style" || head == "title" {
        return Some(format!("style.{name}"));
    }
    c.effects.iter().find(|e| e.kind == head).map(|e| format!("e{}.{name}", e.id))
}

fn remove(ed: &mut Editor, a: &Args) -> Res {
    let cid = a.need_id("id")?;
    let param = a.need_str("param")?;
    let c = ed.project.clip(cid).ok_or_else(|| format!("нет клипа c{cid}"))?.clone();
    let path = existing(&c, &param).filter(|p| c.keys.contains_key(p)).ok_or_else(|| format!("у c{cid} нет ключей {param}"))?;
    let lt = local(ed, &c, a.time("at")?);
    let half = 0.5 / ed.project.settings.fps.max(1.0);
    let i = c.keys[&path].iter().position(|k| (k.t - lt).abs() <= half).ok_or_else(|| format!("в {} нет ключа {path}", tc(c.start + lt)))?;
    ed.begin("Убрать ключ");
    let cm = ed.project.clip_mut(cid).unwrap();
    let k = cm.keys.get_mut(&path).unwrap().remove(i);
    if cm.keys[&path].is_empty() {
        cm.keys.remove(&path);
        // постоянное значение — как у последнего ключа
        set_base(cm, &path, k.v);
    }
    Ok(Out::text(format!("Убран ключ {path} в {}", tc(c.start + lt))))
}

/// Записать постоянное значение параметра.
fn set_base(c: &mut Clip, path: &str, v: Value) {
    let Some((head, name)) = path.split_once('.') else { return };
    if head == "style" {
        match &mut c.source {
            Source::Title { style, .. } | Source::Shape { style, .. } => {
                style.insert(name.into(), v);
            }
            _ => {}
        }
    } else if let Some(id) = head.strip_prefix('e').and_then(|s| s.parse::<Id>().ok()) {
        if let Some(e) = c.effects.iter_mut().find(|e| e.id == id) {
            e.params.insert(name.into(), v);
        }
    }
}

static CLEAR: Command = Command {
    id: "key.clear",
    category: "Анимация",
    title: "Снять анимацию",
    shortcut: "",
    keywords: "key clear снять анимацию ключи",
    help: "Убрать все ключи параметра (param) или всего клипа (без param); значение остаётся тем, что было в позиции.",
    args: &[req("id", Ty::Id, "клип (c…)"), arg("param", Ty::Str, "параметр (как в key.set)")],
    danger: false,
    run: clear,
};

fn clear(ed: &mut Editor, a: &Args) -> Res {
    let cid = a.need_id("id")?;
    let c = ed.project.clip(cid).ok_or_else(|| format!("нет клипа c{cid}"))?.clone();
    let paths: Vec<String> = match a.str("param")? {
        Some(p) => vec![existing(&c, &p).filter(|x| c.keys.contains_key(x)).ok_or_else(|| format!("у c{cid} нет ключей {p}"))?],
        None => c.keys.keys().cloned().collect(),
    };
    if paths.is_empty() {
        return Err(format!("у c{cid} нет ключей"));
    }
    let lt = local(ed, &c, None);
    ed.begin("Снять анимацию");
    let cm = ed.project.clip_mut(cid).unwrap();
    for p in &paths {
        if let Some(v) = c.value_at(p, lt) {
            set_base(cm, p, v);
        }
        cm.keys.remove(p);
    }
    Ok(Out::text(format!("Снята анимация: {}", paths.join(", "))))
}

static LIST: Command = Command {
    id: "key.list",
    category: "Анимация",
    title: "Ключи клипа",
    shortcut: "",
    keywords: "key list ключи анимация",
    help: "Ключевые кадры клипа: параметр, время таймлайна, значение, сглаживание.",
    args: &[req("id", Ty::Id, "клип (c…)")],
    danger: false,
    run: list,
};

fn list(ed: &mut Editor, a: &Args) -> Res {
    let cid = a.need_id("id")?;
    let c = ed.project.clip(cid).ok_or_else(|| format!("нет клипа c{cid}"))?;
    if c.keys.is_empty() {
        return Ok(Out::text(format!("У c{cid} нет ключей")));
    }
    let mut lines = Vec::new();
    for (p, keys) in &c.keys {
        let ks: Vec<String> = keys.iter().map(|k| format!("{}={}{}", tc(c.start + k.t), k.v, if k.ease == "linear" { String::new() } else { format!(" ({})", k.ease) })).collect();
        lines.push(format!("{p}: {}", ks.join(", ")));
    }
    Ok(Out::with(lines.join("\n"), json!(c.keys)))
}

static MOVE: Command = Command {
    id: "key.move",
    category: "Анимация",
    title: "Сдвинуть ключ",
    shortcut: "",
    keywords: "key move сдвинуть ключ время значение",
    help: "Сдвинуть ключевой кадр параметра: from — его время (таймлайн), to — новое время, value — новое значение (можно одно из двух). Внутри клипа, не поверх соседнего ключа.",
    args: &[req("id", Ty::Id, "клип (c…)"), req("param", Ty::Str, "параметр (как в key.set)"), req("from", Ty::Time, "время ключа"), arg("to", Ty::Time, "новое время"), arg("value", Ty::Any, "новое значение")],
    danger: false,
    run: move_key,
};

fn move_key(ed: &mut Editor, a: &Args) -> Res {
    let cid = a.need_id("id")?;
    let param = a.need_str("param")?;
    let c = ed.project.clip(cid).ok_or_else(|| format!("нет клипа c{cid}"))?.clone();
    let path = existing(&c, &param).filter(|p| c.keys.contains_key(p)).ok_or_else(|| format!("у c{cid} нет ключей {param}"))?;
    let from = local(ed, &c, a.time("from")?);
    let half = 0.5 / ed.project.settings.fps.max(1.0);
    let i = c.keys[&path].iter().position(|k| (k.t - from).abs() <= half).ok_or_else(|| format!("в {} нет ключа {path}", tc(c.start + from)))?;
    let to = match a.time("to")? {
        Some(t) => local(ed, &c, Some(t)),
        None => c.keys[&path][i].t,
    };
    if c.keys[&path].iter().enumerate().any(|(j, k)| j != i && (k.t - to).abs() <= half) {
        return Err(format!("в {} уже есть ключ {path}", tc(c.start + to)));
    }
    let value = a.value("value").filter(|v| !v.is_null());
    if let Some(v) = &value {
        if !(v.is_number() || v.as_str().is_some_and(|s| s.starts_with('#'))) {
            return Err(format!("value — число или цвет #rrggbb, а не {v}"));
        }
    }
    ed.begin("Сдвинуть ключ");
    let keys = ed.project.clip_mut(cid).unwrap().keys.get_mut(&path).unwrap();
    keys[i].t = to;
    if let Some(v) = value {
        keys[i].v = v;
    }
    keys.sort_by(|a, b| a.t.total_cmp(&b.t));
    Ok(Out::text(format!("Ключ {path}: {} → {}", tc(c.start + from), tc(c.start + to))))
}

static MOTION_LIST: Command = Command {
    id: "motion.list",
    category: "Анимация",
    title: "Пресеты движения",
    shortcut: "",
    keywords: "motion preset пресет движение анимация вход выход зум панорама",
    help: "Список пресетов движения (вход, движение по клипу, выход): id, название, описание. Применять через motion.apply.",
    args: &[arg("query", Ty::Str, "поиск по названию или разделу")],
    danger: false,
    run: motion_list,
};

fn motion_list(_ed: &mut Editor, a: &Args) -> Res {
    let q = a.str("query")?.unwrap_or_default().to_lowercase();
    let hits: Vec<_> = crate::motion::PRESETS
        .iter()
        .filter(|p| q.is_empty() || format!("{} {} {} {}", p.id, p.title, p.category, p.desc).to_lowercase().contains(&q))
        .collect();
    let lines: Vec<String> = hits.iter().map(|p| format!("{} [{}] — {}: {}", p.id, p.category, p.title, p.desc)).collect();
    let data: Vec<_> = hits.iter().map(|p| json!({"id": p.id, "title": p.title, "category": p.category, "desc": p.desc})).collect();
    Ok(Out::with(lines.join("\n"), json!(data)))
}

static MOTION_APPLY: Command = Command {
    id: "motion.apply",
    category: "Анимация",
    title: "Пресет движения",
    shortcut: "",
    keywords: "motion preset apply движение выезд подъем выпрыгивание отскок наезд отъезд панорама пульсация покачивание уход",
    help: "Применить пресет движения к клипам (создаёт ключевые кадры эффекта transform; без ids — выделенные). Пресеты: slide_in_left, slide_in_right, slide_in_up, pop_in, bounce_in, spin_in (вход), ken_burns_in, ken_burns_out, pan_left, pan_right, float, pulse (движение по клипу), zoom_out_exit, slide_out_down (выход). Вход и выход сочетаются на одном клипе.",
    args: &[req("preset", Ty::Str, "id пресета (motion.list)"), arg("ids", Ty::Ids, "клипы (c…), по умолчанию выделенные"), arg("duration", Ty::Time, "длительность анимации, с")],
    danger: false,
    run: motion_apply,
};

fn motion_apply(ed: &mut Editor, a: &Args) -> Res {
    let name = a.need_str("preset")?;
    let preset = crate::motion::find(&name).ok_or_else(|| format!("нет пресета «{name}»; есть: {}", crate::motion::PRESETS.iter().map(|p| p.id).collect::<Vec<_>>().join(", ")))?;
    let ids = a.ids("ids")?.unwrap_or_else(|| ed.selection.clone());
    if ids.is_empty() {
        return Err("не к чему применить: выделите клипы или передайте ids".into());
    }
    let dur = a.time("duration")?;
    for id in &ids {
        ed.project.find_clip(*id).ok_or_else(|| format!("нет клипа c{id}"))?;
    }
    ed.begin(&format!("Движение: {}", preset.title));
    for id in &ids {
        crate::motion::apply(&mut ed.project, *id, preset, dur)?;
    }
    Ok(Out::text(format!("Движение «{}» → {}", preset.title, ids.iter().map(|i| format!("c{i}")).collect::<Vec<_>>().join(", "))))
}
