//! Графы панели «Граф» для агента и сокета: вкладки, холст текстом,
//! объединить ноды в подграф (любые: ядро, звук, просмотр, ноды программ,
//! другие подграфы) и разгруппировать. Холст живёт в окне — команды идут
//! через мост, который окно ставит при запуске; без окна — понятный отказ.

use std::sync::OnceLock;

use super::{arg, req, Args, Command, Out, Res, Ty};
use crate::editor::Editor;

pub static COMMANDS: &[&Command] = &[&TABS, &SHOW, &GROUP, &UNGROUP];

pub struct Bridge {
    pub tabs: fn(Option<String>) -> Result<String, String>,
    pub show: fn() -> Result<String, String>,
    pub group: fn(Vec<u64>, String) -> Result<String, String>,
    pub ungroup: fn(u64) -> Result<String, String>,
}

static BRIDGE: OnceLock<Bridge> = OnceLock::new();

pub fn install(b: Bridge) {
    let _ = BRIDGE.set(b);
}

fn bridge() -> Result<&'static Bridge, String> {
    BRIDGE.get().ok_or_else(|| "графы панели «Граф» доступны только в открытом окне редактора".to_string())
}

static TABS: Command = Command {
    id: "nodes.tabs",
    category: "Граф",
    title: "Вкладки графов",
    shortcut: "",
    keywords: "граф вкладки графы сценарии рецепты панель граф открыть вкладку",
    help: "Вкладки панели «Граф» (графы сценариев, рецепты, свои): какая активна; activate — сделать активной вкладку по части названия. Дальше — nodes.show.",
    args: &[arg("activate", Ty::Str, "часть названия вкладки")],
    danger: false,
    run: tabs,
};

fn tabs(_ed: &mut Editor, a: &Args) -> Res {
    (bridge()?.tabs)(a.str("activate")?).map(Out::text)
}

static SHOW: Command = Command {
    id: "nodes.show",
    category: "Граф",
    title: "Граф вкладки текстом",
    shortcut: "",
    keywords: "граф ноды провода подграф показать холст",
    help: "Активная вкладка панели «Граф» текстом: ноды (#id, название, вид: ядро/программа/встроенная/подграф, в каком подграфе), состав и порты подграфов, провода.",
    args: &[],
    danger: false,
    run: show,
};

fn show(_ed: &mut Editor, _a: &Args) -> Res {
    (bridge()?.show)().map(Out::text)
}

static GROUP: Command = Command {
    id: "nodes.group",
    category: "Граф",
    title: "Объединить ноды в подграф",
    shortcut: "",
    keywords: "подграф объединить сгруппировать ноды группа свернуть граф",
    help: "Объединить ноды активной вкладки «Граф» в одну ноду-подграф (nodes — #id из nodes.show; любые виды, в том числе другие подграфы — вложенный подграф). Провода через границу становятся портами подграфа («нода: порт»), выполнение графа не меняется. title — название. Развернуть — nodes.ungroup.",
    args: &[req("nodes", Ty::Ids, "ноды (#id)"), arg("title", Ty::Str, "название подграфа")],
    danger: false,
    run: group,
};

fn group(_ed: &mut Editor, a: &Args) -> Res {
    let ids = a.ids("nodes")?.unwrap_or_default();
    (bridge()?.group)(ids, a.str("title")?.unwrap_or_default()).map(Out::text)
}

static UNGROUP: Command = Command {
    id: "nodes.ungroup",
    category: "Граф",
    title: "Разгруппировать подграф",
    shortcut: "",
    keywords: "подграф разгруппировать развернуть группа",
    help: "Развернуть подграф активной вкладки «Граф» (node — #id подграфа): его ноды снова видны на холсте.",
    args: &[req("node", Ty::Id, "подграф (#id)")],
    danger: false,
    run: ungroup,
};

fn ungroup(_ed: &mut Editor, a: &Args) -> Res {
    (bridge()?.ungroup)(a.need_id("node")?).map(Out::text)
}
