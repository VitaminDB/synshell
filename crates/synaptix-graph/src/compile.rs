//! Рецепт → граф ядра `synaptix-flow`. Подстановки становятся проводами:
//! поле целиком `{{узел.file}}` — провод с главного выхода, текст со
//! ссылками — нода «Шаблон текста», `{{узел.lines}}` — нода «Строки»,
//! массив — нода «Список». `each` не нужен ядру отдельно: список на
//! одиночном входе — пакет, нода выполняется для каждого элемента.
//! Параметры подставляются значениями.

use std::collections::HashMap;

use serde_json::{Map, Value};
use synaptix_flow::{Flow, FlowNode, PortType};

use crate::{refs, Graph, Node, Scope};

/// Откуда берётся значение ссылки: провод (нода, порт) или готовое значение.
enum Src {
    Port(u64, &'static str),
    Static(Value),
}

struct Compiler<'g> {
    g: &'g Graph,
    params: &'g Map<String, Value>,
    flow: Flow,
    /// id узла рецепта → id ноды.
    ids: HashMap<String, u64>,
    /// Узел рецепта → нода «Строки» по его тексту.
    lines: HashMap<String, u64>,
}

impl<'g> Compiler<'g> {
    fn add(&mut self, kind: &str, label: String) -> u64 {
        let id = self.flow.next_id();
        let mut n = FlowNode::new(id, kind);
        n.label = label;
        self.flow.add(n)
    }

    fn set(&mut self, id: u64, k: &str, v: Value) {
        if let Some(n) = self.flow.nodes.iter_mut().find(|n| n.id == id) {
            n.fields.insert(k.into(), v);
        }
    }

    /// Значение ссылки `key` (`param`, `узел.поле`, `item`) для узла `n`.
    fn source(&mut self, n: &Node, key: &str, each: Option<&Src>) -> Result<Src, String> {
        if key == "item" {
            return match each {
                Some(Src::Port(i, p)) => Ok(Src::Port(*i, p)),
                Some(Src::Static(v)) => Ok(Src::Static(v.clone())),
                None => Err(format!("узел «{}»: {{{{item}}}} без each", n.id)),
            };
        }
        if key.starts_with("item.") {
            return Err(format!("узел «{}»: {{{{{key}}}}} не поддерживается — элементы each не объекты", n.id));
        }
        let Some((node, field)) = key.split_once('.') else {
            let scope = Scope { params: self.params, outputs: &HashMap::new(), item: None };
            return Ok(Src::Static(scope.resolve(&Value::String(format!("{{{{{key}}}}}")))));
        };
        let Some(&src) = self.ids.get(node) else {
            return Err(format!("узел «{}» ссылается на неизвестный узел «{node}»", n.id));
        };
        let kind = self.flow.node(src).map(|x| x.kind.clone()).unwrap_or_default();
        let spec = synaptix_flow::ops::spec(&kind).ok_or("нет вида ноды")?;
        let main = spec.main_output().map(|p| p.name).unwrap_or("");
        let text_port = spec.outputs.iter().find(|p| p.ty == PortType::Text).map(|p| p.name);
        match field {
            "text" => Ok(text_port.map(|p| Src::Port(src, p)).unwrap_or(Src::Static(Value::String(String::new())))),
            "file" | "files" => Ok(Src::Port(src, main)),
            "label" => Ok(Src::Static(Value::String(String::new()))),
            "lines" => {
                let Some(tp) = text_port else {
                    return Err(format!("узел «{node}» без текста — у него нет lines"));
                };
                let l = match self.lines.get(node) {
                    Some(&l) => l,
                    None => {
                        let l = self.add("lines", format!("{node}-lines"));
                        self.flow.link(src, tp, l, "text");
                        self.lines.insert(node.to_string(), l);
                        l
                    }
                };
                Ok(Src::Port(l, "list"))
            }
            other => Err(format!("узел «{}»: у «{node}» нет поля «{other}» (есть text, file, files, lines)", n.id)),
        }
    }

    /// Значение поля рецепта → провод или готовое значение.
    fn value(&mut self, n: &Node, key: &str, v: &Value, each: Option<&Src>) -> Result<Src, String> {
        match v {
            Value::String(s) => {
                let r = refs(s);
                if r.is_empty() {
                    return Ok(Src::Static(v.clone()));
                }
                let t = s.trim();
                if r.len() == 1 && t.starts_with("{{") && t.ends_with("}}") && t[2..t.len() - 2].trim() == r[0] {
                    return self.source(n, &r[0], each);
                }
                // Текст со ссылками: параметры — сразу, ссылки на узлы — входами шаблона.
                let mut template = s.clone();
                let mut wires = Vec::new();
                for key in r {
                    let src = self.source(n, &key, each)?;
                    let val = match src {
                        Src::Static(Value::String(x)) => x,
                        Src::Static(Value::Array(a)) => a.iter().map(|x| x.as_str().map(String::from).unwrap_or_else(|| x.to_string())).collect::<Vec<_>>().join(" "),
                        Src::Static(Value::Null) => String::new(),
                        Src::Static(other) => other.to_string(),
                        Src::Port(i, p) => {
                            let slot = match wires.iter().position(|&(wi, wp, _)| wi == i && wp == p) {
                                Some(k) => wires[k].2,
                                None => {
                                    let slot = ["a", "b", "c", "d", "e", "f"].get(wires.len()).copied().ok_or_else(|| format!("узел «{}», поле «{key}»: больше 6 ссылок в одном тексте", n.id))?;
                                    wires.push((i, p, slot));
                                    slot
                                }
                            };
                            format!("{{{{{slot}}}}}")
                        }
                    };
                    template = template.replace(&format!("{{{{{key}}}}}"), &val).replace(&format!("{{{{ {key} }}}}"), &val);
                }
                if wires.is_empty() {
                    return Ok(Src::Static(Value::String(template)));
                }
                let f = self.add("format", format!("{}-{key}", n.id));
                self.set(f, "template", Value::String(template));
                for (i, p, slot) in wires {
                    self.flow.link(i, p, f, slot);
                }
                Ok(Src::Port(f, "text"))
            }
            Value::Array(items) => {
                let mut statics = Vec::new();
                let mut srcs = Vec::new();
                for (k, x) in items.iter().enumerate() {
                    match self.value(n, &format!("{key}[{k}]"), x, each)? {
                        Src::Static(Value::Array(a)) => srcs.extend(a.into_iter().map(Src::Static)),
                        s => srcs.push(s),
                    }
                }
                if srcs.iter().all(|s| matches!(s, Src::Static(_))) {
                    for s in srcs {
                        if let Src::Static(v) = s {
                            statics.push(v);
                        }
                    }
                    return Ok(Src::Static(Value::Array(statics)));
                }
                if srcs.len() > 6 {
                    return Err(format!("узел «{}», поле «{key}»: в списке больше 6 элементов со ссылками", n.id));
                }
                // Порядок сохраняется: готовые значения — нодами «Текст» на входы списка.
                let l = self.add("list", format!("{}-{key}", n.id));
                for (k, s) in srcs.into_iter().enumerate() {
                    let slot = ["in1", "in2", "in3", "in4", "in5", "in6"][k];
                    let (i, p) = match s {
                        Src::Port(i, p) => (i, p),
                        Src::Static(v) => {
                            let t = self.add("text", format!("{}-{key}-{}", n.id, k + 1));
                            self.set(t, "text", v);
                            (t, "text")
                        }
                    };
                    self.flow.link(i, p, l, slot);
                }
                Ok(Src::Port(l, "list"))
            }
            other => Ok(Src::Static(other.clone())),
        }
    }

    fn node(&mut self, n: &Node) -> Result<(), String> {
        let spec = synaptix_flow::ops::spec(&n.op).ok_or_else(|| format!("узел «{}»: неизвестная операция «{}» (есть: {})", n.id, n.op, crate::OPS.join(", ")))?;
        let each = match &n.each {
            Some(e) => Some(match self.value(n, "each", &Value::String(e.clone()), None)? {
                // Готовый список (параметр-файлы) — нода «Список».
                Src::Static(v) => {
                    let items = match v {
                        Value::Array(a) => a,
                        Value::String(s) => s.split_whitespace().map(|x| Value::String(x.into())).collect(),
                        _ => Vec::new(),
                    };
                    let l = self.add("list", format!("{}-each", n.id));
                    self.set(l, "items", Value::Array(items));
                    Src::Port(l, "list")
                }
                s => s,
            }),
            None => None,
        };
        let id = self.add(&n.op, n.id.clone());
        self.ids.insert(n.id.clone(), id);
        for (k, v) in &n.args {
            match self.value(n, k, v, each.as_ref())? {
                Src::Static(v) => self.set(id, k, v),
                Src::Port(i, p) => {
                    if spec.input(k).is_none() {
                        return Err(format!("узел «{}»: поле «{k}» не принимает результат другого узла (входы: {})", n.id, spec.inputs.iter().map(|p| p.name).collect::<Vec<_>>().join(", ")));
                    }
                    self.flow.link(i, p, id, k);
                }
            }
        }
        Ok(())
    }
}

/// Рецепт с параметрами → граф ядра. Параметры — уже с умолчаниями.
pub fn to_flow(g: &Graph, params: &Map<String, Value>) -> Result<Flow, String> {
    let mut c = Compiler { g, params, flow: Flow::default(), ids: HashMap::new(), lines: HashMap::new() };
    for i in g.order()? {
        c.node(&g.nodes[i])?;
    }
    let outs: Vec<String> = if c.g.output.is_empty() { g.order()?.last().map(|&i| vec![g.nodes[i].id.clone()]).unwrap_or_default() } else { g.output.clone() };
    c.flow.outputs = outs.iter().filter_map(|o| c.ids.get(o).copied()).collect();
    Ok(c.flow)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(g: &Graph, given: Value) -> Map<String, Value> {
        let mut p = given.as_object().cloned().unwrap_or_default();
        for (k, d) in &g.params {
            if !p.contains_key(k) {
                if let Some(v) = &d.default {
                    p.insert(k.clone(), v.clone());
                }
            }
        }
        p
    }

    #[test]
    fn builtin_recipes_compile() {
        for g in crate::recipes::builtin() {
            let p = params(&g, serde_json::json!({"photo": "/tmp/a.png", "photos": ["/tmp/a.png", "/tmp/b.png"], "video": "/tmp/v.mp4", "text": "привет", "topic": "море", "about": "кот", "story": "ёж"}));
            let f = to_flow(&g, &p).unwrap_or_else(|e| panic!("{}: {e}", g.name));
            f.validate().unwrap_or_else(|e| panic!("{}: {e}", g.name));
            assert!(!f.outputs.is_empty(), "{}", g.name);
        }
    }

    #[test]
    fn story_wires() {
        let g = crate::recipes::builtin().into_iter().find(|g| g.name == "story-video").unwrap();
        let p = params(&g, serde_json::json!({"story": "ёж"}));
        let f = to_flow(&g, &p).unwrap();
        let by = |l: &str| f.nodes.iter().find(|n| n.label == l).unwrap().id;
        // plan → lines → format(prompt) → shots.prompt (пакет, цепочкой); opening → shots.first_frame → cut.files.
        let lines = by("plan-lines");
        let fmt = by("shots-prompt");
        assert!(f.links.iter().any(|l| l.from_node == lines && l.to_node == fmt && l.to_port == "a"));
        assert!(f.links.iter().any(|l| l.from_node == fmt && l.to_node == by("shots") && l.to_port == "prompt"));
        assert!(f.links.iter().any(|l| l.from_node == by("opening") && l.to_node == by("shots") && l.to_port == "first_frame"));
        assert!(f.links.iter().any(|l| l.from_node == by("shots") && l.to_node == by("cut") && l.to_port == "files"));
        let shots = f.node(by("shots")).unwrap();
        assert_eq!(shots.fields["chain"], true);
        assert_eq!(shots.fields["seconds"], 10);
        let plan = f.node(by("plan")).unwrap();
        assert!(plan.fields["prompt"].as_str().unwrap().contains("«ёж»"));
    }
}
