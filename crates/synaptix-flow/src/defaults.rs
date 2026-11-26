//! Модели по умолчанию для нод с полем `model`, заданные программой
//! (например, «Параметры → Модели» видеоредактора): нода с пустым полем
//! «Модель» берёт модель своего вида отсюда, а не модель службы по умолчанию.
//! Пусто (ничего не задано) — поведение прежнее.

use std::collections::BTreeMap;
use std::sync::RwLock;

use serde_json::Value as Json;

use crate::doc::Flow;
use crate::subflow::SubflowDef;

static MODELS: RwLock<BTreeMap<String, String>> = RwLock::new(BTreeMap::new());

/// Задать модели по видам нод (`kind` → id модели); пустые значения пропускаются.
pub fn set_models(map: BTreeMap<String, String>) {
    let map = map.into_iter().map(|(k, v)| (k, v.trim().to_string())).filter(|(_, v)| !v.is_empty()).collect();
    *MODELS.write().unwrap_or_else(|e| e.into_inner()) = map;
}

/// Модель по умолчанию для вида ноды, если задана.
pub fn model(kind: &str) -> Option<String> {
    MODELS.read().unwrap_or_else(|e| e.into_inner()).get(kind).cloned()
}

/// Виды нод, у которых есть поле `model` (их модель можно задать).
pub fn kinds() -> Vec<&'static str> {
    crate::ops::specs().filter(|s| s.field("model").is_some()).map(|s| s.kind).collect()
}

fn empty(v: Option<&Json>) -> bool {
    match v {
        None | Some(Json::Null) => true,
        Some(Json::String(s)) => s.trim().is_empty(),
        _ => false,
    }
}

/// Вписать модели по умолчанию в ноды с пустым полем `model` (и во внутренние
/// графы подграфов) — перед отправкой графа службе, которая настроек программы
/// не знает.
pub fn apply(flow: &mut Flow) {
    if MODELS.read().map(|m| m.is_empty()).unwrap_or(true) {
        return;
    }
    for n in &mut flow.nodes {
        if n.kind == "subflow" {
            if let Ok(mut def) = SubflowDef::of(n) {
                apply(&mut def.flow);
                if let Ok(v) = serde_json::to_value(&def) {
                    let s = matches!(n.fields.get("subflow"), Some(Json::String(_)));
                    n.fields.insert("subflow".into(), if s { Json::String(v.to_string()) } else { v });
                }
            }
            continue;
        }
        if empty(n.fields.get("model")) {
            if let Some(m) = model(&n.kind) {
                n.fields.insert("model".into(), Json::String(m));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::FlowNode;

    fn node(id: u64, kind: &str, model: Option<&str>) -> FlowNode {
        let mut fields = BTreeMap::new();
        if let Some(m) = model {
            fields.insert("model".to_string(), Json::String(m.into()));
        }
        FlowNode { id, kind: kind.into(), fields, enabled: true, label: String::new() }
    }

    #[test]
    fn fills_empty_models_and_subflows() {
        set_models(BTreeMap::from([("music".to_string(), "ace-step".to_string()), ("speak".to_string(), " ".to_string())]));
        let inner = Flow { nodes: vec![node(1, "music", None)], links: Vec::new(), outputs: Vec::new() };
        let def = SubflowDef { flow: inner, ..Default::default() };
        let mut sub = node(3, "subflow", None);
        sub.fields.insert("subflow".into(), Json::String(serde_json::to_string(&def).unwrap()));
        let mut f = Flow { nodes: vec![node(1, "music", Some("")), node(2, "music", Some("yue")), sub, node(4, "speak", None)], links: Vec::new(), outputs: Vec::new() };
        apply(&mut f);
        assert_eq!(f.nodes[0].fields["model"], Json::String("ace-step".into()));
        assert_eq!(f.nodes[1].fields["model"], Json::String("yue".into()), "явная модель не меняется");
        let d = SubflowDef::of(&f.nodes[2]).unwrap();
        assert_eq!(d.flow.nodes[0].fields["model"], Json::String("ace-step".into()));
        assert!(!f.nodes[3].fields.contains_key("model"), "пустое значение не задаёт модель");
        assert!(kinds().contains(&"music"));
        set_models(BTreeMap::new());
    }
}
