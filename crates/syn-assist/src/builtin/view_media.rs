//! `view_media`: показать модели картинку или видео с диска. Файлы уходят
//! в результате инструмента; агент прикладывает их к сообщению как медиа
//! (vision-башня модели видит содержимое), остальное — описанием.

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::tools::{str_arg, Danger, Tool, ToolOutput, ToolResult};

/// Не больше файлов за вызов.
pub const MAX_FILES: usize = 8;

pub struct ViewMediaTool;

fn paths(args: &Value) -> Vec<String> {
    let mut v: Vec<String> = args.get("paths").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
    if let Some(p) = str_arg(args, "path") {
        v.insert(0, p.to_string());
    }
    v.dedup();
    v
}

fn resolve(p: &str) -> PathBuf {
    super::expand(p.trim().strip_prefix("file://").unwrap_or(p.trim()))
}

impl Tool for ViewMediaTool {
    fn name(&self) -> &'static str {
        "view_media"
    }
    fn group(&self) -> &'static str {
        "файлы"
    }
    fn description(&self) -> String {
        format!("Посмотреть изображения или видео с диска: файлы прикладываются к ответу инструмента, и ты видишь их содержимое (если модель понимает картинки). path — один файл, paths — до {MAX_FILES}. Для каталога сначала найди файлы (bash find / files list).")
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "путь к файлу (~ можно)" },
                "paths": { "type": "array", "items": { "type": "string" }, "description": "несколько файлов" }
            }
        })
    }
    fn danger(&self, _: &Value) -> Danger {
        Danger::Safe
    }
    fn summary(&self, args: &Value) -> String {
        format!("смотреть {}", paths(args).join(", "))
    }
    fn run(&self, args: &Value) -> ToolResult {
        let asked = paths(args);
        if asked.is_empty() {
            return Err("нужен path или paths".into());
        }
        let mut files = Vec::new();
        let mut lines = Vec::new();
        for p in asked.iter().take(MAX_FILES) {
            let f = resolve(p);
            match std::fs::metadata(&f) {
                Ok(m) if m.is_dir() => lines.push(format!("- {} — это каталог: найди в нём файлы (find) и передай их", f.display())),
                Ok(m) => {
                    let shown = !crate::attach::media_of(std::slice::from_ref(&f)).is_empty();
                    lines.push(format!("- {} — {} КБ{}", f.display(), m.len() / 1024, if shown { ", показан" } else { ", не картинка и не видео — прочитай его другим инструментом" }));
                    if shown {
                        files.push(f);
                    }
                }
                Err(e) => lines.push(format!("- {} — нет файла: {e}", f.display())),
            }
        }
        if asked.len() > MAX_FILES {
            lines.push(format!("… показаны первые {MAX_FILES}"));
        }
        Ok(ToolOutput { text: format!("view_media: показано {} из {}.\n{}", files.len(), asked.len(), lines.join("\n")), files })
    }
}
