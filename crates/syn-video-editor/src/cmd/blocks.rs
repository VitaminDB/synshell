//! Блоки графа кадра (D2): список, наложение, вынесенные параметры, сохранение своих.

use serde_json::{json, Map, Value};

use super::{arg, req, Args, Command, Out, Res, Ty};
use crate::blocks::{self, Block, Expose};
use crate::editor::Editor;
use crate::framegraph::{FrameGraph, GLink, GNode};

pub static COMMANDS: &[&Command] = &[&LIST, &APPLY, &SET, &SAVE];

static LIST: Command = Command {
    id: "block.list",
    category: "Граф",
    title: "Блоки",
    shortcut: "",
    keywords: "block блок пресет образ look свечение кино сон чб неон",
    help: "Блоки — готовые подграфы кадра с вынесенными параметрами (встроенные и свои): имя, описание, параметры (ключ, пределы, по умолчанию).",
    args: &[],
    danger: false,
    run: list,
};

fn list(_ed: &mut Editor, _a: &Args) -> Res {
    let all = blocks::all();
    let lines: Vec<String> = all
        .iter()
        .map(|b| format!("{}{} — {}; параметры: {}", b.name, if b.builtin { "" } else { " (свой)" }, b.description, b.expose.iter().map(|e| format!("{} «{}» {}…{} ({})", e.key, e.label, e.min, e.max, e.default)).collect::<Vec<_>>().join(", ")))
        .collect();
    Ok(Out::with(lines.join("\n"), json!(all.iter().map(|b| json!({"name": b.name, "builtin": b.builtin})).collect::<Vec<_>>())))
}

static APPLY: Command = Command {
    id: "block.apply",
    category: "Граф",
    title: "Наложить блок",
    shortcut: "",
    keywords: "block блок наложить применить",
    help: "Наложить блок на клипы (ids, по умолчанию выделенные): у клипа открывается граф кадра, блок встаёт нодой перед выходом; values — значения вынесенных параметров {ключ: число}. Один шаг отмены.",
    args: &[arg("ids", Ty::Ids, "клипы (c…), по умолчанию выделенные"), req("name", Ty::Str, "блок (block.list)"), arg("values", Ty::Object, "вынесенные параметры")],
    danger: false,
    run: apply,
};

fn apply(ed: &mut Editor, a: &Args) -> Res {
    let name = a.need_str("name")?;
    let b = blocks::find(&name).ok_or_else(|| format!("блока «{name}» нет — block.list"))?;
    let values = a.object("values")?;
    let ids = a.ids("ids")?.unwrap_or_else(|| ed.selection.clone());
    let video: Vec<u64> = ids.iter().copied().filter(|id| ed.project.find_clip(*id).is_some_and(|(ti, _)| ed.project.tracks[ti].kind == crate::doc::TrackKind::Video)).collect();
    if video.is_empty() {
        return Err("нет видеоклипов: выделите клип или укажите ids".into());
    }
    // всё на копии: ошибка — без изменений
    let mut tmp = Editor::new();
    tmp.project = ed.project.clone();
    let mut made = Vec::new();
    for id in &video {
        super::graph::ensure(&mut tmp, *id)?;
        let g = tmp.project.clip_mut(*id).unwrap().graph.as_mut().unwrap();
        let nid = g.next_id();
        let (oid, ox, oy) = g.nodes.iter().find(|n| n.kind == "output").map(|n| (n.id, n.x, n.y)).ok_or("нет выхода")?;
        let prev = g.input(oid, "in");
        g.links.retain(|l| !(l.to == oid && l.port == "in"));
        if let Some(p) = prev {
            g.links.push(GLink { from: p, to: nid, port: "in".into() });
        }
        g.links.push(GLink { from: nid, to: oid, port: "in".into() });
        g.nodes.push(GNode { id: nid, kind: "block".into(), params: blocks::node_params(&b, values.as_ref()), x: ox, y: oy });
        if let Some(o) = g.nodes.iter_mut().find(|n| n.id == oid) {
            o.x += 260.0;
        }
        g.validate()?;
        if g.crowded() {
            g.layout();
        }
        made.push(format!("c{id}: нода {nid}"));
    }
    ed.begin("Блок");
    ed.project = tmp.project;
    Ok(Out::text(format!("Блок «{}»: {}", b.name, made.join(", "))))
}

static SET: Command = Command {
    id: "block.set",
    category: "Граф",
    title: "Параметры блока",
    shortcut: "",
    keywords: "block блок параметры значения",
    help: "Изменить вынесенные параметры ноды-блока графа кадра: values {ключ: число} (сливаются).",
    args: &[req("id", Ty::Id, "клип (c…)"), req("node", Ty::Num, "нода блока (id)"), req("values", Ty::Object, "значения")],
    danger: false,
    run: set,
};

fn set(ed: &mut Editor, a: &Args) -> Res {
    let id = a.need_id("id")?;
    let nid = a.num("node")?.unwrap_or(0.0) as u32;
    let vals = a.object("values")?.unwrap_or_default();
    let c = ed.project.clip(id).ok_or_else(|| format!("нет клипа c{id}"))?;
    let n = c.graph.as_ref().and_then(|g| g.node(nid)).filter(|n| n.kind == "block").ok_or_else(|| format!("у c{id} нет ноды-блока {nid}"))?;
    let ex = blocks::exposes(&n.params);
    let mut cur = n.params.get("values").and_then(Value::as_object).cloned().unwrap_or_default();
    for (k, v) in vals {
        let e = ex.iter().find(|e| e.key == k).ok_or_else(|| format!("у блока нет параметра «{k}»; есть: {}", ex.iter().map(|e| e.key.as_str()).collect::<Vec<_>>().join(", ")))?;
        let x = v.as_f64().ok_or_else(|| format!("{k}: нужно число"))?;
        cur.insert(k, json!(x.clamp(e.min, e.max)));
    }
    ed.begin("Параметры эффекта");
    let n = ed.project.clip_mut(id).unwrap().graph.as_mut().unwrap().nodes.iter_mut().find(|n| n.id == nid).unwrap();
    n.params.insert("values".into(), Value::Object(cur.clone()));
    Ok(Out::text(format!("c{id} блок {nid}: {}", Value::Object(cur))))
}

static SAVE: Command = Command {
    id: "block.save",
    category: "Граф",
    title: "Сохранить блок",
    shortcut: "",
    keywords: "block блок сохранить свой подграф",
    help: "Сохранить часть графа кадра клипа своим блоком: nodes — id нод (по умолчанию все, кроме кадра и выхода); у них один вход извне и один выход наружу. expose — вынесенные параметры [{key, label, node, param, min, max}] (по умолчанию — все числовые параметры нод). Блок появляется в «Эффектах» и block.list.",
    args: &[req("id", Ty::Id, "клип (c…)"), req("name", Ty::Str, "имя блока"), arg("description", Ty::Str, "описание"), arg("nodes", Ty::Ids, "ноды (id)"), arg("expose", Ty::Any, "вынесенные параметры")],
    danger: false,
    run: save,
};

fn save(ed: &mut Editor, a: &Args) -> Res {
    let id = a.need_id("id")?;
    let name = a.need_str("name")?.trim().to_string();
    if name.is_empty() {
        return Err("пустое имя".into());
    }
    let g = ed.project.clip(id).and_then(|c| c.graph.clone()).ok_or_else(|| format!("у c{id} нет графа кадра"))?;
    let pick: Vec<u32> = match a.ids("nodes")? {
        Some(v) => v.into_iter().map(|x| x as u32).collect(),
        None => g.nodes.iter().filter(|n| !matches!(n.kind.as_str(), "source" | "output")).map(|n| n.id).collect(),
    };
    if pick.is_empty() || pick.iter().any(|p| g.node(*p).is_none_or(|n| matches!(n.kind.as_str(), "source" | "output"))) {
        return Err("nodes: ноды графа, кроме кадра и выхода".into());
    }
    // вход: все связи извне — из одной ноды; выход: наружу уходит одна нода
    let ins: Vec<&GLink> = g.links.iter().filter(|l| pick.contains(&l.to) && !pick.contains(&l.from)).collect();
    let outs: Vec<u32> = {
        let mut v: Vec<u32> = g.links.iter().filter(|l| pick.contains(&l.from) && !pick.contains(&l.to)).map(|l| l.from).collect();
        v.dedup();
        v
    };
    let src_from: Vec<u32> = {
        let mut v: Vec<u32> = ins.iter().map(|l| l.from).collect();
        v.sort();
        v.dedup();
        v
    };
    if src_from.len() > 1 || outs.len() != 1 {
        return Err("у выбранных нод должен быть один вход извне и один выход наружу".into());
    }
    // новая нумерация: 1 — кадр (вход блока), дальше ноды, последняя — выход
    let mut map = std::collections::HashMap::new();
    let mut sub = FrameGraph { nodes: vec![GNode { id: 1, kind: "source".into(), params: Map::new(), x: 0.0, y: 0.0 }], links: Vec::new() };
    for (i, p) in pick.iter().enumerate() {
        let n = g.node(*p).unwrap();
        map.insert(*p, i as u32 + 2);
        sub.nodes.push(GNode { id: i as u32 + 2, kind: n.kind.clone(), params: n.params.clone(), x: 0.0, y: 0.0 });
    }
    let out_id = pick.len() as u32 + 2;
    sub.nodes.push(GNode { id: out_id, kind: "output".into(), params: Map::new(), x: 0.0, y: 0.0 });
    for l in &g.links {
        match (map.get(&l.from), map.get(&l.to)) {
            (Some(f), Some(t)) => sub.links.push(GLink { from: *f, to: *t, port: l.port.clone() }),
            (None, Some(t)) => sub.links.push(GLink { from: 1, to: *t, port: l.port.clone() }),
            _ => {}
        }
    }
    sub.links.push(GLink { from: map[&outs[0]], to: out_id, port: "in".into() });
    sub.validate()?;
    sub.layout();
    let expose: Vec<Expose> = match a.value("expose") {
        Some(v) => {
            let mut ex: Vec<Expose> = serde_json::from_value(v).map_err(|e| format!("expose: {e}"))?;
            for e in &mut ex {
                let nn = map.get(&e.node).copied().ok_or_else(|| format!("expose: нода {} не в блоке", e.node))?;
                e.default = sub.node(nn).and_then(|n| n.params.get(&e.param)).and_then(Value::as_f64).unwrap_or(e.default);
                e.node = nn;
            }
            ex
        }
        None => sub
            .nodes
            .iter()
            .flat_map(|n| n.params.iter().filter_map(move |(k, v)| v.as_f64().map(|x| (n, k.clone(), x))))
            .map(|(n, k, x)| Expose { key: format!("n{}_{k}", n.id), label: format!("{}: {k}", crate::framegraph::kind_title(&n.kind)), node: n.id, param: k, min: if x < 0.0 { x * 2.0 } else { 0.0 }, max: (x.abs() * 2.0).max(1.0), default: x })
            .collect(),
    };
    let b = Block { name: name.clone(), description: a.str("description")?.unwrap_or_default(), graph: sub, expose, builtin: false };
    let f = blocks::file_for(&name);
    std::fs::create_dir_all(blocks::dir()).map_err(|e| e.to_string())?;
    std::fs::write(&f, serde_json::to_string_pretty(&b).unwrap_or_default()).map_err(|e| format!("{}: {e}", f.display()))?;
    Ok(Out::text(format!("Блок «{name}» сохранён: {} нод, параметров {} — {}", pick.len(), b.expose.len(), f.display())))
}
