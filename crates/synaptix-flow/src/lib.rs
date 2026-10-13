//! Безголовое ядро нодового движка — общее для ассистента (рецепты),
//! службы (`op flow`) и редактора узлов Synthos.
//!
//! - [`schema`] — типы портов и полей, описание вида ноды;
//! - [`doc::Flow`] — граф: ноды с полями и связи порт → порт (формат связей
//!   шаблонов Synthos);
//! - [`order::topo`] — порядок исполнения (им же пользуется Synthos);
//! - [`run()`] — прогон: нода за нодой, список на входе не типа `List` — пакет
//!   (нода выполняется для каждого элемента), отмена и ход генерации;
//! - [`ops`] — виды нод: данные, генерация через службу synaptixd, монтаж ffmpeg.

pub mod doc;
pub mod ops;
pub mod order;
pub mod run;
pub mod schema;
pub mod value;

pub use doc::{Flow, FlowNode, Link};
pub use run::{run, run_node, Cancel, Cx, Event, Outputs, Report};
pub use schema::{FieldSpec, FieldType, NodeSpec, PortSpec, PortType};
pub use value::Val;

/// Длительность медиафайла, с (ffprobe).
pub use ops::media::media_seconds;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn flow(json: serde_json::Value) -> Flow {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn validate_ports_and_types() {
        let f = flow(serde_json::json!({
            "nodes": [{"id": 1, "kind": "text", "fields": {"text": "кот"}}, {"id": 2, "kind": "image"}],
            "links": [{"from_node": 1, "from_port": "text", "to_node": 2, "to_port": "prompt"}]
        }));
        f.validate().unwrap();
        let bad = flow(serde_json::json!({
            "nodes": [{"id": 1, "kind": "text"}, {"id": 2, "kind": "image"}],
            "links": [{"from_node": 1, "from_port": "text", "to_node": 2, "to_port": "init"}]
        }));
        assert!(bad.validate().unwrap_err().contains("нельзя подать"));
        let nope = flow(serde_json::json!({"nodes": [{"id": 1, "kind": "teleport"}]}));
        assert!(nope.validate().is_err());
    }

    #[test]
    fn batch_runs_per_item() {
        let dir = tempfile::tempdir().unwrap();
        // Строки → шаблон (пакет из трёх) → список.
        let f = flow(serde_json::json!({
            "nodes": [
                {"id": 1, "kind": "text", "fields": {"text": "- кот\n- пёс\n\n3. ёж"}},
                {"id": 2, "kind": "lines"},
                {"id": 3, "kind": "format", "fields": {"template": "фото: {{a}}"}}
            ],
            "links": [
                {"from_node": 1, "from_port": "text", "to_node": 2, "to_port": "text"},
                {"from_node": 2, "from_port": "list", "to_node": 3, "to_port": "a"}
            ]
        }));
        let mut events = Vec::new();
        let r = run(&f, dir.path(), &Cancel::new(), &mut |e| events.push(e)).unwrap();
        // Список на входе format.a — пакет: шаблон для каждой строки.
        assert_eq!(r.values[&3]["text"], Val::List(vec![Val::Text("фото: кот".into()), Val::Text("фото: пёс".into()), Val::Text("фото: ёж".into())]));
        // На одиночном (Text) входе тот же список — пакет.
        let mut inputs = HashMap::new();
        inputs.insert("text".to_string(), Val::List(vec![Val::Text("a".into()), Val::Text("b".into())]));
        let (o, _) = run_node(&FlowNode::new(9, "lines"), inputs, dir.path(), &Cancel::new(), &mut |_| {}).unwrap();
        assert_eq!(o["list"], Val::List(vec![Val::List(vec![Val::Text("a".into())]), Val::List(vec![Val::Text("b".into())])]));
        assert!(events.iter().any(|e| matches!(e, Event::Finished { id: 3, .. })));
    }

    #[test]
    fn cancel_stops_before_node() {
        let dir = tempfile::tempdir().unwrap();
        let f = flow(serde_json::json!({"nodes": [{"id": 1, "kind": "text", "fields": {"text": "x"}}]}));
        let c = Cancel::new();
        c.cancel();
        assert!(run(&f, dir.path(), &c, &mut |_| {}).unwrap_err().contains("остановлен"));
    }
}
