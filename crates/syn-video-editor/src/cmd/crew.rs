//! Команды съёмочной группы: статус сцен конвейера, бриф шага, записки
//! между ролями, запуск хода роли (`crew.run` — ход отдельным агентом роли,
//! выполняется агентом окна без блокировки проекта, см. `agent.rs`).

use serde_json::json;

use super::{arg, req, Args, Command as Cmd, Out, Ty};
use crate::crew::{self, Stage, Status};
use crate::editor::Editor;

pub static COMMANDS: &[&Cmd] = &[&STATUS, &BRIEF, &NOTE, &NOTE_SET, &NOTES, &RUN];

fn project(ed: &Editor) -> Result<std::path::PathBuf, String> {
    ed.path.clone().ok_or_else(|| "проект ещё не сохранён".into())
}

/// Сцен в фильме: по структуре, иначе по источнику, иначе по плану.
pub fn scene_count(project: &std::path::Path) -> u32 {
    let dir = crate::store::media_dir(project);
    let structure = std::fs::read_to_string(dir.join("03_структура.md")).unwrap_or_default();
    let rows = structure
        .lines()
        .filter(|l| l.trim_start().starts_with('|'))
        .filter_map(|l| l.trim().trim_start_matches('|').split('|').next().map(|c| c.trim().to_lowercase()))
        .filter_map(|c| c.trim_start_matches("сцена").trim().split(|ch: char| !ch.is_ascii_digit()).next().and_then(|d| d.parse::<u32>().ok()))
        .max();
    rows.or_else(|| crate::cmd::doc::source().map(|s| s.scenes as u32)).unwrap_or(0)
}

static STATUS: Cmd = Cmd {
    id: "crew.status",
    category: "Съёмочная группа",
    title: "Доска производства",
    shortcut: "",
    keywords: "crew группа конвейер статус сцены доска роли",
    help: "Статус каждой сцены фильма по конвейеру (оригинал → сценарий → сверка → ассеты → озвучка → раскадровка → съёмка → проверка → склейка → готово), чья работа следующая и кто её проверяет, открытые записки. Текущая сцена — первая неготовая.",
    args: &[],
    danger: false,
    run: |ed, _| {
        let p = project(ed)?;
        let n = scene_count(&p);
        let mut lines = Vec::new();
        let mut rows = Vec::new();
        let mut current: Option<(u32, Stage)> = None;
        for s in 1..=n {
            let st = crew::stage(&p, s, Some(&ed.project));
            let (a, b) = st.roles();
            let open = crew::notes(&p).iter().filter(|x| x.scene == s && x.status != Status::Closed).count();
            lines.push(format!("сцена {s}: {} — делает {}, проверяет {}{}", st.title(), crew::role(a).map(|r| r.title).unwrap_or(a), crew::role(b).map(|r| r.title).unwrap_or(b), if open > 0 { format!(", записок {open}") } else { String::new() }));
            rows.push(json!({"scene": s, "stage": st.title(), "author": a, "reviewer": b, "open_notes": open}));
            if current.is_none() && st != Stage::Done {
                current = Some((s, st));
            }
        }
        let head = match current {
            Some((s, st)) => format!("Текущая: сцена {s}, шаг «{}» — {} (crew.run {{scene: {s}}}).", st.title(), crew::role(st.roles().0).map(|r| r.title).unwrap_or("")),
            None if n > 0 => "Все сцены готовы — монтаж фильма.".to_string(),
            None => "Сцен нет: сначала 03_структура.".to_string(),
        };
        Ok(Out::with(format!("{head}\n{}", lines.join("\n")), json!({"scenes": rows, "current": current.map(|(s, st)| json!({"scene": s, "stage": st.title()}))})))
    },
};

static BRIEF: Cmd = Cmd {
    id: "crew.brief",
    category: "Съёмочная группа",
    title: "Бриф шага",
    shortcut: "",
    keywords: "crew бриф роль сцена контекст",
    help: "Бриф роли на шаг сцены: задача, выжимка проекта (шапка и правила библии), сцена в структуре, текст сцены источника, документ сцены, карточки героев, мест и реквизита сцены, её ассеты, открытые записки. Без role и stage — по текущему шагу сцены.",
    args: &[req("scene", Ty::Num, "номер сцены"), arg("role", Ty::Str, "роль (director, writer, supervisor, designer, camera, sound, editor)")],
    danger: false,
    run: |ed, a| {
        let p = project(ed)?;
        let n = a.need_num("scene")? as u32;
        let st = crew::stage(&p, n, Some(&ed.project));
        let r = a.str("role")?.unwrap_or_else(|| st.roles().0.to_string());
        let b = crew::brief(&p, &r, n, st);
        Ok(Out::with(b.clone(), json!({"scene": n, "stage": st.title(), "role": r, "chars": b.chars().count()})))
    },
};

static NOTE: Cmd = Cmd {
    id: "crew.note",
    category: "Съёмочная группа",
    title: "Записка роли",
    shortcut: "",
    keywords: "crew записка замечание роль передать",
    help: "Записка между ролями (замечание, просьба, вопрос): кому, по какой сцене, предмет и текст. Тот же предмет снова — следующий круг; после 2 кругов записка уходит режиссёру, ещё после 2 — пользователю.",
    args: &[req("to", Ty::Str, "кому (роль)"), req("scene", Ty::Num, "сцена"), req("text", Ty::Str, "что не так или что нужно"), arg("subject", Ty::Str, "предмет (например «scene_03 место», «ассет @x»)"), arg("from", Ty::Str, "от кого (роль; по умолчанию режиссёр)")],
    danger: false,
    run: |ed, a| {
        let p = project(ed)?;
        let to = a.need_str("to")?;
        crew::role(&to).ok_or_else(|| format!("нет роли «{to}»"))?;
        let from = a.str("from")?.unwrap_or_else(|| "director".into());
        let n = crew::add_note(&p, crew::role(&from).map(|r| r.id).unwrap_or("director"), crew::role(&to).map(|r| r.id).unwrap_or("director"), a.need_num("scene")? as u32, &a.str("subject")?.unwrap_or_default(), &a.need_str("text")?)?;
        crew::journal(&p, &format!("записка {}", crew::note_line(&n)));
        let esc = match n.to.as_str() {
            "user" => " — эскалация пользователю: спроси его и дождись ответа",
            "director" if n.rounds > crew::ROUNDS => " — эскалация режиссёру: реши спор сам",
            _ => "",
        };
        Ok(Out::with(format!("Записка {}{esc}", crew::note_line(&n)), json!({"id": n.id, "to": n.to, "rounds": n.rounds})))
    },
};

static NOTE_SET: Cmd = Cmd {
    id: "crew.note.set",
    category: "Съёмочная группа",
    title: "Статус записки",
    shortcut: "",
    keywords: "crew записка закрыть в работе итог",
    help: "Записку взять в работу (status: work) или закрыть с итогом (status: closed, result — что сделано).",
    args: &[req("id", Ty::Num, "номер записки"), req("status", Ty::Str, "work | closed"), arg("result", Ty::Str, "итог")],
    danger: false,
    run: |ed, a| {
        let p = project(ed)?;
        let st = match a.need_str("status")?.as_str() {
            "work" | "в работе" => Status::Work,
            "closed" | "закрыта" | "close" => Status::Closed,
            "open" => Status::Open,
            s => return Err(format!("статус «{s}»: work | closed")),
        };
        let n = crew::set_note(&p, a.need_num("id")? as u32, st, &a.str("result")?.unwrap_or_default())?;
        crew::journal(&p, &format!("записка {}", crew::note_line(&n)));
        Ok(Out::with(crew::note_line(&n), json!({"id": n.id})))
    },
};

static NOTES: Cmd = Cmd {
    id: "crew.notes",
    category: "Съёмочная группа",
    title: "Записки",
    shortcut: "",
    keywords: "crew записки лента замечания",
    help: "Записки между ролями: по сцене, кому, открытые (all — и закрытые).",
    args: &[arg("scene", Ty::Num, "сцена"), arg("to", Ty::Str, "кому (роль)"), arg("all", Ty::Bool, "и закрытые")],
    danger: false,
    run: |ed, a| {
        let p = project(ed)?;
        let scene = a.num("scene")?.map(|v| v as u32);
        let to = a.str("to")?.and_then(|t| crew::role(&t).map(|r| r.id.to_string()));
        let all = a.bool("all")?.unwrap_or(false);
        let v: Vec<String> = crew::notes(&p).iter().filter(|n| all || n.status != Status::Closed).filter(|n| scene.is_none_or(|s| n.scene == s)).filter(|n| to.as_ref().is_none_or(|t| &n.to == t)).map(crew::note_line).collect();
        Ok(Out::text(if v.is_empty() { "Записок нет".into() } else { v.join("\n") }))
    },
};

static RUN: Cmd = Cmd {
    id: "crew.run",
    category: "Съёмочная группа",
    title: "Ход роли",
    shortcut: "",
    keywords: "crew роль ход шаг сцена поручить",
    help: "Поручить шаг сцены роли: она получает только бриф (задача, выжимка, сцена источника, карточки, ассеты, записки) и свои навыки и делает шаг отдельным ходом; итог — в журнал. Без role — роль текущего шага сцены по конвейеру; task — уточнить задание (по умолчанию — задача шага).",
    args: &[req("scene", Ty::Num, "номер сцены"), arg("role", Ty::Str, "роль"), arg("task", Ty::Str, "задание (по умолчанию — задача шага)")],
    danger: false,
    // ход роли выполняет агент окна (agent.rs, без блокировки проекта); из скрипта — нельзя
    run: |_, _| Err("crew.run выполняется агентом (ход роли — отдельным агентом)".into()),
};

/// Задание и бриф хода роли (для `crew.run`).
pub fn prepare_run(ed: &Editor, a: &Args) -> Result<(std::path::PathBuf, &'static crew::Role, u32, Stage, String), String> {
    let p = project(ed)?;
    let n = a.need_num("scene")? as u32;
    let st = crew::stage(&p, n, Some(&ed.project));
    let rid = a.str("role")?.unwrap_or_else(|| st.roles().0.to_string());
    let r = crew::role(&rid).ok_or_else(|| format!("нет роли «{rid}»"))?;
    let mut brief = crew::brief(&p, r.id, n, st);
    if let Some(t) = a.str("task")?.filter(|t| !t.trim().is_empty()) {
        brief = format!("**Задание режиссёра:** {t}\n\n{brief}");
    }
    Ok((p, r, n, st, brief))
}
