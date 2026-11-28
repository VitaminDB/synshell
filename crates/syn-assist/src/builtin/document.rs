//! `document`: читать документ (PDF) по частям через службу — обзор,
//! страницы «с–по» (текст и картинки по порядку, картинки — в ответе),
//! поиск. Для больших документов: вложение даёт обзор, остальное агент
//! дочитывает этим инструментом.

use serde_json::{json, Value};

use crate::tools::{str_arg, Danger, Tool, ToolOutput, ToolResult};

pub struct DocumentTool;

impl Tool for DocumentTool {
    fn name(&self) -> &'static str {
        "document"
    }
    fn group(&self) -> &'static str {
        "файлы"
    }
    fn description(&self) -> String {
        format!(
            "Читать документ (PDF) по частям: без pages и search — обзор (страниц, сканы, токены, оглавление); pages \"a-b\" — страницы по порядку, текст и картинки на своих местах (до {} страниц за вызов; ocr: true — распознать сканы текстом); search — найти слова и страницы. Для больших документов, которые во вложении показаны обзором.",
            crate::pdf::READ_PAGES
        )
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "путь к документу" },
                "pages": { "type": "string", "description": "страницы «a-b» или «a»" },
                "search": { "type": "string", "description": "что найти в тексте" },
                "ocr": { "type": "boolean", "description": "распознать страницы-сканы текстом (моделью со зрением)" }
            },
            "required": ["path"]
        })
    }
    fn danger(&self, _: &Value) -> Danger {
        Danger::Safe
    }
    fn numbered_series(&self) -> bool {
        true
    }
    fn summary(&self, args: &Value) -> String {
        let p = str_arg(args, "path").unwrap_or("");
        match (str_arg(args, "pages"), str_arg(args, "search")) {
            (Some(r), _) => format!("документ {p}: страницы {r}"),
            (_, Some(q)) => format!("документ {p}: поиск «{q}»"),
            _ => format!("документ {p}: обзор"),
        }
    }
    fn run(&self, args: &Value) -> ToolResult {
        let path = super::expand(str_arg(args, "path").ok_or("нужен path")?);
        if !path.is_file() {
            return Err(format!("{}: нет файла", path.display()));
        }
        if let Some(q) = str_arg(args, "search").filter(|q| !q.trim().is_empty()) {
            let r = crate::pdf::search(&path, q)?;
            if r.hits.is_empty() {
                return Ok(ToolOutput::text(format!("«{q}» в документе не найдено ({} стр.)", r.pages)));
            }
            let lines: Vec<String> = r.hits.iter().map(|(p, s)| format!("- стр. {p}: …{s}…")).collect();
            return Ok(ToolOutput::text(format!("Найдено «{q}» ({} мест, показаны первые):\n{}", lines.len(), lines.join("\n"))));
        }
        if let Some(range) = str_arg(args, "pages") {
            let (a, b) = crate::pdf::parse_range(range).ok_or("pages — «a-b» или «a»")?;
            let ocr = args.get("ocr").and_then(Value::as_bool).unwrap_or(false);
            let r = crate::pdf::read(&path, "", a, b, ocr)?;
            let mut media = Vec::new();
            let (text, skipped) = crate::pdf::render(&r.parts, &mut media);
            let last = r.parts.iter().filter_map(|p| if let synaptixd::DocPart::Page(n) = p { Some(*n) } else { None }).max().unwrap_or(a);
            let more = if last < r.pages { format!("\n[дальше — страницы {}–{} из {}]", last + 1, (last + crate::pdf::READ_PAGES).min(r.pages), r.pages) } else { String::new() };
            let note = if skipped > 0 { format!("\n[ещё {skipped} картинок — путями]") } else { String::new() };
            let files = media.into_iter().map(|m| std::path::PathBuf::from(m.path)).collect();
            return Ok(ToolOutput { text: format!("Документ {}, страницы {a}–{last} из {}:\n{text}{note}{more}", path.display(), r.pages), files });
        }
        let o = crate::pdf::overview(&path, "")?;
        let mut s = format!("Документ {}: {} стр., {} КБ, текст ≈{} токенов", path.display(), o.pages, o.bytes / 1024, o.tokens);
        if !o.scan_pages.is_empty() {
            s.push_str(&format!(", сканы: {}", crate::pdf::pages_list(&o.scan_pages)));
        }
        if o.images > 0 {
            s.push_str(&format!(", картинок {}", o.images));
        }
        s.push_str(".\n");
        for (p, h) in o.outline.iter().take(120) {
            s.push_str(&format!("- стр. {p}: {h}\n"));
        }
        Ok(ToolOutput::text(s))
    }
}
