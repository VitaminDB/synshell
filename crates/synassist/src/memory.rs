//! Память ассистента — как у Claude Code: каталог `.md`-файлов, один факт —
//! один файл с frontmatter (`name`, `description`, `type`), и индекс
//! `MEMORY.md` (строка на память), который целиком идёт в системный промпт.
//! Файлы можно править руками; ассистент пишет их инструментом `memory`.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::tools::{need_str, str_arg, Danger, Tool, ToolOutput, ToolResult};

const INDEX: &str = "MEMORY.md";
/// Сколько индекса показывать в промпте (символов).
const INDEX_LIMIT: usize = 6000;
const TYPES: [&str; 4] = ["user", "feedback", "project", "reference"];

/// Каталог памяти из настройки (`~` раскрывается); пусто — по умолчанию.
pub fn resolve(setting: &str) -> PathBuf {
    if setting.trim().is_empty() {
        return crate::chat::data_dir().join("memory");
    }
    crate::builtin::expand(setting)
}

/// Имя файла памяти: латиница, цифры и дефисы.
fn slug(name: &str) -> String {
    let mut s: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    s.trim_matches('-').chars().take(60).collect()
}

fn file_of(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{}.md", slug(name)))
}

/// Раздел системного промпта: правила и индекс.
pub fn prompt_section(dir: &Path) -> String {
    let index = std::fs::read_to_string(dir.join(INDEX)).unwrap_or_default();
    let mut s = String::from("\n\n## Память\n");
    s.push_str(&format!("У тебя есть постоянная память между разговорами — каталог {} (инструмент memory). ", dir.display()));
    s.push_str("Сохраняй (memory save) то, что пригодится в будущих разговорах и не следует из самих файлов: кто пользователь и как он любит работать (user), его указания и поправки к твоей работе с причиной (feedback), текущие дела и договорённости с абсолютными датами (project), ссылки на внешние ресурсы (reference). ");
    s.push_str("Один факт — одна память; прежде чем создать новую, проверь индекс и обнови существующую (тот же name). Не сохраняй разовое, секреты и то, что легко узнать заново. Если память оказалась неверной — исправь или забудь её (memory forget). ");
    s.push_str("Когда пользователь просит запомнить или забыть — сделай это сразу. Подробности памяти читай memory read по name.\n");
    if index.trim().is_empty() {
        s.push_str("Индекс памяти пока пуст.\n");
    } else {
        s.push_str("Индекс памяти (MEMORY.md):\n");
        s.push_str(&crate::tools::truncate(index.trim(), INDEX_LIMIT));
        s.push('\n');
    }
    s
}

/// Строки индекса без строки этой памяти.
fn index_without(dir: &Path, file: &str) -> Vec<String> {
    let index = std::fs::read_to_string(dir.join(INDEX)).unwrap_or_default();
    index.lines().filter(|l| !l.contains(&format!("({file})"))).map(str::to_string).collect()
}

fn write_index(dir: &Path, lines: &[String]) -> std::io::Result<()> {
    let mut body = lines.join("\n");
    if !body.is_empty() {
        body.push('\n');
    }
    std::fs::write(dir.join(INDEX), body)
}

pub fn save(dir: &Path, name: &str, description: &str, kind: &str, content: &str) -> Result<PathBuf, String> {
    let slug = slug(name);
    if slug.is_empty() {
        return Err("пустое имя памяти".into());
    }
    let kind = if TYPES.contains(&kind) { kind } else { "project" };
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = file_of(dir, &slug);
    let description = description.lines().next().unwrap_or("").trim();
    let body = format!("---\nname: {slug}\ndescription: {description}\nmetadata:\n  type: {kind}\n---\n\n{}\n", content.trim());
    std::fs::write(&path, body).map_err(|e| format!("{}: {e}", path.display()))?;
    let file = format!("{slug}.md");
    let mut lines = index_without(dir, &file);
    lines.push(format!("- [{slug}]({file}) — {description}"));
    write_index(dir, &lines).map_err(|e| e.to_string())?;
    Ok(path)
}

pub fn read(dir: &Path, name: &str) -> Result<String, String> {
    let path = file_of(dir, name);
    std::fs::read_to_string(&path).map_err(|_| format!("памяти «{}» нет", slug(name)))
}

pub fn forget(dir: &Path, name: &str) -> Result<(), String> {
    let slug = slug(name);
    let path = file_of(dir, &slug);
    let existed = std::fs::remove_file(&path).is_ok();
    let file = format!("{slug}.md");
    let before = std::fs::read_to_string(dir.join(INDEX)).unwrap_or_default();
    let lines = index_without(dir, &file);
    let changed = before.lines().count() != lines.len();
    if changed {
        write_index(dir, &lines).map_err(|e| e.to_string())?;
    }
    if existed || changed {
        Ok(())
    } else {
        Err(format!("памяти «{slug}» нет"))
    }
}

pub struct MemoryTool {
    pub dir: PathBuf,
}

impl Tool for MemoryTool {
    fn name(&self) -> &'static str {
        "memory"
    }
    fn group(&self) -> &'static str {
        "память"
    }
    fn description(&self) -> String {
        "Постоянная память между разговорами. action: save — сохранить или обновить память (name, description — одна строка для индекса, type: user|feedback|project|reference, content — сам факт; для feedback и project добавь строки «Почему:» и «Как применять:»); read — прочитать память по name; forget — удалить по name; list — индекс.".into()
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["save", "read", "forget", "list"] },
                "name": { "type": "string", "description": "короткое имя-слаг памяти" },
                "description": { "type": "string", "description": "одна строка для индекса" },
                "type": { "type": "string", "enum": TYPES },
                "content": { "type": "string" }
            },
            "required": ["action"]
        })
    }
    fn danger(&self, _args: &Value) -> Danger {
        Danger::Safe
    }
    fn summary(&self, args: &Value) -> String {
        let name = str_arg(args, "name").unwrap_or("");
        match str_arg(args, "action").unwrap_or("") {
            "save" => format!("Запоминаю: {name}"),
            "read" => format!("Вспоминаю: {name}"),
            "forget" => format!("Забываю: {name}"),
            _ => "Смотрю память".into(),
        }
    }
    fn run(&self, args: &Value) -> ToolResult {
        match need_str(args, "action")? {
            "save" => {
                let name = need_str(args, "name")?;
                let content = need_str(args, "content")?;
                let description = str_arg(args, "description").unwrap_or(name);
                let path = save(&self.dir, name, description, str_arg(args, "type").unwrap_or("project"), content)?;
                Ok(ToolOutput::text(format!("Сохранено: {}", path.display())))
            }
            "read" => read(&self.dir, need_str(args, "name")?).map(ToolOutput::text),
            "forget" => forget(&self.dir, need_str(args, "name")?).map(|_| ToolOutput::text("Забыто.")),
            "list" => {
                let index = std::fs::read_to_string(self.dir.join(INDEX)).unwrap_or_default();
                Ok(ToolOutput::text(if index.trim().is_empty() { "Память пуста.".into() } else { index }))
            }
            other => Err(format!("неизвестное действие «{other}»")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_read_forget() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        save(d, "Любимый редактор", "редактор пользователя — helix", "user", "Пользователь пишет код в helix.").unwrap();
        save(d, "любимый-редактор", "редактор пользователя — neovim", "user", "Теперь neovim.").unwrap();
        let index = std::fs::read_to_string(d.join(INDEX)).unwrap();
        assert_eq!(index.lines().count(), 1, "обновление, а не дубль: {index}");
        assert!(index.contains("neovim"));
        assert!(read(d, "Любимый редактор").unwrap().contains("Теперь neovim."));
        assert!(prompt_section(d).contains("любимый-редактор"));
        forget(d, "любимый-редактор").unwrap();
        assert!(read(d, "любимый-редактор").is_err());
        assert_eq!(std::fs::read_to_string(d.join(INDEX)).unwrap().trim(), "");
    }
}
