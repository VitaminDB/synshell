//! Ноды ядра synaptix-flow — общий нодовый движок synshell (ассистент,
//! рецепты, Synthos). Генерация идёт через службу synaptixd (LLM, картинки,
//! видео, музыка, речь, расшифровка), монтаж — ffmpeg (склейка, наложение
//! звука, кадр, обрезка, картинка в ролик), данные — текст, файл, шаблон,
//! строки, список.
//!
//! Стадии моделей картинок (FLUX.1, FLUX.2, Qwen-Image, SDXL) и MiniMax-H3:
//! модель → условие → латент → сэмплер → декодер; модели грузит служба,
//! дескрипторы между стадиями живут в её памяти.
//!
//! Метаданные (порты, поля) строятся по схемам ядра; исполняет граф служба
//! (`synaptixd::flow`), тем же ядром, что рецепты ассистента: список на
//! входе — пакет, нода выполняется для каждого элемента. Запуск ноды
//! отправляет её вместе с нодами ядра, от которых к ней идут дескрипторы
//! (их служба берёт из кэша стадий, если входы не менялись); остальные
//! входы — готовые значения: с нод ядра — их последние выходы, с родных
//! нод Synthos — через файлы (картинка на выходе — [`ImageData`], звук —
//! `AudioBuffer`, на входе картинка и звук пишутся во временные файлы).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::Value as Json;
use synaptix_flow::{self as sf, FieldType as FT, PortType as PT, Val};
use syngui::core::sync::Mutex;
use syngui::layout::CrossAxisAlignment;
use syngui::prelude::*;
use syngui::widgets::input::dropdown::DropdownItem;
use syngui::widgets::input::MultilineTextEdit;
use syngui::widgets::{Column, DecoratedBox, GestureDetector, Padding, Reactive, Row, ToolButton, TooltipPosition};

use super::super::eval::{EvalContext, NodeExecutor};
use super::super::registry::{NodeCategory, NodeKindMeta};
use super::super::state::NodeEditorCtx;
use super::super::types::{
    DataBlob, FieldSchema, FieldType, FieldValue, ImageData, NodeId, NodeInstance, NodeKind, NodeRuntime, PortKind,
    PortSchema, PortValue, PortsLayout, PortsSpec,
};
use super::{log_worker_done, log_worker_start};
use crate::icons::{
    MI_ARTICLE, MI_ASPECT_RATIO, MI_AUDIOTRACK, MI_AUTO_AWESOME, MI_CHEVRON_RIGHT, MI_CROP_SQUARE, MI_EDIT_NOTE,
    MI_EXPAND_MORE, MI_FOLDER_OPEN, MI_GRAPHIC_EQ, MI_GROUPS, MI_IMAGE_ICON, MI_INVENTORY_2, MI_LANGUAGE, MI_LAYERS,
    MI_LIBRARY_MUSIC, MI_MERGE_TYPE, MI_MIC, MI_MOVIE, MI_PALETTE, MI_PSYCHOLOGY, MI_RECORD_VOICE_OVER, MI_TRANSLATE,
    MI_TUNE, MI_ZOOM_IN,
};

/// Вид ноды Synthos ↔ вид ноды ядра.
const KINDS: &[(NodeKind, &str, &str, &str)] = &[
    (NodeKind::FlowText, "text", "Flow Text", MI_ARTICLE),
    (NodeKind::FlowFile, "file", "Flow File", MI_FOLDER_OPEN),
    (NodeKind::FlowFormat, "format", "Flow Template", MI_EDIT_NOTE),
    (NodeKind::FlowLines, "lines", "Flow Lines", MI_TUNE),
    (NodeKind::FlowList, "list", "Flow List", MI_LAYERS),
    (NodeKind::FlowLlm, "llm", "Flow LLM", MI_PSYCHOLOGY),
    (NodeKind::FlowImage, "image", "Flow Image", MI_IMAGE_ICON),
    (NodeKind::FlowVideo, "video", "Flow Video", MI_MOVIE),
    (NodeKind::FlowMusic, "music", "Flow Music", MI_LIBRARY_MUSIC),
    (NodeKind::FlowSpeak, "speak", "Flow Speech", MI_RECORD_VOICE_OVER),
    (NodeKind::FlowTranscribe, "transcribe", "Flow Transcribe", MI_MIC),
    (NodeKind::FlowConcat, "concat", "Flow Concat", MI_MERGE_TYPE),
    (NodeKind::FlowMux, "mux", "Flow Mux Audio", MI_AUDIOTRACK),
    (NodeKind::FlowFrame, "frame", "Flow Frame", MI_IMAGE_ICON),
    (NodeKind::FlowTrim, "trim", "Flow Trim", MI_MOVIE),
    (NodeKind::FlowStill, "still", "Flow Still Video", MI_LANGUAGE),
    (NodeKind::FlowVfxUpscale, "vfx_upscale", "Flow Upscale", MI_ZOOM_IN),
    (NodeKind::FlowVfxInterpolate, "vfx_interpolate", "Flow RIFE", MI_MOVIE),
    (NodeKind::FlowSubflow, "subflow", "Flow Subgraph", MI_LAYERS),
    (NodeKind::FlowImageModel, "image_model", "Image Model", MI_INVENTORY_2),
    (NodeKind::FlowImageEncode, "image_encode", "Prompt Encoder", MI_TRANSLATE),
    (NodeKind::FlowImageRefs, "image_refs", "References", MI_LAYERS),
    (NodeKind::FlowImageLatent, "image_latent", "Empty Latent", MI_ASPECT_RATIO),
    (NodeKind::FlowImageToLatent, "image_to_latent", "Image to Latent", MI_IMAGE_ICON),
    (NodeKind::FlowImageSample, "image_sample", "Sampler", MI_AUTO_AWESOME),
    (NodeKind::FlowImageDecode, "image_decode", "Image Decoder", MI_PALETTE),
    (NodeKind::FlowH3Model, "h3_model", "H3 Model", MI_INVENTORY_2),
    (NodeKind::FlowH3Latent, "h3_latent", "H3 Video Size", MI_CROP_SQUARE),
    (NodeKind::FlowH3Encode, "h3_encode", "H3 Encoder", MI_TRANSLATE),
    (NodeKind::FlowH3Sample, "h3_sample", "H3 Sampler", MI_AUTO_AWESOME),
    (NodeKind::FlowH3DecodeVideo, "h3_decode_video", "H3 Video Decoder", MI_MOVIE),
    (NodeKind::FlowH3DecodeAudio, "h3_decode_audio", "H3 Audio Decoder", MI_GRAPHIC_EQ),
    (NodeKind::FlowLtxModel, "ltx_model", "LTX Model", MI_INVENTORY_2),
    (NodeKind::FlowLtxEncode, "ltx_encode", "LTX Encoder", MI_TRANSLATE),
    (NodeKind::FlowLtxNag, "ltx_nag", "LTX NAG", MI_TUNE),
    (NodeKind::FlowLtxImage, "ltx_image", "LTX Frame", MI_IMAGE_ICON),
    (NodeKind::FlowLtxStage1, "ltx_stage1", "LTX Stage1", MI_AUTO_AWESOME),
    (NodeKind::FlowLtxUpscale, "ltx_upscale", "LTX Upscale ×2", MI_ASPECT_RATIO),
    (NodeKind::FlowLtxStage2, "ltx_stage2", "LTX Stage2", MI_AUTO_AWESOME),
    (NodeKind::FlowLtxRetake, "ltx_retake", "LTX Retake", MI_MOVIE),
    (NodeKind::FlowLtxIcLora, "ltx_ic_lora", "LTX IC-LoRA", MI_MOVIE),
    (NodeKind::FlowLtxLipdub, "ltx_lipdub", "LTX LipDub", MI_RECORD_VOICE_OVER),
    (NodeKind::FlowLtxA2v, "ltx_a2v", "LTX Audio to Video", MI_AUDIOTRACK),
    (NodeKind::FlowLtxDecodeVideo, "ltx_decode_video", "LTX Video Decoder", MI_MOVIE),
    (NodeKind::FlowLtxDecodeAudio, "ltx_decode_audio", "LTX Audio Decoder", MI_GRAPHIC_EQ),
    (NodeKind::FlowAcestepModel, "acestep_model", "ACE-Step Model", MI_INVENTORY_2),
    (NodeKind::FlowAcestepEncode, "acestep_encode", "ACE-Step Audio to Latent", MI_GRAPHIC_EQ),
    (NodeKind::FlowAcestepGenerate, "acestep_generate", "ACE-Step Generate", MI_LIBRARY_MUSIC),
    (NodeKind::FlowYue2Model, "yue2_model", "YuE2 Model", MI_INVENTORY_2),
    (NodeKind::FlowYue2Generate, "yue2_generate", "YuE2 Song", MI_LIBRARY_MUSIC),
    (NodeKind::FlowYue2Decode, "yue2_decode", "YuE2 Decoder", MI_GRAPHIC_EQ),
    (NodeKind::FlowYue2Transcribe, "yue2_transcribe", "Audio to Score", MI_EDIT_NOTE),
    (NodeKind::FlowOmnivoice, "omnivoice", "OmniVoice", MI_RECORD_VOICE_OVER),
    (NodeKind::FlowVoxcpm, "voxcpm", "VoxCPM2", MI_RECORD_VOICE_OVER),
    (NodeKind::FlowVibevoice, "vibevoice", "VibeVoice", MI_GROUPS),
    (NodeKind::FlowDiarize, "diarize", "Who Speaks", MI_GRAPHIC_EQ),
];

/// Подкатегория палитры по разделу ядра (ключ `node.subcategory.*`).
fn subcategory(spec: &sf::NodeSpec) -> Option<&'static str> {
    match spec.category {
        "Модели картинок" => Some("flow_image"),
        "MiniMax-H3" => Some("flow_h3"),
        "LTX-2.3" => Some("flow_ltx"),
        "Музыка" => Some("flow_music"),
        "Речь" => Some("flow_speech"),
        _ => None,
    }
}

impl NodeKind {
    /// Вид ноды ядра synaptix-flow, если это нода ядра.
    pub fn flow_kind(self) -> Option<&'static str> {
        KINDS.iter().find(|k| k.0 == self).map(|k| k.1)
    }

    pub fn from_flow_kind(kind: &str) -> Option<NodeKind> {
        KINDS.iter().find(|k| k.1 == kind).map(|k| k.0)
    }
}

fn spec(kind: NodeKind) -> Option<&'static sf::NodeSpec> {
    kind.flow_kind().and_then(sf::ops::spec)
}

fn port_kind(t: PT) -> PortKind {
    match t {
        PT::Text => PortKind::Text,
        PT::Number => PortKind::Data,
        PT::Image => PortKind::Image,
        PT::Audio => PortKind::Audio,
        PT::Video | PT::Media => PortKind::Video,
        PT::List | PT::Any | PT::Model | PT::Cond | PT::Latent => PortKind::Data,
    }
}

fn field_type(t: FT) -> FieldType {
    match t {
        FT::Text | FT::LongText | FT::File => FieldType::Text,
        FT::Number => FieldType::Float,
        FT::Bool => FieldType::Bool,
        FT::Choice(o) => FieldType::Choice(o),
    }
}

fn leak<T>(v: Vec<T>) -> &'static [T] {
    Box::leak(v.into_boxed_slice())
}

/// Метаданные всех нод ядра (строятся один раз).
pub fn metas() -> Vec<NodeKindMeta> {
    KINDS
        .iter()
        .filter_map(|&(kind, fk, title, icon)| {
            let s = sf::ops::spec(fk)?;
            let ports = |p: &'static [sf::PortSpec]| leak(p.iter().map(|p| PortSchema { name: p.name, label: p.label, kind: port_kind(p.ty) }).collect());
            let (inputs, outputs) = if kind == NodeKind::FlowSubflow {
                (PortsSpec::Dynamic { pool: ports(s.inputs), runtime: subflow_inputs }, PortsSpec::Dynamic { pool: ports(s.outputs), runtime: subflow_outputs })
            } else {
                (PortsSpec::Static(ports(s.inputs)), PortsSpec::Static(ports(s.outputs)))
            };
            Some(NodeKindMeta {
                kind,
                icon,
                title,
                category: NodeCategory::Service,
                subcategory: subcategory(s),
                inputs,
                outputs,
                fields: leak(s.fields.iter().map(|f| FieldSchema { name: f.name, label: f.label, ty: field_type(f.ty) }).collect()),
                body: Some(body),
                ports_layout: PortsLayout::Rows,
                port_row_extra: None,
                executor: &FLOW_EXEC,
                on_run: Some(on_run),
                busy_signal: Some(busy_signal),
                runtime: None,
            })
        })
        .collect()
}

/// Поля свежей ноды — с умолчаниями схемы ядра.
pub fn default_fields(kind: NodeKind) -> Arc<Mutex<HashMap<&'static str, FieldValue>>> {
    let mut map = HashMap::new();
    for f in spec(kind).map(|s| s.fields).unwrap_or(&[]) {
        let v = match f.ty {
            FT::Text | FT::LongText | FT::File => FieldValue::Text(use_signal(f.default.to_string())),
            FT::Number => FieldValue::Float(use_signal(f.default.parse::<f32>().unwrap_or(0.0))),
            FT::Bool => FieldValue::Bool(use_signal(f.default == "true")),
            FT::Choice(o) => FieldValue::Choice(use_signal(o.iter().position(|x| *x == f.default).unwrap_or(0))),
        };
        map.insert(f.name, v);
    }
    Arc::new(Mutex::new(map))
}

/// Состояние ноды ядра.
#[derive(Clone)]
pub struct FlowRuntime {
    pub running: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
    /// Что получилось (или «элемент k из n» во время пакета).
    pub status: RwSignal<String>,
    pub progress: RwSignal<f32>,
    pub cancel: Arc<AtomicBool>,
    /// Выходы последнего прогона — значения ядра.
    pub outputs: Arc<Mutex<sf::Outputs>>,
    /// Те же выходы для проводов Synthos.
    pub ports: Arc<Mutex<HashMap<String, PortValue>>>,
    pub version: RwSignal<u32>,
    /// Сид прошлого запуска при поле «Сид» = 0: прогон потомка (декодера)
    /// берёт тот же, и служба отдаёт результат сэмплера из кэша.
    pub seed: Arc<Mutex<Option<u64>>>,
}

impl FlowRuntime {
    pub fn new() -> Self {
        Self {
            running: use_signal(false),
            error: use_signal(None),
            status: use_signal(String::new()),
            progress: use_signal(0.0),
            cancel: Arc::new(AtomicBool::new(false)),
            outputs: Arc::new(Mutex::new(BTreeMap::new())),
            ports: Arc::new(Mutex::new(HashMap::new())),
            version: use_signal(0),
            seed: Arc::new(Mutex::new(None)),
        }
    }
}

fn runtime(node: &NodeInstance) -> Option<FlowRuntime> {
    match &*node.runtime.lock().ok()? {
        NodeRuntime::Flow(f) => Some(f.clone()),
        _ => None,
    }
}

pub struct FlowExec;
pub static FLOW_EXEC: FlowExec = FlowExec;

impl NodeExecutor for FlowExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let Some(rt) = runtime(ctx.node) else { return };
        if ctx.track {
            rt.version.get();
        }
        let ports = rt.ports.lock().map(|p| p.clone()).unwrap_or_default();
        let Some(s) = spec(ctx.node.kind) else { return };
        for p in s.outputs {
            ctx.write_output(p.name, ports.get(p.name).cloned().unwrap_or(PortValue::Empty));
        }
    }
}

pub fn busy_signal(node: &NodeInstance) -> Option<RwSignal<bool>> {
    runtime(node).map(|r| r.running)
}

/// Поля ноды → поля ядра (выбор — строкой варианта, 0 в числе — «не задано»).
fn fields_json(node: &NodeInstance) -> BTreeMap<String, Json> {
    let mut out = BTreeMap::new();
    let Some(s) = spec(node.kind) else { return out };
    let map = node.fields.lock().unwrap_or_else(|e| e.into_inner());
    for f in s.fields {
        let v = match (map.get(f.name), f.ty) {
            (Some(FieldValue::Text(sig)), _) => Json::String(sig.get_untracked()),
            (Some(FieldValue::Float(sig)), _) => serde_json::Number::from_f64(sig.get_untracked() as f64).map(Json::Number).unwrap_or(Json::Null),
            (Some(FieldValue::Bool(sig)), _) => Json::Bool(sig.get_untracked()),
            (Some(FieldValue::Choice(sig)), FT::Choice(o)) => Json::String(o.get(sig.get_untracked()).copied().unwrap_or("").to_string()),
            _ => continue,
        };
        out.insert(f.name.to_string(), v);
    }
    out
}

/// Каталог файлов прогонов нод ядра.
fn runs_dir() -> PathBuf {
    let base = std::env::var("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cache"));
    base.join(crate::host::app_name()).join("flow")
}

/// Значение провода Synthos → значение ядра (картинка и звук — во временные файлы).
fn to_val(pv: &PortValue, dir: &Path, name: &str) -> Val {
    match pv {
        PortValue::Empty => Val::Empty,
        PortValue::Text(s) => Val::Text(s.clone()),
        PortValue::Float(f) => Val::Number(*f as f64),
        PortValue::Audio(buf) => {
            let p = dir.join(format!("{name}.wav"));
            match super::common::write_wav(&p, buf) {
                Ok(()) => Val::File(p),
                Err(_) => Val::Empty,
            }
        }
        PortValue::Data(blob) => match &**blob {
            DataBlob::Flow(v) => v.clone(),
            DataBlob::Image(img) => match &img.source {
                Some(src) => Val::File(src.clone()),
                None => {
                    let p = dir.join(format!("{name}.png"));
                    match super::image::write_image(img, &p) {
                        Ok(()) => Val::File(p),
                        Err(_) => Val::Empty,
                    }
                }
            },
            _ => Val::Empty,
        },
        _ => Val::Empty,
    }
}

/// Значение ядра → провод Synthos: одиночная картинка — `ImageData`, звук — `AudioBuffer`.
fn to_port(v: &Val, ty: PT) -> PortValue {
    match (ty, v) {
        (_, Val::Empty) => PortValue::Empty,
        (PT::Text, v) => PortValue::Text(v.text()),
        (PT::Number, v) => v.number().map(|n| PortValue::Float(n as f32)).unwrap_or(PortValue::Empty),
        (PT::Image, Val::File(p)) => match ImageData::load(p) {
            Ok(img) => PortValue::Data(Arc::new(DataBlob::Image(Arc::new(img)))),
            Err(_) => PortValue::Data(Arc::new(DataBlob::Flow(v.clone()))),
        },
        (PT::Audio, Val::File(p)) => match super::decode::decode_file(p) {
            Ok(buf) => PortValue::Audio(Arc::new(buf)),
            Err(_) => PortValue::Data(Arc::new(DataBlob::Flow(v.clone()))),
        },
        (_, v) => PortValue::Data(Arc::new(DataBlob::Flow(v.clone()))),
    }
}

/// Подграф запуска ноды: она сама и ноды ядра, от которых к ней (по цепочке)
/// идут дескрипторы — модель, условие, латент существуют только в службе.
/// Прочие входы — готовые значения: с нод ядра — последние выходы, с родных
/// нод — через файлы.
fn subflow(node: &NodeInstance, ctx: &NodeEditorCtx, dir: &Path) -> (sf::Flow, HashMap<(u64, String), Val>, HashMap<u64, FlowRuntime>) {
    let nodes = ctx.nodes.get_untracked();
    let conns = ctx.connections.get_untracked();
    let values = ctx.values.get_untracked();
    let by_id: HashMap<NodeId, &NodeInstance> = nodes.iter().map(|n| (n.id, n)).collect();
    let mut included = vec![node.id];
    let mut i = 0;
    while i < included.len() {
        let id = included[i];
        i += 1;
        for c in conns.iter().filter(|c| c.to_node == id) {
            let Some(src) = by_id.get(&c.from_node) else { continue };
            let handle = spec(src.kind).and_then(|s| s.output(c.from_port)).is_some_and(|p| p.ty.is_handle());
            if handle && !included.contains(&src.id) {
                included.push(src.id);
            }
        }
    }
    let mut flow = sf::Flow::default();
    let mut rts = HashMap::new();
    for id in &included {
        let n = by_id[id];
        let (Some(fk), Some(rt)) = (n.kind.flow_kind(), runtime(n)) else { continue };
        let mut f = sf::FlowNode::new(id.0, fk);
        f.fields = fields_json(n);
        // Сид 0 — случайный: свой запуск — новый, у предков — прошлый.
        if f.fields.get("seed").and_then(Json::as_f64) == Some(0.0) {
            let mut last = rt.seed.lock().unwrap_or_else(|e| e.into_inner());
            let seed = match *last {
                Some(s) if *id != node.id => s,
                _ => random_seed(id.0),
            };
            *last = Some(seed);
            f.fields.insert("seed".into(), Json::from(seed));
        }
        flow.nodes.push(f);
        rts.insert(id.0, rt);
    }
    let mut inputs = HashMap::new();
    for c in conns.iter().filter(|c| rts.contains_key(&c.to_node.0)) {
        if rts.contains_key(&c.from_node.0) {
            flow.links.push(sf::Link { from_node: c.from_node.0, from_port: c.from_port.into(), to_node: c.to_node.0, to_port: c.to_port.into() });
            continue;
        }
        let src = by_id.get(&c.from_node);
        let from_flow = src.and_then(|n| runtime(n)).and_then(|r| r.outputs.lock().ok().and_then(|o| o.get(c.from_port).cloned()));
        let v = match from_flow {
            Some(v) => v,
            None => values.get(&(c.from_node, c.from_port)).map(|pv| to_val(pv, dir, &format!("{}-{}", c.to_node.0, c.to_port))).unwrap_or_default(),
        };
        inputs.insert((c.to_node.0, c.to_port.to_string()), v);
    }
    flow.outputs = vec![node.id.0];
    (flow, inputs, rts)
}

fn random_seed(salt: u64) -> u64 {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
    (nanos ^ salt.wrapping_mul(0x9e37_79b9_7f4a_7c15)) % 1_000_000_007 + 1
}

/// Выходы ноды ядра → её состояние и провода Synthos.
fn publish(rt: &FlowRuntime, fk: &str, outs: sf::Outputs, summary: String) {
    let Some(spec) = sf::ops::spec(fk) else { return };
    let ports: HashMap<String, PortValue> = spec.outputs.iter().map(|p| (p.name.to_string(), outs.get(p.name).map(|v| to_port(v, p.ty)).unwrap_or_default())).collect();
    if let Ok(mut o) = rt.outputs.lock() {
        *o = outs;
    }
    if let Ok(mut p) = rt.ports.lock() {
        *p = ports;
    }
    rt.error.set(None);
    rt.status.set(summary);
    // update — только в главном потоке (set годится из любого).
    let version = rt.version;
    syngui::async_runtime::run_on_main_thread(move || version.update(|v| *v = v.wrapping_add(1)));
}

pub fn on_run(node: &NodeInstance, ctx: &NodeEditorCtx) {
    let Some(rt) = runtime(node) else { return };
    let Some(fk) = node.kind.flow_kind() else { return };
    if rt.running.get_untracked() {
        return;
    }
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let dir = runs_dir().join(format!("{fk}-{}-{stamp}", node.id.0));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        rt.error.set(Some(format!("{}: {e}", dir.display())));
        return;
    }
    let (flow, inputs, rts) = subflow(node, ctx, &dir);
    let kinds: HashMap<u64, String> = flow.nodes.iter().map(|n| (n.id, n.kind.clone())).collect();
    let target = node.id.0;
    rt.cancel.store(false, Ordering::SeqCst);
    rt.running.set(true);
    rt.error.set(None);
    rt.progress.set(0.0);
    rt.status.set(String::new());
    let _ = std::thread::Builder::new().name(format!("node-flow-{fk}")).spawn(move || {
        let started = log_worker_start("flow", fk);
        let cancel = sf::Cancel::new();
        // Флаг Stop из run_controls → отмена прогона (рвёт запрос к службе).
        let done = Arc::new(AtomicBool::new(false));
        {
            let (flag, done, cancel) = (rt.cancel.clone(), done.clone(), cancel.clone());
            std::thread::spawn(move || {
                while !done.load(Ordering::SeqCst) {
                    if flag.load(Ordering::SeqCst) {
                        cancel.cancel();
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(150));
                }
            });
        }
        let opts = sf::RunOpts { cancel, inputs, ..Default::default() };
        // В метриках службы прогон виден как «Synthos · нодовый редактор».
        let label = tr!("service.label.node_editor");
        let res = synaptixd::with_label(&label, || synaptixd::flow::run_with(&flow, &dir, &opts, &mut |e| match e {
            sf::Event::Started { id, item, .. } => {
                if let Some(r) = rts.get(&id) {
                    r.progress.set(0.0);
                    r.status.set(match item {
                        Some((k, n)) => format!("{k} / {n}"),
                        None if id == target => String::new(),
                        None => tr!("nodes.flow.running"),
                    });
                }
            }
            sf::Event::Progress { id, step, total } if total > 0 => {
                if let Some(r) = rts.get(&id) {
                    r.progress.set(step as f32 / total as f32);
                    if id != target {
                        r.status.set(tr!("nodes.flow.step", step = step, total = total));
                    }
                }
            }
            sf::Event::Finished { id, outputs, summary, .. } => {
                if let (Some(r), Some(k)) = (rts.get(&id), kinds.get(&id)) {
                    publish(r, k, outputs, summary);
                }
            }
            _ => {}
        }));
        done.store(true, Ordering::SeqCst);
        log_worker_done("flow", started, &res);
        if let Err(e) = res {
            rt.error.set(Some(e));
        }
        rt.running.set(false);
    });
}

// ── Тело карточки ─────────────────────────────────────────────────────────

/// Вид моделей службы для поля «Модель» ноды (и фильтр по id).
fn model_kind(fk: &str) -> Option<(synaptixd::ModelKind, Option<&'static str>)> {
    use synaptixd::ModelKind as K;
    Some(match fk {
        "image" | "image_model" => (K::Image, None),
        "video" => (K::Video, None),
        "h3_model" => (K::Video, Some("h3")),
        "ltx_model" => (K::Video, Some("ltx")),
        "music" => (K::Music, None),
        "acestep_model" => (K::Music, Some("acestep")),
        "yue2_model" => (K::Music, Some("yue")),
        "llm" => (K::Llm, None),
        "speak" => (K::Tts, None),
        "omnivoice" => (K::Tts, Some("omnivoice")),
        "voxcpm" => (K::Tts, Some("voxcpm")),
        "vibevoice" => (K::Tts, Some("vibevoice")),
        "transcribe" => (K::Asr, None),
        "diarize" => (K::Diarize, None),
        _ => return None,
    })
}

/// Модели службы (список кэшируется на минуту: карточки строятся часто).
fn service_models(kind: synaptixd::ModelKind, filter: Option<&str>) -> Vec<String> {
    static CACHE: std::sync::OnceLock<Mutex<Option<(std::time::Instant, Vec<synaptixd::ModelEntry>)>>> = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    let mut g = cache.lock().unwrap_or_else(|e| e.into_inner());
    let fresh = g.as_ref().is_some_and(|(t, _)| t.elapsed() < std::time::Duration::from_secs(60));
    if !fresh {
        let list = synaptixd::Client::connect().and_then(|mut c| c.list()).unwrap_or_default();
        *g = Some((std::time::Instant::now(), list));
    }
    let mut ids: Vec<String> = g.as_ref().map(|(_, l)| l.iter().filter(|m| m.kind == kind && filter.is_none_or(|f| m.id.contains(f))).map(|m| m.id.clone()).collect()).unwrap_or_default();
    ids.sort();
    ids
}

/// Поле «Модель»: модели службы нужного вида; пусто — модель по умолчанию;
/// путь или чужой id из шаблона остаётся вариантом списка.
fn model_control(kind: synaptixd::ModelKind, filter: Option<&'static str>, sig: RwSignal<String>) -> Box<dyn Widget> {
    let auto = tr!("nodes.flow.model_default");
    let current = sig.get_untracked();
    let mut ids = service_models(kind, filter);
    if !current.is_empty() && !ids.contains(&current) {
        ids.insert(0, current.clone());
    }
    let mut items = vec![DropdownItem::simple(auto.clone())];
    items.extend(ids.iter().map(|i| DropdownItem::simple(i.clone())));
    Box::new(
        Dropdown::with_items(items)
            .selected(if current.is_empty() { auto.clone() } else { current })
            .on_change(move |val| sig.set(if val == auto { String::new() } else { val.to_string() }))
            .class("node-input-dropdown flow-node-dropdown"),
    )
}

/// Путь к файлу: поле и кнопка выбора.
fn file_control(sig: RwSignal<String>) -> Box<dyn Widget> {
    let field = TextField::new().text(sig.get_untracked()).placeholder(tr!("nodes.flow.path_placeholder")).on_change(move |s| sig.set(s.to_string())).class("node-input-text");
    let pick = ToolButton::new(MI_FOLDER_OPEN)
        .tooltip(tr!("nodes.flow.pick_file"))
        .on_click(move || {
            if let Some(p) = rfd::FileDialog::new().pick_file() {
                sig.set(p.display().to_string());
            }
        })
        .class("node-file-picker-btn");
    Box::new(Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).children(vec![Box::new(field) as Box<dyn Widget>, Box::new(pick)]))
}

fn field_control(fk: &str, f: &'static sf::FieldSpec, value: FieldValue) -> Box<dyn Widget> {
    if let (FieldValue::Text(sig), Some((kind, filter)), "model") = (&value, model_kind(fk), f.name) {
        return model_control(kind, filter, *sig);
    }
    if let (FT::File, FieldValue::Text(sig)) = (f.ty, &value) {
        return file_control(*sig);
    }
    match (f.ty, value) {
        (FT::LongText, FieldValue::Text(sig)) => Box::new(
            MultilineTextEdit::new()
                .text(sig.get_untracked())
                .soft_wrap(true)
                .auto_height(true)
                .on_change(move |s| sig.set(s.to_string()))
                .class("node-input-text flow-node-longtext"),
        ),
        (_, FieldValue::Text(sig)) => Box::new(
            TextField::new()
                .text(sig.get_untracked())
                .placeholder(if f.ty == FT::File { tr!("nodes.flow.path_placeholder") } else { String::new() })
                .on_change(move |s| sig.set(s.to_string()))
                .class("node-input-text"),
        ),
        (_, FieldValue::Float(sig)) => {
            // Дробные поля (дробное умолчание схемы или значение) — два знака
            // и мелкий шаг, остальные — целые. Отрицательное умолчание
            // (-1 — «до конца», «решает модель») разрешает отрицательные.
            let def: f64 = f.default.parse().unwrap_or(0.0);
            let frac = f.default.contains('.') || sig.get_untracked().fract() != 0.0;
            let (dp, step) = match f.name {
                _ if frac => (2, 0.05),
                "volume" | "strength" | "guidance" | "denoise" | "alpha" | "tau" | "canny_low" | "canny_high" | "lora_strength" => (2, 0.05),
                "seconds" | "start" | "end" => (1, 0.5),
                _ => (0, 1.0),
            };
            Box::new(
                SpinBox::new()
                    .min(if def < 0.0 { -1.0 } else { 0.0 })
                    .max(4_294_967_295.0)
                    .step(step)
                    .decimal_places(dp)
                    .value(sig.get_untracked() as f64)
                    .on_change(move |v| sig.set(v as f32))
                    .width(120.0)
                    .class("node-input-spinbox"),
            )
        }
        (_, FieldValue::Bool(sig)) => Box::new(Checkbox::checked(sig.get_untracked()).on_change(move |v| sig.set(v)).class("node-input-checkbox")),
        (FT::Choice(opts), FieldValue::Choice(sig)) => {
            let label = |o: &str| if o.is_empty() { "—".to_string() } else { o.to_string() };
            let items: Vec<DropdownItem> = opts.iter().map(|o| DropdownItem::simple(label(o))).collect();
            Box::new(
                Dropdown::with_items(items)
                    .selected(label(opts.get(sig.get_untracked()).copied().unwrap_or("")))
                    .on_change(move |val| {
                        if let Some(i) = opts.iter().position(|o| label(o) == val) {
                            sig.set(i);
                        }
                    })
                    .class("node-input-dropdown"),
            )
        }
        _ => Box::new(Text::new("?").class("node-card-field-error")),
    }
}

/// Результат: текст, превью картинки, файлы с кнопкой «открыть».
fn result_view(rt: &FlowRuntime, spec: &'static sf::NodeSpec) -> Vec<Box<dyn Widget>> {
    let mut out: Vec<Box<dyn Widget>> = Vec::new();
    let outputs = rt.outputs.lock().map(|o| o.clone()).unwrap_or_default();
    let ports = rt.ports.lock().map(|p| p.clone()).unwrap_or_default();
    for p in spec.outputs {
        let Some(v) = outputs.get(p.name) else { continue };
        // Дескриптор (модель, условие, латент) уже описан строкой статуса.
        if v.handle().is_some() {
            continue;
        }
        if let Some(PortValue::Data(blob)) = ports.get(p.name) {
            if let DataBlob::Image(img) = &**blob {
                out.push(super::image::image_preview(Some(img.clone())));
            }
        }
        let files = v.files();
        if files.is_empty() {
            let t = v.text();
            if !t.trim().is_empty() {
                let short: String = t.chars().take(600).collect();
                out.push(Box::new(Text::new(short).class("audio-node-meta flow-node-text")));
            }
        }
        for f in files.into_iter().take(8) {
            let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let open = f.clone();
            out.push(Box::new(
                Row::new()
                    .gap(6.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .children(vec![
                        Box::new(
                            ToolButton::new(MI_FOLDER_OPEN)
                                .tooltip(tr!("nodes.flow.open_file"))
                                .on_click(move || {
                                    let _ = crate::host::open_with_system(&open);
                                })
                                .class("node-file-picker-btn"),
                        ) as Box<dyn Widget>,
                        Box::new(Text::new(name).class("audio-node-meta")),
                    ]),
            ));
        }
    }
    out
}

fn field_is_modified(f: &'static sf::FieldSpec, value: &FieldValue) -> bool {
    match (f.ty, value) {
        (FT::Text | FT::LongText | FT::File, FieldValue::Text(sig)) => sig.get() != f.default,
        (FT::Number, FieldValue::Float(sig)) => {
            let def = f.default.parse::<f32>().unwrap_or(0.0);
            (sig.get() - def).abs() > 1e-4
        }
        (FT::Bool, FieldValue::Bool(sig)) => sig.get() != (f.default == "true"),
        (FT::Choice(opts), FieldValue::Choice(sig)) => {
            let def = opts.iter().position(|x| *x == f.default).unwrap_or(0);
            sig.get() != def
        }
        _ => false,
    }
}

fn label_with_tooltip(label: &str, f: &'static sf::FieldSpec) -> Box<dyn Widget> {
    let text = Text::new(label).class("node-card-field-label");
    if f.hint.is_empty() {
        Box::new(text)
    } else {
        Box::new(crate::help::with_rich_tooltip(
            text,
            crate::help::field_tooltip_content(label, &syngui::i18n::t(f.hint), f.default),
            TooltipPosition::Above,
            280.0,
        ))
    }
}

fn flow_field_row(fk: &str, f: &'static sf::FieldSpec, value: FieldValue) -> Box<dyn Widget> {
    let label = syngui::i18n::t(f.label);
    if f.ty == FT::LongText {
        let mut children: Vec<Box<dyn Widget>> = vec![label_with_tooltip(&label, f)];
        // Для «Шаблон текста» — кликабельные чипы вставки меток {{a}}…{{f}}.
        if fk == "format" && f.name == "template" {
            if let FieldValue::Text(sig) = value {
                let mut chips: Vec<Box<dyn Widget>> = Vec::new();
                for (i, key) in ["a", "b", "c", "d", "e", "f"].iter().enumerate() {
                    let token = format!("{{{{{key}}}}}");
                    let tok_click = token.clone();
                    let hint = format!("Вставить {token} — значение со входа «Вставка {}»", i + 1);
                    let chip = GestureDetector::new()
                        .cursor(syngui::input::CursorIcon::Pointer)
                        .on_click(move || {
                            sig.update(|s| {
                                if !s.is_empty() && !s.ends_with(' ') && !s.ends_with('\n') {
                                    s.push(' ');
                                }
                                s.push_str(&tok_click);
                            });
                        })
                        .child(
                            DecoratedBox::new()
                                .child(Padding::symmetric(6.0, 2.0).child(Text::new(token).class("flow-chip-text")))
                                .class("flow-placeholder-chip"),
                        );
                    chips.push(Box::new(crate::help::with_rich_tooltip(
                        chip,
                        crate::help::field_tooltip_content(&format!("Вставка {}", i + 1), &hint, ""),
                        TooltipPosition::Above,
                        250.0,
                    )));
                }
                children.push(Box::new(
                    Row::new()
                        .gap(4.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .children(chips),
                ));
            }
        }
        children.push(field_control(fk, f, value));
        return Box::new(
            Padding::symmetric(10.0, 2.0).child(
                Column::new()
                    .gap(4.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .children(children),
            ),
        );
    }

    let label_cell = DecoratedBox::new()
        .child(label_with_tooltip(&label, f))
        .class("flow-field-label-cell");
    let mut ctrl_box = DecoratedBox::new();
    ctrl_box.child = Some(field_control(fk, f, value));
    let control_cell = ctrl_box.class("flow-field-control-cell");
    Box::new(
        Padding::symmetric(10.0, 2.0).child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .children(vec![Box::new(label_cell) as Box<dyn Widget>, Box::new(control_cell)]),
        ),
    )
}

fn render_field(node_id: NodeId, spec: &'static sf::NodeSpec, f: &'static sf::FieldSpec, value: FieldValue) -> Box<dyn Widget> {
    // Если у поля есть одноимённый входной порт и к нему подключён провод —
    // показываем компактную плашку вместо пустого поля ввода.
    if spec.input(f.name).is_some() {
        let port_name = f.name;
        let label = syngui::i18n::t(f.label);
        return Box::new(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let connected = use_context::<NodeEditorCtx>()
                .connections
                .get()
                .iter()
                .any(|c| c.to_node == node_id && c.to_port == port_name);
            if connected {
                vec![Box::new(
                    Padding::symmetric(10.0, 2.0).child(
                        DecoratedBox::new()
                            .child(
                                Padding::symmetric(8.0, 4.0).child(
                                    Text::new(format!("← {label}: берётся со входа")).class("flow-wired-pill-text"),
                                ),
                            )
                            .class("flow-wired-pill"),
                    ),
                )]
            } else {
                vec![flow_field_row(spec.kind, f, value.clone())]
            }
        }));
    }
    flow_field_row(spec.kind, f, value)
}

fn utility_banner(fk: &str) -> Option<&'static str> {
    match fk {
        "concat" => Some("Подключите «Список клипов» или отдельные «Клип 1…4» по порядку."),
        "lines" => Some("Каждая непустая строка становится отдельным запуском следующей ноды."),
        _ => None,
    }
}

fn is_wide_flow(fk: &str) -> bool {
    matches!(
        fk,
        "llm" | "video" | "image" | "music" | "speak" | "omnivoice" | "voxcpm" | "vibevoice" | "h3_encode" | "ltx_encode"
    )
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let (Some(rt), Some(spec)) = (runtime(node), spec(node.kind)) else {
        return Box::new(Column::new());
    };
    let node_id = node.id;
    let mut col = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    if let Some(banner) = utility_banner(spec.kind) {
        col = col.children(vec![Box::new(
            Padding::symmetric(10.0, 2.0).child(
                DecoratedBox::new()
                    .child(Padding::symmetric(8.0, 6.0).child(Text::new(banner).class("flow-banner-text")))
                    .class("flow-banner"),
            ),
        ) as Box<dyn Widget>]);
    }
    let map = node.fields.lock().map(|m| m.clone()).unwrap_or_default();
    let mut adv_fields: Vec<(&'static sf::FieldSpec, FieldValue)> = Vec::new();
    for f in spec.fields {
        let Some(v) = map.get(f.name).cloned() else { continue };
        if f.hidden {
            continue;
        }
        if f.advanced {
            adv_fields.push((f, v));
        } else {
            col = col.children(vec![render_field(node_id, spec, f, v)]);
        }
    }
    if !adv_fields.is_empty() {
        let expanded = use_signal(false);
        let adv_for_view = adv_fields.clone();
        col = col.children(vec![Box::new(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let is_open = expanded.get();
            let modified = adv_for_view.iter().filter(|(f, v)| field_is_modified(f, v)).count();
            let chevron = if is_open { MI_EXPAND_MORE } else { MI_CHEVRON_RIGHT };
            let mut header_children: Vec<Box<dyn Widget>> = vec![
                Box::new(Text::new(chevron).class("flow-adv-chevron")),
                Box::new(Text::new("Дополнительно").class("flow-adv-title")),
            ];
            if modified > 0 {
                header_children.push(Box::new(
                    DecoratedBox::new()
                        .child(Padding::symmetric(6.0, 1.0).child(Text::new(format!("{modified} изм.")).class("flow-adv-badge-text")))
                        .class("flow-adv-badge"),
                ));
            }
            let toggle = GestureDetector::new()
                .cursor(syngui::input::CursorIcon::Pointer)
                .on_click(move || expanded.set(!expanded.get_untracked()))
                .child(
                    DecoratedBox::new()
                        .child(
                            Padding::symmetric(8.0, 4.0).child(
                                Row::new()
                                    .gap(6.0)
                                    .cross_axis_alignment(CrossAxisAlignment::Center)
                                    .children(header_children),
                            ),
                        )
                        .class("flow-adv-toggle"),
                );
            let mut out: Vec<Box<dyn Widget>> = vec![Box::new(Padding::symmetric(10.0, 2.0).child(toggle))];
            if is_open {
                let mut adv_col = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
                for (f, v) in &adv_for_view {
                    adv_col = adv_col.children(vec![render_field(node_id, spec, f, v.clone())]);
                }
                out.push(Box::new(
                    Padding::only(6.0, 2.0, 6.0, 4.0).child(
                        DecoratedBox::new()
                            .child(Padding::symmetric(0.0, 4.0).child(adv_col))
                            .class("flow-adv-panel"),
                    ),
                ));
            }
            out
        })) as Box<dyn Widget>]);
    }
    let (running, error, status, progress, version) = (rt.running, rt.error, rt.status, rt.progress, rt.version);
    col = col.children(vec![super::common::progress_row(running, progress)]);
    let rt2 = rt.clone();
    col = col.children(vec![Box::new(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        version.get();
        let mut v: Vec<Box<dyn Widget>> = Vec::new();
        if let Some(e) = error.get() {
            v.push(Box::new(Padding::symmetric(10.0, 2.0).child(Text::new(tr!("nodes.common.error", error = e)).class("audio-node-error"))));
        }
        let s = status.get();
        if running.get() {
            v.push(Box::new(Padding::symmetric(10.0, 2.0).child(Text::new(if s.is_empty() { tr!("nodes.flow.running") } else { s }).class("audio-node-meta"))));
            return v;
        }
        if !s.is_empty() {
            v.push(Box::new(Padding::symmetric(10.0, 2.0).child(Text::new(s).class("audio-node-meta"))));
        }
        v.extend(result_view(&rt2, spec));
        v
    })) as Box<dyn Widget>]);
    let host_class = if is_wide_flow(spec.kind) { "flow-node-host-wide" } else { "flow-node-host" };
    Box::new(DecoratedBox::new().child(col).class(host_class))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PortSide;
    use crate::convert::{resolve_field_name, resolve_port_name};
    use crate::model::Template;

    /// Холст: «шаблон → строки» в подграф и обратно — поля, провода, внешний вход на месте.
    #[test]
    fn canvas_group_ungroup() {
        let ctx = NodeEditorCtx::new();
        let t = ctx.add_node(NodeKind::FlowText, Point::new(0.0, 0.0));
        let f = ctx.add_node(NodeKind::FlowFormat, Point::new(300.0, 0.0));
        let l = ctx.add_node(NodeKind::FlowLines, Point::new(600.0, 0.0));
        let nodes = ctx.nodes.get_untracked();
        let fi = nodes.iter().find(|n| n.id == f).unwrap();
        set_field_json(fi, "template", &Json::from("{{a}} и кот"));
        ctx.connections.set(vec![
            Connection { from_node: t, from_port: "text", to_node: f, to_port: "a" },
            Connection { from_node: f, from_port: "text", to_node: l, to_port: "text" },
        ]);
        let before = canvas_flow(&ctx);
        let sid = group_nodes(&ctx, &[f, l]).unwrap();
        let nodes = ctx.nodes.get_untracked();
        assert!(nodes.iter().any(|n| n.id == sid && n.kind == NodeKind::FlowSubflow));
        assert!(!nodes.iter().any(|n| n.id == f || n.id == l));
        assert!(ctx.connections.get_untracked().iter().any(|c| c.from_node == t && c.to_node == sid && c.to_port == "in1"));
        let sn = ctx.nodes.get_untracked().into_iter().find(|n| n.id == sid).unwrap();
        let ins = subflow_inputs(&sn);
        assert_eq!(ins.len(), 1, "только задействованный вход");
        assert!(ins[0].label.contains(": ") && !ins[0].label.starts_with("Вход"), "{}", ins[0].label);
        assert!(subflow_outputs(&sn).iter().all(|p| p.label.contains(": ")));
        // внутренний граф — в скрытом поле, движок его читает
        let fl = canvas_flow(&ctx);
        let def = synaptix_flow::subflow::SubflowDef::of(fl.node(sid.0).unwrap()).unwrap();
        assert_eq!(def.flow.nodes.len(), 2);
        let added = ungroup_node(&ctx, sid).unwrap();
        assert_eq!(added.len(), 2);
        let after = canvas_flow(&ctx);
        let tmpl = |fl: &synaptix_flow::Flow| fl.nodes.iter().find(|n| n.kind == "format").and_then(|n| n.fields.get("template").cloned());
        assert_eq!(tmpl(&after), tmpl(&before));
        assert_eq!(after.links.len(), before.links.len());
        assert!(after.links.iter().any(|l| l.from_node == t.0 && l.to_port == "a"));
    }

    /// Шаблон в форме экспорта synaptix-graph: виды, поля и порты резолвятся.
    #[test]
    fn exported_template_resolves() {
        let t: Template = serde_json::from_value(serde_json::json!({
            "name": "t", "kind": "full",
            "nodes": [
                {"id": 1, "kind": "flow_file", "pos": {"x": 0, "y": 0}, "fields": {"path": {"type": "Text", "value": "/tmp/a.png"}}, "state": null},
                {"id": 2, "kind": "flow_video", "pos": {"x": 0, "y": 0}, "fields": {"seconds": {"type": "Float", "value": 3.0}, "control": {"type": "Choice", "value": 2}, "two_stage": {"type": "Bool", "value": true}}, "state": null},
                {"id": 3, "kind": "flow_mux", "pos": {"x": 0, "y": 0}, "fields": {}, "state": null}
            ],
            "connections": [
                {"from_node": 1, "from_port": "file", "to_node": 2, "to_port": "first_frame"},
                {"from_node": 2, "from_port": "video", "to_node": 3, "to_port": "video"}
            ]
        }))
        .unwrap();
        assert_eq!(t.nodes[1].kind, NodeKind::FlowVideo);
        for n in &t.nodes {
            for f in n.fields.keys() {
                assert!(resolve_field_name(n.kind, f).is_some(), "{:?}.{f}", n.kind);
            }
        }
        for c in &t.connections {
            assert!(resolve_port_name(t.kind_of(c.from_node), PortSide::Output, &c.from_port).is_some(), "{}", c.from_port);
            assert!(resolve_port_name(t.kind_of(c.to_node), PortSide::Input, &c.to_port).is_some(), "{}", c.to_port);
        }
        // Все виды ядра есть в Synthos и наоборот.
        for s in synaptix_flow::ops::specs() {
            assert!(NodeKind::from_flow_kind(s.kind).is_some(), "{}", s.kind);
        }
        assert_eq!(metas().len(), synaptix_flow::ops::specs().count());
    }
}

// ── Подграф одной нодой ───────────────────────────────────────────────────

use crate::types::Connection;

/// Имя порта вида ноды как `&'static str` (для проводов холста).
fn static_port(kind: NodeKind, input: bool, name: &str) -> Option<&'static str> {
    let s = spec(kind)?;
    let list = if input { s.inputs } else { s.outputs };
    list.iter().find(|p| p.name == name).map(|p| p.name)
}

/// Значение поля ядра (JSON) → поле карточки.
fn set_field_json(inst: &NodeInstance, name: &str, v: &Json) {
    let Some(s) = spec(inst.kind) else { return };
    let Some(f) = s.fields.iter().find(|f| f.name == name) else { return };
    let map = inst.fields.lock().unwrap_or_else(|e| e.into_inner());
    match (map.get(f.name), f.ty) {
        (Some(FieldValue::Text(sig)), _) => sig.set(match v {
            Json::String(s) => s.clone(),
            Json::Null => String::new(),
            o => o.to_string(),
        }),
        (Some(FieldValue::Float(sig)), _) => sig.set(v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok())).unwrap_or(0.0) as f32),
        (Some(FieldValue::Bool(sig)), _) => sig.set(v.as_bool().unwrap_or(v.as_str() == Some("true"))),
        (Some(FieldValue::Choice(sig)), FT::Choice(o)) => {
            let s = v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
            if let Some(i) = o.iter().position(|x| *x == s) {
                sig.set(i);
            }
        }
        _ => {}
    }
}

/// Граф ядра со всех нод ядра холста (id холста = id графа).
fn canvas_flow(ctx: &NodeEditorCtx) -> sf::Flow {
    let nodes = ctx.nodes.get_untracked();
    let conns = ctx.connections.get_untracked();
    let is_flow = |id: NodeId| nodes.iter().any(|n| n.id == id && n.kind.flow_kind().is_some());
    let mut flow = sf::Flow::default();
    for n in nodes.iter().filter(|n| n.kind.flow_kind().is_some()) {
        let mut f = sf::FlowNode::new(n.id.0, n.kind.flow_kind().unwrap());
        f.fields = fields_json(n);
        f.enabled = n.enabled.get_untracked();
        flow.nodes.push(f);
    }
    for c in conns.iter().filter(|c| is_flow(c.from_node) && is_flow(c.to_node)) {
        flow.links.push(sf::Link { from_node: c.from_node.0, from_port: c.from_port.into(), to_node: c.to_node.0, to_port: c.to_port.into() });
    }
    flow
}

/// Можно ли свернуть выделенные ноды в подграф (все — ноды ядра).
pub fn can_group(ctx: &NodeEditorCtx, ids: &[NodeId]) -> bool {
    let nodes = ctx.nodes.get_untracked();
    !ids.is_empty() && ids.iter().all(|id| nodes.iter().any(|n| n.id == *id && n.kind.flow_kind().is_some()))
}

/// Свернуть выделенные ноды ядра в ноду-подграф на холсте.
pub fn group_nodes(ctx: &NodeEditorCtx, ids: &[NodeId]) -> std::result::Result<NodeId, String> {
    if !can_group(ctx, ids) {
        return Err("в подграф объединяются только ноды ядра (генерация, данные, монтаж)".into());
    }
    let nodes = ctx.nodes.get_untracked();
    let conns = ctx.connections.get_untracked();
    // провод к ноде не из ядра в подграф не перенести
    if conns.iter().any(|c| (ids.contains(&c.from_node) || ids.contains(&c.to_node)) && !(can_group(ctx, &[c.from_node]) && can_group(ctx, &[c.to_node]))) {
        return Err("выделенная нода связана с нодой не из ядра — отключите этот провод или выделите иначе".into());
    }
    let mut flow = canvas_flow(ctx);
    let n = ctx.picked.get_untracked().len().max(ids.len());
    let word = match (n % 10, n % 100) {
        (1, x) if x != 11 => "нода",
        (2..=4, x) if !(12..=14).contains(&x) => "ноды",
        _ => "нод",
    };
    let title = format!("Подграф ({n} {word})");
    let raw: Vec<u64> = ids.iter().map(|i| i.0).collect();
    let sid = flow.group(&raw, &title)?;
    let sn = flow.node(sid).cloned().ok_or("подграф не создан")?;
    let mut def = sf::subflow::SubflowDef::of(&sn)?;
    let mut cx = 0.0f32;
    let mut cy = 0.0f32;
    for n in nodes.iter().filter(|n| ids.contains(&n.id)) {
        let p = n.pos.get_untracked();
        def.layout.insert(n.id.0, (p.x, p.y));
        cx += p.x;
        cy += p.y;
    }
    cx /= ids.len() as f32;
    cy /= ids.len() as f32;
    let new_id = NodeId(ctx.next_id.get_untracked());
    ctx.next_id.set(new_id.0 + 1);
    let inst = NodeInstance {
        id: new_id,
        kind: NodeKind::FlowSubflow,
        pos: use_signal(Point::new(cx, cy)),
        bounds: use_signal(syngui::core::Rect::zero()),
        fields: default_fields(NodeKind::FlowSubflow),
        runtime: crate::registry::default_runtime(NodeKind::FlowSubflow),
        style: use_signal(Default::default()),
        enabled: use_signal(true),
        timing: crate::timing::Stopwatch::new(),
        title: use_signal(String::new()),
    };
    set_field_json(&inst, "title", &Json::from(title));
    set_field_json(&inst, "subflow", &Json::from(serde_json::to_string(&def).map_err(|e| e.to_string())?));
    let cid = |x: u64| if x == sid { new_id } else { NodeId(x) };
    let mut new_conns: Vec<Connection> = conns.iter().filter(|c| !ids.contains(&c.from_node) && !ids.contains(&c.to_node)).cloned().collect();
    for l in flow.links.iter().filter(|l| l.from_node == sid || l.to_node == sid) {
        let (fk, tk) = (if l.from_node == sid { NodeKind::FlowSubflow } else { nodes.iter().find(|n| n.id.0 == l.from_node).map(|n| n.kind).unwrap_or(NodeKind::FlowSubflow) }, if l.to_node == sid { NodeKind::FlowSubflow } else { nodes.iter().find(|n| n.id.0 == l.to_node).map(|n| n.kind).unwrap_or(NodeKind::FlowSubflow) });
        let (Some(fp), Some(tp)) = (static_port(fk, false, &l.from_port), static_port(tk, true, &l.to_port)) else { continue };
        new_conns.push(Connection { from_node: cid(l.from_node), from_port: fp, to_node: cid(l.to_node), to_port: tp });
    }
    let mut v: Vec<NodeInstance> = nodes.into_iter().filter(|n| !ids.contains(&n.id)).collect();
    v.push(inst);
    ctx.nodes.set(v);
    ctx.connections.set(new_conns);
    ctx.pick_clear();
    Ok(new_id)
}

/// Развернуть ноду-подграф на холсте обратно в её ноды.
pub fn ungroup_node(ctx: &NodeEditorCtx, id: NodeId) -> std::result::Result<Vec<NodeId>, String> {
    let nodes = ctx.nodes.get_untracked();
    let conns = ctx.connections.get_untracked();
    let sub = nodes.iter().find(|n| n.id == id && n.kind == NodeKind::FlowSubflow).cloned().ok_or("это не подграф")?;
    let mut flow = canvas_flow(ctx);
    let def = sf::subflow::SubflowDef::of(flow.node(id.0).ok_or("подграф не найден")?)?;
    let map = flow.ungroup(id.0)?;
    // id из графа ядра могут совпасть с нодами холста не из ядра — новые свободные
    let mut next = ctx.next_id.get_untracked();
    let mut remap: HashMap<u64, NodeId> = HashMap::new();
    for (_, fid) in &map {
        let taken = nodes.iter().any(|n| n.id.0 == *fid && n.id != id);
        let nid = if taken || *fid >= next { let x = NodeId(next); next += 1; x } else { NodeId(*fid) };
        remap.insert(*fid, nid);
    }
    ctx.next_id.set(next.max(remap.values().map(|n| n.0 + 1).max().unwrap_or(next)));
    let to_canvas = |x: u64| remap.get(&x).copied().unwrap_or(NodeId(x));
    // положение: как было, со сдвигом за перенесённой нодой-подграфом
    let sp = sub.pos.get_untracked();
    let (lx, ly, ln) = def.layout.values().fold((0.0f32, 0.0f32, 0usize), |(a, b, n), (x, y)| (a + x, b + y, n + 1));
    let (dx, dy) = if ln > 0 { (sp.x - lx / ln as f32, sp.y - ly / ln as f32) } else { (0.0, 0.0) };
    let mut v: Vec<NodeInstance> = nodes.iter().filter(|n| n.id != id).cloned().collect();
    let mut added = Vec::new();
    for (inner, fid) in &map {
        let Some(fnode) = flow.node(*fid) else { continue };
        let Some(kind) = NodeKind::from_flow_kind(&fnode.kind) else { continue };
        let (x, y) = def.layout.get(inner).map(|(x, y)| (x + dx, y + dy)).unwrap_or((sp.x + 40.0 * added.len() as f32, sp.y + 40.0 * added.len() as f32));
        let nid = to_canvas(*fid);
        let inst = NodeInstance {
            id: nid,
            kind,
            pos: use_signal(Point::new(x, y)),
            bounds: use_signal(syngui::core::Rect::zero()),
            fields: default_fields(kind),
            runtime: crate::registry::default_runtime(kind),
            style: use_signal(Default::default()),
            enabled: use_signal(fnode.enabled),
            timing: crate::timing::Stopwatch::new(),
            title: use_signal(String::new()),
        };
        for (k, val) in &fnode.fields {
            set_field_json(&inst, k, val);
        }
        v.push(inst);
        added.push(nid);
    }
    let kind_of = |nid: NodeId| v.iter().find(|n| n.id == nid).map(|n| n.kind);
    let mut new_conns: Vec<Connection> = conns.iter().filter(|c| c.from_node != id && c.to_node != id).cloned().collect();
    for l in &flow.links {
        let (f, t) = (to_canvas(l.from_node), to_canvas(l.to_node));
        if !added.contains(&f) && !added.contains(&t) {
            continue;
        }
        let (Some(fk), Some(tk)) = (kind_of(f), kind_of(t)) else { continue };
        let (Some(fp), Some(tp)) = (static_port(fk, false, &l.from_port), static_port(tk, true, &l.to_port)) else { continue };
        let c = Connection { from_node: f, from_port: fp, to_node: t, to_port: tp };
        if !new_conns.contains(&c) {
            new_conns.push(c);
        }
    }
    ctx.nodes.set(v);
    ctx.connections.set(new_conns);
    ctx.pick_clear();
    Ok(added)
}

// ── Порты ноды-подграфа ядра: по описанию внутреннего графа ───────────────

fn subflow_ports(node: &NodeInstance, input: bool) -> &'static [PortSchema] {
    static CACHE: std::sync::OnceLock<Mutex<HashMap<String, &'static [PortSchema]>>> = std::sync::OnceLock::new();
    let raw = match node.fields.lock().ok().and_then(|m| m.get("subflow").cloned()) {
        Some(FieldValue::Text(sig)) => sig.get_untracked(),
        _ => String::new(),
    };
    let key = format!("{}{raw}", if input { "i" } else { "o" });
    let mut c = CACHE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if let Some(v) = c.get(&key) {
        return v;
    }
    let Some(s) = spec(NodeKind::FlowSubflow) else { return &[] };
    let pool = if input { s.inputs } else { s.outputs };
    let def: Option<sf::subflow::SubflowDef> = serde_json::from_str(&raw).ok();
    let v: Vec<PortSchema> = match &def {
        // без описания — все порты (свежая нода)
        None => pool.iter().map(|p| PortSchema { name: p.name, label: p.label, kind: port_kind(p.ty) }).collect(),
        Some(d) => {
            let used: Vec<(String, String)> = if input { d.inputs.iter().map(|i| (i.port.clone(), i.label.clone())).collect() } else { d.outputs.iter().map(|o| (o.port.clone(), o.label.clone())).collect() };
            pool.iter()
                .filter_map(|p| used.iter().find(|u| u.0 == p.name).map(|u| PortSchema { name: p.name, label: Box::leak(u.1.clone().into_boxed_str()), kind: port_kind(p.ty) }))
                .collect()
        }
    };
    let v: &'static [PortSchema] = Box::leak(v.into_boxed_slice());
    c.insert(key, v);
    v
}

fn subflow_inputs(node: &NodeInstance) -> &'static [PortSchema] {
    subflow_ports(node, true)
}

fn subflow_outputs(node: &NodeInstance) -> &'static [PortSchema] {
    subflow_ports(node, false)
}
