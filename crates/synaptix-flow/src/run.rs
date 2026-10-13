//! Исполнение: нода за нодой по порядку зависимостей; список на входе не
//! типа `List` — пакет (нода выполняется для каждого элемента, выходы собираются
//! в списки); отмена и ход — общие для всех нод.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;

use crate::doc::{Flow, FlowNode};
use crate::ops;
use crate::schema::{FieldType, NodeSpec, PortType};
use crate::value::Val;

/// Выходы ноды: порт → значение.
pub type Outputs = BTreeMap<String, Val>;

/// Реализация вида ноды.
pub type OpFn = fn(&mut Cx) -> Result<(), String>;

/// Отмена прогона. Нода, ждущая службу, регистрирует обработчик
/// ([`Cancel::on_cancel`]) — он рвёт соединение, и служба прерывает генерацию.
#[derive(Clone, Default)]
pub struct Cancel(Arc<CancelInner>);

#[derive(Default)]
struct CancelInner {
    flag: AtomicBool,
    next: AtomicU64,
    hooks: Mutex<Vec<(u64, Box<dyn Fn() + Send>)>>,
}

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.flag.store(true, Ordering::SeqCst);
        if let Ok(h) = self.0.hooks.lock() {
            for (_, f) in h.iter() {
                f();
            }
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.flag.load(Ordering::SeqCst)
    }

    /// Обработчик на время жизни охранника.
    pub fn on_cancel(&self, f: impl Fn() + Send + 'static) -> CancelGuard {
        let id = self.0.next.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut h) = self.0.hooks.lock() {
            h.push((id, Box::new(f)));
        }
        if self.is_cancelled() {
            self.cancel();
        }
        CancelGuard { cancel: self.clone(), id }
    }

    pub fn check(&self) -> Result<(), String> {
        if self.is_cancelled() {
            Err("прогон остановлен".into())
        } else {
            Ok(())
        }
    }
}

pub struct CancelGuard {
    cancel: Cancel,
    id: u64,
}

impl Drop for CancelGuard {
    fn drop(&mut self) {
        if let Ok(mut h) = self.cancel.0.hooks.lock() {
            h.retain(|(i, _)| *i != self.id);
        }
    }
}

/// Событие прогона.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event", rename_all = "lowercase")]
pub enum Event {
    /// Нода начата; `item` — (номер, всего) в пакете.
    Started { id: u64, name: String, item: Option<(usize, usize)> },
    /// Шаг генерации внутри ноды.
    Progress { id: u64, step: usize, total: usize },
    /// Нода готова.
    Finished { id: u64, name: String, summary: String },
    /// Нода выключена — пропущена.
    Skipped { id: u64, name: String },
}

/// Контекст исполнения одной ноды (одного элемента пакета).
pub struct Cx<'a> {
    pub node: &'a FlowNode,
    pub spec: &'static NodeSpec,
    inputs: &'a HashMap<String, Val>,
    outputs: Outputs,
    base: PathBuf,
    progress: &'a mut dyn FnMut(usize, usize),
    pub cancel: &'a Cancel,
    /// Описание результата для отчёта (модель, длительность…).
    pub label: String,
}

impl<'a> Cx<'a> {
    /// Значение: подключённый вход, иначе поле, иначе умолчание схемы.
    pub fn val(&self, name: &str) -> Val {
        if let Some(v) = self.inputs.get(name) {
            if !v.is_empty() {
                return v.clone();
            }
        }
        if let Some(v) = self.node.fields.get(name) {
            let v = Val::from_json(v);
            if !v.is_empty() {
                return v;
            }
        }
        match self.spec.field(name) {
            Some(f) if !f.default.is_empty() => match f.ty {
                FieldType::Number => f.default.parse().map(Val::Number).unwrap_or(Val::Empty),
                _ => Val::Text(f.default.into()),
            },
            _ => Val::Empty,
        }
    }

    pub fn str(&self, name: &str) -> Option<String> {
        let v = self.val(name);
        let s = v.text();
        let s = s.trim();
        (!s.is_empty()).then(|| s.to_string())
    }

    pub fn need(&self, name: &str) -> Result<String, String> {
        self.str(name).ok_or_else(|| format!("нужно «{}»", self.title_of(name)))
    }

    pub fn num(&self, name: &str) -> Option<f64> {
        self.val(name).number()
    }

    /// Число > 0 (0 в поле — «не задано»).
    pub fn pos(&self, name: &str) -> Option<f64> {
        self.num(name).filter(|v| *v > 0.0)
    }

    pub fn bool(&self, name: &str) -> bool {
        matches!(self.val(name).text().trim(), "true" | "1" | "yes" | "да")
    }

    /// Путь к файлу со входа или из поля.
    pub fn file(&self, name: &str) -> Option<String> {
        let v = self.val(name);
        v.file().map(|p| p.display().to_string()).or_else(|| match v {
            Val::Text(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
            _ => None,
        })
    }

    pub fn need_file(&self, name: &str) -> Result<String, String> {
        self.file(name).ok_or_else(|| format!("нужен файл «{}»", self.title_of(name)))
    }

    /// Все файлы со входа/поля (список, путь или пути через пробел).
    pub fn files(&self, name: &str) -> Vec<String> {
        match self.val(name) {
            Val::Text(s) => s.split_whitespace().map(String::from).collect(),
            v => v.files().into_iter().map(|p| p.display().to_string()).collect(),
        }
    }

    pub fn out(&mut self, port: &str, v: Val) {
        self.outputs.insert(port.into(), v);
    }

    /// Путь выходного файла ноды с расширением.
    pub fn out_path(&self, ext: &str) -> PathBuf {
        self.base.with_extension(ext)
    }

    /// Каталог для промежуточных файлов ноды.
    pub fn scratch(&self) -> PathBuf {
        self.base.with_extension("parts")
    }

    pub fn progress(&mut self, step: usize, total: usize) {
        (self.progress)(step, total);
    }

    fn title_of(&self, name: &str) -> String {
        self.spec
            .field(name)
            .map(|f| f.label)
            .or_else(|| self.spec.input(name).map(|p| p.label))
            .unwrap_or(name)
            .to_string()
    }
}

/// Входы ноды из выходов предшественников.
fn gather(flow: &Flow, id: u64, values: &HashMap<u64, Outputs>) -> HashMap<String, Val> {
    flow.links
        .iter()
        .filter(|l| l.to_node == id)
        .filter_map(|l| values.get(&l.from_node).and_then(|o| o.get(&l.from_port)).map(|v| (l.to_port.clone(), v.clone())))
        .collect()
}

/// Выполнить одну ноду с готовыми входами (пакет — по элементам).
/// Файлы — в `dir` с именем по id ноды.
pub fn run_node(node: &FlowNode, inputs: HashMap<String, Val>, dir: &Path, cancel: &Cancel, on_event: &mut dyn FnMut(Event)) -> Result<(Outputs, String), String> {
    let (spec, op) = ops::lookup(&node.kind).ok_or_else(|| format!("неизвестный вид ноды «{}»", node.kind))?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    // Пакетные входы: список на одиночном порту.
    let batch: Vec<(String, Vec<Val>)> = inputs
        .iter()
        .filter(|(name, v)| matches!(v, Val::List(_)) && spec.input(name).map(|p| p.ty != PortType::List).unwrap_or(false))
        .map(|(n, v)| (n.clone(), v.items()))
        .collect();
    let stem = if node.label.is_empty() { format!("n{}", node.id) } else { sanitize(&node.label) };
    let name = node.name();
    if batch.is_empty() {
        cancel.check()?;
        on_event(Event::Started { id: node.id, name: name.clone(), item: None });
        let id = node.id;
        let mut prog = |s: usize, t: usize| on_event(Event::Progress { id, step: s, total: t });
        let mut cx = Cx { node, spec, inputs: &inputs, outputs: Outputs::new(), base: dir.join(&stem), progress: &mut prog, cancel, label: String::new() };
        op(&mut cx)?;
        let label = cx.label;
        return Ok((cx.outputs, label));
    }
    let n = batch.iter().map(|(_, v)| v.len()).max().unwrap_or(0);
    let mut lists: BTreeMap<String, Vec<Val>> = spec.outputs.iter().map(|p| (p.name.to_string(), Vec::new())).collect();
    let mut label = String::new();
    for k in 0..n {
        cancel.check()?;
        on_event(Event::Started { id: node.id, name: name.clone(), item: Some((k + 1, n)) });
        let mut inp = inputs.clone();
        for (port, items) in &batch {
            inp.insert(port.clone(), items.get(k).or(items.last()).cloned().unwrap_or_default());
        }
        let id = node.id;
        let mut prog = |s: usize, t: usize| on_event(Event::Progress { id, step: s, total: t });
        let mut cx = Cx { node, spec, inputs: &inp, outputs: Outputs::new(), base: dir.join(format!("{stem}-{k:03}")), progress: &mut prog, cancel, label: String::new() };
        op(&mut cx).map_err(|e| format!("элемент {} из {n}: {e}", k + 1))?;
        label = cx.label;
        for (port, v) in cx.outputs {
            lists.entry(port).or_default().push(v);
        }
    }
    let outs = lists.into_iter().map(|(p, v)| (p, Val::List(v))).collect();
    Ok((outs, format!("{n} × {label}")))
}

fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

/// Итог прогона.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Report {
    /// Выходы всех нод.
    pub values: HashMap<u64, Outputs>,
    /// Шаги: нода — что получилось.
    pub steps: Vec<String>,
    /// Файлы нод-результатов.
    pub files: Vec<PathBuf>,
    /// Текст нод-результатов (у нод без файлов).
    pub text: String,
    pub dir: PathBuf,
}

/// Выполнить граф; файлы — в `dir`.
pub fn run(flow: &Flow, dir: &Path, cancel: &Cancel, on_event: &mut dyn FnMut(Event)) -> Result<Report, String> {
    flow.validate()?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut values: HashMap<u64, Outputs> = HashMap::new();
    let mut steps = Vec::new();
    for id in flow.order()? {
        let node = flow.node(id).unwrap();
        if !node.enabled {
            on_event(Event::Skipped { id, name: node.name() });
            values.insert(id, Outputs::new());
            continue;
        }
        let inputs = gather(flow, id, &values);
        let (outs, label) = run_node(node, inputs, dir, cancel, on_event).map_err(|e| format!("{}: {e}", node.name()))?;
        let summary = summarize(&outs, &label);
        on_event(Event::Finished { id, name: node.name(), summary: summary.clone() });
        steps.push(format!("{}: {summary}", node.name()));
        values.insert(id, outs);
    }
    let mut report = Report { steps, dir: dir.to_path_buf(), ..Default::default() };
    let mut texts = Vec::new();
    for id in flow.result_nodes() {
        let Some(node) = flow.node(id) else { continue };
        let Some(main) = ops::spec(&node.kind).and_then(|s| s.main_output()) else { continue };
        let Some(v) = values.get(&id).and_then(|o| o.get(main.name)) else { continue };
        let files = v.files();
        if files.is_empty() {
            let t = v.text();
            if !t.trim().is_empty() {
                texts.push(t);
            }
        } else {
            report.files.extend(files);
        }
    }
    report.text = texts.join("\n\n");
    report.values = values;
    Ok(report)
}

fn summarize(outs: &Outputs, label: &str) -> String {
    let files: usize = outs.values().map(|v| v.files().len()).sum();
    let what = if files > 0 {
        format!("{files} файл(ов)")
    } else {
        let t: String = outs.values().map(Val::text).collect::<Vec<_>>().join(" ").chars().take(80).collect();
        format!("текст: {t}")
    };
    if label.is_empty() {
        what
    } else {
        format!("{what}, {label}")
    }
}
