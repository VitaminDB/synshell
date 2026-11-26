//! Навыки монтажа (E3): сценарии Rhai `library/skills/*.rhai` (встроенные) и
//! свои в `~/.config/syn-video-editor/skills/*.rhai` (с тем же именем —
//! заменяют встроенный). Заголовок: `//! name`, `//! title`, `//! description`,
//! `//! params`. Навык выполняется как `script.run` с переменной `params`:
//! одним шагом отмены, ошибка откатывает его целиком.

use serde_json::{json, Value};

use super::{arg, req, Args, Command, Out, Res, Ty};
use crate::editor::Editor;
use crate::script::header;

pub static COMMANDS: &[&Command] = &[&LIST, &RUN];

const BUILTIN: &[(&str, &str)] = &[
    ("reels", include_str!("../../library/skills/reels.rhai")),
    ("talking_head", include_str!("../../library/skills/talking_head.rhai")),
    ("podcast", include_str!("../../library/skills/podcast.rhai")),
    ("beat_clip", include_str!("../../library/skills/beat_clip.rhai")),
];

pub struct Skill {
    pub name: String,
    pub title: String,
    pub description: String,
    pub params: String,
    pub code: String,
    pub path: Option<std::path::PathBuf>,
}

/// Встроенный ли сценарий (есть в программе).
pub fn is_builtin(name: &str) -> bool {
    BUILTIN.iter().any(|(n, _)| *n == name)
}

pub fn dir() -> std::path::PathBuf {
    crate::layout::config_dir().join("skills")
}

fn parse(name: &str, code: &str, path: Option<std::path::PathBuf>) -> Skill {
    Skill {
        name: header(code, "name").unwrap_or_else(|| name.to_string()),
        title: header(code, "title").unwrap_or_else(|| name.to_string()),
        description: header(code, "description").unwrap_or_default(),
        params: header(code, "params").unwrap_or_default(),
        code: code.to_string(),
        path,
    }
}

pub fn all() -> Vec<Skill> {
    let mut v: Vec<Skill> = BUILTIN.iter().map(|(n, c)| parse(n, c, None)).collect();
    if let Ok(rd) = std::fs::read_dir(dir()) {
        let mut files: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "rhai")).collect();
        files.sort();
        for f in files {
            let Ok(code) = std::fs::read_to_string(&f) else { continue };
            let stem = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let s = parse(&stem, &code, Some(f));
            v.retain(|x| x.name != s.name);
            v.push(s);
        }
    }
    v
}

static LIST: Command = Command {
    id: "skill.list",
    category: "ИИ",
    title: "Навыки монтажа",
    shortcut: "",
    keywords: "навыки skills reels shorts говорящая голова подкаст клип под музыку бит шаблон монтажа",
    help: "Готовые навыки монтажа (сценарии Rhai, можно читать и добавлять свои в ~/.config/syn-video-editor/skills): что делают и какие параметры. Запуск — skill.run.",
    args: &[],
    danger: false,
    run: list,
};

/// Включённые сценарии (выключенные — в «Настройки → Скилы»).
fn enabled() -> Vec<Skill> {
    let set = crate::settings::load();
    all().into_iter().filter(|s| set.skill_on(&s.name)).collect()
}

fn list(_ed: &mut Editor, _a: &Args) -> Res {
    let v = enabled();
    let lines: Vec<String> = v.iter().map(|s| format!("{} — {}: {}{}", s.name, s.title, s.description, if s.params.is_empty() { String::new() } else { format!(" | параметры: {}", s.params) })).collect();
    let data: Vec<Value> = v.iter().map(|s| json!({"name": s.name, "title": s.title, "description": s.description, "params": s.params, "path": s.path.as_ref().map(|p| p.display().to_string())})).collect();
    Ok(Out::with(lines.join("\n"), Value::Array(data)))
}

static RUN: Command = Command {
    id: "skill.run",
    category: "ИИ",
    title: "Выполнить навык монтажа",
    shortcut: "",
    keywords: "навык skill reels shorts вертикальный говорящая голова подкаст клип под музыку по битам",
    help: "Выполнить навык монтажа (skill.list): reels — вертикальный ролик 9:16 с вырезанными паузами и крупными субтитрами; talking_head — говорящая голова: паузы, наезды на чередующихся кусках, субтитры; podcast — подкаст: паузы, подписи дикторов; beat_clip — клип под музыку: нарезка роликов по ударам трека. params — объект параметров навыка. Всё — одним шагом отмены.",
    args: &[req("name", Ty::Str, "навык"), arg("params", Ty::Object, "параметры навыка")],
    danger: false,
    run,
};

fn run(ed: &mut Editor, a: &Args) -> Res {
    let name = a.need_str("name")?;
    let s = enabled().into_iter().find(|s| s.name == name.trim()).ok_or_else(|| format!("навыка «{name}» нет или он выключен в настройках — skill.list"))?;
    let params = a.raw("params").cloned().unwrap_or(Value::Null);
    let own = !ed.in_group();
    let snapshot = (!own).then(|| (ed.project.clone(), ed.selection.clone()));
    if own {
        ed.begin_group(&format!("Навык: {}", s.title));
    }
    match crate::script::run_with(ed, &s.code, params) {
        Ok(t) => {
            if own {
                ed.end_group();
            }
            Ok(Out::text(format!("Навык «{}»:\n{t}", s.title)))
        }
        Err(e) => {
            if own {
                ed.abort_group();
            } else if let Some((p, sel)) = snapshot {
                if ed.project != p {
                    ed.begin("Откат навыка");
                    ed.project = p;
                    ed.selection = sel;
                }
            }
            Err(format!("навык «{}»: {e}", s.title))
        }
    }
}
