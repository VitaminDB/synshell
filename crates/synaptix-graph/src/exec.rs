//! Исполнение рецепта: компиляция в граф ядра `synaptix-flow` и его
//! прогон службой synaptixd (`op flow`). Ход — строками «узел (операция)», «… — k из n», «… · шаг/всего».

use std::collections::HashMap;
use std::path::Path;

use serde_json::{Map, Value};
use synaptix_flow::{Cancel, Event};

use crate::{Graph, Report};

pub(crate) const OPS: &[&str] = &["text", "llm", "image", "video", "music", "speak", "transcribe", "concat", "mux", "frame", "trim", "still", "vfx_upscale", "vfx_interpolate"];

/// Параметры: заданные + умолчания.
pub(crate) fn with_defaults(graph: &Graph, params: &Map<String, Value>) -> Map<String, Value> {
    let mut params = params.clone();
    for (k, p) in &graph.params {
        if !params.contains_key(k) {
            if let Some(d) = &p.default {
                params.insert(k.clone(), d.clone());
            }
        }
    }
    params
}

/// Выполнить граф с параметрами; файлы — в `dir`. `on_step(текст)` — ход.
pub fn run(graph: &Graph, params: &Map<String, Value>, dir: &Path, on_step: &mut dyn FnMut(&str)) -> Result<Report, String> {
    run_cancellable(graph, params, dir, &Cancel::new(), on_step)
}

/// То же с отменой.
pub fn run_cancellable(graph: &Graph, params: &Map<String, Value>, dir: &Path, cancel: &Cancel, on_step: &mut dyn FnMut(&str)) -> Result<Report, String> {
    run_observed(graph, params, dir, cancel, on_step, &mut |_, _| {})
}

/// То же, плюс `on_files(узел, файлы)` — файлы узла рецепта, как только он
/// отработал (картинки героев, ролики сцен): программа может показывать их
/// сразу, а не после всего прогона.
pub fn run_observed(graph: &Graph, params: &Map<String, Value>, dir: &Path, cancel: &Cancel, on_step: &mut dyn FnMut(&str), on_files: &mut dyn FnMut(&str, &[std::path::PathBuf])) -> Result<Report, String> {
    let params = with_defaults(graph, params);
    let mut graph = graph.clone();
    graph.normalize_output()?;
    graph.validate(&params)?;
    let flow = crate::compile::to_flow(&graph, &params)?;
    // Заголовки нод для хода: узлы рецепта — «id (op)», служебные — пропускаются.
    let ops: HashMap<String, String> = graph.nodes.iter().map(|n| (n.id.clone(), n.op.clone())).collect();
    let heads: HashMap<u64, String> = flow.nodes.iter().filter_map(|n| ops.get(&n.label).map(|op| (n.id, format!("{} ({op})", n.label)))).collect();
    let mut current: HashMap<u64, String> = HashMap::new();
    let labels: HashMap<u64, String> = flow.nodes.iter().filter(|n| ops.contains_key(&n.label)).map(|n| (n.id, n.label.clone())).collect();
    let mut nodes: std::collections::BTreeMap<String, crate::Output> = std::collections::BTreeMap::new();
    let r = synaptixd::flow::run(&flow, dir, cancel, &mut |e| match e {
        Event::Finished { id, ref outputs, ref summary, .. } => {
            if let Some(label) = labels.get(&id) {
                let fresh: Vec<std::path::PathBuf> = outputs.values().flat_map(|v| v.files()).collect();
                if !fresh.is_empty() {
                    on_files(label, &fresh);
                }
                let o = nodes.entry(label.clone()).or_default();
                for v in outputs.values() {
                    for f in v.files() {
                        if !o.files.contains(&f) {
                            o.files.push(f);
                        }
                    }
                    let t = v.text();
                    if !t.is_empty() && v.files().is_empty() {
                        if !o.text.is_empty() {
                            o.text.push('\n');
                        }
                        o.text.push_str(&t);
                    }
                }
                o.label = summary.clone();
            }
        }
        Event::Started { id, item, .. } => {
            if let Some(h) = heads.get(&id) {
                let h = match item {
                    Some((k, n)) => format!("{h} — {k} из {n}"),
                    None => h.clone(),
                };
                on_step(&h);
                current.insert(id, h);
            }
        }
        Event::Progress { id, step, total } => {
            if let Some(h) = current.get(&id) {
                on_step(&format!("{h} · {step}/{total}"));
            }
        }
        _ => {}
    })?;
    let steps = r
        .steps
        .iter()
        .filter(|s| flow.nodes.iter().any(|n| heads.contains_key(&n.id) && s.starts_with(&format!("{}:", n.label))))
        .map(|s| {
            let (label, rest) = s.split_once(':').unwrap_or((s, ""));
            format!("{label} ({}):{rest}", ops.get(label).map(String::as_str).unwrap_or(""))
        })
        .collect();
    Ok(Report { files: r.files, text: r.text, steps, dir: dir.to_path_buf(), nodes })
}
