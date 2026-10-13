//! Графы-сценарии поверх службы нейросетей `synaptixd`.
//!
//! Граф — JSON: параметры и узлы. Узел — операция службы (`llm`, `image`,
//! `video`, `music`, `speak`, `transcribe`) или утилита (`text`, `concat`,
//! `mux`, `frame`, `trim`, `still`). Поля узла — строки с подстановками
//! `{{параметр}}`, `{{узел.text}}`, `{{узел.file}}` (`{{узел.files}}` — список),
//! по ним же строится порядок выполнения. `each` — выполнить узел для каждого
//! элемента списка (`{{item}}`). Служба не меняется: граф только вызывает её
//! операции по очереди, поэтому существующая генерация от него не зависит.
//!
//! Рецепт — граф с именем, описанием и параметрами; встроенные лежат в
//! `recipes/`, пользовательские — в каталоге рецептов (одноимённый заменяет
//! встроенный).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

mod exec;
pub mod recipes;
pub mod synthos;

pub use exec::run;

/// Граф (он же рецепт).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Graph {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Параметры: имя → описание (тип `text` | `file` | `files` | `number`, по умолчанию).
    #[serde(default)]
    pub params: BTreeMap<String, Param>,
    pub nodes: Vec<Node>,
    /// Узлы-результаты; пусто — последний узел.
    #[serde(default)]
    pub output: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Param {
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub default: Option<Value>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Node {
    pub id: String,
    pub op: String,
    /// Выполнить для каждого элемента списка (`{{item}}` внутри полей).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub each: Option<String>,
    /// Остальные поля операции.
    #[serde(flatten)]
    pub args: Map<String, Value>,
}

/// Результат узла.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Output {
    pub text: String,
    pub files: Vec<PathBuf>,
    /// Описание (модель, длительность…) для отчёта.
    pub label: String,
}

impl Output {
    fn file(&self) -> Option<&PathBuf> {
        self.files.first()
    }
}

/// Итог прогона.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct Report {
    /// Файлы узлов-результатов.
    pub files: Vec<PathBuf>,
    /// Текст узлов-результатов (если текстовые).
    pub text: String,
    /// Шаги: узел, операция, что получилось.
    pub steps: Vec<String>,
    /// Каталог прогона (все промежуточные файлы).
    pub dir: PathBuf,
}

/// Ссылки `{{x}}` / `{{x.y}}` в строке.
fn refs(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(a) = rest.find("{{") {
        let Some(b) = rest[a..].find("}}") else { break };
        out.push(rest[a + 2..a + b].trim().to_string());
        rest = &rest[a + b + 2..];
    }
    out
}

fn value_refs(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => out.extend(refs(s)),
        Value::Array(a) => a.iter().for_each(|x| value_refs(x, out)),
        Value::Object(o) => o.values().for_each(|x| value_refs(x, out)),
        _ => {}
    }
}

impl Node {
    /// Узлы, от которых зависит этот (по подстановкам).
    pub fn deps(&self) -> Vec<String> {
        let mut r = Vec::new();
        for v in self.args.values() {
            value_refs(v, &mut r);
        }
        if let Some(e) = &self.each {
            r.extend(refs(e));
        }
        let mut ids: Vec<String> = r.into_iter().filter_map(|x| x.split_once('.').map(|(n, _)| n.to_string())).collect();
        ids.sort();
        ids.dedup();
        ids
    }
}

impl Graph {
    /// Порядок выполнения (топологический); ошибка — цикл или ссылка на неизвестный узел.
    pub fn order(&self) -> Result<Vec<usize>, String> {
        let index: HashMap<&str, usize> = self.nodes.iter().enumerate().map(|(i, n)| (n.id.as_str(), i)).collect();
        if index.len() != self.nodes.len() {
            return Err("у узлов повторяются id".into());
        }
        let mut deps: Vec<Vec<usize>> = Vec::with_capacity(self.nodes.len());
        for n in &self.nodes {
            let mut d = Vec::new();
            for id in n.deps() {
                match index.get(id.as_str()) {
                    Some(&i) => d.push(i),
                    None if self.params.contains_key(&id) => {}
                    None => return Err(format!("узел «{}» ссылается на неизвестный узел «{id}»", n.id)),
                }
            }
            deps.push(d);
        }
        let mut done = vec![false; self.nodes.len()];
        let mut order = Vec::with_capacity(self.nodes.len());
        while order.len() < self.nodes.len() {
            let before = order.len();
            for i in 0..self.nodes.len() {
                if !done[i] && deps[i].iter().all(|&d| done[d]) {
                    done[i] = true;
                    order.push(i);
                }
            }
            if order.len() == before {
                return Err("в графе цикл".into());
            }
        }
        Ok(order)
    }

    /// Проверка до запуска: порядок, известные операции, обязательные параметры.
    /// `output` в форме `id.поле` → `id` (модели часто пишут так); неизвестные узлы — ошибка.
    pub fn normalize_output(&mut self) -> Result<(), String> {
        let ids: Vec<String> = self.nodes.iter().map(|n| n.id.clone()).collect();
        let mut out = Vec::new();
        for o in &self.output {
            let id = o.trim().trim_start_matches("{{").trim_end_matches("}}").split('.').next().unwrap_or("").trim().to_string();
            if !ids.contains(&id) {
                return Err(format!("output «{o}»: нет узла «{id}» (output — список id узлов)"));
            }
            if !out.contains(&id) {
                out.push(id);
            }
        }
        self.output = out;
        Ok(())
    }

    pub fn validate(&self, params: &Map<String, Value>) -> Result<(), String> {
        self.order()?;
        for n in &self.nodes {
            if !exec::OPS.contains(&n.op.as_str()) {
                return Err(format!("узел «{}»: неизвестная операция «{}» (есть: {})", n.id, n.op, exec::OPS.join(", ")));
            }
        }
        for (name, p) in &self.params {
            if p.default.is_none() && !params.contains_key(name) {
                return Err(format!("нужен параметр «{name}»{}", if p.description.is_empty() { String::new() } else { format!(" — {}", p.description) }));
            }
        }
        Ok(())
    }
}

/// Значения для подстановок: параметры и результаты узлов.
pub(crate) struct Scope<'a> {
    pub params: &'a Map<String, Value>,
    pub outputs: &'a HashMap<String, Output>,
    pub item: Option<&'a Value>,
}

impl Scope<'_> {
    fn lookup(&self, key: &str) -> Option<Value> {
        if key == "item" {
            return self.item.cloned();
        }
        if let Some(rest) = key.strip_prefix("item.") {
            return self.item.and_then(|v| v.get(rest).cloned());
        }
        match key.split_once('.') {
            Some((node, field)) => {
                let o = self.outputs.get(node)?;
                Some(match field {
                    "text" => Value::String(o.text.clone()),
                    "file" => Value::String(o.file().map(|p| p.display().to_string()).unwrap_or_default()),
                    "files" => Value::Array(o.files.iter().map(|p| Value::String(p.display().to_string())).collect()),
                    "label" => Value::String(o.label.clone()),
                    // Непустые строки текста (без маркеров списка) — для `each` по сценам.
                    "lines" => Value::Array(
                        o.text
                            .lines()
                            .map(|l| l.trim().trim_start_matches(|c: char| c == '-' || c == '*' || c == '•' || c.is_ascii_digit() || c == '.' || c == ')').trim())
                            .filter(|l| !l.is_empty())
                            .map(|l| Value::String(l.to_string()))
                            .collect(),
                    ),
                    _ => return None,
                })
            }
            None => self.params.get(key).cloned(),
        }
    }

    /// Подставить в значение: строка целиком `{{x}}` сохраняет тип (список, число),
    /// иначе — текстовая подстановка.
    pub fn resolve(&self, v: &Value) -> Value {
        match v {
            Value::String(s) => {
                let r = refs(s);
                let t = s.trim();
                let whole = r.len() == 1 && t.starts_with("{{") && t.ends_with("}}") && t[2..t.len() - 2].trim() == r[0];
                if whole {
                    if let Some(x) = self.lookup(&r[0]) {
                        return x;
                    }
                }
                let mut out = s.clone();
                for key in r {
                    let val = match self.lookup(&key) {
                        Some(Value::String(t)) => t,
                        Some(Value::Array(a)) => a.iter().map(|x| x.as_str().map(String::from).unwrap_or_else(|| x.to_string())).collect::<Vec<_>>().join(" "),
                        Some(other) => other.to_string(),
                        None => String::new(),
                    };
                    out = out.replace(&format!("{{{{{key}}}}}", key = key), &val);
                    out = out.replace(&format!("{{{{ {key} }}}}", key = key), &val);
                }
                Value::String(out)
            }
            Value::Array(a) => Value::Array(a.iter().map(|x| self.resolve(x)).collect()),
            Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), self.resolve(x))).collect()),
            other => other.clone(),
        }
    }
}

/// Длительность медиафайла, с (ffprobe).
pub(crate) fn media_seconds(path: &Path) -> Option<f64> {
    let out = Command::new("ffprobe").args(["-v", "error", "-show_entries", "format=duration", "-of", "default=nw=1:nk=1"]).arg(path).output().ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn order_and_resolve() {
        let g: Graph = serde_json::from_value(json!({
            "params": {"topic": {"type": "text"}},
            "nodes": [
                {"id": "vid", "op": "video", "prompt": "{{img.text}}", "first_frame": "{{img.file}}"},
                {"id": "img", "op": "image", "prompt": "a picture of {{topic}}"}
            ]
        }))
        .unwrap();
        assert_eq!(g.order().unwrap(), vec![1, 0]);
        let params = json!({"topic": "a cat"}).as_object().unwrap().clone();
        g.validate(&params).unwrap();
        assert!(g.validate(&Map::new()).is_err(), "нет обязательного параметра");
        let mut outputs = HashMap::new();
        outputs.insert("img".to_string(), Output { text: "кот".into(), files: vec!["/tmp/a.png".into(), "/tmp/b.png".into()], label: String::new() });
        let s = Scope { params: &params, outputs: &outputs, item: None };
        assert_eq!(s.resolve(&json!("a picture of {{topic}}")), json!("a picture of a cat"));
        assert_eq!(s.resolve(&json!("{{img.files}}")), json!(["/tmp/a.png", "/tmp/b.png"]));
        assert_eq!(s.resolve(&json!("{{img.file}}")), json!("/tmp/a.png"));
        let cyc: Graph = serde_json::from_value(json!({"nodes": [{"id": "a", "op": "text", "text": "{{b.text}}"}, {"id": "b", "op": "text", "text": "{{a.text}}"}]})).unwrap();
        assert!(cyc.order().is_err());
    }
}
