//! Запрос `Document`: чтение документов (PDF) службой для всех клиентов.
//!
//! Сначала дешёвый обзор (без распаковки картинок и без моделей): страницы,
//! сканы, картинки, заголовки, токены текстового слоя по токенизатору модели
//! клиента (`tokenizer.json`, модель не грузится) и её окно — клиент решает,
//! вкладывать документ целиком или дать обзор и читать по страницам. Затем,
//! по запросу, страницы «с–по» в порядке чтения с картинками на своих местах
//! (файлы в кэше службы) и поиск по тексту. Сканы распознаются моделью со
//! зрением только у запрошенных страниц и только с `ocr`. Всё кэшируется по
//! содержимому файла.

use std::io::Read;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use synaptix_io::document::pdf::{self, Part};

use super::manager::Manager;
use crate::{DocPart, DocumentReply, Error, ErrorCode, ModelKind, Reply};

/// Больше стольких байт документ не читается (защита службы).
const MAX_BYTES: u64 = 1 << 30;
/// Страниц за один запрос чтения — не больше.
pub const MAX_PAGES_PER_READ: u32 = 40;
/// Заголовков в обзоре — не больше.
const MAX_OUTLINE: usize = 300;
/// Найденного поиском — не больше.
const MAX_HITS: usize = 40;

fn fail(m: impl Into<String>) -> Error {
    Error::new(ErrorCode::Failed, m.into())
}

/// Обзор документа (кэш `overview.json`): текст страниц и места картинок.
#[derive(Serialize, Deserialize, Default)]
struct Overview {
    pages: u32,
    bytes: u64,
    texts: Vec<String>,
    scan_pages: Vec<u32>,
    images: usize,
    outline: Vec<(u32, String)>,
}

fn cache_root() -> PathBuf {
    std::env::var_os("SYNAPTIX_CACHE_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("docs")
}

/// Ключ по содержимому (FNV-1a 64 по байтам и длине).
fn key(bytes: &[u8]) -> String {
    let h = bytes.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3));
    format!("{h:016x}{:x}", bytes.len())
}

/// Строка — похожа на заголовок: короткая, без точки в конце, с буквы или
/// номера, не вся из цифр.
fn heading(line: &str) -> bool {
    let t = line.trim();
    let n = t.chars().count();
    if !(3..=80).contains(&n) || t.ends_with(['.', ',', ';', ':']) || !t.chars().any(char::is_alphabetic) {
        return false;
    }
    let first = t.chars().next().unwrap_or(' ');
    let caps = t.chars().filter(|c| c.is_alphabetic()).all(char::is_uppercase);
    (first.is_uppercase() || first.is_ascii_digit()) && (caps || n <= 60)
}

fn overview(bytes: &[u8], dir: &Path) -> Result<Overview, Error> {
    let path = dir.join("overview.json");
    if let Some(o) = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Overview>(&b).ok()) {
        return Ok(o);
    }
    let parts = pdf::layout(bytes, None, None).map_err(|e| fail(e.to_string()))?;
    let mut o = Overview { bytes: bytes.len() as u64, ..Default::default() };
    for p in parts {
        match p {
            Part::Page(n) => {
                o.pages = n;
                o.texts.push(String::new());
            }
            Part::Text(t) => {
                let page = o.pages;
                // первая строка блока, похожая на заголовок
                if let Some(first) = t.lines().next().filter(|l| heading(l)) {
                    if o.outline.len() < MAX_OUTLINE {
                        o.outline.push((page, first.trim().to_string()));
                    }
                }
                if let Some(last) = o.texts.last_mut() {
                    if !last.is_empty() {
                        last.push_str("\n\n");
                    }
                    last.push_str(&t);
                }
            }
            Part::Image { scan, .. } => {
                o.images += 1;
                if scan {
                    o.scan_pages.push(o.pages);
                }
            }
        }
    }
    o.pages = o.texts.len() as u32;
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(&path, serde_json::to_vec(&o).unwrap_or_default());
    Ok(o)
}

/// Токены текста страниц по токенизатору модели (кэш на модель).
fn page_tokens(mgr: &Arc<Manager>, model: &str, o: &Overview, dir: &Path) -> Vec<usize> {
    let cached = dir.join(format!("tokens-{}.json", model.replace(['/', ' '], "_")));
    if let Some(v) = std::fs::read(&cached).ok().and_then(|b| serde_json::from_slice::<Vec<usize>>(&b).ok()) {
        return v;
    }
    let Some(file) = mgr.model_file(model) else {
        // без токенизатора — оценка: ~4 знака на токен
        return o.texts.iter().map(|t| t.chars().count().div_ceil(4)).collect();
    };
    let v: Vec<usize> = o.texts.iter().map(|t| if t.is_empty() { 0 } else { synaptix::facade::llm::token_spans(&file.path, t).map(|s| s.len()).unwrap_or_else(|_| t.chars().count().div_ceil(4)) }).collect();
    let _ = std::fs::write(&cached, serde_json::to_vec(&v).unwrap_or_default());
    v
}

/// Найти в тексте страниц (без учёта регистра и «ё»).
fn search(o: &Overview, query: &str) -> Vec<(u32, String)> {
    let norm = |s: &str| s.to_lowercase().replace('ё', "е");
    let q = norm(query.trim());
    if q.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for (i, t) in o.texts.iter().enumerate() {
        let low = norm(t);
        let mut from = 0;
        while let Some(pos) = low[from..].find(&q).map(|p| p + from) {
            // отрывок ±120 знаков по границам символов
            let chars: Vec<(usize, char)> = t.char_indices().collect();
            let ci = low[..pos].chars().count();
            let a = ci.saturating_sub(120);
            let b = (ci + q.chars().count() + 120).min(chars.len());
            let start = chars.get(a).map(|c| c.0).unwrap_or(0);
            let end = chars.get(b).map(|c| c.0).unwrap_or(t.len());
            hits.push((i as u32 + 1, t[start..end].replace('\n', " ")));
            if hits.len() >= MAX_HITS {
                return hits;
            }
            from = pos + q.len().max(1);
        }
    }
    hits
}

/// Самая лёгкая языковая модель со зрением (для распознавания сканов).
fn vision_model(mgr: &Arc<Manager>) -> Option<String> {
    let mut list: Vec<_> = mgr.list().into_iter().filter(|m| m.kind == ModelKind::Llm).collect();
    list.sort_by_key(|m| m.disk_bytes);
    list.into_iter().map(|m| m.id).find(|id| mgr.llm_info(id).map(|i| i.media).unwrap_or(false))
}

fn ocr(mgr: &Arc<Manager>, model: &str, image: &Path, cancel: &Arc<AtomicBool>) -> Result<String, Error> {
    let mut msg = crate::Message::user(format!(
        "{}\nЭто страница документа. Перепиши весь её текст дословно, на языке оригинала, сохраняя порядок строк и абзацев, заголовки и имена говорящих. Ничего не добавляй и не пересказывай; только текст страницы.",
        crate::MEDIA_MARK
    ));
    msg.media.push(crate::Media { kind: crate::MediaKind::Image, path: image.display().to_string() });
    let params = crate::Params::new().with("temperature", 0.0).with("thinking", false).with("max_tokens", 4096i64);
    let msgs = [msg];
    let g = mgr.run(model, |m| m.generate(&msgs, &params, &[], &|_| {}, cancel))?;
    Ok(g.text.trim().to_string())
}

/// Обслужить запрос `Document`.
#[allow(clippy::too_many_arguments)]
pub fn serve(mgr: &Arc<Manager>, sock: BorrowedFd, id: u64, file: Option<OwnedFd>, model: String, pages: Option<(u32, u32)>, query: String, want_ocr: bool) -> Result<DocumentReply, Error> {
    let fd = file.ok_or_else(|| Error::new(ErrorCode::Protocol, "нет файла документа"))?;
    let mut f = std::fs::File::from(fd);
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if len > MAX_BYTES {
        return Err(fail(format!("документ слишком большой: {} МБ (предел {} МБ)", len >> 20, MAX_BYTES >> 20)));
    }
    let mut bytes = Vec::with_capacity(len as usize);
    f.read_to_end(&mut bytes).map_err(|e| fail(e.to_string()))?;
    if !bytes.starts_with(b"%PDF") && !bytes.windows(5).take(1024).any(|w| w == b"%PDF-") {
        return Err(fail("пока читаются только PDF"));
    }
    let dir = cache_root().join(key(&bytes));
    // отмена: клиент закрыл сокет
    let cancel = Arc::new(AtomicBool::new(false));
    let _watch = {
        let c = cancel.clone();
        super::hangup::watch(sock.as_raw_fd(), move || c.store(true, Ordering::SeqCst))
    };
    let progress = |step: usize, total: usize| {
        let r = Reply { id, event: Some(crate::Event::Progress { step, total }), ..Default::default() };
        let _ = crate::wire::send(sock.as_fd(), &serde_json::to_vec(&r).unwrap_or_default(), &[]);
    };
    progress(0, 0);
    let o = overview(&bytes, &dir)?;
    let model = mgr.resolve_id(&model, ModelKind::Llm).unwrap_or(model);
    let window = mgr.llm_info(&model).map(|i| i.window).unwrap_or(0);
    let page_tokens = page_tokens(mgr, &model, &o, &dir);
    let mut reply = DocumentReply {
        pages: o.pages,
        bytes: o.bytes,
        scan_pages: o.scan_pages.clone(),
        images: o.images,
        tokens: page_tokens.iter().sum(),
        page_tokens,
        window,
        model: model.clone(),
        outline: o.outline.clone(),
        parts: Vec::new(),
        hits: search(&o, &query),
    };
    let Some((a, b)) = pages else { return Ok(reply) };
    let a = a.max(1);
    let b = b.min(o.pages).min(a + MAX_PAGES_PER_READ - 1);
    if a > b {
        return Ok(reply);
    }
    let img_dir = dir.join("img");
    let _ = std::fs::create_dir_all(&img_dir);
    let parts = pdf::layout(&bytes, Some(a..=b), Some(&img_dir)).map_err(|e| fail(e.to_string()))?;
    let scans = parts.iter().filter(|p| matches!(p, Part::Image { scan: true, .. })).count();
    let vision = if want_ocr && scans > 0 { vision_model(mgr) } else { None };
    let mut done = 0usize;
    for p in parts {
        if cancel.load(Ordering::Relaxed) {
            return Err(fail("отменено клиентом"));
        }
        reply.parts.push(match p {
            Part::Page(n) => DocPart::Page(n),
            Part::Text(t) => DocPart::Text(t),
            Part::Image { path, scan } => {
                let path = path.unwrap_or_default();
                let text = match (&vision, scan) {
                    (Some(vm), true) => {
                        // распознанный текст скана — в кэше рядом с картинкой
                        let txt = path.with_extension("txt");
                        let t = match std::fs::read_to_string(&txt) {
                            Ok(t) => t,
                            Err(_) => {
                                let t = ocr(mgr, vm, &path, &cancel)?;
                                let _ = std::fs::write(&txt, &t);
                                t
                            }
                        };
                        done += 1;
                        progress(done, scans);
                        Some(t)
                    }
                    _ => None,
                };
                DocPart::Image { path: path.display().to_string(), scan, text }
            }
        });
    }
    Ok(reply)
}

#[cfg(test)]
mod tests {
    #[test]
    fn headings_and_search() {
        assert!(super::heading("INT. HOUSE - KITCHEN - DAY"));
        assert!(super::heading("Глава 3. Начало пути"));
        assert!(!super::heading("Это обычное длинное предложение текста, которое заканчивается точкой."));
        assert!(!super::heading("12"));
        let o = super::Overview { pages: 2, texts: vec!["Первая страница".into(), "Здесь ёлка и ещё ЁЛКА".into()], ..Default::default() };
        let hits = super::search(&o, "елка");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].0, 2);
    }
}

#[cfg(test)]
mod live {
    /// Встроенная служба на своём сокете (рабочая служба не трогается): обзор,
    /// токены до обработки, чтение страниц с картинками, поиск.
    /// `SYN_DOC=файл.pdf [SYN_DOC_PAGES=a-b] [SYN_DOC_SEARCH=…] cargo test -p synaptixd --features daemon --lib document_live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn document_live() {
        let d = tempfile::tempdir().unwrap();
        std::env::set_var("SYNAPTIX_CACHE_DIR", d.path().join("cache"));
        let sock = d.path().join("s.sock");
        crate::server::embedded(&sock, &d.path().join("state")).unwrap();
        crate::set_socket_path(sock.clone());
        for _ in 0..100 {
            if sock.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let doc = std::path::PathBuf::from(std::env::var("SYN_DOC").unwrap());
        let mut c = crate::Client::connect().unwrap();
        let t0 = std::time::Instant::now();
        let r = c.document(&doc, "", None, &std::env::var("SYN_DOC_SEARCH").unwrap_or_default(), false, |_, _| {}).unwrap();
        println!("обзор за {:.2} с: страниц {}, {} КБ, сканов {} {:?}, картинок {}, токенов {} (модель {}, окно {}), заголовков {}", t0.elapsed().as_secs_f32(), r.pages, r.bytes / 1024, r.scan_pages.len(), &r.scan_pages[..r.scan_pages.len().min(10)], r.images, r.tokens, r.model, r.window, r.outline.len());
        for (p, h) in r.outline.iter().take(12) {
            println!("  заголовок стр. {p}: {h}");
        }
        for (p, s) in &r.hits {
            println!("  найдено стр. {p}: {}", s.chars().take(100).collect::<String>());
        }
        if let Some((a, b)) = std::env::var("SYN_DOC_PAGES").ok().and_then(|s| s.split_once('-').map(|(a, b)| (a.parse().unwrap(), b.parse().unwrap()))) {
            let t1 = std::time::Instant::now();
            let r = c.document(&doc, "", Some((a, b)), "", false, |_, _| {}).unwrap();
            println!("страницы {a}–{b} за {:.2} с:", t1.elapsed().as_secs_f32());
            for p in &r.parts {
                match p {
                    crate::DocPart::Page(n) => println!("== стр. {n}"),
                    crate::DocPart::Text(t) => println!("[текст {} зн.] {}", t.chars().count(), t.lines().next().unwrap_or("")),
                    crate::DocPart::Image { path, scan, .. } => println!("[картинка{}] {path}", if *scan { " — скан" } else { "" }),
                }
            }
        }
    }
}
