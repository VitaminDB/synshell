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
    ("bright_montage", include_str!("../../library/skills/bright_montage.rhai")),
    ("cinematic_montage", include_str!("../../library/skills/cinematic_montage.rhai")),
    ("calm_montage", include_str!("../../library/skills/calm_montage.rhai")),
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
    help: "Выполнить навык монтажа (skill.list): reels — вертикальный ролик 9:16 с вырезанными паузами и крупными субтитрами; talking_head — говорящая голова: паузы, наезды на чередующихся кусках, субтитры; podcast — подкаст: паузы, подписи дикторов; beat_clip — клип под музыку: нарезка роликов по ударам трека; bright_montage — яркий промо-монтаж (Fusion-переходы, наезды, блики, свуши, титры шаблонами), cinematic_montage — кинематографичный (прямые склейки, наплывы, каше, зерно), calm_montage — спокойный (наплывы, нижние трети). params — объект параметров навыка. Всё — одним шагом отмены.",
    args: &[req("name", Ty::Str, "навык"), arg("params", Ty::Object, "параметры навыка")],
    danger: false,
    run,
};

/// Текущий год по системным часам.
fn this_year() -> i32 {
    let days = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0) as i64;
    // гражданская дата из числа дней (алгоритм Хиннанта)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2)) as i32
}

/// Год выпуска в надписи («© 2025», «Год 2025», «(c) 2025», «2025 г.») → `year`;
/// прочие числа (годы внутри сюжета) не трогаются.
pub fn current_year_in(text: &str, year: i32) -> String {
    let marks = ["©", "(c)", "(C)", "Год", "год", "Year", "year"];
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let is_year = i + 4 <= chars.len()
            && chars[i..i + 4].iter().all(|c| c.is_ascii_digit())
            && (chars[i] == '1' || chars[i] == '2')
            && chars.get(i + 4).is_none_or(|c| !c.is_ascii_digit())
            && (i == 0 || !chars[i - 1].is_ascii_digit());
        if is_year {
            let before: String = chars[..i].iter().collect::<String>();
            let after: String = chars[i + 4..].iter().take(3).collect();
            let b = before.trim_end().trim_end_matches([':', '—', '-']).trim_end();
            if marks.iter().any(|m| b.ends_with(m)) || after.trim_start().starts_with("г.") || after.trim_start().starts_with("год") {
                out.push_str(&year.to_string());
                i += 4;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn run(ed: &mut Editor, a: &Args) -> Res {
    let name = a.need_str("name")?;
    let s = enabled().into_iter().find(|s| s.name == name.trim()).ok_or_else(|| format!("навыка «{name}» нет или он выключен в настройках — skill.list"))?;
    let mut params = a.raw("params").cloned().unwrap_or(Value::Null);
    // год в надписях (© 2025, «Год 2025») — всегда текущий: модель берёт год из обучения
    if let Some(o) = params.as_object_mut() {
        for k in ["credits", "title", "subtitle", "cta"] {
            if let Some(t) = o.get(k).and_then(Value::as_str) {
                let fixed = current_year_in(t, this_year());
                o.insert(k.into(), Value::String(fixed));
            }
        }
    }
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

#[cfg(test)]
mod year_tests {
    #[test]
    fn release_year_is_current() {
        assert_eq!(super::current_year_in("«Фильм» · Год 2025", 2026), "«Фильм» · Год 2026");
        assert_eq!(super::current_year_in("© 2024 Студия", 2026), "© 2026 Студия");
        assert_eq!(super::current_year_in("2025 г.", 2026), "2026 г.");
        // год внутри сюжета — не трогать
        assert_eq!(super::current_year_in("Лето 1985 года", 2026), "Лето 1985 года");
        assert!(super::this_year() >= 2026);
    }
}
