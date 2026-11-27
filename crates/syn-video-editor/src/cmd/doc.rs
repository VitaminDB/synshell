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
    let mut msg = format!("Документ {} ({} строк)", p.display(), body.lines().count());
    let thin = thin_scenes(&body);
    if !thin.is_empty() {
        msg.push_str(&format!(
            ". Сцены {} описаны неполно — допиши каждую по шаблону навыка screenplay (поля: Задача, План и камера, Действие, Эмоции, Звук, Реплики) и перезапиши документ",
            thin.join(", ")
        ));
    }
    Ok(Out::with(msg, json!({"path": p, "thin_scenes": thin})))
}

/// Разделы сцен («### Сцена N …») без обязательных полей сценария.
pub fn thin_scenes(text: &str) -> Vec<String> {
    const FIELDS: &[&str] = &["план", "действие", "звук", "реплик"];
    let mut out = Vec::new();
    let mut cur: Option<(String, String)> = None;
    let mut check = |c: Option<(String, String)>, out: &mut Vec<String>| {
        if let Some((name, body)) = c {
            let low = body.to_lowercase();
            if FIELDS.iter().filter(|f| low.contains(*f)).count() < 3 {
                out.push(name);
            }
        }
    };
    for line in text.lines() {
        let t = line.trim_start_matches('#').trim();
        if line.starts_with('#') && t.to_lowercase().starts_with("сцена ") {
            check(cur.take(), &mut out);
            let n: String = t[t.find(' ').map(|i| i + 1).unwrap_or(0)..].chars().take_while(|c| c.is_ascii_digit()).collect();
            cur = Some((if n.is_empty() { t.chars().take(20).collect() } else { n }, String::new()));
        } else if line.starts_with('#') {
            check(cur.take(), &mut out);
        } else if let Some((_, b)) = cur.as_mut() {
            b.push_str(line);
            b.push('\n');
        }
    }
    check(cur, &mut out);
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn thin() {
        let t = "# X\n## Сцены\n### Сцена 1 — А\n- Задача: x\n### Сцена 2 — Б\n- План и камера: x\n- Действие: y\n- Звук: z\n- Реплики: w\n";
        assert_eq!(super::thin_scenes(t), vec!["1".to_string()]);
    }
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
