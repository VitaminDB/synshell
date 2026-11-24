//! Вид ноды программы без правки крейта: регистрация, раздел палитры, тип
//! порта, исполнитель над своим состоянием, сохранение графа в JSON и
//! загрузка обратно.

use syngui::core::Point;
use syngui::prelude::*;

use syn_node_editor::convert::{load_into_ctx, snapshot};
use syn_node_editor::eval::{evaluate_graph, EvalContext, NodeExecutor};
use syn_node_editor::model::{Template, TemplateKind};
use syn_node_editor::registry::{self, CategoryInfo, NodeCategory, NodeKindMeta};
use syn_node_editor::types::{
    Connection, ExtKind, ExtRuntime, FieldValue, NodeKind, NodeRuntime, PortKind, PortSchema, PortValue,
    PortsLayout, PortsSpec,
};
use syn_node_editor::NodeEditorCtx;

const SCALE: NodeKind = NodeKind::Ext(ExtKind("test_scale"));

/// Состояние ноды: множитель.
struct ScaleRt {
    factor: RwSignal<f32>,
}

impl ExtRuntime for ScaleRt {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn save_state(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({ "factor": self.factor.get_untracked() }))
    }
    fn load_state(&self, v: &serde_json::Value) {
        if let Some(f) = v.get("factor").and_then(|f| f.as_f64()) {
            self.factor.set(f as f32);
        }
    }
}

struct ScaleExec;
impl NodeExecutor for ScaleExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let x = ctx.read_input("in").as_float();
        let k = ctx.runtime().lock().ok().and_then(|rt| rt.ext::<ScaleRt>().map(|s| s.factor.get_untracked()));
        ctx.write_output("out", PortValue::Float(x * k.unwrap_or(1.0)));
        // Значение своего типа на порт `frame`.
        ctx.write_output("frame", PortValue::ext(format!("кадр×{}", k.unwrap_or(1.0))));
    }
}
static SCALE_EXEC: ScaleExec = ScaleExec;

const INPUTS: &[PortSchema] = &[PortSchema { name: "in", label: "in", kind: PortKind::Data }];
const OUTPUTS: &[PortSchema] = &[
    PortSchema { name: "out", label: "out", kind: PortKind::Data },
    PortSchema { name: "frame", label: "frame", kind: PortKind::Custom("frame") },
];

fn register() {
    registry::register_category(CategoryInfo { key: "compositing", label: "Compositing", icon: "\u{E3B6}" });
    registry::register_port_kind("frame", syngui::core::Color::from_hex("#22D3EE"));
    registry::register_kind(NodeKindMeta {
        kind: SCALE,
        icon: "\u{E8B8}",
        title: "Scale",
        category: NodeCategory::Custom("compositing"),
        subcategory: None,
        inputs: PortsSpec::Static(INPUTS),
        outputs: PortsSpec::Static(OUTPUTS),
        fields: &[],
        body: None,
        ports_layout: PortsLayout::Rows,
        port_row_extra: None,
        executor: &SCALE_EXEC,
        on_run: None,
        busy_signal: None,
        runtime: Some(|| NodeRuntime::Ext(Box::new(ScaleRt { factor: use_signal(1.0) }))),
    });
}

fn factor(ctx: &NodeEditorCtx, id: syn_node_editor::types::NodeId) -> f32 {
    let n = ctx.find_node(id).expect("нода");
    let rt = n.runtime.lock().unwrap();
    rt.ext::<ScaleRt>().expect("состояние ScaleRt").factor.get_untracked()
}

#[test]
fn ext_kind_registers_evaluates_and_roundtrips() {
    register();

    // Палитра видит раздел и вид, подпись — из описания (каталога нет).
    assert!(NodeCategory::all().contains(&NodeCategory::Custom("compositing")));
    assert_eq!(syn_node_editor::i18n::node_kind_key(SCALE), "test_scale");
    assert_eq!(registry::meta(SCALE).title, "Scale");
    assert!(NodeKind::all().contains(&SCALE));
    assert_eq!(registry::port_kind_color("frame"), Some(syngui::core::Color::from_hex("#22D3EE")));

    // Граф: Number(2) → Scale(×3).
    let ctx = NodeEditorCtx::new();
    let num = ctx.nodes.get_untracked()[0].id;
    if let Some(FieldValue::Float(v)) = ctx.nodes.get_untracked()[0].fields.lock().unwrap().get("value") {
        v.set(2.0);
    }
    let sc = ctx.add_node(SCALE, Point::new(300.0, 80.0));
    ctx.find_node(sc).unwrap().runtime.lock().unwrap().ext::<ScaleRt>().unwrap().factor.set(3.0);
    ctx.connections.set(vec![Connection { from_node: num, from_port: "out", to_node: sc, to_port: "in" }]);
    let values = evaluate_graph(&ctx.nodes.get_untracked(), &ctx.connections.get_untracked(), false);
    assert_eq!(values.get(&(sc, "out")).map(|v| v.as_float()), Some(6.0));
    let frame = values.get(&(sc, "frame")).and_then(|v| v.as_ext::<String>()).expect("значение своего типа");
    assert_eq!(frame.as_str(), "кадр×3");

    // Сохранение графа: вид — по имени, состояние — JSON программы.
    let (nodes, connections, viewport) = snapshot(&ctx);
    let t = Template {
        id: String::new(),
        builtin: false,
        name: "ext".into(),
        description: String::new(),
        kind: TemplateKind::Full,
        nodes,
        connections,
        viewport,
    };
    let json = serde_json::to_string(&t).unwrap();
    assert!(json.contains("\"kind\":\"test_scale\""), "{json}");
    assert!(json.contains("\"factor\":3.0"), "{json}");

    let back: Template = serde_json::from_str(&json).unwrap();
    let ctx2 = NodeEditorCtx::new();
    ctx2.nodes.set(Vec::new());
    load_into_ctx(&ctx2, &back);
    let sc2 = ctx2.nodes.get_untracked().iter().find(|n| n.kind == SCALE).expect("вид восстановлен").id;
    assert_eq!(factor(&ctx2, sc2), 3.0);
    let values = evaluate_graph(&ctx2.nodes.get_untracked(), &ctx2.connections.get_untracked(), false);
    assert_eq!(values.get(&(sc2, "out")).map(|v| v.as_float()), Some(6.0));

    // Незарегистрированный вид не разбирается — как неизвестный встроенный.
    assert!(serde_json::from_str::<NodeKind>("\"no_such_kind\"").is_err());
    assert_eq!(serde_json::from_str::<NodeKind>("\"number\"").unwrap(), NodeKind::Number);
}
