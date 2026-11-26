//! Подграф на холсте: любые ноды (ядра, звуковые, просмотр, картинки, ноды
//! программ, другие подграфы) сворачиваются в одну ноду-группу. Граф не
//! меняется — участники остаются в графе со всеми проводами и считаются как
//! прежде, только скрыты внутри группы. Провода через границу группы
//! рисуются к её портам; портов столько, сколько нужно; подпись порта —
//! «нода: порт» внутри. Новый провод к порту группы подключается к
//! настоящей внутренней ноде. Группы вкладываются друг в друга.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use syngui::core::Point;
use syngui::prelude::*;

use crate::registry;
use crate::state::NodeEditorCtx;
use crate::types::*;

/// Порт группы и внутренняя нода с её портом за ним.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Proxy {
    pub name: &'static str,
    pub node: NodeId,
    pub port: &'static str,
}

#[derive(Clone, Debug, Default)]
pub struct GroupPorts {
    pub ins: Vec<Proxy>,
    pub outs: Vec<Proxy>,
    ins_schema: &'static [PortSchema],
    outs_schema: &'static [PortSchema],
}

/// Состояние ноды-группы.
#[derive(Clone)]
pub struct GroupState {
    pub title: RwSignal<String>,
    /// Непосредственные участники (ноды и вложенные группы).
    pub members: RwSignal<Vec<NodeId>>,
    /// Центр участников при объединении: «Разгруппировать» сдвигает их за группой.
    pub origin: RwSignal<Point>,
    pub ports: Arc<Mutex<GroupPorts>>,
    /// Номер набора портов — карточка пересобирается при изменении.
    pub ver: RwSignal<u64>,
}

impl GroupState {
    pub fn new() -> Self {
        Self { title: use_signal("Подграф".to_string()), members: use_signal(Vec::new()), origin: use_signal(Point::zero()), ports: Arc::new(Mutex::new(GroupPorts::default())), ver: use_signal(0) }
    }
}

impl Default for GroupState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn new_runtime() -> NodeRuntime {
    NodeRuntime::Group(GroupState::new())
}

pub fn state(node: &NodeInstance) -> Option<GroupState> {
    match &*node.runtime.lock().ok()? {
        NodeRuntime::Group(g) => Some(g.clone()),
        _ => None,
    }
}

fn leak_str(s: String) -> &'static str {
    static CACHE: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let mut c = CACHE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if let Some(v) = c.get(&s) {
        return v;
    }
    let v: &'static str = Box::leak(s.clone().into_boxed_str());
    c.insert(s, v);
    v
}

fn leak_schema(v: Vec<PortSchema>) -> &'static [PortSchema] {
    static CACHE: OnceLock<Mutex<HashMap<Vec<(&'static str, &'static str)>, &'static [PortSchema]>>> = OnceLock::new();
    let key: Vec<(&'static str, &'static str)> = v.iter().map(|p| (p.name, p.label)).collect();
    let mut c = CACHE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = c.get(&key) {
        if s.iter().zip(&v).all(|(a, b)| a.kind == b.kind) {
            return s;
        }
    }
    let s: &'static [PortSchema] = Box::leak(v.into_boxed_slice());
    c.insert(key, s);
    s
}

/// `PortsSpec::Dynamic.runtime` — входы группы.
pub fn inputs(node: &NodeInstance) -> &'static [PortSchema] {
    state(node).and_then(|g| g.ports.lock().ok().map(|p| p.ins_schema)).unwrap_or(&[])
}

/// `PortsSpec::Dynamic.runtime` — выходы группы.
pub fn outputs(node: &NodeInstance) -> &'static [PortSchema] {
    state(node).and_then(|g| g.ports.lock().ok().map(|p| p.outs_schema)).unwrap_or(&[])
}

/// Группа-владелец каждой скрытой ноды (непосредственная).
pub fn owners(nodes: &[NodeInstance]) -> HashMap<NodeId, NodeId> {
    let mut m = HashMap::new();
    for n in nodes.iter().filter(|n| n.kind == NodeKind::Group) {
        if let Some(g) = state(n) {
            for id in g.members.get_untracked() {
                m.insert(id, n.id);
            }
        }
    }
    m
}

/// Видимая нода, за которой спрятана `id` (сама `id`, если не скрыта).
pub fn top(owners: &HashMap<NodeId, NodeId>, mut id: NodeId) -> NodeId {
    let mut guard = 0;
    while let Some(o) = owners.get(&id) {
        id = *o;
        guard += 1;
        if guard > 64 {
            break;
        }
    }
    id
}

/// Все потомки группы (ноды и вложенные группы).
pub fn descendants(nodes: &[NodeInstance], gid: NodeId) -> HashSet<NodeId> {
    let mut out = HashSet::new();
    let mut stack = vec![gid];
    while let Some(id) = stack.pop() {
        let Some(n) = nodes.iter().find(|n| n.id == id) else { continue };
        if let Some(g) = state(n) {
            for m in g.members.get_untracked() {
                if out.insert(m) {
                    stack.push(m);
                }
            }
        }
    }
    out
}

fn node_label(n: &NodeInstance) -> String {
    let own = n.title.get_untracked();
    if !own.trim().is_empty() {
        return own;
    }
    if let Some(g) = state(n) {
        return g.title.get_untracked();
    }
    crate::i18n::node_title(registry::meta(n.kind))
}

/// Пересчитать порты всех групп по текущим проводам.
pub fn refresh(ctx: &NodeEditorCtx) {
    let nodes = ctx.nodes.get_untracked();
    let conns = ctx.connections.get_untracked();
    for g in nodes.iter().filter(|n| n.kind == NodeKind::Group) {
        let Some(st) = state(g) else { continue };
        let desc = descendants(&nodes, g.id);
        let real = |id: &NodeId| desc.contains(id) && nodes.iter().any(|n| n.id == *id && n.kind != NodeKind::Group);
        let pos = |id: NodeId| nodes.iter().find(|n| n.id == id).map(|n| n.pos.get_untracked()).unwrap_or_default();
        // входы: внутренние порты, на которые идёт провод снаружи
        let mut ins: Vec<(NodeId, &'static str)> = Vec::new();
        for c in &conns {
            if real(&c.to_node) && !desc.contains(&c.from_node) && !ins.contains(&(c.to_node, c.to_port)) {
                ins.push((c.to_node, c.to_port));
            }
        }
        // выходы: внутренние порты, от которых идёт провод наружу, и выходы
        // нод-«стоков» внутри (результаты: к ним можно подключиться позже)
        let mut outs: Vec<(NodeId, &'static str)> = Vec::new();
        for c in &conns {
            if real(&c.from_node) && !desc.contains(&c.to_node) && !outs.contains(&(c.from_node, c.from_port)) {
                outs.push((c.from_node, c.from_port));
            }
        }
        for n in nodes.iter().filter(|n| real(&n.id)) {
            if conns.iter().any(|c| c.from_node == n.id) {
                continue;
            }
            for p in registry::meta(n.kind).outputs.resolve(n) {
                if !outs.contains(&(n.id, p.name)) {
                    outs.push((n.id, p.name));
                }
            }
        }
        let order = |v: &mut Vec<(NodeId, &'static str)>, side: PortSide| {
            v.sort_by(|a, b| {
                let (pa, pb) = (pos(a.0), pos(b.0));
                pa.y.total_cmp(&pb.y).then(pa.x.total_cmp(&pb.x)).then(port_index(&nodes, a, side).cmp(&port_index(&nodes, b, side)))
            });
        };
        order(&mut ins, PortSide::Input);
        order(&mut outs, PortSide::Output);
        let mk = |list: &[(NodeId, &'static str)], side: PortSide, tag: &str| -> (Vec<Proxy>, Vec<PortSchema>) {
            let mut px = Vec::new();
            let mut sc = Vec::new();
            for (nid, port) in list {
                let Some(n) = nodes.iter().find(|n| n.id == *nid) else { continue };
                let meta = registry::meta(n.kind);
                let ports = match side {
                    PortSide::Input => meta.inputs.resolve(n),
                    PortSide::Output => meta.outputs.resolve(n),
                };
                let Some(s) = ports.iter().find(|p| p.name == *port) else { continue };
                let name = leak_str(format!("g{}{tag}{}.{}", g.id.0, nid.0, port));
                let label = leak_str(format!("{}: {}", node_label(n), syngui::i18n::t(s.label)));
                px.push(Proxy { name, node: *nid, port });
                sc.push(PortSchema { name, label, kind: s.kind });
            }
            (px, sc)
        };
        let (pi, si) = mk(&ins, PortSide::Input, "i");
        let (po, so) = mk(&outs, PortSide::Output, "o");
        let changed = {
            let mut p = st.ports.lock().unwrap_or_else(|e| e.into_inner());
            let ch = p.ins != pi || p.outs != po;
            *p = GroupPorts { ins: pi, outs: po, ins_schema: leak_schema(si), outs_schema: leak_schema(so) };
            ch
        };
        if changed {
            st.ver.update(|v| *v += 1);
        }
    }
}

fn port_index(nodes: &[NodeInstance], p: &(NodeId, &'static str), side: PortSide) -> usize {
    let Some(n) = nodes.iter().find(|n| n.id == p.0) else { return 0 };
    let meta = registry::meta(n.kind);
    let ports = match side {
        PortSide::Input => meta.inputs.resolve(n),
        PortSide::Output => meta.outputs.resolve(n),
    };
    ports.iter().position(|x| x.name == p.1).unwrap_or(0)
}

/// Порт группы `gid` → внутренняя нода и порт.
pub fn real_port(nodes: &[NodeInstance], gid: NodeId, side: PortSide, proxy: &str) -> Option<(NodeId, &'static str)> {
    let g = nodes.iter().find(|n| n.id == gid).and_then(state)?;
    let p = g.ports.lock().ok()?;
    let list = match side {
        PortSide::Input => &p.ins,
        PortSide::Output => &p.outs,
    };
    list.iter().find(|x| x.name == proxy).map(|x| (x.node, x.port))
}

/// Порт группы `gid` для внутренней ноды и порта.
pub fn proxy_port(nodes: &[NodeInstance], gid: NodeId, side: PortSide, node: NodeId, port: &str) -> Option<&'static str> {
    let g = nodes.iter().find(|n| n.id == gid).and_then(state)?;
    let p = g.ports.lock().ok()?;
    let list = match side {
        PortSide::Input => &p.ins,
        PortSide::Output => &p.outs,
    };
    list.iter().find(|x| x.node == node && x.port == port).map(|x| x.name)
}

/// Провод на холсте: видимые концы (группа и её порт вместо скрытой ноды);
/// `None` — провод целиком внутри группы.
pub fn visible_wire(nodes: &[NodeInstance], owners: &HashMap<NodeId, NodeId>, c: &Connection) -> Option<(NodeId, &'static str, NodeId, &'static str)> {
    let (tf, tt) = (top(owners, c.from_node), top(owners, c.to_node));
    if tf == tt && (tf != c.from_node || tt != c.to_node) {
        return None;
    }
    let fp = if tf == c.from_node { c.from_port } else { proxy_port(nodes, tf, PortSide::Output, c.from_node, c.from_port)? };
    let tp = if tt == c.to_node { c.to_port } else { proxy_port(nodes, tt, PortSide::Input, c.to_node, c.to_port)? };
    Some((tf, fp, tt, tp))
}

/// Свернуть ноды в группу. Возвращает id группы.
pub fn group(ctx: &NodeEditorCtx, ids: &[NodeId], title: &str) -> std::result::Result<NodeId, String> {
    let nodes = ctx.nodes.get_untracked();
    let own = owners(&nodes);
    let ids: Vec<NodeId> = ids.iter().copied().filter(|id| nodes.iter().any(|n| n.id == *id) && !own.contains_key(id)).collect();
    if ids.is_empty() {
        return Err("нечего объединять: выделите ноды".into());
    }
    let (mut cx, mut cy) = (0.0f32, 0.0f32);
    for n in nodes.iter().filter(|n| ids.contains(&n.id)) {
        let p = n.pos.get_untracked();
        cx += p.x;
        cy += p.y;
    }
    cx /= ids.len() as f32;
    cy /= ids.len() as f32;
    let gid = ctx.add_node(NodeKind::Group, Point::new(cx, cy));
    let nodes = ctx.nodes.get_untracked();
    let g = nodes.iter().find(|n| n.id == gid).and_then(state).ok_or("группа не создана")?;
    let word = match (ids.len() % 10, ids.len() % 100) {
        (1, x) if x != 11 => "нода",
        (2..=4, x) if !(12..=14).contains(&x) => "ноды",
        _ => "нод",
    };
    g.title.set(if title.trim().is_empty() { format!("Подграф ({} {word})", ids.len()) } else { title.trim().to_string() });
    g.members.set(ids);
    g.origin.set(Point::new(cx, cy));
    ctx.pick_clear();
    refresh(ctx);
    // пересборка холста: скрытые ноды уходят, провода — к портам группы
    ctx.nodes.update(|_| {});
    Ok(gid)
}

/// Развернуть группу: участники снова видны (со сдвигом за группой).
pub fn ungroup(ctx: &NodeEditorCtx, gid: NodeId) -> std::result::Result<Vec<NodeId>, String> {
    let nodes = ctx.nodes.get_untracked();
    let gn = nodes.iter().find(|n| n.id == gid).ok_or("нет такой ноды")?;
    let g = state(gn).ok_or("это не подграф")?;
    let members = g.members.get_untracked();
    let (gp, o) = (gn.pos.get_untracked(), g.origin.get_untracked());
    let (dx, dy) = (gp.x - o.x, gp.y - o.y);
    for n in nodes.iter().filter(|n| members.contains(&n.id)) {
        let p = n.pos.get_untracked();
        n.pos.set(Point::new(p.x + dx, p.y + dy));
    }
    // внешняя группа получает участников развёрнутой
    if let Some(parent) = owners(&nodes).get(&gid).and_then(|p| nodes.iter().find(|n| n.id == *p)).and_then(state) {
        parent.members.update(|m| {
            m.retain(|x| *x != gid);
            m.extend(members.iter().copied());
        });
    }
    ctx.nodes.update(|v| v.retain(|n| n.id != gid));
    ctx.pick_clear();
    refresh(ctx);
    Ok(members)
}

/// Тело карточки группы: название (правится) и сколько внутри.
pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let Some(g) = state(node) else { return Box::new(Text::new("")) };
    let title = g.title;
    let n = g.members.get_untracked().len();
    Box::new(
        Padding::symmetric(10.0, 6.0).child(
            Column::new()
                .gap(4.0)
                .child(syngui::widgets::TextField::new().text(title.get_untracked()).on_change(move |s: &str| title.set(s.to_string())).class("flow-field-input"))
                .child(Text::new(format!("внутри нод: {n} · правый щелчок — «Разгруппировать»")).class("node-card-hint")),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn(f: NodeId, fp: &'static str, t: NodeId, tp: &'static str) -> Connection {
        Connection { from_node: f, from_port: fp, to_node: t, to_port: tp }
    }

    /// Смешанный граф: нативные числа и сложение + нода ядра; группа, вложенная группа,
    /// порты с подписями, провод через порт группы, разворот.
    #[test]
    fn mixed_nested_groups() {
        let ctx = NodeEditorCtx::new();
        ctx.nodes.set(Vec::new());
        let a = ctx.add_node(NodeKind::Number, Point::new(0.0, 0.0));
        let b = ctx.add_node(NodeKind::Number, Point::new(0.0, 100.0));
        let add = ctx.add_node(NodeKind::Add, Point::new(200.0, 50.0));
        let out = ctx.add_node(NodeKind::Output, Point::new(400.0, 50.0));
        let txt = ctx.add_node(NodeKind::FlowText, Point::new(0.0, 300.0));
        let fmt = ctx.add_node(NodeKind::FlowFormat, Point::new(200.0, 300.0));
        ctx.connections.set(vec![conn(a, "out", add, "a"), conn(b, "out", add, "b"), conn(add, "out", out, "in"), conn(txt, "text", fmt, "a")]);
        let before = ctx.connections.get_untracked();
        let v0 = ctx.values.get_untracked().get(&(out, "in")).cloned();

        // число b, сложение и нода ядра — в одну группу
        let g = group(&ctx, &[b, add, fmt], "Счёт").unwrap();
        let nodes = ctx.nodes.get_untracked();
        let own = owners(&nodes);
        assert_eq!(top(&own, add), g);
        let st = state(nodes.iter().find(|n| n.id == g).unwrap()).unwrap();
        let ports = st.ports.lock().unwrap().clone();
        // входы: add.a (от a снаружи), fmt.a (от txt); выходы: add.out (к out), fmt.text (сток)
        assert_eq!(ports.ins.len(), 2, "{:?}", ports.ins);
        assert!(ports.outs.iter().any(|p| p.node == add) && ports.outs.iter().any(|p| p.node == fmt));
        assert!(inputs(nodes.iter().find(|n| n.id == g).unwrap()).iter().all(|s| s.label.contains(": ")));
        // провода не тронуты, значение на выходе то же
        assert_eq!(ctx.connections.get_untracked(), before);
        assert_eq!(ctx.values.get_untracked().get(&(out, "in")).cloned(), v0);
        // видимые провода: a → группа, группа → out; внутренний b→add не рисуется
        let vis: Vec<_> = before.iter().filter_map(|c| visible_wire(&nodes, &own, c)).collect();
        assert!(vis.iter().any(|w| w.0 == a && w.2 == g));
        assert!(vis.iter().any(|w| w.0 == g && w.2 == out));
        assert!(!vis.iter().any(|w| w.0 == g && w.2 == g));

        // провод к порту группы подключается к настоящей ноде
        let out2 = ctx.add_node(NodeKind::Output, Point::new(500.0, 300.0));
        let fmt_out = proxy_port(&ctx.nodes.get_untracked(), g, PortSide::Output, fmt, "text").unwrap();
        ctx.start_wire(g, fmt_out, PortKind::Text, Point::zero());
        ctx.complete_wire(out2, "in");
        assert!(ctx.connections.get_untracked().iter().any(|c| c.from_node == fmt && c.from_port == "text" && c.to_node == out2));

        // вложенная группа: группа «Счёт» + число a
        let g2 = group(&ctx, &[g, a], "Внешняя").unwrap();
        let nodes = ctx.nodes.get_untracked();
        let own = owners(&nodes);
        assert_eq!(top(&own, add), g2);
        assert!(descendants(&nodes, g2).contains(&add));

        // развернуть внешнюю, затем внутреннюю — всё на месте
        ungroup(&ctx, g2).unwrap();
        ungroup(&ctx, g).unwrap();
        let nodes = ctx.nodes.get_untracked();
        assert!(owners(&nodes).is_empty());
        assert!(!nodes.iter().any(|n| n.kind == NodeKind::Group));
        assert_eq!(nodes.iter().filter(|n| n.kind != NodeKind::Group).count(), 7);
    }

    /// Сохранение и загрузка: группа с вложенной переживают шаблон (id переводятся).
    #[test]
    fn groups_roundtrip_template() {
        let ctx = NodeEditorCtx::new();
        ctx.nodes.set(Vec::new());
        let a = ctx.add_node(NodeKind::Number, Point::new(0.0, 0.0));
        let add = ctx.add_node(NodeKind::Add, Point::new(200.0, 0.0));
        let out = ctx.add_node(NodeKind::Output, Point::new(400.0, 0.0));
        ctx.connections.set(vec![conn(a, "out", add, "a"), conn(add, "out", out, "in")]);
        let g = group(&ctx, &[add], "Сложение").unwrap();
        let _g2 = group(&ctx, &[g, out], "Внешняя").unwrap();
        let (nodes, connections, viewport) = crate::convert::snapshot(&ctx);
        let t = crate::model::Template { id: String::new(), builtin: false, name: "t".into(), description: String::new(), kind: crate::model::TemplateKind::Full, nodes, connections, viewport };
        let json = serde_json::to_string(&t).unwrap();
        let t2: crate::model::Template = serde_json::from_str(&json).unwrap();
        let ctx2 = NodeEditorCtx::new();
        ctx2.nodes.set(Vec::new());
        crate::convert::apply_to_ctx(&ctx2, &t2, Point::zero());
        refresh(&ctx2);
        let nodes = ctx2.nodes.get_untracked();
        let own = owners(&nodes);
        let groups: Vec<&NodeInstance> = nodes.iter().filter(|n| n.kind == NodeKind::Group).collect();
        assert_eq!(groups.len(), 2);
        let titles: Vec<String> = groups.iter().map(|g| state(g).unwrap().title.get_untracked()).collect();
        assert!(titles.contains(&"Сложение".to_string()) && titles.contains(&"Внешняя".to_string()));
        let add2 = nodes.iter().find(|n| n.kind == NodeKind::Add).unwrap().id;
        let outer = groups.iter().find(|g| state(g).unwrap().title.get_untracked() == "Внешняя").unwrap().id;
        assert_eq!(top(&own, add2), outer);
    }
}
