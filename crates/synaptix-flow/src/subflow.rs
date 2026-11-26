//! Подграф одной нодой (`subflow`): часть графа сворачивается в ноду с
//! портами `in1…in4` / `out1…out4`; внутренний граф и сопоставление портов —
//! в поле ноды `subflow` (JSON), так что граф остаётся одним документом
//! (переносится, сохраняется рецептом). Выполнение — вложенный прогон с
//! готовыми значениями на внутренних входах ([`crate::RunOpts::inputs`]).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::doc::{Flow, FlowNode, Link};

/// Сколько портов у ноды-подграфа.
pub const PORTS: usize = 4;
pub const IN_PORTS: [&str; PORTS] = ["in1", "in2", "in3", "in4"];
pub const OUT_PORTS: [&str; PORTS] = ["out1", "out2", "out3", "out4"];

/// Вход подграфа: что снаружи пришло на порт `port`, подаётся на `targets`
/// (нода внутреннего графа, её вход).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubIn {
    pub port: String,
    #[serde(default)]
    pub label: String,
    pub targets: Vec<(u64, String)>,
}

/// Выход подграфа: порт `port` — выход `from_port` внутренней ноды `node`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubOut {
    pub port: String,
    #[serde(default)]
    pub label: String,
    pub node: u64,
    pub from_port: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SubflowDef {
    pub flow: Flow,
    #[serde(default)]
    pub inputs: Vec<SubIn>,
    #[serde(default)]
    pub outputs: Vec<SubOut>,
    /// Положение внутренних нод на холсте редактора (id → x, y) — чтобы
    /// «Разгруппировать» вернуло их на место.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub layout: BTreeMap<u64, (f32, f32)>,
}

impl SubflowDef {
    /// Описание из полей ноды.
    pub fn of(node: &FlowNode) -> Result<Self, String> {
        let v = node.fields.get("subflow").ok_or("у ноды-подграфа нет внутреннего графа (поле subflow)")?;
        match v {
            // из редактора узлов поле приходит строкой JSON
            Json::String(s) => serde_json::from_str(s).map_err(|e| format!("подграф: {e}")),
            v => serde_json::from_value(v.clone()).map_err(|e| format!("подграф: {e}")),
        }
    }
}

fn node_name(f: &Flow, id: u64) -> String {
    let Some(n) = f.node(id) else { return format!("#{id}") };
    if !n.label.is_empty() {
        return n.label.clone();
    }
    crate::ops::spec(&n.kind).map(|s| s.title.to_string()).unwrap_or_else(|| n.name())
}

/// Подпись порта по внутренней ноде: «Нода: порт».
fn port_label(f: &Flow, id: u64, port: &str, input: bool) -> String {
    let spec = f.node(id).and_then(|n| crate::ops::spec(&n.kind));
    let p = spec.and_then(|s| if input { s.input(port) } else { s.output(port) }).map(|p| p.label).unwrap_or(port);
    format!("{}: {p}", node_name(f, id))
}

impl Flow {
    /// Свернуть ноды `ids` в одну ноду-подграф. Провода извне на выделенные
    /// ноды становятся входами подграфа (один вход — на один источник), провода
    /// из них наружу — выходами (один выход — на один выход внутренней ноды).
    /// Возвращает id новой ноды.
    pub fn group(&mut self, ids: &[u64], title: &str) -> Result<u64, String> {
        if ids.is_empty() {
            return Err("нечего объединять: выделите ноды".into());
        }
        for id in ids {
            if self.node(*id).is_none() {
                return Err(format!("нет ноды {id}"));
            }
        }
        let inside = |id: u64| ids.contains(&id);
        // входы: по источникам снаружи
        let mut ins: Vec<((u64, String), Vec<(u64, String)>)> = Vec::new();
        for l in self.links.iter().filter(|l| inside(l.to_node) && !inside(l.from_node)) {
            let src = (l.from_node, l.from_port.clone());
            match ins.iter_mut().find(|(s, _)| *s == src) {
                Some((_, t)) => t.push((l.to_node, l.to_port.clone())),
                None => ins.push((src, vec![(l.to_node, l.to_port.clone())])),
            }
        }
        // выходы: по выходам внутренних нод, уходящим наружу
        let mut outs: Vec<(u64, String)> = Vec::new();
        for l in self.links.iter().filter(|l| inside(l.from_node) && !inside(l.to_node)) {
            let o = (l.from_node, l.from_port.clone());
            if !outs.contains(&o) {
                outs.push(o);
            }
        }
        // ноды-результаты внутри — их главный выход тоже наружу (иначе результат пропадёт)
        for r in self.result_nodes().into_iter().filter(|r| inside(*r)) {
            if let Some(main) = self.node(r).and_then(|n| crate::ops::spec(&n.kind)).and_then(|s| s.main_output()) {
                let o = (r, main.name.to_string());
                if !outs.contains(&o) {
                    outs.push(o);
                }
            }
        }
        if ins.len() > PORTS || outs.len() > PORTS {
            return Err(format!("у подграфа не больше {PORTS} входов и {PORTS} выходов (выделено: входов {}, выходов {}) — объедините меньше нод", ins.len(), outs.len()));
        }
        let inner = Flow {
            nodes: self.nodes.iter().filter(|n| inside(n.id)).cloned().collect(),
            links: self.links.iter().filter(|l| inside(l.from_node) && inside(l.to_node)).cloned().collect(),
            outputs: outs.iter().map(|o| o.0).collect::<std::collections::BTreeSet<_>>().into_iter().collect(),
        };
        let def = SubflowDef {
            inputs: ins
                .iter()
                .enumerate()
                .map(|(i, (_, t))| SubIn { port: IN_PORTS[i].into(), label: port_label(self, t[0].0, &t[0].1, true), targets: t.clone() })
                .collect(),
            outputs: outs.iter().enumerate().map(|(i, (n, p))| SubOut { port: OUT_PORTS[i].into(), label: port_label(self, *n, p, false), node: *n, from_port: p.clone() }).collect(),
            flow: inner,
            layout: BTreeMap::new(),
        };
        let id = self.nodes.iter().map(|n| n.id).max().unwrap_or(0) + 1;
        let mut node = FlowNode::new(id, "subflow");
        node.fields.insert("title".into(), Json::from(if title.trim().is_empty() { "Подграф" } else { title.trim() }));
        node.fields.insert("subflow".into(), serde_json::to_value(&def).map_err(|e| e.to_string())?);
        // провода снаружи — к портам подграфа
        let mut links: Vec<Link> = self.links.iter().filter(|l| !inside(l.from_node) && !inside(l.to_node)).cloned().collect();
        for (i, (src, _)) in ins.iter().enumerate() {
            links.push(Link { from_node: src.0, from_port: src.1.clone(), to_node: id, to_port: IN_PORTS[i].into() });
        }
        for l in self.links.iter().filter(|l| inside(l.from_node) && !inside(l.to_node)) {
            let k = outs.iter().position(|o| o.0 == l.from_node && o.1 == l.from_port).unwrap();
            links.push(Link { from_node: id, from_port: OUT_PORTS[k].into(), to_node: l.to_node, to_port: l.to_port.clone() });
        }
        let was_result: bool = self.result_nodes().iter().any(|o| inside(*o));
        let explicit = !self.outputs.is_empty();
        self.nodes.retain(|n| !inside(n.id));
        self.nodes.push(node);
        self.links = links;
        self.outputs.retain(|o| !inside(*o));
        if was_result && explicit {
            self.outputs.push(id);
        }
        self.validate()?;
        Ok(id)
    }

    /// Развернуть ноду-подграф обратно: внутренние ноды (с новыми id, если
    /// заняты) и провода к ним. Возвращает id внутренней ноды → id в графе.
    pub fn ungroup(&mut self, id: u64) -> Result<BTreeMap<u64, u64>, String> {
        let node = self.node(id).filter(|n| n.kind == "subflow").cloned().ok_or_else(|| format!("нода {id} — не подграф"))?;
        let def = SubflowDef::of(&node)?;
        let mut next = self.nodes.iter().map(|n| n.id).max().unwrap_or(0) + 1;
        let mut map: BTreeMap<u64, u64> = BTreeMap::new();
        for n in &def.flow.nodes {
            let nid = if n.id != id && self.node(n.id).is_none() {
                n.id
            } else {
                next += 1;
                next - 1
            };
            map.insert(n.id, nid);
            next = next.max(nid + 1);
        }
        let mut links: Vec<Link> = self.links.iter().filter(|l| l.from_node != id && l.to_node != id).cloned().collect();
        for l in &def.flow.links {
            links.push(Link { from_node: map[&l.from_node], from_port: l.from_port.clone(), to_node: map[&l.to_node], to_port: l.to_port.clone() });
        }
        for l in self.links.iter().filter(|l| l.to_node == id) {
            if let Some(si) = def.inputs.iter().find(|s| s.port == l.to_port) {
                for (n, p) in &si.targets {
                    links.push(Link { from_node: l.from_node, from_port: l.from_port.clone(), to_node: map[n], to_port: p.clone() });
                }
            }
        }
        for l in self.links.iter().filter(|l| l.from_node == id) {
            if let Some(so) = def.outputs.iter().find(|s| s.port == l.from_port) {
                links.push(Link { from_node: map[&so.node], from_port: so.from_port.clone(), to_node: l.to_node, to_port: l.to_port.clone() });
            }
        }
        let was_result = self.outputs.contains(&id);
        self.nodes.retain(|n| n.id != id);
        for n in &def.flow.nodes {
            let mut c = n.clone();
            c.id = map[&n.id];
            self.nodes.push(c);
        }
        self.links = links;
        self.outputs.retain(|o| *o != id);
        if was_result {
            for o in &def.outputs {
                let m = map[&o.node];
                if !self.outputs.contains(&m) {
                    self.outputs.push(m);
                }
            }
        }
        self.validate()?;
        Ok(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{run, Cancel};

    /// text → format(a) → lines; text2 → format(b). Свернуть format+lines, прогнать — как до свёртки.
    fn sample() -> Flow {
        Flow {
            nodes: vec![
                FlowNode::new(1, "text").with("text", "кот"),
                FlowNode::new(2, "text").with("text", "фонарь"),
                FlowNode::new(3, "format").with("template", "{{a}} и {{b}}\n{{b}} и {{a}}"),
                FlowNode::new(4, "lines"),
            ],
            links: vec![
                Link { from_node: 1, from_port: "text".into(), to_node: 3, to_port: "a".into() },
                Link { from_node: 2, from_port: "text".into(), to_node: 3, to_port: "b".into() },
                Link { from_node: 3, from_port: "text".into(), to_node: 4, to_port: "text".into() },
            ],
            outputs: vec![4],
        }
    }

    #[test]
    fn group_run_ungroup() {
        let dir = std::env::temp_dir().join(format!("flow-subflow-{}", std::process::id()));
        let base = run(&sample(), &dir, &Cancel::new(), &mut |_| {}).unwrap();
        let mut f = sample();
        let id = f.group(&[3, 4], "Склейка строк").unwrap();
        assert_eq!(f.nodes.len(), 3);
        assert_eq!(f.links.iter().filter(|l| l.to_node == id).count(), 2);
        assert_eq!(f.outputs, vec![id]);
        let def = SubflowDef::of(f.node(id).unwrap()).unwrap();
        assert!(def.inputs.iter().all(|i| i.label.contains(": ")), "{:?}", def.inputs);
        assert!(def.inputs[0].label.contains("Вставка 1") || def.inputs[0].label.contains("{{a}}"), "{}", def.inputs[0].label);
        let r = run(&f, &dir, &Cancel::new(), &mut |_| {}).unwrap();
        assert_eq!(r.text, base.text, "подграф даёт то же, что исходный граф");
        let back = f.ungroup(id).unwrap();
        assert_eq!(back.len(), 2);
        let r2 = run(&f, &dir, &Cancel::new(), &mut |_| {}).unwrap();
        assert_eq!(r2.text, base.text);
        assert!(f.links.iter().any(|l| l.from_node == 1 && l.to_port == "a"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn too_many_ports() {
        let mut f = Flow { nodes: (1..=6).map(|i| FlowNode::new(i, "text").with("text", "x")).collect(), links: vec![], outputs: vec![] };
        f.nodes.push(FlowNode::new(7, "list"));
        for i in 1..=5u64 {
            f.links.push(Link { from_node: i, from_port: "text".into(), to_node: 7, to_port: format!("in{i}") });
        }
        // 5 источников снаружи на одну ноду — больше 4 входов
        if f.validate().is_ok() {
            assert!(f.group(&[7], "x").is_err());
        }
    }
}
