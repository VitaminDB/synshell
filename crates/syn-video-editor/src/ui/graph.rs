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
use syn_node_editor::types::{DataBlob, ExtKind, ExtRuntime, NodeKind, NodeRuntime, PortKind, PortSchema, PortValue, PortsLayout, PortsSpec};
use syn_node_editor::EditorWorkspace;

use super::{ed, St};

static ST: OnceLock<St> = OnceLock::new();

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
