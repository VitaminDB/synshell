//! Действия окна для агента и сокета: всё, что есть в меню и палитре, но не
//! команда документа (виды и раскладки, панели, масштаб просмотра, полный
//! экран, фон поля, галереи и окна). Окно при запуске ставит мост; без окна
//! (CLI) действия недоступны — команда так и отвечает.

use std::sync::OnceLock;

use serde_json::{json, Value};

use super::{req, Args, Command, Out, Res, Ty};
use crate::editor::Editor;

pub static COMMANDS: &[&Command] = &[&LIST, &RUN];

pub struct Bridge {
    /// Действия окна: (id, раздел, заголовок, клавиши).
    pub list: fn() -> Vec<(String, String, String, String)>,
    /// Выполнить действие (в главном потоке окна, без ожидания).
    pub run: fn(&str) -> Result<String, String>,
    /// Состояние окна текстом: вид, открытые панели, масштаб, полный экран.
    pub state: fn() -> String,
}

static BRIDGE: OnceLock<Bridge> = OnceLock::new();

pub fn install(b: Bridge) {
    let _ = BRIDGE.set(b);
}

fn bridge() -> Result<&'static Bridge, String> {
    BRIDGE.get().ok_or_else(|| "действия окна доступны только в открытом окне редактора (здесь — без окна)".to_string())
}

static LIST: Command = Command {
    id: "ui.list",
    category: "Вид",
    title: "Действия окна",
    shortcut: "",
    keywords: "окно меню вид панели раскладка масштаб полный экран действия интерфейса",
    help: "Состояние окна (вид, панели, масштаб просмотра, полный экран) и все действия окна, которых нет среди команд документа: виды и раскладки, открыть панель, масштаб и фон просмотра, полный экран, галереи, окна управления. Выполнить — ui.run.",
    args: &[],
    danger: false,
    run: list,
};

fn list(_ed: &mut Editor, _a: &Args) -> Res {
    let b = bridge()?;
    let items = (b.list)();
    let mut text = format!("Окно: {}\nДействия:", (b.state)());
    for (id, cat, title, keys) in &items {
        text.push_str(&format!("\n  {id} — {cat}: {title}{}", if keys.is_empty() { String::new() } else { format!(" ({keys})") }));
    }
    let data: Vec<Value> = items.iter().map(|(id, cat, title, keys)| json!({"id": id, "category": cat, "title": title, "keys": keys})).collect();
    Ok(Out::with(text, Value::Array(data)))
}

static RUN: Command = Command {
    id: "ui.run",
    category: "Вид",
    title: "Действие окна",
    shortcut: "",
    keywords: "выполнить пункт меню вид панель раскладка масштаб полный экран открыть",
    help: "Выполнить действие окна по id из ui.list: view.layout.<вид> (edit, motion, graph, templates, color, ai, phone…), view.panel.<панель> (media, effects, viewer, timeline, inspector, ai, templates, color, graph…), view.zoom_fit / view.zoom_100 / view.zoom_in / view.zoom_out, view.viewer_bg, view.fullscreen, view.palette, view.layout.reset, template.coverage (окно покрытия) и т. п. Команды документа (clip.*, effect.*…) вызываются напрямую, не через ui.run.",
    args: &[req("action", Ty::Str, "id действия (ui.list)")],
    danger: false,
    run,
};

fn run(_ed: &mut Editor, a: &Args) -> Res {
    let b = bridge()?;
    let id = a.need_str("action")?;
    let id = id.trim();
    if !(b.list)().iter().any(|x| x.0 == id) {
        if super::find(id).is_some() {
            return Err(format!("«{id}» — команда документа: вызовите её напрямую ({})", id.replace('.', "_")));
        }
        return Err(format!("нет действия окна «{id}» — ui.list"));
    }
    (b.run)(id).map(Out::text)
}
