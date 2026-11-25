//! Граф кадра клипа (D1): открыть эффекты как граф, ноды, связи, слияние.

use serde_json::{json, Map, Value};

use super::{arg, req, Args, Command, Out, Res, Ty};
use crate::doc::{Effect, Id};
use crate::editor::Editor;
use crate::framegraph::{FrameGraph, GLink, GNode, IMAGE_FX, KINDS};

pub static COMMANDS: &[&Command] = &[&OPEN, &SHOW, &SET, &ADD, &LINK, &UNLINK, &NODE, &REMOVE, &CLOSE];

fn kinds_help() -> String {
    KINDS.iter().map(|(k, t, ins, p)| format!("{k} ({t}{}{})", if ins.is_empty() { String::new() } else { format!("; входы {}", ins.join(",")) }, if p.is_empty() { String::new() } else { format!("; {p}") })).collect::<Vec<_>>().join("; ")
}

fn graph_of(ed: &Editor, id: Id) -> Result<FrameGraph, String> {
    let c = ed.project.clip(id).ok_or_else(|| format!("нет клипа c{id}"))?;
    c.graph.clone().ok_or_else(|| format!("у c{id} нет графа кадра — graph.open"))
}

/// Записать граф клипа (с проверкой) одним шагом отмены.
fn store(ed: &mut Editor, id: Id, g: FrameGraph, label: &str) -> Res {
    store_as(ed, id, g, label, "Граф кадра")
}

/// `step` — подпись шага отмены (правки с холста склеиваются: «Граф кадра (холст)»).
fn store_as(ed: &mut Editor, id: Id, mut g: FrameGraph, label: &str, step: &str) -> Res {
    g.validate()?;
    if g.crowded() {
        g.layout();
    }
    ed.begin(step);
    let text = g.describe();
    ed.project.clip_mut(id).unwrap().graph = Some(g);
    Ok(Out::with(format!("c{id}: {label}\n{text}"), json!({"id": id})))
}

static OPEN: Command = Command {
    id: "graph.open",
    category: "Граф",
    title: "Открыть как граф",
    shortcut: "",
    keywords: "graph граф кадра ноды узлы ветвление merge слияние эффекты",
    help: "Открыть кадр клипа как граф: эффекты изображения (color, blur, sharpen, vignette, lut, upscale) становятся цепочкой нод source → … → output; дальше можно ветвить и сводить нодой merge (например, размытая копия в режиме screen — свечение). Положение, затухания, маска и наложение слоя остаются свойствами клипа. Уже открыт — показывает граф.",
    args: &[req("id", Ty::Id, "клип (c…)")],
    danger: false,
    run: open,
};

fn open(ed: &mut Editor, a: &Args) -> Res {
    let id = a.need_id("id")?;
    let c = ed.project.clip(id).ok_or_else(|| format!("нет клипа c{id}"))?.clone();
    if let Some(g) = &c.graph {
        return Ok(Out::with(format!("c{id}: граф уже открыт\n{}", g.describe()), json!({"id": id})));
    }
    if ed.project.find_clip(id).is_some_and(|(ti, _)| ed.project.tracks[ti].kind == crate::doc::TrackKind::Audio) {
        return Err(format!("c{id} — звуковой клип"));
    }
    let g = FrameGraph::chain(&c.effects);
    let moved: Vec<Id> = c.effects.iter().filter(|e| IMAGE_FX.contains(&e.kind.as_str())).map(|e| e.id).collect();
    let lost_keys = c.keys.keys().filter(|k| moved.iter().any(|m| k.starts_with(&format!("e{m}.")))).count();
    ed.begin("Граф кадра");
    let cm = ed.project.clip_mut(id).unwrap();
    cm.effects.retain(|e| !IMAGE_FX.contains(&e.kind.as_str()));
    cm.keys.retain(|k, _| !moved.iter().any(|m| k.starts_with(&format!("e{m}."))));
    let text = g.describe();
    cm.graph = Some(g);
    let warn = if lost_keys > 0 { format!(" (ключи анимации этих эффектов сняты: {lost_keys})") } else { String::new() };
    Ok(Out::with(format!("c{id}: открыт граф кадра{warn}\n{text}"), json!({"id": id})))
}

static SHOW: Command = Command {
    id: "graph.show",
    category: "Граф",
    title: "Граф кадра текстом",
    shortcut: "",
    keywords: "graph граф кадра показать ноды",
    help: "Ноды графа кадра клипа: id, вид, входы (порт←нода), параметры.",
    args: &[req("id", Ty::Id, "клип (c…)")],
    danger: false,
    run: show,
};

fn show(ed: &mut Editor, a: &Args) -> Res {
    let id = a.need_id("id")?;
    let g = graph_of(ed, id)?;
    Ok(Out::with(format!("c{id}:\n{}", g.describe()), serde_json::to_value(&g).unwrap_or(Value::Null)))
}

static SET: Command = Command {
    id: "graph.set",
    category: "Граф",
    title: "Задать граф кадра",
    shortcut: "",
    keywords: "graph граф кадра задать целиком",
    help: "Задать граф кадра целиком: graph = {nodes: [{id, kind, params}], links: [{from, to, port}]}. Ровно один source и один output; порт по умолчанию in, у merge — bg и fg.",
    args: &[req("id", Ty::Id, "клип (c…)"), req("graph", Ty::Object, "граф {nodes, links}")],
    danger: false,
    run: set,
};

fn set(ed: &mut Editor, a: &Args) -> Res {
    let id = a.need_id("id")?;
    ed.project.clip(id).ok_or_else(|| format!("нет клипа c{id}"))?;
    let raw = a.object("graph")?.unwrap_or_default();
    let mut g: FrameGraph = serde_json::from_value(Value::Object(raw)).map_err(|e| format!("graph: {e}"))?;
    // без координат — лесенкой
    for (i, n) in g.nodes.iter_mut().enumerate() {
        if n.x == 0.0 && n.y == 0.0 && i > 0 {
            n.x = 220.0 * i as f32;
        }
    }
    let had = ed.project.clip(id).unwrap().graph.is_some();
    let r = store_as(ed, id, g, "граф задан", "Граф кадра (холст)")?;
    if !had {
        let cm = ed.project.clip_mut(id).unwrap();
        cm.effects.retain(|e| !IMAGE_FX.contains(&e.kind.as_str()));
    }
    Ok(r)
}

static ADD: Command = Command {
    id: "graph.add",
    category: "Граф",
    title: "Добавить ноду",
    shortcut: "",
    keywords: "graph граф нода добавить вставить",
    help: "Добавить ноду в граф кадра. after — вставить в цепочку после этой ноды (её потребители получат новую ноду; у merge — на вход bg). Без after нода свободна — соедините graph.link. Виды: color {brightness, contrast, saturation, temperature}, blur {radius}, sharpen {amount}, vignette {amount}, lut {path, strength}, glow {size, strength}, upscale {model}, merge {mode, opacity; входы bg, fg}.",
    args: &[req("id", Ty::Id, "клип (c…)"), req("kind", Ty::Str, "вид ноды"), arg("params", Ty::Object, "параметры"), arg("after", Ty::Num, "вставить после ноды (id)")],
    danger: false,
    run: add,
};

fn add(ed: &mut Editor, a: &Args) -> Res {
    let id = a.need_id("id")?;
    let mut g = graph_of(ed, id)?;
    let kind = a.need_str("kind")?;
    if matches!(kind.as_str(), "source" | "output") || !KINDS.iter().any(|k| k.0 == kind) {
        return Err(format!("нельзя добавить «{kind}»; виды: {}", kinds_help()));
    }
    let nid = g.next_id();
    let after = a.num("after")?.map(|v| v as u32);
    let (x, y) = match after.and_then(|f| g.node(f)) {
        Some(n) => (n.x + 110.0, n.y + 90.0),
        None => (g.nodes.iter().map(|n| n.x).fold(0.0, f32::max) / 2.0, 160.0),
    };
    g.nodes.push(GNode { id: nid, kind: kind.clone(), params: a.object("params")?.unwrap_or_default(), x, y });
    if let Some(f) = after {
        if g.node(f).is_none() {
            return Err(format!("нет ноды {f}"));
        }
        for l in g.links.iter_mut().filter(|l| l.from == f) {
            l.from = nid;
        }
        let port = if kind == "merge" { "bg" } else { "in" };
        g.links.push(GLink { from: f, to: nid, port: port.into() });
    }
    store(ed, id, g, &format!("нода {nid} {kind}"))
}

static LINK: Command = Command {
    id: "graph.link",
    category: "Граф",
    title: "Соединить ноды",
    shortcut: "",
    keywords: "graph граф связь соединить провод",
    help: "Соединить выход ноды from со входом port ноды to (in; у merge — bg или fg). Прежняя связь на этот вход заменяется.",
    args: &[req("id", Ty::Id, "клип (c…)"), req("from", Ty::Num, "откуда (id ноды)"), req("to", Ty::Num, "куда (id ноды)"), arg("port", Ty::Str, "вход (in)")],
    danger: false,
    run: link,
};

fn link(ed: &mut Editor, a: &Args) -> Res {
    let id = a.need_id("id")?;
    let mut g = graph_of(ed, id)?;
    let (from, to) = (a.num("from")?.unwrap_or(0.0) as u32, a.num("to")?.unwrap_or(0.0) as u32);
    let port = a.str("port")?.unwrap_or_else(|| "in".into());
    g.links.retain(|l| !(l.to == to && l.port == port));
    g.links.push(GLink { from, to, port: port.clone() });
    store(ed, id, g, &format!("{from} → {to}.{port}"))
}

static UNLINK: Command = Command {
    id: "graph.unlink",
    category: "Граф",
    title: "Разорвать связь",
    shortcut: "",
    keywords: "graph граф связь разорвать",
    help: "Убрать связь, входящую в port ноды to.",
    args: &[req("id", Ty::Id, "клип (c…)"), req("to", Ty::Num, "нода (id)"), arg("port", Ty::Str, "вход (in)")],
    danger: false,
    run: unlink,
};

fn unlink(ed: &mut Editor, a: &Args) -> Res {
    let id = a.need_id("id")?;
    let mut g = graph_of(ed, id)?;
    let to = a.num("to")?.unwrap_or(0.0) as u32;
    let port = a.str("port")?.unwrap_or_else(|| "in".into());
    let n = g.links.len();
    g.links.retain(|l| !(l.to == to && l.port == port));
    if g.links.len() == n {
        return Err(format!("на {to}.{port} связи нет"));
    }
    store(ed, id, g, &format!("связь на {to}.{port} убрана"))
}

static NODE: Command = Command {
    id: "graph.node",
    category: "Граф",
    title: "Параметры ноды",
    shortcut: "",
    keywords: "graph граф нода параметры",
    help: "Изменить параметры ноды графа кадра (поля сливаются); x, y — положение на холсте.",
    args: &[req("id", Ty::Id, "клип (c…)"), req("node", Ty::Num, "нода (id)"), arg("params", Ty::Object, "параметры"), arg("x", Ty::Num, "x на холсте"), arg("y", Ty::Num, "y на холсте")],
    danger: false,
    run: node,
};

fn node(ed: &mut Editor, a: &Args) -> Res {
    let id = a.need_id("id")?;
    let mut g = graph_of(ed, id)?;
    let nid = a.num("node")?.unwrap_or(0.0) as u32;
    let n = g.nodes.iter_mut().find(|n| n.id == nid).ok_or_else(|| format!("нет ноды {nid}"))?;
    if let Some(p) = a.object("params")? {
        n.params.extend(p);
    }
    if let Some(x) = a.num("x")? {
        n.x = x as f32;
    }
    if let Some(y) = a.num("y")? {
        n.y = y as f32;
    }
    let kind = n.kind.clone();
    store(ed, id, g, &format!("нода {nid} {kind}"))
}

static REMOVE: Command = Command {
    id: "graph.remove",
    category: "Граф",
    title: "Убрать ноду",
    shortcut: "",
    keywords: "graph граф нода удалить убрать",
    help: "Убрать ноду из графа кадра; её вход (in, у merge — bg) соединяется с её потребителями. source и output не убираются.",
    args: &[req("id", Ty::Id, "клип (c…)"), req("node", Ty::Num, "нода (id)")],
    danger: false,
    run: remove,
};

fn remove(ed: &mut Editor, a: &Args) -> Res {
    let id = a.need_id("id")?;
    let mut g = graph_of(ed, id)?;
    let nid = a.num("node")?.unwrap_or(0.0) as u32;
    let n = g.node(nid).ok_or_else(|| format!("нет ноды {nid}"))?.clone();
    if matches!(n.kind.as_str(), "source" | "output") {
        return Err(format!("{} не убирается", n.kind));
    }
    let up = g.input(nid, if n.kind == "merge" { "bg" } else { "in" });
    g.links.retain(|l| l.to != nid);
    match up {
        Some(u) => {
            for l in g.links.iter_mut().filter(|l| l.from == nid) {
                l.from = u;
            }
        }
        None => g.links.retain(|l| l.from != nid),
    }
    g.nodes.retain(|x| x.id != nid);
    store(ed, id, g, &format!("нода {nid} убрана"))
}

static CLOSE: Command = Command {
    id: "graph.close",
    category: "Граф",
    title: "Закрыть граф",
    shortcut: "",
    keywords: "graph граф закрыть эффекты цепочка",
    help: "Вернуть кадр клипа к списку эффектов: простая цепочка становится эффектами по порядку. С ветвлением или слиянием — только с force: true (граф отбрасывается).",
    args: &[req("id", Ty::Id, "клип (c…)"), arg("force", Ty::Bool, "отбросить граф с ветвлением")],
    danger: false,
    run: close,
};

fn close(ed: &mut Editor, a: &Args) -> Res {
    let id = a.need_id("id")?;
    let g = graph_of(ed, id)?;
    // свечения в списке эффектов нет — с ним граф не сворачивается
    let chain: Option<Vec<(String, Map<String, Value>)>> = g.as_chain().filter(|v| v.iter().all(|n| n.kind != "glow")).map(|v| v.into_iter().map(|n| (n.kind.clone(), n.params.clone())).collect());
    let force = a.bool("force")?.unwrap_or(false);
    if chain.is_none() && !force {
        return Err(format!("c{id}: в графе ветвление, слияние или свечение — эффектами не выразить; force: true отбросит граф"));
    }
    ed.begin("Граф кадра");
    let mut made = Vec::new();
    for (kind, params) in chain.unwrap_or_default() {
        let eid = ed.project.alloc();
        made.push(format!("{kind} e{eid}"));
        ed.project.clip_mut(id).unwrap().effects.push(Effect { id: eid, kind, params });
    }
    ed.project.clip_mut(id).unwrap().graph = None;
    Ok(Out::text(format!("c{id}: граф закрыт{}", if made.is_empty() { String::new() } else { format!(", эффекты: {}", made.join(", ")) })))
}
