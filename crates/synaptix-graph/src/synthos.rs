//! Экспорт графа в шаблон редактора узлов Synthos. Рецепт компилируется в
//! граф ядра `synaptix-flow` ([`crate::compile::to_flow`]), и каждая его
//! нода переносится один к одному в ноду ядра Synthos (`flow_<вид>`):
//! склейка, наложение звука, кадр, обрезка, видео во всех режимах и пакеты
//! (`each`) исполняются в Synthos тем же ядром, что у ассистента.
//! Значения, поданные на входы-файлы без поля (кадры, звук, референсы),
//! становятся нодами «Файл» / «Текст» / «Список». Шаблон кладётся в
//! `~/.config/synthos/templates/` (раздел «Свои»), затем Synthos запускается.

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::{json, Map, Value};
use synaptix_flow::{Flow, FlowNode, PortType};

use crate::Graph;

/// Значения поданных на входы-без-поля констант → ноды-источники.
fn materialize(flow: &mut Flow) {
    let snapshot: Vec<FlowNode> = flow.nodes.clone();
    for n in snapshot {
        let Some(spec) = synaptix_flow::ops::spec(&n.kind) else { continue };
        for (k, v) in &n.fields {
            if spec.field(k).is_some() {
                continue;
            }
            let Some(port) = spec.input(k) else { continue };
            if v.is_null() || v.as_str().is_some_and(|s| s.trim().is_empty()) {
                continue;
            }
            let id = flow.next_id();
            let src = match (port.ty, v) {
                (PortType::List, Value::Array(a)) => FlowNode::new(id, "list").with("items", Value::Array(a.clone())),
                (PortType::List, other) => FlowNode::new(id, "list").with("items", Value::Array(vec![other.clone()])),
                (t, Value::String(s)) if t.is_media() => FlowNode::new(id, "file").with("path", s.clone()),
                (_, other) => FlowNode::new(id, "text").with("text", other.as_str().map(String::from).unwrap_or_else(|| other.to_string())),
            };
            let out = synaptix_flow::ops::spec(&src.kind).and_then(|s| s.main_output()).map(|p| p.name).unwrap_or("");
            let mut src = src;
            src.label = format!("{}-{k}", n.label);
            flow.add(src);
            flow.link(id, out, n.id, k);
        }
        if let Some(node) = flow.nodes.iter_mut().find(|x| x.id == n.id) {
            node.fields.retain(|k, _| spec.field(k).is_some());
        }
    }
}

/// Поле ноды ядра → значение поля шаблона Synthos (`{"type", "value"}`).
fn field_data(f: &synaptix_flow::FieldSpec, v: Option<&Value>) -> Value {
    use synaptix_flow::FieldType as F;
    let text = |v: Option<&Value>| match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => f.default.to_string(),
        Some(other) => other.to_string(),
    };
    match f.ty {
        F::Text | F::LongText | F::File => json!({"type": "Text", "value": text(v)}),
        F::Number => {
            let x = match v {
                Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
                Some(Value::String(s)) => s.trim().parse().unwrap_or(0.0),
                _ => f.default.parse().unwrap_or(0.0),
            };
            json!({"type": "Float", "value": x})
        }
        F::Bool => {
            let b = match v {
                Some(Value::Bool(b)) => *b,
                Some(Value::String(s)) => s == "true",
                _ => f.default == "true",
            };
            json!({"type": "Bool", "value": b})
        }
        F::Choice(opts) => {
            let s = text(v);
            json!({"type": "Choice", "value": opts.iter().position(|o| *o == s).unwrap_or(0)})
        }
    }
}

/// Граф ядра → JSON шаблона Synthos; раскладка — столбцы по глубине.
pub fn flow_to_template(flow: &Flow, name: &str, description: &str) -> Result<Value, String> {
    let order = flow.order()?;
    let mut depth: HashMap<u64, usize> = HashMap::new();
    for &id in &order {
        let d = flow.links.iter().filter(|l| l.to_node == id).filter_map(|l| depth.get(&l.from_node)).max().map(|d| d + 1).unwrap_or(0);
        depth.insert(id, d);
    }
    let mut rows: HashMap<usize, f32> = HashMap::new();
    let mut nodes = Vec::new();
    for &id in &order {
        let n = flow.node(id).unwrap();
        let spec = synaptix_flow::ops::spec(&n.kind).ok_or_else(|| format!("неизвестный вид ноды «{}»", n.kind))?;
        let d = depth[&id];
        let y = rows.entry(d).or_insert(40.0);
        let pos = json!({"x": 40.0 + d as f32 * 460.0, "y": *y});
        *y += 120.0 + 46.0 * spec.fields.len().max(spec.inputs.len()) as f32;
        let fields: Map<String, Value> = spec.fields.iter().map(|f| (f.name.to_string(), field_data(f, n.fields.get(f.name)))).collect();
        nodes.push(json!({"id": id, "kind": format!("flow_{}", n.kind), "pos": pos, "fields": fields, "style": {"tint": null, "shadow": true}, "enabled": n.enabled, "state": null}));
    }
    let conns: Vec<Value> = flow.links.iter().map(|l| json!({"from_node": l.from_node, "from_port": l.from_port, "to_node": l.to_node, "to_port": l.to_port})).collect();
    Ok(json!({"name": name, "description": description, "kind": "full", "nodes": nodes, "connections": conns, "viewport": null}))
}

/// Граф → JSON шаблона Synthos.
pub fn to_template(g: &Graph, params: &Map<String, Value>) -> Result<Value, String> {
    let params = crate::exec::with_defaults(g, params);
    let mut graph = g.clone();
    graph.normalize_output()?;
    let mut flow = crate::compile::to_flow(&graph, &params)?;
    materialize(&mut flow);
    let name = if g.name.is_empty() { "Сценарий ассистента".to_string() } else { format!("Рецепт: {}", g.name) };
    flow_to_template(&flow, &name, &g.description)
}

fn templates_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/synthos/templates")
}

/// Записать шаблон, вернуть путь файла.
pub fn export(g: &Graph, params: &Map<String, Value>) -> Result<PathBuf, String> {
    let t = to_template(g, params)?;
    let dir = templates_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = template_path(if g.name.is_empty() { "graph" } else { &g.name });
    std::fs::write(&path, serde_json::to_string_pretty(&t).map_err(|e| e.to_string())?).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// Экспорт и запуск Synthos (если ещё не открыт).
pub fn export_and_open(g: &Graph, params: &Map<String, Value>) -> Result<String, String> {
    let path = export(g, params)?;
    let running = std::process::Command::new("pgrep").args(["-x", "synthos"]).output().map(|o| o.status.success()).unwrap_or(false);
    if !running {
        std::process::Command::new("synthos").spawn().map_err(|e| format!("Synthos не запущен: {e}"))?;
    }
    let name = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(format!("Шаблон «{name}» сохранён в Synthos: редактор узлов → шаблоны → «Свои»"))
}

/// Путь файла шаблона по имени графа.
pub fn template_path(name: &str) -> PathBuf {
    let slug: String = format!("synshell-{name}").chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' }).collect();
    templates_dir().join(format!("{slug}.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_recipes_export() {
        for g in crate::recipes::builtin() {
            let t = to_template(&g, &Map::new()).unwrap_or_else(|e| panic!("{}: {e}", g.name));
            let nodes = t["nodes"].as_array().unwrap();
            assert!(!nodes.is_empty(), "{}", g.name);
            let ids: Vec<u64> = nodes.iter().map(|n| n["id"].as_u64().unwrap()).collect();
            for n in nodes {
                assert!(n["kind"].as_str().unwrap().starts_with("flow_"), "{}", g.name);
            }
            for c in t["connections"].as_array().unwrap() {
                assert!(ids.contains(&c["from_node"].as_u64().unwrap()) && ids.contains(&c["to_node"].as_u64().unwrap()), "{}: связь на несуществующую ноду", g.name);
            }
        }
    }

    #[test]
    fn talking_avatar_files_become_nodes() {
        let g = crate::recipes::builtin().into_iter().find(|g| g.name == "talking-avatar").unwrap();
        let params = serde_json::json!({"photo": "/tmp/face.png", "text": "привет"}).as_object().cloned().unwrap();
        let t = to_template(&g, &params).unwrap();
        let nodes = t["nodes"].as_array().unwrap();
        // Фото — элемент списка референсов через ноду «Текст»; итог — mux.
        assert!(nodes.iter().any(|n| n["kind"] == "flow_mux"));
        assert!(nodes.iter().any(|n| n["kind"] == "flow_list"));
        assert!(nodes.iter().any(|n| n["fields"]["text"]["value"] == "/tmp/face.png"));
        let speak = nodes.iter().find(|n| n["kind"] == "flow_speak").unwrap();
        assert_eq!(speak["fields"]["text"]["value"], "привет");
    }
}
