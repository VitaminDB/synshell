use std::path::Path;

use serde_json::{json, Value};

use super::expand;
use crate::tools::{need_str, str_arg, truncate, Danger, Tool, ToolOutput, ToolResult};

pub struct FilesTool;

const READ_LIMIT: usize = 20_000;

fn human(b: u64) -> String {
    if b >= 1 << 30 {
        format!("{:.1} ГБ", b as f64 / (1u64 << 30) as f64)
    } else if b >= 1 << 20 {
        format!("{:.1} МБ", b as f64 / (1u64 << 20) as f64)
    } else if b >= 1 << 10 {
        format!("{:.0} КБ", b as f64 / 1024.0)
    } else {
        format!("{b} Б")
    }
}

fn list(path: &Path) -> ToolResult {
    let mut items: Vec<(bool, String, u64)> = std::fs::read_dir(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            let meta = e.metadata().ok()?;
            Some((meta.is_dir(), name, meta.len()))
        })
        .collect();
    items.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.to_lowercase().cmp(&b.1.to_lowercase())));
    let total = items.len();
    let lines: Vec<String> = items.iter().take(200).map(|(d, n, s)| if *d { format!("{n}/") } else { format!("{n} ({})", human(*s)) }).collect();
    let mut out = format!("{} — {} элементов\n{}", path.display(), total, lines.join("\n"));
    if total > 200 {
        out.push_str(&format!("\n… и ещё {}", total - 200));
    }
    Ok(ToolOutput::text(out))
}

fn read(path: &Path) -> ToolResult {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.is_dir() {
        return list(path);
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.iter().take(8000).any(|b| *b == 0) {
        return Ok(ToolOutput { text: format!("{} — двоичный файл, {}", path.display(), human(meta.len())), files: vec![path.to_path_buf()] });
    }
    Ok(ToolOutput::text(truncate(&String::from_utf8_lossy(&bytes), READ_LIMIT)))
}

fn search(root: &Path, query: &str) -> ToolResult {
    let q = query.to_lowercase();
    let mut found: Vec<String> = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut visited = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            visited += 1;
            if visited > 200_000 || found.len() >= 50 {
                break;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let p = e.path();
            if name.to_lowercase().contains(&q) {
                found.push(p.display().to_string());
            }
            if depth < 8 && e.file_type().is_ok_and(|t| t.is_dir()) {
                stack.push((p, depth + 1));
            }
        }
    }
    Ok(ToolOutput::text(if found.is_empty() { format!("В {} ничего не найдено по «{query}»", root.display()) } else { found.join("\n") }))
}

impl Tool for FilesTool {
    fn name(&self) -> &'static str {
        "files"
    }
    fn group(&self) -> &'static str {
        "файлы"
    }
    fn description(&self) -> String {
        "Файлы и папки пользователя. Пути абсолютные или от домашней папки (~). action: list (path), read (path — текст до 20 тыс. символов), search (query — часть имени, path — где искать, по умолчанию ~), write (path, content — создать или перезаписать), append (path, content), mkdir (path), move (path, to), delete (path — в корзину не попадает), open (path — открыть программой по умолчанию).".into()
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["list", "read", "search", "write", "append", "mkdir", "move", "delete", "open"]},
                "path": {"type": "string"},
                "query": {"type": "string"},
                "content": {"type": "string"},
                "to": {"type": "string"}
            },
            "required": ["action"]
        })
    }
    fn danger(&self, args: &Value) -> Danger {
        match str_arg(args, "action") {
            Some("write" | "append" | "move" | "delete") => Danger::Confirm,
            _ => Danger::Safe,
        }
    }
    fn summary(&self, args: &Value) -> String {
        let p = str_arg(args, "path").unwrap_or("~");
        match str_arg(args, "action").unwrap_or("?") {
            "list" => format!("Папка {p}"),
            "read" => format!("Прочитать {p}"),
            "search" => format!("Найти «{}» в {p}", str_arg(args, "query").unwrap_or("")),
            "write" => format!("Записать {p}"),
            "append" => format!("Дописать в {p}"),
            "mkdir" => format!("Создать папку {p}"),
            "move" => format!("Переместить {p} → {}", str_arg(args, "to").unwrap_or("?")),
            "delete" => format!("Удалить {p}"),
            "open" => format!("Открыть {p}"),
            other => other.to_string(),
        }
    }
    fn run(&self, args: &Value) -> ToolResult {
        let action = need_str(args, "action")?;
        let path = expand(str_arg(args, "path").unwrap_or("~"));
        match action {
            "list" => list(&path),
            "read" => read(&path),
            "search" => search(&path, need_str(args, "query")?),
            "write" | "append" => {
                let content = args.get("content").and_then(Value::as_str).unwrap_or("");
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                }
                if action == "write" {
                    std::fs::write(&path, content).map_err(|e| e.to_string())?;
                } else {
                    use std::io::Write;
                    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path).map_err(|e| e.to_string())?;
                    f.write_all(content.as_bytes()).map_err(|e| e.to_string())?;
                }
                Ok(ToolOutput { text: format!("Записано {} ({} символов)", path.display(), content.chars().count()), files: vec![path] })
            }
            "mkdir" => {
                std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
                Ok(ToolOutput::text(format!("Создана папка {}", path.display())))
            }
            "move" => {
                let to = expand(need_str(args, "to")?);
                let to = if to.is_dir() { to.join(path.file_name().ok_or("нет имени файла")?) } else { to };
                std::fs::rename(&path, &to).map_err(|e| e.to_string())?;
                Ok(ToolOutput::text(format!("Перемещено в {}", to.display())))
            }
            "delete" => {
                if path == super::home() || path.as_os_str().len() < 3 {
                    return Err("эту папку удалять нельзя".into());
                }
                let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
                if meta.is_dir() {
                    std::fs::remove_dir_all(&path).map_err(|e| e.to_string())?;
                } else {
                    std::fs::remove_file(&path).map_err(|e| e.to_string())?;
                }
                Ok(ToolOutput::text(format!("Удалено {}", path.display())))
            }
            "open" => {
                std::process::Command::new("xdg-open").arg(&path).spawn().map_err(|e| e.to_string())?;
                Ok(ToolOutput { text: format!("Открыто {}", path.display()), files: vec![path] })
            }
            other => Err(format!("неизвестное действие {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_read_search_delete() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("notes/план.txt");
        let t = FilesTool;
        let path = p.to_str().unwrap();
        t.run(&json!({"action":"write","path":path,"content":"купить хлеб"})).unwrap();
        assert_eq!(t.run(&json!({"action":"read","path":path})).unwrap().text, "купить хлеб");
        let found = t.run(&json!({"action":"search","path":dir.path().to_str().unwrap(),"query":"ПЛАН"})).unwrap().text;
        assert!(found.contains("план.txt"));
        assert_eq!(t.danger(&json!({"action":"delete"})), Danger::Confirm);
        t.run(&json!({"action":"delete","path":path})).unwrap();
        assert!(!p.exists());
    }
}
