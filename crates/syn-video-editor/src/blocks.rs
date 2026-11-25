//! Блоки (D2): подграф кадра с вынесенными параметрами — аналог
//! MacroOperator в Fusion. Нода `block` графа кадра хранит копию графа
//! блока и значения вынесенных параметров (проект переносим), поэтому
//! правка файла блока не меняет уже наложенные. Встроенные блоки — здесь,
//! свои — `~/.config/syn-video-editor/blocks/*.json` (`block.save`).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::framegraph::FrameGraph;

/// Вынесенный параметр: ключ, подпись, нода и её параметр, пределы, по умолчанию.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Expose {
    pub key: String,
    pub label: String,
    pub node: u32,
    pub param: String,
    #[serde(default)]
    pub min: f64,
    #[serde(default = "one")]
    pub max: f64,
    #[serde(default)]
    pub default: f64,
}

fn one() -> f64 {
    1.0
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Block {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub graph: FrameGraph,
    #[serde(default)]
    pub expose: Vec<Expose>,
    /// Встроенный (не из файла).
    #[serde(skip)]
    pub builtin: bool,
}

const BUILTIN: &str = r#"[
 {"name": "Мягкое свечение", "description": "размытая копия поверх оригинала экраном",
  "graph": {"nodes": [{"id":1,"kind":"source"},{"id":2,"kind":"blur","params":{"radius":24}},{"id":3,"kind":"merge","params":{"mode":"screen","opacity":0.6}},{"id":4,"kind":"output"}],
            "links": [{"from":1,"to":2},{"from":1,"to":3,"port":"bg"},{"from":2,"to":3,"port":"fg"},{"from":3,"to":4}]},
  "expose": [{"key":"size","label":"Размер","node":2,"param":"radius","min":2,"max":80,"default":24},
             {"key":"amount","label":"Сила","node":3,"param":"opacity","min":0,"max":1,"default":0.6}]},
 {"name": "Тёплое кино", "description": "теплее, контрастнее, мягкая виньетка",
  "graph": {"nodes": [{"id":1,"kind":"source"},{"id":2,"kind":"color","params":{"temperature":0.5,"contrast":0.15,"saturation":-0.1}},{"id":3,"kind":"vignette","params":{"amount":0.35}},{"id":4,"kind":"output"}],
            "links": [{"from":1,"to":2},{"from":2,"to":3},{"from":3,"to":4}]},
  "expose": [{"key":"warmth","label":"Тепло","node":2,"param":"temperature","min":-1,"max":1,"default":0.5},
             {"key":"contrast","label":"Контраст","node":2,"param":"contrast","min":-0.5,"max":0.8,"default":0.15},
             {"key":"vignette","label":"Виньетка","node":3,"param":"amount","min":0,"max":1,"default":0.35}]},
 {"name": "Сон", "description": "мечтательная дымка: размытие мягким светом и приглушённый цвет",
  "graph": {"nodes": [{"id":1,"kind":"source"},{"id":2,"kind":"blur","params":{"radius":36}},{"id":3,"kind":"merge","params":{"mode":"soft_light","opacity":0.8}},{"id":4,"kind":"color","params":{"saturation":-0.25,"brightness":0.05}},{"id":5,"kind":"output"}],
            "links": [{"from":1,"to":2},{"from":1,"to":3,"port":"bg"},{"from":2,"to":3,"port":"fg"},{"from":3,"to":4},{"from":4,"to":5}]},
  "expose": [{"key":"haze","label":"Дымка","node":3,"param":"opacity","min":0,"max":1,"default":0.8},
             {"key":"color","label":"Цвет","node":4,"param":"saturation","min":-1,"max":0.5,"default":-0.25}]},
 {"name": "Резкий ч/б", "description": "чёрно-белый с контрастом и резкостью",
  "graph": {"nodes": [{"id":1,"kind":"source"},{"id":2,"kind":"color","params":{"saturation":-1,"contrast":0.3}},{"id":3,"kind":"sharpen","params":{"amount":0.6}},{"id":4,"kind":"output"}],
            "links": [{"from":1,"to":2},{"from":2,"to":3},{"from":3,"to":4}]},
  "expose": [{"key":"contrast","label":"Контраст","node":2,"param":"contrast","min":-0.5,"max":1,"default":0.3},
             {"key":"sharp","label":"Резкость","node":3,"param":"amount","min":0,"max":2,"default":0.6}]},
 {"name": "Неон", "description": "яркое свечение и насыщенный цвет",
  "graph": {"nodes": [{"id":1,"kind":"source"},{"id":2,"kind":"glow","params":{"size":30,"strength":0.9}},{"id":3,"kind":"color","params":{"saturation":0.5,"contrast":0.1}},{"id":4,"kind":"output"}],
            "links": [{"from":1,"to":2},{"from":2,"to":3},{"from":3,"to":4}]},
  "expose": [{"key":"glow","label":"Свечение","node":2,"param":"strength","min":0,"max":2,"default":0.9},
             {"key":"sat","label":"Насыщенность","node":3,"param":"saturation","min":-0.5,"max":1,"default":0.5}]}
]"#;

pub fn dir() -> PathBuf {
    if let Some(d) = std::env::var_os("SYN_VIDEO_EDITOR_BLOCKS") {
        return d.into();
    }
    crate::layout::config_dir().join("blocks")
}

/// Встроенные и свои блоки (свой с тем же именем заменяет встроенный).
pub fn all() -> Vec<Block> {
    let mut v: Vec<Block> = serde_json::from_str::<Vec<Block>>(BUILTIN).unwrap_or_default().into_iter().map(|mut b| {
        b.builtin = true;
        b
    }).collect();
    if let Ok(rd) = std::fs::read_dir(dir()) {
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
        files.sort();
        for f in files {
            if let Some(b) = std::fs::read_to_string(&f).ok().and_then(|t| serde_json::from_str::<Block>(&t).ok()) {
                v.retain(|x| x.name != b.name);
                v.push(b);
            }
        }
    }
    v
}

pub fn find(name: &str) -> Option<Block> {
    let n = name.trim().to_lowercase();
    let all = all();
    all.iter().find(|b| b.name.to_lowercase() == n).or_else(|| all.iter().find(|b| b.name.to_lowercase().contains(&n))).cloned()
}

/// Параметры ноды `block` для блока: копия графа, вынесенные, значения по умолчанию.
pub fn node_params(b: &Block, values: Option<&Map<String, Value>>) -> Map<String, Value> {
    let mut vals = Map::new();
    for e in &b.expose {
        let v = values.and_then(|m| m.get(&e.key)).and_then(Value::as_f64).unwrap_or(e.default).clamp(e.min, e.max);
        vals.insert(e.key.clone(), json!(v));
    }
    let mut p = Map::new();
    p.insert("block".into(), json!(b.name));
    p.insert("graph".into(), serde_json::to_value(&b.graph).unwrap_or(Value::Null));
    p.insert("expose".into(), serde_json::to_value(&b.expose).unwrap_or(Value::Null));
    p.insert("values".into(), Value::Object(vals));
    p
}

/// Вынесенные параметры ноды блока.
pub fn exposes(p: &Map<String, Value>) -> Vec<Expose> {
    p.get("expose").cloned().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
}

/// Внутренний граф ноды блока со значениями вынесенных параметров.
pub fn inner(p: &Map<String, Value>) -> Option<FrameGraph> {
    let mut g: FrameGraph = match p.get("graph") {
        Some(v) => serde_json::from_value(v.clone()).ok()?,
        None => find(p.get("block")?.as_str()?)?.graph,
    };
    let vals = p.get("values").and_then(Value::as_object);
    for e in exposes(p) {
        let v = vals.and_then(|m| m.get(&e.key)).and_then(Value::as_f64).unwrap_or(e.default);
        if let Some(n) = g.nodes.iter_mut().find(|n| n.id == e.node) {
            n.params.insert(e.param.clone(), json!(v));
        }
    }
    g.validate().ok()?;
    Some(g)
}

/// Имя файла блока.
pub fn file_for(name: &str) -> PathBuf {
    let safe: String = name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
    dir().join(format!("{}.json", safe.trim()))
}
