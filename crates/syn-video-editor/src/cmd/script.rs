//! Сценарий Rhai одной командой (F1): массовые правки за один вызов.

use super::{req, Args, Command, Out, Res, Ty};
use crate::editor::Editor;

pub static COMMANDS: &[&Command] = &[&RUN];

static RUN: Command = Command {
    id: "script.run",
    category: "Правка",
    title: "Выполнить сценарий Rhai",
    shortcut: "",
    keywords: "сценарий скрипт rhai массово цикл все клипы пакет автоматизация",
    help: "Массовые правки сценарием Rhai за один вызов (вместо десятков вызовов): `cmd(\"clip.set\", #{id: 5, name: \"Сцена 1\"})` — любая команда таблицы, возвращает #{text, data}; `project()` — проект данными (tracks[].clips[] с id, start, duration, media, name…), `selection()`, `playhead()`, `dump()` — проект текстом; `print(x)` — в ответ. Пример: `for t in project().tracks { for c in t.clips { if c.duration < 1.0 { cmd(\"clip.remove\", #{ids: [c.id]}) } } }`. Весь сценарий — один шаг отмены; ошибка любой команды останавливает сценарий и откатывает его правки. Необратимые команды внутри нельзя.",
    args: &[req("code", Ty::Str, "текст сценария Rhai")],
    danger: false,
    run,
};

fn run(ed: &mut Editor, a: &Args) -> Res {
    let code = a.str("code")?.unwrap_or_default();
    let own = !ed.in_group();
    let snapshot = (!own).then(|| (ed.project.clone(), ed.selection.clone()));
    if own {
        ed.begin_group("Сценарий");
    }
    let r = crate::script::run(ed, &code);
    match r {
        Ok(text) => {
            if own {
                ed.end_group();
            }
            Ok(Out::text(text))
        }
        Err(e) => {
            if own {
                ed.abort_group();
            } else if let Some((p, sel)) = snapshot {
                if ed.project != p {
                    ed.begin("Откат сценария");
                    ed.project = p;
                    ed.selection = sel;
                }
            }
            Err(e)
        }
    }
}
