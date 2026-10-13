//! Базы знаний — папки с документами: текст (и PDF) режется на фрагменты по
//! абзацам, фрагменты — в векторы моделью эмбеддингов службы (bge-m3).
//! Индекс папки — в `~/.local/share/synassist/kb/<хэш>.json` (фрагменты и
//! отпечатки файлов) и `.f32` (векторы подряд); обновляется по изменённым
//! файлам. Ассистент ищет инструментом `knowledge`: близкие фрагменты с файлом
//! и строками.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::tools::{need_str, str_arg, Danger, Tool, ToolOutput, ToolResult};

/// Целевой размер фрагмента, символов.
const CHUNK: usize = 900;
/// Файлы больше — пропускаются.
const MAX_FILE: u64 = 20 << 20;
const MAX_FILES: usize = 20_000;
const TOP_K: usize = 6;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Chunk {
    pub file: String,
    pub line: usize,
    pub end: usize,
    pub text: String,
}

#[derive(Serialize, Deserialize, Default)]
struct Meta {
    root: String,
    dim: usize,
    /// Отпечаток файла: «размер:mtime».
    files: HashMap<String, String>,
    chunks: Vec<Chunk>,
}

/// Состояние индекса для Параметров.
#[derive(Clone, Debug, Default)]
pub struct Status {
    pub files: usize,
    pub chunks: usize,
    pub indexed: bool,
}

fn index_dir() -> PathBuf {
    crate::chat::data_dir().join("kb")
}

fn key(root: &Path) -> String {
    let s = root.display().to_string();
    format!("{:016x}", s.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3)))
}

fn paths(root: &Path) -> (PathBuf, PathBuf) {
    let k = key(root);
    (index_dir().join(format!("{k}.json")), index_dir().join(format!("{k}.f32")))
}

/// Имя базы — имя папки.
pub fn name_of(root: &Path) -> String {
    root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| root.display().to_string())
}

fn indexable(p: &Path) -> bool {
    matches!(crate::attach::kind(p), crate::attach::Kind::Text | crate::attach::Kind::Pdf)
}

fn files_of(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let hidden = p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'));
            if hidden || matches!(p.file_name().and_then(|n| n.to_str()), Some("node_modules" | "target" | "__pycache__")) {
                continue;
            }
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(p),
                Ok(t) if t.is_file() && indexable(&p) => {
                    if e.metadata().map(|m| m.len() <= MAX_FILE).unwrap_or(false) {
                        out.push(p);
                    }
                }
                _ => {}
            }
            if out.len() >= MAX_FILES {
                return out;
            }
        }
    }
    out.sort();
    out
}

fn stamp(p: &Path) -> String {
    let m = std::fs::metadata(p).ok();
    let mtime = m.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
    format!("{}:{mtime}", m.map(|m| m.len()).unwrap_or(0))
}

fn read_text(p: &Path) -> Option<String> {
    match crate::attach::kind(p) {
        crate::attach::Kind::Pdf => {
            let out = std::process::Command::new("pdftotext").args(["-layout", "-enc", "UTF-8"]).arg(p).arg("-").output().ok()?;
            out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
        }
        _ => std::fs::read(p).ok().map(|b| String::from_utf8_lossy(&b).into_owned()),
    }
}

/// Фрагменты по абзацам (пустые строки), не длиннее ~`CHUNK` символов; с номерами строк.
pub fn chunk(file: &str, text: &str) -> Vec<Chunk> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut start = 1;
    let mut last = 0;
    let flush = |buf: &mut String, start: usize, end: usize, out: &mut Vec<Chunk>| {
        let t = buf.trim();
        if t.chars().count() >= 20 {
            out.push(Chunk { file: file.to_string(), line: start, end, text: t.to_string() });
        }
        buf.clear();
    };
    for (i, line) in text.lines().enumerate() {
        let n = i + 1;
        let para_end = line.trim().is_empty();
        if buf.is_empty() {
            if para_end {
                continue;
            }
            start = n;
        }
        // Слишком длинная строка — режется по символам.
        if line.chars().count() > CHUNK * 2 {
            flush(&mut buf, start, last.max(start), &mut out);
            let chars: Vec<char> = line.chars().collect();
            for part in chars.chunks(CHUNK) {
                buf.extend(part);
                flush(&mut buf, n, n, &mut out);
            }
            continue;
        }
        buf.push_str(line);
        buf.push('\n');
        last = n;
        if buf.chars().count() >= CHUNK && (para_end || buf.chars().count() >= CHUNK * 2) {
            flush(&mut buf, start, n, &mut out);
        }
    }
    flush(&mut buf, start, last.max(start), &mut out);
    out
}

fn load(root: &Path) -> Option<(Meta, Vec<f32>)> {
    let (meta, vecs) = paths(root);
    let meta: Meta = serde_json::from_str(&std::fs::read_to_string(meta).ok()?).ok()?;
    let raw = std::fs::read(vecs).ok()?;
    let v: Vec<f32> = raw.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    (meta.dim > 0 && v.len() == meta.dim * meta.chunks.len()).then_some((meta, v))
}

fn store(root: &Path, meta: &Meta, vecs: &[f32]) -> Result<(), String> {
    let (mp, vp) = paths(root);
    std::fs::create_dir_all(index_dir()).map_err(|e| e.to_string())?;
    std::fs::write(&mp, serde_json::to_string(meta).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let mut raw = Vec::with_capacity(vecs.len() * 4);
    for x in vecs {
        raw.extend_from_slice(&x.to_le_bytes());
    }
    std::fs::write(&vp, raw).map_err(|e| e.to_string())
}

pub fn status(root: &Path) -> Status {
    match load(root) {
        Some((m, _)) => Status { files: m.files.len(), chunks: m.chunks.len(), indexed: true },
        None => Status::default(),
    }
}

/// Обновить индекс папки: изменённые и новые файлы — заново, удалённые — прочь.
/// `progress(готово, всего)` — по файлам.
pub fn refresh(root: &Path, progress: &mut dyn FnMut(usize, usize)) -> Result<Status, String> {
    if !root.is_dir() {
        return Err(format!("папки {} нет", root.display()));
    }
    let (mut meta, mut vecs) = load(root).unwrap_or_default();
    meta.root = root.display().to_string();
    let files = files_of(root);
    let now: HashMap<String, String> = files.iter().map(|f| (f.display().to_string(), stamp(f))).collect();
    let stale: Vec<String> = meta.files.iter().filter(|(f, s)| now.get(*f) != Some(*s)).map(|(f, _)| f.clone()).collect();
    let fresh: Vec<&PathBuf> = files.iter().filter(|f| meta.files.get(&f.display().to_string()) != Some(&now[&f.display().to_string()])).collect();
    if stale.is_empty() && fresh.is_empty() && meta.dim > 0 {
        return Ok(status(root));
    }
    // Убрать фрагменты устаревших файлов.
    if !stale.is_empty() && meta.dim > 0 {
        let dim = meta.dim;
        let mut keep_chunks = Vec::new();
        let mut keep_vecs = Vec::new();
        for (i, c) in meta.chunks.iter().enumerate() {
            if !stale.contains(&c.file) {
                keep_chunks.push(c.clone());
                keep_vecs.extend_from_slice(&vecs[i * dim..(i + 1) * dim]);
            }
        }
        meta.chunks = keep_chunks;
        vecs = keep_vecs;
    }
    for f in &stale {
        meta.files.remove(f);
    }
    let total = fresh.len();
    let mut client = synaptixd::Client::connect().map_err(|e| e.to_string())?;
    for (i, f) in fresh.iter().enumerate() {
        progress(i, total);
        let name = f.display().to_string();
        let chunks = read_text(f).map(|t| chunk(&name, &t)).unwrap_or_default();
        if !chunks.is_empty() {
            // Имя файла в тексте фрагмента помогает поиску по названию.
            let texts: Vec<String> = chunks.iter().map(|c| format!("{}\n{}", name_of(f), c.text)).collect();
            let e = client.embed("", &texts).map_err(|e| e.to_string())?;
            if meta.dim == 0 {
                meta.dim = e.dim;
            }
            for v in e.vectors {
                vecs.extend(v);
            }
            meta.chunks.extend(chunks);
        }
        meta.files.insert(name.clone(), now[&name].clone());
        // Сохраняем по ходу — прерванная индексация продолжится с места.
        if i % 50 == 49 {
            store(root, &meta, &vecs)?;
        }
    }
    progress(total, total);
    store(root, &meta, &vecs)?;
    Ok(status(root))
}

/// Ближайшие фрагменты баз к запросу: (база, фрагмент, сходство).
pub fn search(roots: &[PathBuf], query: &str, k: usize) -> Result<Vec<(String, Chunk, f32)>, String> {
    let mut client = synaptixd::Client::connect().map_err(|e| e.to_string())?;
    let q = client.embed("", &[query.to_string()]).map_err(|e| e.to_string())?;
    let qv = q.vectors.into_iter().next().ok_or("пустой эмбеддинг запроса")?;
    let mut hits = Vec::new();
    for root in roots {
        let Some((meta, vecs)) = load(root) else { continue };
        if meta.dim != qv.len() {
            continue;
        }
        for (i, c) in meta.chunks.iter().enumerate() {
            let v = &vecs[i * meta.dim..(i + 1) * meta.dim];
            let score: f32 = v.iter().zip(&qv).map(|(a, b)| a * b).sum();
            hits.push((name_of(root), c.clone(), score));
        }
    }
    hits.sort_by(|a, b| b.2.total_cmp(&a.2));
    hits.truncate(k);
    Ok(hits)
}

pub fn prompt_section(roots: &[PathBuf]) -> String {
    if roots.is_empty() {
        return String::new();
    }
    let mut s = String::from("\n\n## Базы знаний\nВопросы по этим материалам пользователя сначала ищи инструментом knowledge (поиск по смыслу), отвечай по найденному и указывай источник (файл и строки):\n");
    for r in roots {
        s.push_str(&format!("- {} ({})\n", name_of(r), r.display()));
    }
    s
}

pub struct KnowledgeTool {
    pub roots: Vec<PathBuf>,
}

impl Tool for KnowledgeTool {
    fn name(&self) -> &'static str {
        "knowledge"
    }
    fn group(&self) -> &'static str {
        "базы знаний"
    }
    fn description(&self) -> String {
        "Поиск по смыслу в базах знаний пользователя (папки с документами): возвращает близкие фрагменты с файлом и строками. query — вопрос или ключевая мысль; base — имя базы (необязательно, иначе все).".into()
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "base": { "type": "string", "description": "имя базы из списка; пусто — все" }
            },
            "required": ["query"]
        })
    }
    fn danger(&self, _args: &Value) -> Danger {
        Danger::Safe
    }
    fn summary(&self, args: &Value) -> String {
        format!("Ищу в базах знаний: {}", str_arg(args, "query").unwrap_or(""))
    }
    fn run(&self, args: &Value) -> ToolResult {
        let query = need_str(args, "query")?;
        let roots: Vec<PathBuf> = match str_arg(args, "base") {
            Some(b) => self.roots.iter().filter(|r| name_of(r).eq_ignore_ascii_case(b)).cloned().collect(),
            None => self.roots.clone(),
        };
        if roots.is_empty() {
            return Err("такой базы нет".into());
        }
        // Изменённые файлы — в индекс перед поиском (быстро, если ничего не менялось).
        let mut notes = Vec::new();
        for r in &roots {
            if let Err(e) = refresh(r, &mut |_, _| {}) {
                notes.push(format!("{}: {e}", name_of(r)));
            }
        }
        let hits = search(&roots, query, TOP_K)?;
        if hits.is_empty() {
            return Ok(ToolOutput::text(format!("Ничего не найдено.{}", if notes.is_empty() { String::new() } else { format!(" ({})", notes.join("; ")) })));
        }
        let mut s = String::new();
        for (i, (base, c, score)) in hits.iter().enumerate() {
            s.push_str(&format!("[{}] {base}: {} строки {}–{} (сходство {score:.2})\n{}\n\n", i + 1, c.file, c.line, c.end, crate::tools::truncate(&c.text, 1500)));
        }
        Ok(ToolOutput::text(s.trim_end().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_by_paragraphs() {
        let para = "слово ".repeat(100);
        let text = format!("{para}\n\n{para}\n\nкороткий абзац, но достаточно длинный\n");
        let cs = chunk("f.md", &text);
        assert!(cs.len() >= 2, "{cs:?}");
        assert_eq!(cs[0].line, 1);
        assert!(cs.last().unwrap().text.contains("короткий абзац"));
        assert!(cs.iter().all(|c| c.end >= c.line));
    }
}
