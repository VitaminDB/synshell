//! Навыки — как skills у Claude Code: папка на навык с `SKILL.md`
//! (frontmatter `name`, `description`, дальше — инструкции). Модели видны
//! только описания (в описании инструмента `autoskill`); полный текст она
//! подгружает им же, когда задача подходит под навык. Встроенные навыки
//! (`models`, `nodes`) собираются из живой системы — [`crate::autoguide`].

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::tools::{need_str, Danger, Tool, ToolOutput, ToolResult};

const SKILL_FILE: &str = "SKILL.md";

#[derive(Clone, Debug, PartialEq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub dir: PathBuf,
}

/// Каталог навыков из настройки; пусто — по умолчанию.
pub fn resolve(setting: &str) -> PathBuf {
    if setting.trim().is_empty() {
        return crate::chat::data_dir().join("skills");
    }
    crate::builtin::expand(setting)
}

/// Поле frontmatter (`key: value`) в начале файла между `---`.
pub fn front(text: &str, key: &str) -> Option<String> {
    let rest = text.strip_prefix("---")?;
    let end = rest.find("\n---")?;
    rest[..end].lines().find_map(|l| l.trim().strip_prefix(&format!("{key}:")).map(|v| v.trim().trim_matches('"').to_string()))
}

/// Тело после frontmatter.
fn body(text: &str) -> &str {
    text.strip_prefix("---").and_then(|r| r.find("\n---").map(|e| &r[e + 4..])).unwrap_or(text).trim_start_matches(['\r', '\n'])
}

pub fn list(dir: &Path) -> Vec<Skill> {
    let mut out: Vec<Skill> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let d = e.path();
            let text = std::fs::read_to_string(d.join(SKILL_FILE)).ok()?;
            let folder = d.file_name()?.to_string_lossy().into_owned();
            let name = front(&text, "name").filter(|n| !n.is_empty()).unwrap_or(folder);
            let description = front(&text, "description").unwrap_or_default();
            Some(Skill { name, description, dir: d })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Навыки из нескольких каталогов: одноимённый из более позднего каталога
/// заменяет ранний; отключённые (по имени) пропускаются.
pub fn list_all(dirs: &[PathBuf], disabled: &[String]) -> Vec<Skill> {
    let mut out: Vec<Skill> = Vec::new();
    for d in dirs {
        for k in list(d) {
            match out.iter_mut().find(|x| x.name == k.name) {
                Some(x) => *x = k,
                None => out.push(k),
            }
        }
    }
    out.retain(|k| !disabled.iter().any(|d| d == &k.name));
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Раздел системного промпта: как пользоваться навыками и пулом.
pub fn prompt_section(_skills: &[Skill]) -> String {
    "\n\n## Навыки и инструменты по запросу\nНавыки — инструкции и справка под задачу: если задача подходит под навык, сначала загрузи его инструментом autoskill (name) и следуй ему. Встроенные навыки models (нейросети системы и их параметры) и nodes (ноды графов) собираются из системы сейчас — читай их, а не угадывай id моделей и поля. Часть инструментов (генерация медиа, сценарии) не объявлена сразу: их список — в описании autotools; перед первым вызовом загрузи нужный инструментом autotools (id) — получишь его параметры, установленные модели и правила.\n".into()
}

pub struct SkillTool {
    pub dirs: Vec<PathBuf>,
    /// Отключённые навыки (по имени).
    pub disabled: Vec<String>,
    /// Встроенные навыки системы (`models`, `nodes`) — для оболочки synshell.
    pub system: bool,
}

impl SkillTool {
    pub fn new(dir: PathBuf) -> Self {
        Self { dirs: vec![dir], disabled: Vec::new(), system: true }
    }

    fn skills(&self) -> Vec<Skill> {
        list_all(&self.dirs, &self.disabled)
    }

    fn system_skills(&self) -> Vec<(&'static str, &'static str, fn() -> String)> {
        if self.system {
            crate::autoguide::skills().to_vec()
        } else {
            Vec::new()
        }
    }
}

impl Tool for SkillTool {
    fn name(&self) -> &'static str {
        "autoskill"
    }
    fn group(&self) -> &'static str {
        "навыки"
    }
    fn description(&self) -> String {
        let mut s = String::from("Загрузить навык по имени: полные инструкции (и список его файлов — читай их инструментом files). Навыки (имя — когда загружать):\n");
        for (name, when, _) in self.system_skills() {
            s.push_str(&format!("- {name} — {when}\n"));
        }
        for k in self.skills() {
            s.push_str(&format!("- {} — {}\n", k.name, k.description));
        }
        s
    }
    fn parameters(&self) -> Value {
        let mut names: Vec<String> = self.system_skills().iter().map(|(n, _, _)| n.to_string()).collect();
        names.extend(self.skills().into_iter().map(|k| k.name));
        json!({
            "type": "object",
            "properties": { "name": { "type": "string", "enum": names, "description": "имя навыка" } },
            "required": ["name"]
        })
    }
    fn danger(&self, _args: &Value) -> Danger {
        Danger::Safe
    }
    fn summary(&self, args: &Value) -> String {
        format!("Навык: {}", crate::tools::str_arg(args, "name").unwrap_or(""))
    }
    fn run(&self, args: &Value) -> ToolResult {
        let name = need_str(args, "name")?;
        if let Some((n, _, make)) = self.system_skills().into_iter().find(|(n, _, _)| n.eq_ignore_ascii_case(name)) {
            return Ok(ToolOutput::text(format!("Навык «{n}» (собран из системы сейчас):\n\n{}", make())));
        }
        let skills = self.skills();
        let k = skills.iter().find(|k| k.name.eq_ignore_ascii_case(name)).ok_or_else(|| {
            format!("навыка «{name}» нет; есть: {}", skills.iter().map(|k| k.name.as_str()).collect::<Vec<_>>().join(", "))
        })?;
        let text = std::fs::read_to_string(k.dir.join(SKILL_FILE)).map_err(|e| e.to_string())?;
        let mut out = format!("Навык «{}» (папка {}):\n\n{}", k.name, k.dir.display(), body(&text));
        let files: Vec<String> = std::fs::read_dir(&k.dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.file_name().is_some_and(|n| n != SKILL_FILE))
            .map(|p| p.display().to_string())
            .collect();
        if !files.is_empty() {
            out.push_str(&format!("\n\nФайлы навыка:\n{}", files.join("\n")));
        }
        Ok(ToolOutput::text(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_and_loads() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().join("отчёт");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(SKILL_FILE), "---\nname: weekly-report\ndescription: недельный отчёт по задачам\n---\n\n1. Собери задачи.\n").unwrap();
        std::fs::write(d.join("шаблон.md"), "шаблон").unwrap();
        let skills = list(dir.path());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "weekly-report");
        assert!(SkillTool::new(dir.path().into()).description().contains("weekly-report — недельный отчёт"));
        let out = SkillTool::new(dir.path().into()).run(&json!({"name": "weekly-report"})).unwrap();
        assert!(out.text.contains("1. Собери задачи."));
        assert!(out.text.contains("шаблон.md"));
    }
}
