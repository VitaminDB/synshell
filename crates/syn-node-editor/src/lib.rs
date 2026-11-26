//! Редактор узлов — общий для Synthos и видеоредактора synshell
//! (`syn-video-editor`): холст с панорамой, масштабом и щипком, карточки нод,
//! порты и провода, палитра нод, поля, сохранение графа, прогон
//! (реактивный пересчёт и очередь «Пуск/Пауза/Стоп»), ноды ядра
//! synaptix-flow (генерация через службу synaptixd) и звуковые ноды.
//!
//! Программа подключает:
//! - хост ([`host::set_host`]) — уведомления, имя программы, пути;
//! - строки ([`i18n::CATALOGS`] и `synaptix_flow::CATALOGS`) и стили ([`STYLES`]);
//! - свои виды нод ([`registry::register_kind`]) и разделы палитры
//!   ([`registry::register_category`]);
//! - холст ([`canvas::view`]) в своё окно, рядом с контекстом
//!   [`tabs::EditorWorkspace`] (`provide_context`) и
//!   [`run_controls::install_run_watcher`].

pub mod canvas;
pub mod controls;
pub mod convert;
pub mod eval;
pub mod help;
pub mod host;
pub mod i18n;
pub mod icons;
pub mod model;
pub mod native;
pub mod node_view;
pub mod nodes;
pub mod persist;
pub mod registry;
pub mod run_controls;
pub mod state;
pub mod style_dialog;
pub mod tabs;
pub mod template_preview;
pub mod timing;
pub mod types;
pub mod wires;

pub use registry::{register_category, register_kind, register_port_kind, CategoryInfo, NodeCategory, NodeKindMeta};
pub use state::NodeEditorCtx;

/// Свои пункты контекстного меню ноды от программы: по ноде и выделению
/// (`picked`) — пункты (id, подпись); выбор пункта — `run`.
pub struct NodeMenuHook {
    pub items: fn(&NodeEditorCtx, types::NodeId, &[types::NodeId]) -> Vec<(String, String)>,
    pub run: fn(&NodeEditorCtx, types::NodeId, &[types::NodeId], &str),
}

static NODE_MENU: std::sync::OnceLock<NodeMenuHook> = std::sync::OnceLock::new();

/// Поставить крючок меню нод (один на программу).
pub fn set_node_menu(h: NodeMenuHook) {
    let _ = NODE_MENU.set(h);
}

pub(crate) fn node_menu() -> Option<&'static NodeMenuHook> {
    NODE_MENU.get()
}
pub use tabs::EditorWorkspace;
pub use types::{ExtKind, ExtRuntime, NodeKind, NodeRuntime, PortValue};

/// Таблица стилей редактора (холст, карточки нод, поля, прогон, звуковые и
/// видео-ноды). Переменные цвета (`--bg`, `--accent`…) — из темы программы.
pub const STYLES: &str = concat!(
    include_str!("../styles/node_editor.mss"),
    "\n",
    include_str!("../styles/node_editor_node.mss"),
    "\n",
    include_str!("../styles/node_editor_inputs.mss"),
    "\n",
    "\n",
    include_str!("../styles/node_editor_run_controls.mss"),
    "\n",
    include_str!("../styles/node_editor_timers.mss"),
    "\n",
    include_str!("../styles/markdown_node.mss"),
    "\n",
    include_str!("../styles/node_style_dialog.mss"),
    "\n",
    include_str!("../styles/audio_node.mss"),
    "\n",
    include_str!("../styles/equalizer_node.mss"),
    "\n",
    include_str!("../styles/asr_node.mss"),
    "\n",
    include_str!("../styles/omnivoice_node.mss"),
    "\n",
    include_str!("../styles/voxcpm_node.mss"),
    "\n",
    include_str!("../styles/vibevoice_node.mss"),
    "\n",
    include_str!("../styles/acestep_nodes.mss"),
    "\n",
    include_str!("../styles/h3_nodes.mss"),
    "\n",
    include_str!("../styles/flux_nodes.mss"),
    "\n",
    include_str!("../styles/ltx_nodes.mss"),
    "\n",
    include_str!("../styles/sortformer_node.mss"),
    "\n",
    include_str!("../styles/ffmpeg_player_node.mss"),
    "\n",
);
