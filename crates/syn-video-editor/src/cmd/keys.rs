//! Ключевые кадры: анимация параметров эффектов (положение, масштаб, поворот,
//! прозрачность, цвет, размытие…) и титра (размер, цвет). Время ключа — время
//! таймлайна в команде, внутри клипа хранится относительно его начала.

use serde_json::{json, Map, Value};

use super::{arg, req, Args, Command, Out, Res, Ty};
use crate::doc::{tc, Clip, Effect, Id, Key, Source};
use crate::editor::Editor;

pub static COMMANDS: &[&Command] = &[&SET, &REMOVE, &CLEAR, &LIST];

const EASES: &[&str] = &["linear", "ease", "in", "out", "hold"];

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
        ("style", "size") => n(p.settings.width.min(p.settings.height) as f64 * 0.075),
        ("style", "color") => Some(json!("#ffffff")),
        ("style", "anim") => n(0.4),
        _ => None,
    }
}

/// Параметры, которые можно анимировать: эффект → поля.
const ANIMATABLE: &[(&str, &[&str])] = &[
    ("transform", &["x", "y", "scale", "rotation", "opacity"]),
    ("color", &["brightness", "contrast", "saturation", "temperature"]),
    ("blur", &["radius"]),
    ("sharpen", &["amount"]),
    ("vignette", &["amount"]),
    ("lut", &["strength"]),
    ("style", &["size", "color"]),
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
    if head == "style" || head == "title" {
        if !matches!(c.source, Source::Title { .. }) {
            return Err(format!("c{cid} не титр — у него нет style.{name}"));
        }
        if !matches!(name, "size" | "color") {
            return Err(format!("у титра анимируются size и color, а не {name}"));
        }
        return Ok(Target::Path(format!("style.{name}"), "style".into()));
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
    help: "Поставить ключевой кадр параметра клипа. param: transform.x|y|scale|rotation|opacity (x, y — доли кадра от центра), color.brightness|contrast|saturation|temperature, blur.radius, vignette.amount, sharpen.amount, lut.strength, style.size|color (титр) или e<id>.<поле>; нет эффекта — добавится. at — время таймлайна (по умолчанию позиция), value — значение (по умолчанию текущее), ease — сглаживание до следующего ключа: linear, ease, in, out, hold. Между ключами значение меняется плавно.",
    args: &[req("id", Ty::Id, "клип (c…)"), req("param", Ty::Str, "параметр"), arg("at", Ty::Time, "момент"), arg("value", Ty::Any, "значение (число или #цвет)"), arg("ease", Ty::Choice(EASES), "сглаживание")],
    danger: false,
    run: set,
};

fn set(ed: &mut Editor, a: &Args) -> Res {
    let cid = a.need_id("id")?;
    let param = a.need_str("param")?;
    let ease = a.str("ease")?.unwrap_or_else(|| "linear".into());
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
        None => cur.or_else(|| default_value(&kind, &name, &ed.project)).ok_or_else(|| format!("{param}: укажите value"))?,
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
    keys.retain(|k| (k.t - lt).abs() > half);
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
        if let Source::Title { style, .. } = &mut c.source {
            style.insert(name.into(), v);
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
