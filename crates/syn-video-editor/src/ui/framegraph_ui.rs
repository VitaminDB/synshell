//! Граф кадра клипа (D1) в общем редакторе узлов: ноды раздела «Кадр»
//! (порты типа `frame` соединяются только между собой), холст открытого
//! графа — в панели «Граф». Правки на холсте (ноды, провода, поля,
//! положение) уходят в клип командой `graph.set` (серия правок — один шаг
//! отмены); отмена, агент и команды обновляют холст обратно.

use std::sync::OnceLock;

use serde_json::{json, Map, Value};
use syngui::prelude::*;
use syn_node_editor::eval::{EvalContext, NodeExecutor};
use syn_node_editor::registry::{self, CategoryInfo, NodeCategory, NodeKindMeta};
use syn_node_editor::types::{ExtKind, FieldSchema, FieldType, FieldValue, NodeId, NodeInstance, NodeKind, PortKind, PortSchema, PortsLayout, PortsSpec, Connection};
use syn_node_editor::NodeEditorCtx;

use super::{ed, St};
use crate::framegraph::{FrameGraph, GLink, GNode};

const FRAME: PortKind = PortKind::Custom("frame");

struct Noop;
impl NodeExecutor for Noop {
    fn evaluate(&self, _ctx: &mut EvalContext<'_>) {}
}
static NOOP: Noop = Noop;

const IN: &[PortSchema] = &[PortSchema { name: "in", label: "кадр", kind: FRAME }];
const OUT: &[PortSchema] = &[PortSchema { name: "out", label: "кадр", kind: FRAME }];
const MERGE_IN: &[PortSchema] = &[PortSchema { name: "bg", label: "фон", kind: FRAME }, PortSchema { name: "fg", label: "поверх", kind: FRAME }];
const MODES: &[&str] = &["normal", "screen", "add", "multiply", "overlay", "darken", "lighten", "soft_light", "hard_light", "difference"];

const F_COLOR: &[FieldSchema] = &[
    FieldSchema { name: "brightness", label: "Яркость", ty: FieldType::Float },
    FieldSchema { name: "contrast", label: "Контраст", ty: FieldType::Float },
    FieldSchema { name: "saturation", label: "Насыщенность", ty: FieldType::Float },
    FieldSchema { name: "temperature", label: "Температура", ty: FieldType::Float },
];
const F_BLUR: &[FieldSchema] = &[FieldSchema { name: "radius", label: "Радиус", ty: FieldType::Float }];
const F_SHARPEN: &[FieldSchema] = &[FieldSchema { name: "amount", label: "Сила", ty: FieldType::Float }];
const F_VIGNETTE: &[FieldSchema] = &[FieldSchema { name: "amount", label: "Сила", ty: FieldType::Float }];
const F_LUT: &[FieldSchema] = &[FieldSchema { name: "path", label: "Файл .cube", ty: FieldType::Text }, FieldSchema { name: "strength", label: "Сила", ty: FieldType::Float }];
const F_GLOW: &[FieldSchema] = &[FieldSchema { name: "size", label: "Размер", ty: FieldType::Float }, FieldSchema { name: "strength", label: "Сила", ty: FieldType::Float }];
const F_UPSCALE: &[FieldSchema] = &[FieldSchema { name: "model", label: "Модель", ty: FieldType::Text }];
const F_MERGE: &[FieldSchema] = &[FieldSchema { name: "mode", label: "Режим", ty: FieldType::Choice(MODES) }, FieldSchema { name: "opacity", label: "Непрозрачность", ty: FieldType::Float }];

/// Вид ноды графа ↔ вид ноды редактора; поля и их значения по умолчанию.
const SPEC: &[(&str, &str, &[FieldSchema], &[(&str, f32)])] = &[
    ("source", "vefg_source", &[], &[]),
    ("output", "vefg_output", &[], &[]),
    ("color", "vefg_color", F_COLOR, &[]),
    ("blur", "vefg_blur", F_BLUR, &[("radius", 6.0)]),
    ("sharpen", "vefg_sharpen", F_SHARPEN, &[("amount", 0.5)]),
    ("vignette", "vefg_vignette", F_VIGNETTE, &[("amount", 0.5)]),
    ("lut", "vefg_lut", F_LUT, &[("strength", 1.0)]),
    ("glow", "vefg_glow", F_GLOW, &[("size", 20.0), ("strength", 0.6)]),
    ("upscale", "vefg_upscale", F_UPSCALE, &[]),
    ("merge", "vefg_merge", F_MERGE, &[("opacity", 1.0)]),
    ("block", "vefg_block", &[], &[]),
];

fn ext(kind: &str) -> Option<NodeKind> {
    SPEC.iter().find(|s| s.0 == kind).map(|s| NodeKind::Ext(ExtKind(s.1)))
}

fn graph_kind(k: NodeKind) -> Option<&'static str> {
    match k {
        NodeKind::Ext(ExtKind(e)) => SPEC.iter().find(|s| s.1 == e).map(|s| s.0),
        _ => None,
    }
}

/// Раздел «Кадр» палитры и его ноды — до создания окна.
pub fn register() {
    registry::register_category(CategoryInfo { key: "ve_frame", label: "Кадр клипа", icon: "\u{E3B6}" });
    registry::register_port_kind("frame", Color::from_srgb(250, 176, 64, 1.0));
    for (kind, e, fields, _) in SPEC {
        let (inputs, outputs): (&'static [PortSchema], &'static [PortSchema]) = match *kind {
            "source" => (&[], OUT),
            "output" => (IN, &[]),
            "merge" => (MERGE_IN, OUT),
            _ => (IN, OUT),
        };
        registry::register_kind(NodeKindMeta {
            kind: NodeKind::Ext(ExtKind(e)),
            icon: match *kind {
                "source" => "\u{E04B}",
                "output" => "\u{E5DD}",
                "merge" => "\u{E3A5}",
                "lut" => "\u{E40A}",
                "upscale" => "\u{E8FF}",
                "block" => "\u{E1BD}",
                _ => "\u{E3A1}",
            },
            title: crate::framegraph::kind_title(kind),
            category: NodeCategory::Custom("ve_frame"),
            subcategory: None,
            inputs: PortsSpec::Static(inputs),
            outputs: PortsSpec::Static(outputs),
            fields,
            body: None,
            ports_layout: PortsLayout::Rows,
            port_row_extra: None,
            executor: &NOOP,
            on_run: None,
            busy_signal: None,
            runtime: None,
        });
    }
}

/// Открытый граф кадра: клип и холст.
#[derive(Clone, Copy)]
pub struct Open {
    pub clip: u64,
    pub ctx: NodeEditorCtx,
    /// Номер открытия: эффекты прежнего холста по нему узнают, что устарели.
    pub gen: u64,
}

impl PartialEq for Open {
    fn eq(&self, o: &Self) -> bool {
        self.gen == o.gen
    }
}

static GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

static CUR: OnceLock<RwSignal<Option<Open>>> = OnceLock::new();

/// Сигнал создаётся при запуске окна ([`init`]) — вне перерисовываемых
/// панелей, иначе он погибает вместе с их областью.
pub fn current() -> RwSignal<Option<Open>> {
    *CUR.get_or_init(|| use_signal(None))
}

pub fn init() {
    let _ = current();
}

/// Значения полей ноды графа — в сигналы карточки.
fn set_fields(inst: &NodeInstance, n: &GNode) {
    let defaults = SPEC.iter().find(|s| s.0 == n.kind).map(|s| s.3).unwrap_or(&[]);
    for (name, fv) in inst.fields.lock().unwrap().iter() {
        let v = n.params.get(*name);
        match fv {
            FieldValue::Float(s) => {
                let x = v.and_then(Value::as_f64).map(|x| x as f32).or_else(|| defaults.iter().find(|d| d.0 == *name).map(|d| d.1)).unwrap_or(0.0);
                if s.get_untracked() != x {
                    s.set(x);
                }
            }
            FieldValue::Text(s) => {
                let t = v.and_then(Value::as_str).unwrap_or("").to_string();
                if s.get_untracked() != t {
                    s.set(t);
                }
            }
            FieldValue::Choice(s) => {
                let i = MODES.iter().position(|m| Some(*m) == v.and_then(Value::as_str)).unwrap_or(0);
                if s.get_untracked() != i {
                    s.set(i);
                }
            }
            _ => {}
        }
    }
}

/// Заполнить холст графом клипа. Тот же состав нод — значения и положения
/// пишутся в прежние сигналы (карточки холста привязаны к ним), иначе ноды
/// создаются заново.
fn fill(ctx: NodeEditorCtx, g: &FrameGraph) {
    let cur = ctx.nodes.get_untracked();
    let same_set = cur.len() == g.nodes.len() && g.nodes.iter().all(|n| cur.iter().any(|c| c.id.0 == n.id as u64 && graph_kind(c.kind) == Some(n.kind.as_str())));
    if same_set {
        for n in &g.nodes {
            if let Some(c) = cur.iter().find(|c| c.id.0 == n.id as u64) {
                set_fields(c, n);
                let p = Point::new(n.x, n.y);
                if c.pos.get_untracked() != p {
                    c.pos.set(p);
                }
            }
        }
        ctx.connections.set(conns_of(g));
        return;
    }
    let mut nodes = Vec::new();
    for n in &g.nodes {
        let Some(kind) = ext(&n.kind) else { continue };
        let inst = NodeInstance {
            id: NodeId(n.id as u64),
            kind,
            pos: use_signal(Point::new(n.x, n.y)),
            bounds: use_signal(syngui::core::Rect::zero()),
            fields: registry::default_fields(kind),
            runtime: registry::default_runtime(kind),
            style: use_signal(Default::default()),
            enabled: use_signal(true),
            timing: syn_node_editor::timing::Stopwatch::new(),
        };
        set_fields(&inst, n);
        nodes.push(inst);
    }
    let conns = conns_of(g);
    ctx.next_id.set(g.next_id() as u64 + 1);
    ctx.nodes.set(nodes);
    ctx.connections.set(conns);
}

fn conns_of(g: &FrameGraph) -> Vec<Connection> {
    g.links
        .iter()
        .filter_map(|l| {
            let to_kind = g.node(l.to)?.kind.clone();
            let to_port: &'static str = crate::framegraph::inputs_of(&to_kind).iter().copied().find(|p| *p == l.port)?;
            Some(Connection { from_node: NodeId(l.from as u64), from_port: "out", to_node: NodeId(l.to as u64), to_port })
        })
        .collect()
}

/// Граф из холста (только ноды кадра; прочее игнорируется). Параметры,
/// которых нет на карточке (вложенный граф блока, значения), берутся из
/// `base` — текущего графа клипа.
fn read(ctx: NodeEditorCtx, track: bool, base: Option<&FrameGraph>) -> FrameGraph {
    let nodes = if track { ctx.nodes.get() } else { ctx.nodes.get_untracked() };
    let conns = if track { ctx.connections.get() } else { ctx.connections.get_untracked() };
    let mut g = FrameGraph::default();
    for n in &nodes {
        let Some(kind) = graph_kind(n.kind) else { continue };
        let pos = if track { n.pos.get() } else { n.pos.get_untracked() };
        let mut params = base.and_then(|b| b.node(n.id.0 as u32)).filter(|b| b.kind == kind).map(|b| b.params.clone()).unwrap_or_default();
        for (name, fv) in n.fields.lock().unwrap().iter() {
            let v = match fv {
                FieldValue::Float(s) => json!(((if track { s.get() } else { s.get_untracked() }) as f64 * 1000.0).round() / 1000.0),
                FieldValue::Text(s) => {
                    let t = if track { s.get() } else { s.get_untracked() };
                    if t.is_empty() {
                        continue;
                    }
                    json!(t)
                }
                FieldValue::Choice(s) => json!(MODES.get(if track { s.get() } else { s.get_untracked() }).copied().unwrap_or("normal")),
                _ => continue,
            };
            params.insert((*name).to_string(), v);
        }
        g.nodes.push(GNode { id: n.id.0 as u32, kind: kind.to_string(), params, x: pos.x.round(), y: pos.y.round() });
    }
    g.nodes.sort_by_key(|n| n.id);
    for c in &conns {
        if g.node(c.from_node.0 as u32).is_some() && g.node(c.to_node.0 as u32).is_some() {
            g.links.push(GLink { from: c.from_node.0 as u32, to: c.to_node.0 as u32, port: c.to_port.to_string() });
        }
    }
    g.links.sort_by(|a, b| (a.to, &a.port).cmp(&(b.to, &b.port)));
    g
}

/// Сравнение без учёта порядка и значений «по умолчанию».
fn same(a: &FrameGraph, b: &FrameGraph) -> bool {
    let norm = |g: &FrameGraph| {
        let mut g = g.clone();
        g.nodes.sort_by_key(|n| n.id);
        for n in &mut g.nodes {
            let defaults = SPEC.iter().find(|s| s.0 == n.kind).map(|s| s.3).unwrap_or(&[]);
            let fields = SPEC.iter().find(|s| s.0 == n.kind).map(|s| s.2).unwrap_or(&[]);
            // недостающие поля — значения по умолчанию, как на холсте
            for f in fields {
                if !n.params.contains_key(f.name) {
                    match f.ty {
                        FieldType::Float => {
                            n.params.insert(f.name.into(), json!(defaults.iter().find(|d| d.0 == f.name).map(|d| d.1 as f64).unwrap_or(0.0)));
                        }
                        FieldType::Choice(_) => {
                            n.params.insert(f.name.into(), json!("normal"));
                        }
                        _ => {}
                    }
                }
            }
            for v in n.params.values_mut() {
                if let Some(x) = v.as_f64() {
                    *v = json!((x * 1000.0).round() / 1000.0);
                }
            }
        }
        g.links.sort_by(|a, b| (a.to, &a.port).cmp(&(b.to, &b.port)));
        g
    };
    norm(a) == norm(b)
}

/// «Открыть как граф»: граф клипа (создать из эффектов) на холсте панели «Граф».
pub fn open(st: St, cid: u64) {
    if ed().project.clip(cid).is_some_and(|c| c.graph.is_none()) && st.exec("graph.open", json!({"id": cid})).is_none() {
        return;
    }
    let Some(g) = ed().project.clip(cid).and_then(|c| c.graph.clone()) else { return };
    let ctx = NodeEditorCtx::new();
    fill(ctx, &g);
    let gen = GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    current().set(Some(Open { clip: cid, ctx, gen }));
    st.open_panel("graph");
    st.dock.activate("graph");
    // прежние эффекты синхронизации — снять; новые живут без владельца
    // (открытие идёт из обработчика щелчка, а не из построения виджета)
    static FX: std::sync::Mutex<Vec<syngui::signal::EffectId>> = std::sync::Mutex::new(Vec::new());
    for id in FX.lock().unwrap().drain(..) {
        syngui::signal::dispose_effect(id);
    }
    let mut ids = Vec::new();
    // холст → клип
    ids.push(syngui::signal::create_detached_effect(move || {
        let Some(o) = current().get() else { return };
        if o.gen != gen {
            return;
        }
        let cur = ed().project.clip(cid).and_then(|c| c.graph.clone());
        let g = read(ctx, true, cur.as_ref());
        match cur {
            Some(c) if same(&c, &g) => {}
            Some(_) => {
                if g.validate().is_ok() {
                    let v = serde_json::to_value(&g).unwrap_or(Value::Null);
                    syngui::async_runtime::run_on_main_thread(move || {
                        st.exec("graph.set", json!({"id": cid, "graph": v}));
                    });
                }
            }
            None => {}
        }
    }));
    // клип → холст (отмена, агент, команды)
    ids.push(syngui::signal::create_detached_effect(move || {
        let _ = st.rev.get();
        let Some(o) = current().get_untracked() else { return };
        if o.gen != gen {
            return;
        }
        match ed().project.clip(cid).and_then(|c| c.graph.clone()) {
            Some(g) => {
                if !same(&g, &read(ctx, false, Some(&g))) {
                    fill(ctx, &g);
                }
            }
            // граф закрыт или клип удалён
            None => current().set(None),
        }
    }));
    FX.lock().unwrap().extend(ids);
}

/// Шапка холста графа кадра: имя клипа, «Вернуть в эффекты», «К графам».
pub fn head(st: St, o: Open) -> impl Widget {
    let cid = o.clip;
    let name = ed().project.clip(cid).map(|c| if c.name.is_empty() { format!("c{cid}") } else { format!("c{cid} {}", c.name) }).unwrap_or_default();
    Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new(format!("Граф кадра · {name}")).class("graph-tab-text"))
        .child(DecoratedBox::new().class("grow"))
        .child(Button::new("Вернуть в эффекты").on_click(move || {
            if st.exec("graph.close", json!({"id": cid})).is_some() {
                current().set(None);
            }
        }).class("head-btn"))
        .child(Button::new("К графам сценария").on_click(move || current().set(None)).class("head-btn"))
}
