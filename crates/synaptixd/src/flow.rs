//! Прогон графа ядра synaptix-flow службой (`op flow`): клиент отдаёт граф
//! целиком, служба исполняет генерацию, стадии моделей (тензоры между ними
//! остаются в её памяти) и монтаж. Входные файлы уходят дескрипторами —
//! служба не открывает пути клиента сама; файлы выходов приходят
//! дескрипторами по мере готовности нод и копируются в `dir`.

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};

use serde_json::Value;
use synaptix_flow::{Event, FieldType, Flow, Outputs, Report, RunOpts, Val};

use crate::{Client, ErrorCode, FlowInput, FlowReply, Request};

/// Метка входного файла в графе, отданном службе.
pub fn in_marker(i: usize) -> String {
    format!("@in:{i}")
}

/// Метка файла выхода в событии службы.
pub fn out_marker(i: usize) -> String {
    format!("@out:{i}")
}

/// Номер из метки `@in:<n>` / `@out:<n>`.
pub fn marker_index(s: &str, prefix: &str) -> Option<usize> {
    s.strip_prefix(prefix)?.parse().ok()
}

/// Существующий файл клиента по строке (`~/` — домашний каталог).
fn local_file(s: &str) -> Option<PathBuf> {
    let s = s.trim();
    let p = match s.strip_prefix("~/") {
        Some(rest) => PathBuf::from(std::env::var("HOME").ok()?).join(rest),
        None => PathBuf::from(s),
    };
    (p.is_absolute() && p.is_file()).then_some(p)
}

/// Входные файлы запроса.
#[derive(Default)]
struct Uploads {
    files: Vec<File>,
    names: Vec<String>,
    index: HashMap<PathBuf, usize>,
}

impl Uploads {
    fn add(&mut self, p: &Path) -> Result<String, String> {
        if let Some(&i) = self.index.get(p) {
            return Ok(in_marker(i));
        }
        if self.files.len() >= crate::wire::MAX_FDS {
            return Err(format!("в графе больше {} входных файлов", crate::wire::MAX_FDS));
        }
        let f = File::open(p).map_err(|e| format!("{}: {e}", p.display()))?;
        let i = self.files.len();
        self.files.push(f);
        self.names.push(p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| format!("in{i}")));
        self.index.insert(p.to_path_buf(), i);
        Ok(in_marker(i))
    }

    /// Строка поля: путь (поле-файл) или все слова — существующие файлы.
    fn text(&mut self, s: &str, is_file: bool) -> Result<Option<String>, String> {
        let words: Vec<&str> = s.split_whitespace().collect();
        if words.is_empty() {
            return Ok(None);
        }
        if is_file && words.len() > 1 {
            if let Some(p) = local_file(s) {
                return self.add(&p).map(Some);
            }
        }
        let paths: Option<Vec<PathBuf>> = words.iter().map(|w| local_file(w)).collect();
        match paths {
            Some(ps) => Ok(Some(ps.iter().map(|p| self.add(p)).collect::<Result<Vec<_>, _>>()?.join(" "))),
            None => Ok(None),
        }
    }

    fn json(&mut self, v: &mut Value, is_file: bool) -> Result<(), String> {
        match v {
            Value::String(s) => {
                if let Some(m) = self.text(s, is_file)? {
                    *s = m;
                }
            }
            Value::Array(a) => {
                for x in a {
                    self.json(x, is_file)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn val(&mut self, v: &Val) -> Result<Val, String> {
        Ok(match v {
            Val::File(p) => match local_file(&p.to_string_lossy()) {
                Some(p) => Val::File(PathBuf::from(self.add(&p)?)),
                None => v.clone(),
            },
            Val::Text(s) => match self.text(s, false)? {
                Some(m) if !m.contains(' ') => Val::File(PathBuf::from(m)),
                _ => v.clone(),
            },
            Val::List(l) => Val::List(l.iter().map(|x| self.val(x)).collect::<Result<_, _>>()?),
            other => other.clone(),
        })
    }
}

/// Граф с файлами клиента, заменёнными метками, и сами файлы.
fn prepare(flow: &Flow, inputs: &HashMap<(u64, String), Val>) -> Result<(Flow, Vec<FlowInput>, Uploads), String> {
    let mut up = Uploads::default();
    let mut flow = flow.clone();
    for n in &mut flow.nodes {
        let spec = synaptix_flow::ops::spec(&n.kind);
        for (k, v) in n.fields.iter_mut() {
            let is_file = spec.and_then(|s| s.field(k)).map(|f| f.ty == FieldType::File).unwrap_or(false);
            up.json(v, is_file)?;
        }
    }
    let mut ins = Vec::new();
    for ((node, port), v) in inputs {
        ins.push(FlowInput { node: *node, port: port.clone(), value: up.val(v)? });
    }
    Ok((flow, ins, up))
}

/// Файлы выходов, пришедшие от службы: номер → путь в `dir`.
#[derive(Default)]
struct Received {
    paths: HashMap<usize, PathBuf>,
}

impl Received {
    fn store(&mut self, first: usize, names: &[String], fds: Vec<OwnedFd>, dir: &Path) -> Result<(), String> {
        for (k, (fd, name)) in fds.into_iter().zip(names).enumerate() {
            let name = Path::new(name).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| format!("out{}", first + k));
            let mut path = dir.join(&name);
            if path.exists() {
                path = dir.join(format!("{}-{name}", first + k));
            }
            let mut src = File::from(fd);
            let mut dst = File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            std::io::copy(&mut src, &mut dst).map_err(|e| format!("{}: {e}", path.display()))?;
            self.paths.insert(first + k, path);
        }
        Ok(())
    }

    fn local(&self, v: &Val) -> Val {
        v.map_files(&mut |p| match marker_index(&p.to_string_lossy(), "@out:").and_then(|i| self.paths.get(&i)) {
            Some(l) => l.clone(),
            None => p.to_path_buf(),
        })
    }

    fn outputs(&self, o: &Outputs) -> Outputs {
        o.iter().map(|(k, v)| (k.clone(), self.local(v))).collect()
    }
}

/// Выполнить граф службой; файлы выходов — в `dir`.
pub fn run(flow: &Flow, dir: &Path, cancel: &synaptix_flow::Cancel, on_event: &mut dyn FnMut(Event)) -> Result<Report, String> {
    run_with(flow, dir, &RunOpts { cancel: cancel.clone(), ..Default::default() }, on_event)
}

/// То же с готовыми входами (`opts.inputs`); песочница и кэш — дело службы.
pub fn run_with(flow: &Flow, dir: &Path, opts: &RunOpts, on_event: &mut dyn FnMut(Event)) -> Result<Report, String> {
    flow.validate()?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let cancel = &opts.cancel;
    cancel.check()?;
    let (wire_flow, inputs, up) = prepare(flow, &opts.inputs)?;
    let mut c = Client::connect().map_err(|e| format!("служба нейросетей: {e}"))?;
    let _guard = c.canceller().map(|k| cancel.on_cancel(move || k.cancel()));
    let stopped = |e: String| if cancel.is_cancelled() { "прогон остановлен".to_string() } else { e };
    let fds: Vec<BorrowedFd> = up.files.iter().map(|f| f.as_fd()).collect();
    let req = Request::Flow { flow: serde_json::to_value(&wire_flow).map_err(|e| e.to_string())?, inputs, files: up.names.clone() };
    let id = c.send(req, &fds).map_err(|e| stopped(format!("служба нейросетей: {e}")))?;
    drop(fds);
    let mut got = Received::default();
    let mut values: HashMap<u64, Outputs> = HashMap::new();
    loop {
        let (reply, fds) = c.recv().map_err(|e| stopped(format!("служба нейросетей: {e}")))?;
        if reply.id != id {
            continue;
        }
        match reply.event {
            Some(crate::Event::FlowFiles { first, names }) => got.store(first, &names, fds, dir)?,
            Some(crate::Event::Flow { node }) => {
                let node = match node {
                    Event::Finished { id, name, summary, outputs, cached } => {
                        let outputs = got.outputs(&outputs);
                        values.insert(id, outputs.clone());
                        Event::Finished { id, name, summary, outputs, cached }
                    }
                    Event::Skipped { id, name } => {
                        values.insert(id, BTreeMap::new());
                        Event::Skipped { id, name }
                    }
                    other => other,
                };
                on_event(node);
            }
            Some(_) => {}
            None if reply.ok == Some(true) => {
                let r: FlowReply = serde_json::from_value(reply.data).unwrap_or_default();
                return Ok(Report::build(flow, values, r.steps, dir));
            }
            None => {
                let msg = reply.error.unwrap_or_else(|| "ошибка службы нейросетей".into());
                return Err(stopped(match reply.code {
                    Some(ErrorCode::Unavailable) => format!("служба нейросетей: {msg}"),
                    _ => msg,
                }));
            }
        }
    }
}

/// Одна нода с готовыми входами — граф из неё одной.
pub fn run_node(node: &synaptix_flow::FlowNode, inputs: HashMap<String, Val>, dir: &Path, cancel: &synaptix_flow::Cancel, on_event: &mut dyn FnMut(Event)) -> Result<(Outputs, String), String> {
    let flow = Flow { nodes: vec![node.clone()], links: Vec::new(), outputs: vec![node.id] };
    let opts = RunOpts { cancel: cancel.clone(), inputs: inputs.into_iter().map(|(k, v)| ((node.id, k), v)).collect(), ..Default::default() };
    let mut summary = String::new();
    let r = run_with(&flow, dir, &opts, &mut |e| {
        if let Event::Finished { summary: s, .. } = &e {
            summary = s.clone();
        }
        on_event(e)
    })?;
    Ok((r.values.get(&node.id).cloned().unwrap_or_default(), summary))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_replaces_client_files() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.png");
        std::fs::write(&a, b"x").unwrap();
        let flow: Flow = serde_json::from_value(serde_json::json!({
            "nodes": [
                {"id": 1, "kind": "file", "fields": {"path": a.display().to_string()}},
                {"id": 2, "kind": "text", "fields": {"text": format!("нарисуй {}", a.display())}},
                {"id": 3, "kind": "list", "fields": {"items": [a.display().to_string(), "кот"]}}
            ]
        }))
        .unwrap();
        let mut inputs = HashMap::new();
        inputs.insert((2, "x".to_string()), Val::File(a.clone()));
        let (f, ins, up) = prepare(&flow, &inputs).unwrap();
        assert_eq!(f.nodes[0].fields["path"], "@in:0");
        // Текст со словами — не путь.
        assert_eq!(f.nodes[1].fields["text"], format!("нарисуй {}", a.display()));
        assert_eq!(f.nodes[2].fields["items"], serde_json::json!(["@in:0", "кот"]));
        assert_eq!(ins[0].value, Val::File("@in:0".into()));
        assert_eq!(up.files.len(), 1);
        assert_eq!(up.names, vec!["a.png"]);
    }
}
