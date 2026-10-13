//! Документ графа: ноды с полями и связи порт → порт. Формат связей тот
//! же, что у шаблонов редактора узлов Synthos (`from_node/from_port/
//! to_node/to_port`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::ops;
use crate::order;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Flow {
    #[serde(default)]
    pub nodes: Vec<FlowNode>,
    #[serde(default)]
    pub links: Vec<Link>,
    /// Ноды-результаты; пусто — ноды, чьи выходы никуда не идут.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlowNode {
    pub id: u64,
    pub kind: String,
    #[serde(default)]
    pub fields: BTreeMap<String, Json>,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
    /// Подпись (id узла рецепта и т. п.) — для отчёта.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
}

fn yes() -> bool {
    true
}
fn is_true(b: &bool) -> bool {
    *b
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Link {
    pub from_node: u64,
    pub from_port: String,
    pub to_node: u64,
    pub to_port: String,
}

impl FlowNode {
    pub fn new(id: u64, kind: &str) -> Self {
        Self { id, kind: kind.into(), fields: BTreeMap::new(), enabled: true, label: String::new() }
    }

    pub fn with(mut self, k: &str, v: impl Into<Json>) -> Self {
        self.fields.insert(k.into(), v.into());
        self
    }

    /// Подпись для отчёта: своя или «вид #id».
    pub fn name(&self) -> String {
        if self.label.is_empty() {
            format!("{} #{}", self.kind, self.id)
        } else {
            self.label.clone()
        }
    }
}

impl Flow {
    pub fn node(&self, id: u64) -> Option<&FlowNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn next_id(&self) -> u64 {
        self.nodes.iter().map(|n| n.id).max().unwrap_or(0) + 1
    }

    pub fn add(&mut self, node: FlowNode) -> u64 {
        let id = node.id;
        self.nodes.push(node);
        id
    }

    pub fn link(&mut self, from: u64, from_port: &str, to: u64, to_port: &str) {
        self.links.retain(|l| !(l.to_node == to && l.to_port == to_port));
        self.links.push(Link { from_node: from, from_port: from_port.into(), to_node: to, to_port: to_port.into() });
    }

    /// Порядок исполнения; ошибка — цикл.
    pub fn order(&self) -> Result<Vec<u64>, String> {
        let ids: Vec<u64> = self.nodes.iter().map(|n| n.id).collect();
        let (order, cyclic) = order::topo(&ids, self.links.iter().map(|l| (l.from_node, l.to_node)));
        if !cyclic.is_empty() {
            let names: Vec<String> = cyclic.iter().filter_map(|&i| self.node(i)).map(FlowNode::name).collect();
            return Err(format!("в графе цикл: {}", names.join(", ")));
        }
        Ok(order)
    }

    /// Проверка: известные ноды, уникальные id, существующие порты,
    /// совместимые типы, один провод на вход, нет циклов.
    pub fn validate(&self) -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        for n in &self.nodes {
            if !seen.insert(n.id) {
                return Err(format!("повторяется id ноды {}", n.id));
            }
            if ops::spec(&n.kind).is_none() {
                return Err(format!("{}: неизвестный вид ноды «{}» (есть: {})", n.name(), n.kind, ops::kinds().join(", ")));
            }
        }
        let mut inputs = std::collections::HashSet::new();
        for l in &self.links {
            let (Some(a), Some(b)) = (self.node(l.from_node), self.node(l.to_node)) else {
                return Err(format!("связь {}.{} → {}.{}: нет такой ноды", l.from_node, l.from_port, l.to_node, l.to_port));
            };
            let (sa, sb) = (ops::spec(&a.kind).unwrap(), ops::spec(&b.kind).unwrap());
            let out = sa.output(&l.from_port).ok_or_else(|| format!("{}: нет выхода «{}» (есть: {})", a.name(), l.from_port, names(sa.outputs)))?;
            let inp = sb.input(&l.to_port).ok_or_else(|| format!("{}: нет входа «{}» (есть: {})", b.name(), l.to_port, names(sb.inputs)))?;
            if !out.ty.connects_to(inp.ty) {
                return Err(format!("{}.{} ({:?}) нельзя подать на {}.{} ({:?})", a.name(), out.name, out.ty, b.name(), inp.name, inp.ty));
            }
            if !inputs.insert((l.to_node, l.to_port.clone())) {
                return Err(format!("{}: на вход «{}» подано несколько проводов", b.name(), l.to_port));
            }
        }
        for &o in &self.outputs {
            if self.node(o).is_none() {
                return Err(format!("результат: нет ноды {o}"));
            }
        }
        self.order().map(|_| ())
    }

    /// Ноды-результаты: заданные или те, чьи выходы никуда не идут.
    pub fn result_nodes(&self) -> Vec<u64> {
        if !self.outputs.is_empty() {
            return self.outputs.clone();
        }
        let order = self.order().unwrap_or_default();
        order.into_iter().filter(|id| !self.links.iter().any(|l| l.from_node == *id)).collect()
    }
}

fn names(p: &[crate::schema::PortSpec]) -> String {
    p.iter().map(|p| p.name).collect::<Vec<_>>().join(", ")
}
