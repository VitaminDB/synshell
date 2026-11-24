use super::{arg, req, Args, Command, Out, Res, Ty};
use crate::doc::tc;
use crate::editor::Editor;

pub static COMMANDS: &[&Command] = &[&UNDO, &REDO, &PLAYHEAD, &SELECT];

static UNDO: Command = Command {
    id: "edit.undo",
    category: "Правка",
    title: "Отменить",
    shortcut: "Ctrl+Z",
    keywords: "undo отменить назад",
    help: "Отменить последнее изменение (ход агента — целиком).",
    args: &[],
    danger: false,
    run: undo,
};

fn undo(ed: &mut Editor, _a: &Args) -> Res {
    ed.undo().map(|l| Out::text(format!("Отменено: {l}"))).ok_or_else(|| "нечего отменять".into())
}

static REDO: Command = Command {
    id: "edit.redo",
    category: "Правка",
    title: "Повторить",
    shortcut: "Ctrl+Shift+Z",
    keywords: "redo повторить вернуть",
    help: "Повторить отменённое.",
    args: &[],
    danger: false,
    run: redo,
};

fn redo(ed: &mut Editor, _a: &Args) -> Res {
    ed.redo().map(|l| Out::text(format!("Повторено: {l}"))).ok_or_else(|| "нечего повторять".into())
}

static PLAYHEAD: Command = Command {
    id: "playhead.set",
    category: "Воспроизведение",
    title: "Перейти ко времени",
    shortcut: "Ctrl+G",
    keywords: "goto перейти позиция время playhead",
    help: "Поставить позицию воспроизведения.",
    args: &[req("at", Ty::Time, "время")],
    danger: false,
    run: playhead,
};

fn playhead(ed: &mut Editor, a: &Args) -> Res {
    let t = ed.project.snap(a.need_time("at")?);
    ed.playhead = t;
    ed.revision += 1;
    Ok(Out::text(format!("Позиция {}", tc(t))))
}

static SELECT: Command = Command {
    id: "selection.set",
    category: "Правка",
    title: "Выделить",
    shortcut: "",
    keywords: "select выделить",
    help: "Выделить клипы (пустой список — снять выделение).",
    args: &[arg("ids", Ty::Ids, "клипы (c…)")],
    danger: false,
    run: select,
};

fn select(ed: &mut Editor, a: &Args) -> Res {
    let ids = a.ids("ids")?.unwrap_or_default();
    for id in &ids {
        ed.project.find_clip(*id).ok_or_else(|| format!("нет клипа c{id}"))?;
    }
    ed.selection = ids;
    ed.revision += 1;
    Ok(Out::text(if ed.selection.is_empty() { "Выделение снято".to_string() } else { format!("Выделено: {}", ed.selection.iter().map(|i| format!("c{i}")).collect::<Vec<_>>().join(", ")) }))
}
