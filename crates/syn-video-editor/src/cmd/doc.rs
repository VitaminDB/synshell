//! Документы проекта — Markdown-файлы в его папке (сценарий, диалоги,
//! заметки): агент пишет и перечитывает их, пользователь открывает двойным
//! щелчком в медиатеке и правит.

use std::path::{Path, PathBuf};

use serde_json::json;

use super::{arg, req, Args, Command as Cmd, Out, Res, Ty};
use crate::editor::Editor;

pub static COMMANDS: &[&Cmd] = &[&WRITE, &READ, &LIST];

/// Папка документов проекта (папка проекта; без файла — нет).
pub fn dir(project: Option<&Path>) -> Option<PathBuf> {
    project.map(crate::store::media_dir)
}

/// Документы проекта: `*.md` в его папке, по имени.
pub fn list_docs(project: Option<&Path>) -> Vec<PathBuf> {
    let Some(d) = dir(project) else { return Vec::new() };
    let mut v: Vec<PathBuf> = std::fs::read_dir(&d)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("md"))).collect())
        .unwrap_or_default();
    v.sort();
    v
}

fn doc_path(ed: &Editor, name: &str) -> Result<PathBuf, String> {
    let d = dir(ed.path.as_deref()).ok_or("проект ещё не сохранён — у него нет папки")?;
    let stem: String = name.trim().trim_end_matches(".md").chars().map(|c| if matches!(c, '/' | '\\' | '\0') { '-' } else { c }).collect();
    if stem.trim().is_empty() {
        return Err("пустое имя документа".into());
    }
    Ok(d.join(format!("{}.md", stem.trim())))
}

static WRITE: Cmd = Cmd {
    id: "doc.write",
    category: "Проект",
    title: "Записать документ проекта",
    shortcut: "",
    keywords: "doc document документ сценарий диалоги заметки markdown записать",
    help: "Записать Markdown-документ в папку проекта (`<name>.md`): сценарий, диалоги по сценам, заметки. Пользователь видит его в медиатеке («Документы») и может править — перед следующими шагами перечитай doc.read. `append` — дописать в конец.",
    args: &[req("name", Ty::Str, "имя документа (без .md)"), req("text", Ty::Str, "содержимое (Markdown)"), arg("append", Ty::Bool, "дописать в конец")],
    danger: false,
    run: write,
};

static READ: Cmd = Cmd {
    id: "doc.read",
    category: "Проект",
    title: "Прочитать документ проекта",
    shortcut: "",
    keywords: "doc read прочитать документ сценарий диалоги",
    help: "Прочитать Markdown-документ из папки проекта (с правками пользователя).",
    args: &[req("name", Ty::Str, "имя документа (без .md)")],
    danger: false,
    run: read,
};

static LIST: Cmd = Cmd {
    id: "doc.list",
    category: "Проект",
    title: "Документы проекта",
    shortcut: "",
    keywords: "doc list документы проекта",
    help: "Список Markdown-документов в папке проекта.",
    args: &[],
    danger: false,
    run: list,
};

fn write(ed: &mut Editor, a: &Args) -> Res {
    let p = doc_path(ed, &a.need_str("name")?)?;
    let text = crate::yo::yofy(&a.need_str("text")?);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let body = if a.bool("append")?.unwrap_or(false) {
        let old = std::fs::read_to_string(&p).unwrap_or_default();
        if old.is_empty() { text } else { format!("{}\n{text}", old.trim_end()) }
    } else {
        text
    };
    std::fs::write(&p, &body).map_err(|e| format!("{}: {e}", p.display()))?;
    ed.revision += 1;
    Ok(Out::with(format!("Документ {} ({} строк)", p.display(), body.lines().count()), json!({"path": p})))
}

fn read(ed: &mut Editor, a: &Args) -> Res {
    let p = doc_path(ed, &a.need_str("name")?)?;
    let t = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(Out::with(t, json!({"path": p})))
}

fn list(ed: &mut Editor, _a: &Args) -> Res {
    let v = list_docs(ed.path.as_deref());
    if v.is_empty() {
        return Ok(Out::text("Документов нет — doc.write".to_string()));
    }
    let names: Vec<String> = v.iter().filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned())).collect();
    Ok(Out::with(names.join("\n"), json!({"docs": v})))
}
