//! Исполнение: нода за нодой по порядку зависимостей; список на входе не
//! типа `List` — пакет (нода выполняется для каждого элемента, выходы собираются
//! в списки); отмена и ход — общие для всех нод.

use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "lowercase")]
pub enum Event {
    /// Нода начата; `item` — (номер, всего) в пакете.
    Started { id: u64, name: String, item: Option<(usize, usize)> },
    /// Шаг генерации внутри ноды.
    Progress { id: u64, step: usize, total: usize },
    /// Нода готова; `cached` — результат взят из кэша.
    Finished {
        id: u64,
        name: String,
        summary: String,
        #[serde(default, skip_serializing_if = "Outputs::is_empty")]
        outputs: Outputs,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        cached: bool,
    },
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
    sandbox: Option<&'a Sandbox>,
    /// Описание результата для отчёта (модель, длительность…).
    pub label: String,
    uncached: bool,
}

/// Файлы, доступные нодам (служба: входы клиента и каталог прогона).
/// Пути сверяются по написанию, без разыменования ссылок; `..` запрещено.
#[derive(Clone, Debug, Default)]
pub struct Sandbox {
    pub roots: Vec<PathBuf>,
}

impl Sandbox {
    pub fn allows(&self, p: &Path) -> bool {
        p.is_absolute() && !p.components().any(|c| matches!(c, Component::ParentDir)) && self.roots.iter().any(|r| p.starts_with(r))
    }
}

/// Кэш результатов стадий: ключ — вид, поля и входы ноды.
pub trait StageCache: Send + Sync {
    fn get(&self, key: u64) -> Option<Outputs>;
    fn put(&self, key: u64, outputs: &Outputs);
}

/// Параметры прогона.
#[derive(Default)]
pub struct RunOpts<'a> {
    pub cancel: Cancel,
    /// Готовые значения на входах: (нода, порт) → значение (с нод вне графа).
    pub inputs: HashMap<(u64, String), Val>,
    pub sandbox: Option<Sandbox>,
    pub cache: Option<&'a dyn StageCache>,
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

    fn allowed(&self, p: &str) -> bool {
        self.sandbox.map(|s| s.allows(Path::new(p))).unwrap_or(true)
    }

    fn raw_file(&self, name: &str) -> Option<String> {
        let v = self.val(name);
        v.file().map(|p| p.display().to_string()).or_else(|| match v {
            Val::Text(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
            _ => None,
        })
    }

    /// Путь к файлу со входа или из поля (вне песочницы — нет).
    pub fn file(&self, name: &str) -> Option<String> {
        self.raw_file(name).filter(|p| self.allowed(p))
    }

    pub fn need_file(&self, name: &str) -> Result<String, String> {
        match self.raw_file(name) {
            Some(p) if self.allowed(&p) => Ok(p),
            Some(p) => Err(format!("«{}»: нет доступа к {p}", self.title_of(name))),
            None => Err(format!("нужен файл «{}»", self.title_of(name))),
        }
    }

    /// Все файлы со входа/поля (список, путь или пути через пробел).
    pub fn files(&self, name: &str) -> Vec<String> {
        let all: Vec<String> = match self.val(name) {
            Val::Text(s) => s.split_whitespace().map(String::from).collect(),
            v => v.files().into_iter().map(|p| p.display().to_string()).collect(),
        };
        all.into_iter().filter(|p| self.allowed(p)).collect()
    }

    /// Дескриптор со входа с объектом нужного типа.
    pub fn handle<T: std::any::Any + Send + Sync>(&self, name: &str) -> Result<std::sync::Arc<T>, String> {
        let v = self.val(name);
        let h = v.handle().ok_or_else(|| format!("нужен вход «{}»", self.title_of(name)))?;
        h.get::<T>().ok_or_else(|| format!("«{}»: на входе {} — не подходит этой ноде", self.title_of(name), h.label))
    }

    /// Дескриптор, если вход подключён.
    pub fn opt_handle<T: std::any::Any + Send + Sync>(&self, name: &str) -> Result<Option<std::sync::Arc<T>>, String> {
        if self.val(name).handle().is_none() {
            return Ok(None);
        }
        self.handle(name).map(Some)
    }

    /// Результат зависит не только от входов (случайный сид) — не кэшировать.
    pub fn uncached(&mut self) {
        self.uncached = true;
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

/// Входы ноды: готовые значения и выходы предшественников.
fn gather(flow: &Flow, id: u64, values: &HashMap<u64, Outputs>, preset: &HashMap<(u64, String), Val>) -> HashMap<String, Val> {
    let mut out: HashMap<String, Val> = preset.iter().filter(|((n, _), _)| *n == id).map(|((_, p), v)| (p.clone(), v.clone())).collect();
    for l in flow.links.iter().filter(|l| l.to_node == id) {
        if let Some(v) = values.get(&l.from_node).and_then(|o| o.get(&l.from_port)) {
            out.insert(l.to_port.clone(), v.clone());
        }
    }
    out
}

fn hash_val(v: &Val, h: &mut impl Hasher) {
    match v {
        Val::Empty => 0u8.hash(h),
        Val::Text(s) => (1u8, s).hash(h),
        Val::Number(n) => (2u8, n.to_bits()).hash(h),
        Val::File(p) => {
            3u8.hash(h);
            // Файл — по содержимому на диске (inode, размер, время), не по
            // пути: путь к входу клиента меняется от запроса к запросу.
            match std::fs::metadata(p) {
                Ok(m) => {
                    use std::os::unix::fs::MetadataExt;
                    (m.dev(), m.ino(), m.size(), m.mtime(), m.mtime_nsec()).hash(h)
                }
                Err(_) => p.hash(h),
            }
        }
        Val::List(l) => {
            (4u8, l.len()).hash(h);
            for x in l {
                hash_val(x, h);
            }
        }
        Val::Handle(x) => (5u8, x.id()).hash(h),
    }
}

/// Ключ кэша стадии: вид, поля, входы.
fn cache_key(node: &FlowNode, inputs: &HashMap<String, Val>) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    node.kind.hash(&mut h);
    serde_json::to_string(&node.fields).unwrap_or_default().hash(&mut h);
    let mut keys: Vec<&String> = inputs.keys().collect();
    keys.sort();
    for k in keys {
        k.hash(&mut h);
        hash_val(&inputs[k], &mut h);
    }
    h.finish()
}

/// Выполнить одну ноду с готовыми входами (пакет — по элементам).
/// Файлы — в `dir` с именем по id ноды.
pub fn run_node(node: &FlowNode, inputs: HashMap<String, Val>, dir: &Path, cancel: &Cancel, on_event: &mut dyn FnMut(Event)) -> Result<(Outputs, String), String> {
    exec_node(node, inputs, dir, cancel, None, on_event).map(|(o, l, _)| (o, l))
}

/// Нода: выходы, подпись, можно ли кэшировать.
fn exec_node(node: &FlowNode, inputs: HashMap<String, Val>, dir: &Path, cancel: &Cancel, sandbox: Option<&Sandbox>, on_event: &mut dyn FnMut(Event)) -> Result<(Outputs, String, bool), String> {
    let (spec, op) = ops::lookup(&node.kind).ok_or_else(|| format!("неизвестный вид ноды «{}»", node.kind))?;
    let op = op.ok_or_else(|| format!("«{}» исполняет только служба нейросетей (synaptixd)", spec.title))?;
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
        let mut cx = Cx { node, spec, inputs: &inputs, outputs: Outputs::new(), base: dir.join(&stem), progress: &mut prog, cancel, sandbox, label: String::new(), uncached: false };
        op(&mut cx)?;
        let label = cx.label;
        return Ok((cx.outputs, label, !cx.uncached));
    }
    let n = batch.iter().map(|(_, v)| v.len()).max().unwrap_or(0);
    let mut lists: BTreeMap<String, Vec<Val>> = spec.outputs.iter().map(|p| (p.name.to_string(), Vec::new())).collect();
    let mut label = String::new();
    let mut cacheable = true;
    // Цепочка роликов: каждый следующий начинается с последнего кадра
    // предыдущего — одна непрерывная сцена вместо склейки несвязанных.
    let chain = node.kind == "video" && node.fields.get("chain").is_some_and(|v| v.as_bool() == Some(true) || matches!(v.as_str(), Some("true" | "1")));
    let mut last_frame: Option<PathBuf> = None;
    for k in 0..n {
        cancel.check()?;
        on_event(Event::Started { id: node.id, name: name.clone(), item: Some((k + 1, n)) });
        let mut inp = inputs.clone();
        for (port, items) in &batch {
            inp.insert(port.clone(), items.get(k).or(items.last()).cloned().unwrap_or_default());
        }
        if let Some(f) = last_frame.take() {
            inp.insert("first_frame".into(), Val::File(f));
        }
        let id = node.id;
        let mut prog = |s: usize, t: usize| on_event(Event::Progress { id, step: s, total: t });
        let mut cx = Cx { node, spec, inputs: &inp, outputs: Outputs::new(), base: dir.join(format!("{stem}-{k:03}")), progress: &mut prog, cancel, sandbox, label: String::new(), uncached: false };
        op(&mut cx).map_err(|e| format!("элемент {} из {n}: {e}", k + 1))?;
        if chain && k + 1 < n {
            if let Some(Val::File(v)) = cx.outputs.get("video") {
                let png = dir.join(format!("{stem}-{k:03}-last.png"));
                let (vs, ps) = (v.display().to_string(), png.display().to_string());
                ops::media::ffmpeg(&["-sseof", "-0.2", "-i", &vs, "-frames:v", "1", "-update", "1", &ps]).map_err(|e| format!("последний кадр ролика {}: {e}", k + 1))?;
                last_frame = Some(png);
            }
        }
        label = cx.label;
        cacheable &= !cx.uncached;
        for (port, v) in cx.outputs {
            lists.entry(port).or_default().push(v);
        }
    }
    let outs = lists.into_iter().map(|(p, v)| (p, Val::List(v))).collect();
    Ok((outs, format!("{n} × {label}"), cacheable))
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
    run_with(flow, dir, &RunOpts { cancel: cancel.clone(), ..Default::default() }, on_event)
}

/// Выполнить граф с параметрами прогона (служба: песочница, кэш стадий,
/// входы с нод вне графа).
pub fn run_with(flow: &Flow, dir: &Path, opts: &RunOpts, on_event: &mut dyn FnMut(Event)) -> Result<Report, String> {
    flow.validate()?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let cancel = &opts.cancel;
    let mut values: HashMap<u64, Outputs> = HashMap::new();
    let mut steps = Vec::new();
    for id in flow.order()? {
        let node = flow.node(id).unwrap();
        if !node.enabled {
            on_event(Event::Skipped { id, name: node.name() });
            values.insert(id, Outputs::new());
            continue;
        }
        let inputs = gather(flow, id, &values, &opts.inputs);
        let stage = ops::spec(&node.kind).map(|s| s.stage).unwrap_or(false);
        let key = opts.cache.filter(|_| stage).map(|_| cache_key(node, &inputs));
        let hit = key.and_then(|k| opts.cache.and_then(|c| c.get(k)));
        let cached = hit.is_some();
        let (outs, label) = match hit {
            Some(o) => {
                let what = o.values().map(Val::text).collect::<Vec<_>>().join(", ");
                (o, format!("{what} (из кэша)"))
            }
            None => {
                let (outs, label, cacheable) = exec_node(node, inputs, dir, cancel, opts.sandbox.as_ref(), on_event).map_err(|e| format!("{}: {e}", node.name()))?;
                // В кэш — только дескрипторы: файлы живут в каталоге прогона.
                if let (Some(k), Some(c), true) = (key, opts.cache, cacheable) {
                    if !outs.is_empty() && outs.values().all(|v| v.items().iter().all(|x| matches!(x, Val::Handle(_)))) {
                        c.put(k, &outs);
                    }
                }
                (outs, label)
            }
        };
        let summary = summarize(&outs, &label);
        on_event(Event::Finished { id, name: node.name(), summary: summary.clone(), outputs: outs.clone(), cached });
        steps.push(format!("{}: {summary}", node.name()));
        values.insert(id, outs);
    }
    Ok(Report::build(flow, values, steps, dir))
}

impl Report {
    /// Итог по выходам нод: файлы и текст нод-результатов.
    pub fn build(flow: &Flow, values: HashMap<u64, Outputs>, steps: Vec<String>, dir: &Path) -> Report {
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
        report
    }
}

fn summarize(outs: &Outputs, label: &str) -> String {
    let files: usize = outs.values().map(|v| v.files().len()).sum();
    let handles = !outs.is_empty() && outs.values().all(|v| v.handle().is_some());
    let what = if files > 0 {
        format!("{files} файл(ов)")
    } else if handles {
        // Дескриптор: его подпись и есть описание.
        if label.is_empty() {
            outs.values().map(Val::text).collect::<Vec<_>>().join(", ")
        } else {
            String::new()
        }
    } else {
        let t: String = outs.values().map(Val::text).collect::<Vec<_>>().join(" ").chars().take(80).collect();
        format!("текст: {t}")
    };
    match (what.is_empty(), label.is_empty()) {
        (true, _) => label.to_string(),
        (false, true) => what,
        (false, false) => format!("{what}, {label}"),
    }
}
