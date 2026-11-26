//! Вид «Граф»: общий редактор узлов (`syn-node-editor`, тот же, что в Synthos)
//! с нодами synaptix-flow (LLM, генерация картинок, видео, музыки, озвучка,
//! распознавание, ffmpeg) и нодами видеоредактора, связывающими граф с
//! монтажом: «Медиатека проекта» (файлы проекта на выход) и «На таймлайн»
//! (файлы со входа — в медиатеку и на дорожку).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::OnceLock;

use serde_json::json;
use syngui::prelude::*;
use syngui::widgets::*;
use syn_node_editor::eval::{EvalContext, NodeExecutor};
use syn_node_editor::host::{EditorHost, Notice};
use syn_node_editor::registry::{self, CategoryInfo, NodeCategory, NodeKindMeta};
use syn_node_editor::types::{DataBlob, ExtKind, ExtRuntime, FieldSchema, FieldType, NodeKind, NodeRuntime, PortKind, PortSchema, PortValue, PortsLayout, PortsSpec};
use syn_node_editor::EditorWorkspace;

use super::{ed, St};

static ST: OnceLock<St> = OnceLock::new();
static WS: OnceLock<EditorWorkspace> = OnceLock::new();

/// Рабочее место графов (вне дерева виджетов — для обработчиков щелчков).
pub fn workspace() -> Option<EditorWorkspace> {
    WS.get().copied()
}

struct Host;

impl EditorHost for Host {
    fn app_name(&self) -> &'static str {
        "syn-video-editor"
    }
    fn notify(&self, kind: Notice, text: String) {
        if let Some(st) = ST.get().copied() {
            syngui::async_runtime::run_on_main_thread(move || {
                let _ = kind;
                st.toast.set(Some(text));
            });
        }
    }
}

const PROJECT_MEDIA: NodeKind = NodeKind::Ext(ExtKind("ve_project_media"));
const TO_TIMELINE: NodeKind = NodeKind::Ext(ExtKind("ve_to_timeline"));

/// Файлы со входа: значение ноды synaptix-flow (файл, список) или путь текстом.
fn files_of(v: &PortValue) -> Vec<PathBuf> {
    match v {
        PortValue::Data(b) => match b.as_ref() {
            DataBlob::Flow(val) => val.files(),
            _ => Vec::new(),
        },
        PortValue::Text(t) => t.lines().map(str::trim).filter(|l| !l.is_empty()).map(PathBuf::from).filter(|p| p.is_file()).collect(),
        _ => Vec::new(),
    }
}

struct MediaExec;
impl NodeExecutor for MediaExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let files: Vec<synaptix_flow::Val> = ed().project.media.iter().map(|m| synaptix_flow::Val::File(m.path.clone())).collect();
        ctx.write_output("media", PortValue::Data(std::sync::Arc::new(DataBlob::Flow(synaptix_flow::Val::List(files)))));
        ctx.write_output("count", PortValue::Float(ed().project.media.len() as f32));
    }
}
static MEDIA_EXEC: MediaExec = MediaExec;

/// Уже поставленные файлы: пересчёт графа не ставит их снова.
struct TimelineRt {
    done: std::sync::Mutex<HashSet<PathBuf>>,
}
impl ExtRuntime for TimelineRt {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn save_state(&self) -> Option<serde_json::Value> {
        None
    }
    fn load_state(&self, _v: &serde_json::Value) {}
}

struct TimelineExec;
impl NodeExecutor for TimelineExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let files = files_of(&ctx.read_input("media"));
        let fresh: Vec<PathBuf> = {
            let rt = ctx.runtime().lock();
            match rt.ok().and_then(|r| r.ext::<TimelineRt>().map(|t| {
                let mut d = t.done.lock().unwrap();
                files.iter().filter(|f| d.insert((*f).clone())).cloned().collect::<Vec<_>>()
            })) {
                Some(v) => v,
                None => Vec::new(),
            }
        };
        ctx.write_output("count", PortValue::Float(files.len() as f32));
        if fresh.is_empty() {
            return;
        }
        if let Some(st) = ST.get().copied() {
            syngui::async_runtime::run_on_main_thread(move || {
                let paths: Vec<String> = fresh.iter().map(|p| p.display().to_string()).collect();
                if let Some(out) = st.exec("media.import", json!({"paths": paths})) {
                    for id in out.data["ids"].as_array().cloned().unwrap_or_default() {
                        st.exec("clip.add", json!({"media": id}));
                    }
                }
            });
        }
    }
}
static TIMELINE_EXEC: TimelineExec = TimelineExec;

const MEDIA_OUT: &[PortSchema] = &[PortSchema { name: "media", label: "файлы", kind: PortKind::Data }, PortSchema { name: "count", label: "сколько", kind: PortKind::Data }];
const TL_IN: &[PortSchema] = &[PortSchema { name: "media", label: "файлы", kind: PortKind::Data }];
const TL_OUT: &[PortSchema] = &[PortSchema { name: "count", label: "поставлено", kind: PortKind::Data }];

/// Хост, строки и ноды видеоредактора — до создания окна.
pub fn init() {
    syn_node_editor::host::set_host(Host);
    super::framegraph_ui::register();
    register_jobs();
    syngui::i18n::set_source_language(syngui::i18n::Lang::new("ru"));
    syngui::i18n::register_catalogs(&syn_node_editor::i18n::CATALOGS);
    syngui::i18n::register_catalogs(&synaptix_flow::CATALOGS);
    registry::register_category(CategoryInfo { key: "video_editor", label: "Видеоредактор", icon: "\u{E02C}" });
    registry::register_kind(NodeKindMeta {
        kind: PROJECT_MEDIA,
        icon: "\u{E02C}",
        title: "Медиатека проекта",
        category: NodeCategory::Custom("video_editor"),
        subcategory: None,
        inputs: PortsSpec::Static(&[]),
        outputs: PortsSpec::Static(MEDIA_OUT),
        fields: &[],
        body: None,
        ports_layout: PortsLayout::Rows,
        port_row_extra: None,
        executor: &MEDIA_EXEC,
        on_run: None,
        busy_signal: None,
        runtime: None,
    });
    registry::register_kind(NodeKindMeta {
        kind: TO_TIMELINE,
        icon: "\u{E8EF}",
        title: "На таймлайн",
        category: NodeCategory::Custom("video_editor"),
        subcategory: None,
        inputs: PortsSpec::Static(TL_IN),
        outputs: PortsSpec::Static(TL_OUT),
        fields: &[],
        body: None,
        ports_layout: PortsLayout::Rows,
        port_row_extra: None,
        executor: &TIMELINE_EXEC,
        on_run: None,
        busy_signal: None,
        runtime: Some(|| NodeRuntime::Ext(Box::new(TimelineRt { done: std::sync::Mutex::new(HashSet::new()) }))),
    });
}

/// Внутри окна: рабочее место графов (открытые графы сохраняются сами).
pub fn mount(st: St) {
    let _ = ST.set(st);
    super::framegraph_ui::init();
    let ws = EditorWorkspace::new_or_restore();
    let _ = WS.set(ws);
    install_node_bridge();
    if ws.active.get_untracked().is_none() {
        ws.new_untitled();
    }
    provide_context(ws);
    syn_node_editor::persist::install_workspace_autosave();
    syn_node_editor::run_controls::install_run_watcher();
}

/// Панель «Граф»: кнопки прогона и холст активного графа.
pub fn panel() -> Box<dyn Widget> {
    let Some(ws) = try_use_context::<EditorWorkspace>() else {
        return Box::new(Center::new().child(Text::new("Редактор узлов не подключён").class("panel-empty")));
    };
    Box::new(DecoratedBox::new().class("grow").child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        // открыт граф кадра клипа — его холст
        if let (Some(o), Some(st)) = (super::framegraph_ui::current().get(), ST.get().copied()) {
            let ctx = o.ctx;
            return vec![Box::new(
                Column::new()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(DecoratedBox::new().class("graph-head").child(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class("grow").child(super::framegraph_ui::head(st, o))).child(DecoratedBox::new().class("graph-zoom").child(syn_node_editor::canvas::zoom_controls(ctx)))))
                    .child(DecoratedBox::new().class("grow").child(syn_node_editor::canvas::view_bare(ctx, Vec::new()))),
            )];
        }
        vec![match ws.active_ctx() {
            Some(ctx) => Box::new(
                Column::new()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(DecoratedBox::new().class("graph-head").child(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(tabs(ws)).child(DecoratedBox::new().class("grow")).child(DecoratedBox::new().class("graph-zoom").child(syn_node_editor::canvas::zoom_controls(ctx))).child(DecoratedBox::new().class("toolbar-sep")).child(syn_node_editor::run_controls::view(ctx))))
                    // медиа из медиатеки или файл из проводника — нода «Файл» в точке броска
                    .child(DecoratedBox::new().class("grow").child(
                        DropArea::new()
                            .accept_types(vec![super::panels::DRAG_MEDIA.to_string(), syngui::DragData::TYPE_FILE.to_string()])
                            .on_drop_positioned(move |info: syngui::widgets::overlay::DropInfo| drop_file(ctx, &info.data, info.local_position))
                            .child(syn_node_editor::canvas::view_bare(ctx, Vec::new())),
                    )),
            ),
            None => Box::new(Center::new().child(Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new("Нет открытого графа").class("panel-empty")).child(Button::new("Новый граф").on_click(move || {
                ws.new_untitled();
            }).class("primary-btn")))),
        }]
    })))
}

/// Вкладки открытых графов: щелчок — открыть, × — закрыть, + — новый граф.
fn tabs(ws: EditorWorkspace) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let active = ws.active.get();
        let mut row = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center);
        for t in ws.tabs.get().into_iter().filter(|t| !t.hidden.get()) {
            let id = t.id;
            let title = format!("{}{}", t.title.get(), if t.dirty.get() { " ●" } else { "" });
            let on = active == Some(id);
            row = row.child(
                DecoratedBox::new().class(if on { "graph-tab graph-tab-on" } else { "graph-tab" }).child(
                    Row::new()
                        .gap(4.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(GestureDetector::new().on_click(move || ws.activate(id)).child(Text::new(title).max_lines(1).class("graph-tab-text")))
                        .child(GestureDetector::new().on_click(move || ws.request_close(id)).child(Icon::new("\u{E5CD}").class("graph-tab-x"))),
                ),
            );
        }
        row = row.child(ToolButton::new("\u{E145}").tooltip("Новый граф").on_click(move || {
            ws.new_untitled();
        }));
        vec![Box::new(row)]
    })
}

/// Бросок на холст: по ноде «Файл» на каждый файл, в точке броска (лесенкой).
fn drop_file(ctx: syn_node_editor::NodeEditorCtx, data: &syngui::DragData, at: Point) {
    let paths: Vec<PathBuf> = if data.drag_type == super::panels::DRAG_MEDIA {
        data.payload.trim().parse::<u64>().ok().and_then(|id| ed().project.media(id).map(|m| m.path.clone())).into_iter().collect()
    } else {
        data.payload.lines().map(str::trim).filter(|l| !l.is_empty()).map(PathBuf::from).collect()
    };
    let (pan, zoom) = (ctx.pan.get_untracked(), ctx.zoom.get_untracked().max(0.01));
    for (i, p) in paths.into_iter().enumerate() {
        let world = Point::new((at.x - pan.x) / zoom + i as f32 * 24.0, (at.y - pan.y) / zoom + i as f32 * 24.0);
        let id = ctx.add_node(NodeKind::FlowFile, world);
        if let Some(n) = ctx.find_node(id) {
            if let Some(syn_node_editor::types::FieldValue::Text(s)) = n.fields.lock().unwrap().get("path") {
                s.set(p.display().to_string());
            }
        }
    }
}

// ── D3: ноды сценария видеоредактора с явным запуском (▶ сверху) ──────────

/// Состояние долгой ноды: занята, итог, строка статуса.
#[derive(Clone, Copy)]
struct JobRt {
    running: RwSignal<bool>,
    result: RwSignal<Option<PortValue>>,
    count: RwSignal<f32>,
}
impl ExtRuntime for JobRt {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn save_state(&self) -> Option<serde_json::Value> {
        None
    }
    fn load_state(&self, _v: &serde_json::Value) {}
}

fn job(node: &syn_node_editor::types::NodeInstance) -> Option<JobRt> {
    node.runtime.lock().ok().and_then(|r| r.ext::<JobRt>().copied())
}

fn job_busy(node: &syn_node_editor::types::NodeInstance) -> Option<RwSignal<bool>> {
    job(node).map(|j| j.running)
}

struct JobExec;
impl NodeExecutor for JobExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let Some(j) = job(ctx.node) else { return };
        let (r, n) = if ctx.track { (j.result.get(), j.count.get()) } else { (j.result.get_untracked(), j.count.get_untracked()) };
        if let Some(v) = r {
            ctx.write_output("out", v);
        }
        ctx.write_output("count", PortValue::Float(n));
    }
}
static JOB_EXEC: JobExec = JobExec;

/// Файлы на входе `port` ноды (значение подключённого выхода).
fn input_files(node: &syn_node_editor::types::NodeInstance, ctx: &syn_node_editor::NodeEditorCtx, port: &str) -> Vec<PathBuf> {
    let conns = ctx.connections.get_untracked();
    let Some(c) = conns.iter().find(|c| c.to_node == node.id && c.to_port == port) else { return Vec::new() };
    ctx.values.get_untracked().get(&(c.from_node, c.from_port)).map(files_of).unwrap_or_default()
}

fn field_f(node: &syn_node_editor::types::NodeInstance, name: &str, d: f32) -> f32 {
    match node.fields.lock().ok().and_then(|m| m.get(name).cloned()) {
        Some(syn_node_editor::types::FieldValue::Float(s)) if s.get_untracked() != 0.0 => s.get_untracked(),
        _ => d,
    }
}

fn field_t(node: &syn_node_editor::types::NodeInstance, name: &str) -> String {
    match node.fields.lock().ok().and_then(|m| m.get(name).cloned()) {
        Some(syn_node_editor::types::FieldValue::Text(s)) => s.get_untracked(),
        Some(syn_node_editor::types::FieldValue::Choice(s)) => s.get_untracked().to_string(),
        _ => String::new(),
    }
}

/// Клипы таймлайна из этих файлов (пусто — все клипы медиа).
fn clips_of(files: &[PathBuf]) -> Vec<(u64, u64)> {
    let e = ed();
    let p = &e.project;
    p.tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .filter_map(|c| c.media().and_then(|m| p.media(m)).map(|m| (c.id, m.id, m.path.clone())))
        .filter(|(_, _, path)| files.is_empty() || files.iter().any(|f| f == path || std::fs::canonicalize(f).ok().as_ref() == Some(path)))
        .map(|(c, m, _)| (c, m))
        .collect()
}

fn finish(j: JobRt, result: Option<PortValue>, count: usize, msg: String) {
    syngui::async_runtime::run_on_main_thread(move || {
        j.result.set(result);
        j.count.set(count as f32);
        j.running.set(false);
        if let Some(st) = ST.get().copied() {
            st.toast.set(Some(msg));
        }
    });
}

fn files_val(v: &[PathBuf]) -> PortValue {
    PortValue::Data(std::sync::Arc::new(DataBlob::Flow(synaptix_flow::Val::List(v.iter().map(|p| synaptix_flow::Val::File(p.clone())).collect()))))
}

/// «Вырезать паузы»: у клипов этих файлов (или всех со звуком).
fn run_cut(node: &syn_node_editor::types::NodeInstance, ctx: &syn_node_editor::NodeEditorCtx) {
    let (Some(j), Some(st)) = (job(node), ST.get().copied()) else { return };
    let files = input_files(node, ctx, "media");
    let thr = field_f(node, "threshold_db", -35.0);
    let min = field_f(node, "min", 0.6);
    let mut done = 0;
    for (cid, _) in clips_of(&files) {
        let audio = {
            let e = ed();
            let p = &e.project;
            p.find_clip(cid).is_some_and(|(ti, _)| p.tracks[ti].kind == crate::doc::TrackKind::Audio) || p.clip(cid).is_some_and(|c| p.linked_peer(c.id).is_none() && c.volume > 0.0 && c.media().and_then(|m| p.media(m)).is_some_and(|m| m.has_audio))
        };
        if audio && st.exec("clip.cut_silence", json!({"id": cid, "threshold_db": thr, "min": min})).is_some() {
            done += 1;
        }
    }
    j.count.set(done as f32);
    st.toast.set(Some(format!("Паузы вырезаны у клипов: {done}")));
}

/// «Субтитры»: расшифровка (в своём потоке) и субтитры у клипов этих файлов.
fn run_subs(node: &syn_node_editor::types::NodeInstance, ctx: &syn_node_editor::NodeEditorCtx) {
    let Some(j) = job(node) else { return };
    if j.running.get_untracked() {
        return;
    }
    let files = input_files(node, ctx, "media");
    let max_chars = field_f(node, "max_chars", 42.0);
    let lang = field_t(node, "language");
    let todo: Vec<(u64, PathBuf, bool)> = {
        let e = ed();
        let mids: std::collections::BTreeSet<u64> = clips_of(&files).into_iter().map(|(_, m)| m).collect();
        mids.into_iter().filter_map(|m| e.project.media(m).filter(|x| x.has_audio || x.kind == crate::doc::MediaKind::Audio).map(|x| (m, x.path.clone(), x.transcript.is_some()))).collect()
    };
    j.running.set(true);
    std::thread::spawn(move || {
        let mut errs = Vec::new();
        let mut ready = Vec::new();
        for (mid, path, has) in todo {
            if has {
                ready.push((mid, None));
                continue;
            }
            match crate::cmd::audio::transcribe_file(&path, (!lang.is_empty()).then_some(lang.as_str())) {
                Ok(t) => ready.push((mid, Some(t))),
                Err(e) => errs.push(e),
            }
        }
        syngui::async_runtime::run_on_main_thread(move || {
            let Some(st) = ST.get().copied() else { return };
            {
                let mut e = ed();
                if ready.iter().any(|(_, t)| t.is_some()) {
                    e.begin("Расшифровка");
                }
                for (mid, t) in &ready {
                    if let (Some(t), Some(m)) = (t, e.project.media.iter_mut().find(|m| m.id == *mid)) {
                        m.transcript = Some(t.clone());
                    }
                }
            }
            let mids: Vec<u64> = ready.iter().map(|r| r.0).collect();
            let ids: Vec<u64> = clips_of(&[]).into_iter().filter(|(_, m)| mids.contains(m)).map(|(c, _)| c).collect();
            let n = if ids.is_empty() { 0 } else { st.exec("subtitles.add", json!({"ids": ids, "max_chars": max_chars})).and_then(|o| o.data["count"].as_u64()).unwrap_or(0) };
            j.count.set(n as f32);
            j.running.set(false);
            st.toast.set(Some(if errs.is_empty() { format!("Субтитров: {n}") } else { format!("Субтитров: {n}; ошибки: {}", errs.join("; ")) }));
        });
    });
}

/// Нейрофильтр над файлами входа: увеличение или плавность RIFE.
fn run_vfx(node: &syn_node_editor::types::NodeInstance, ctx: &syn_node_editor::NodeEditorCtx, kind: &'static str) {
    let Some(j) = job(node) else { return };
    if j.running.get_untracked() {
        return;
    }
    let files = input_files(node, ctx, "media");
    if files.is_empty() {
        if let Some(st) = ST.get().copied() {
            st.toast.set(Some("Нейрофильтр: подайте на вход файлы (например, «Медиатека проекта»)".into()));
        }
        return;
    }
    let scale = if field_t(node, "scale") == "1" { 4 } else { 2 };
    let model = field_t(node, "model");
    let fps = field_f(node, "fps", 60.0) as f64;
    j.running.set(true);
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let mut errs = Vec::new();
        for f in &files {
            match crate::cmd::media::vfx_file(f, kind, scale, &model, fps, |_, _| {}) {
                Ok(p) => out.push(p),
                Err(e) => errs.push(e),
            }
        }
        let n = out.len();
        let v = files_val(&out);
        let paths: Vec<String> = out.iter().map(|p| p.display().to_string()).collect();
        syngui::async_runtime::run_on_main_thread(move || {
            if let (Some(st), false) = (ST.get().copied(), paths.is_empty()) {
                st.exec("media.import", json!({"paths": paths}));
            }
        });
        finish(j, Some(v), n, if errs.is_empty() { format!("Нейрофильтр: готово файлов {n}") } else { format!("Нейрофильтр: {n} готово; ошибки: {}", errs.join("; ")) });
    });
}

fn run_up(node: &syn_node_editor::types::NodeInstance, ctx: &syn_node_editor::NodeEditorCtx) {
    run_vfx(node, ctx, "upscale");
}
fn run_rife(node: &syn_node_editor::types::NodeInstance, ctx: &syn_node_editor::NodeEditorCtx) {
    run_vfx(node, ctx, "interpolate");
}

/// «Экспорт таймлайна»: проект в видеофайл, путь — на выход.
fn run_export(node: &syn_node_editor::types::NodeInstance, _ctx: &syn_node_editor::NodeEditorCtx) {
    let Some(j) = job(node) else { return };
    if j.running.get_untracked() {
        return;
    }
    let (project, name) = {
        let e = ed();
        (e.project.clone(), e.project.name.clone())
    };
    let path = match field_t(node, "path") {
        p if !p.trim().is_empty() => crate::doc::expand(p.trim()),
        _ => std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join("Videos").join(format!("{}.mp4", if name.is_empty() { "проект".into() } else { name.replace('/', "_") })),
    };
    j.running.set(true);
    std::thread::spawn(move || {
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        match crate::render::run_child(&project, &["export".into(), path.display().to_string()], |_, _| {}) {
            Ok(_) => finish(j, Some(files_val(std::slice::from_ref(&path))), 1, format!("Экспорт: {}", path.display())),
            Err(e) => finish(j, None, 0, format!("Экспорт: {e}")),
        }
    });
}

const IN_MEDIA: &[PortSchema] = &[PortSchema { name: "media", label: "файлы", kind: PortKind::Data }];
const OUT_COUNT: &[PortSchema] = &[PortSchema { name: "count", label: "сколько", kind: PortKind::Data }];
const OUT_FILES: &[PortSchema] = &[PortSchema { name: "out", label: "файлы", kind: PortKind::Data }, PortSchema { name: "count", label: "сколько", kind: PortKind::Data }];
const OUT_FILE: &[PortSchema] = &[PortSchema { name: "out", label: "видео", kind: PortKind::Data }];
const F_CUT: &[FieldSchema] = &[FieldSchema { name: "threshold_db", label: "Порог, дБ (−35)", ty: FieldType::Float }, FieldSchema { name: "min", label: "Пауза от, с (0,6)", ty: FieldType::Float }];
const F_SUBS: &[FieldSchema] = &[FieldSchema { name: "max_chars", label: "Символов в фразе (42)", ty: FieldType::Float }, FieldSchema { name: "language", label: "Язык (пусто — сам)", ty: FieldType::Text }];
const F_UP: &[FieldSchema] = &[FieldSchema { name: "scale", label: "Увеличение", ty: FieldType::Choice(&["×2", "×4"]) }, FieldSchema { name: "model", label: "Модель (пусто — любая)", ty: FieldType::Text }];
const F_RIFE: &[FieldSchema] = &[FieldSchema { name: "fps", label: "До частоты, fps (60)", ty: FieldType::Float }];
const F_EXPORT: &[FieldSchema] = &[FieldSchema { name: "path", label: "Файл (пусто — ~/Videos/<проект>.mp4)", ty: FieldType::Text }];

fn job_rt() -> NodeRuntime {
    NodeRuntime::Ext(Box::new(JobRt { running: use_signal(false), result: use_signal(None), count: use_signal(0.0) }))
}

/// Ноды D3 — в разделе «Видеоредактор».
fn register_jobs() {
    let reg = |kind: &'static str, icon: &'static str, title: &'static str, inputs: &'static [PortSchema], outputs: &'static [PortSchema], fields: &'static [FieldSchema], run: syn_node_editor::registry::NodeRunHook| {
        registry::register_kind(NodeKindMeta {
            kind: NodeKind::Ext(ExtKind(kind)),
            icon,
            title,
            category: NodeCategory::Custom("video_editor"),
            subcategory: None,
            inputs: PortsSpec::Static(inputs),
            outputs: PortsSpec::Static(outputs),
            fields,
            body: None,
            ports_layout: PortsLayout::Rows,
            port_row_extra: None,
            executor: &JOB_EXEC,
            on_run: Some(run),
            busy_signal: Some(job_busy),
            runtime: Some(job_rt),
        });
    };
    reg("ve_cut_silence", "\u{E050}", "Вырезать паузы", IN_MEDIA, OUT_COUNT, F_CUT, run_cut);
    reg("ve_subtitles", "\u{E048}", "Субтитры из речи", IN_MEDIA, OUT_COUNT, F_SUBS, run_subs);
    reg("ve_vfx_upscale", "\u{E8FF}", "Нейроувеличение (vfx)", IN_MEDIA, OUT_FILES, F_UP, run_up);
    reg("ve_vfx_interpolate", "\u{E01F}", "Плавность RIFE (vfx)", IN_MEDIA, OUT_FILES, F_RIFE, run_rife);
    reg("ve_export", "\u{E2C4}", "Экспорт таймлайна", &[], OUT_FILE, F_EXPORT, run_export);
}


/// Мост команд `nodes.*` (агент, сокет): работа с активной вкладкой графа в
/// главном потоке окна.
fn install_node_bridge() {
    use crate::cmd::nodegraph::{install, Bridge};
    use std::time::Duration;
    fn on_main<R: Send + 'static>(f: impl FnOnce() -> std::result::Result<R, String> + Send + 'static) -> std::result::Result<R, String> {
        syngui::async_runtime::call_on_main_thread(Duration::from_secs(10), f).unwrap_or_else(|| Err("окно не ответило".into()))
    }
    fn active() -> std::result::Result<syn_node_editor::NodeEditorCtx, String> {
        let ws = workspace().ok_or("панель «Граф» ещё не открывалась")?;
        ws.active_ctx_untracked().ok_or_else(|| "в панели «Граф» нет открытой вкладки — nodes.tabs activate=…".to_string())
    }
    install(Bridge {
        tabs: |activate| {
            on_main(move || {
                let ws = workspace().ok_or("панель «Граф» ещё не открывалась")?;
                let tabs = ws.tabs.get_untracked();
                if let Some(q) = activate.map(|q| q.to_lowercase()) {
                    let t = tabs.iter().find(|t| t.title.get_untracked().to_lowercase().contains(&q)).ok_or_else(|| format!("нет вкладки «{q}»"))?;
                    ws.activate(t.id);
                    if let Some(st) = ST.get().copied() {
                        st.open_panel("graph");
                    }
                }
                let act = ws.active.get_untracked();
                Ok(tabs.iter().map(|t| format!("{} {}", if Some(t.id) == act { "▶" } else { " " }, t.title.get_untracked())).collect::<Vec<_>>().join("\n"))
            })
        },
        show: || on_main(|| Ok(syn_node_editor::group::describe(&active()?))),
        group: |ids, title| {
            on_main(move || {
                let ctx = active()?;
                let ids: Vec<syn_node_editor::types::NodeId> = ids.into_iter().map(syn_node_editor::types::NodeId).collect();
                let g = syn_node_editor::group::group(&ctx, &ids, &title)?;
                let nodes = ctx.nodes.get_untracked();
                let st = nodes.iter().find(|n| n.id == g).and_then(syn_node_editor::group::state).ok_or("подграф не создан")?;
                let p = st.ports.lock().map(|p| p.clone()).unwrap_or_default();
                Ok(format!("подграф #{} «{}»: нод {}, входов {}, выходов {}", g.0, st.title.get_untracked(), ids.len(), p.ins.len(), p.outs.len()))
            })
        },
        ungroup: |id| {
            on_main(move || {
                let ctx = active()?;
                let back = syn_node_editor::group::ungroup(&ctx, syn_node_editor::types::NodeId(id))?;
                Ok(format!("подграф #{id} развёрнут: {}", back.iter().map(|n| format!("#{}", n.0)).collect::<Vec<_>>().join(", ")))
            })
        },
    });
}
