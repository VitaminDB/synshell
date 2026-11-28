//! Документы (PDF) — через службу synaptixd (запрос `Document`): одна
//! реализация разбора для всех программ. Здесь — бюджет и подача модели:
//! документ целиком во вложении, только если влезает в разумную долю окна
//! модели (токены текста считает служба токенизатором этой модели до любой
//! тяжёлой обработки), иначе — обзор (страницы, сканы, оглавление, первые
//! страницы) и чтение по запросу: страницы «с–по», поиск. Текст и картинки
//! идут в порядке документа: на месте картинки — маркер медиа службы.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use synaptixd::{DocPart, DocumentReply, Media, MediaKind, MEDIA_MARK};

/// Доля окна модели, которую может занять вложенный документ целиком.
const DOC_SHARE: f64 = 0.25;
/// Окно, если служба его не знает.
const DEFAULT_WINDOW: usize = 32_768;
/// Оценка токенов одной картинки для бюджета.
const IMAGE_TOKENS: usize = 800;
/// Картинок одного куска документа в сообщении — не больше.
const MAX_IMAGES: usize = 24;
/// Страниц за одно чтение (предел службы).
pub const READ_PAGES: u32 = 40;
/// Документ длиннее — только обзором.
const MAX_WHOLE_PAGES: u32 = 60;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn client() -> Result<synaptixd::Client, String> {
    synaptixd::Client::connect().map_err(|e| format!("служба нейросетей: {e}"))
}

/// Обзор документа: страницы, сканы, токены для `model` и её окно, оглавление.
pub fn overview(f: &Path, model: &str) -> Result<DocumentReply, String> {
    client()?.document(f, model, None, "", false, |_, _| {}).map_err(err)
}

/// Страницы `a..=b` (не больше [`READ_PAGES`]); `ocr` — распознать сканы текстом.
pub fn read(f: &Path, model: &str, a: u32, b: u32, ocr: bool) -> Result<DocumentReply, String> {
    client()?.document(f, model, Some((a, b)), "", ocr, |_, _| {}).map_err(err)
}

/// Поиск по тексту документа.
pub fn search(f: &Path, query: &str) -> Result<DocumentReply, String> {
    client()?.document(f, "", None, query, false, |_, _| {}).map_err(err)
}

/// Текст всех страниц `(номер, текст, распознан ли скан)` — для потребителей
/// только текста (база знаний, сверка сценария). `ocr` — сканы распознаются.
pub fn pages_with(f: &Path, ocr: bool) -> Result<Vec<(usize, String, bool)>, String> {
    let total = overview(f, "")?.pages;
    let mut out: Vec<(usize, String, bool)> = Vec::new();
    let mut a = 1;
    while a <= total {
        let b = (a + READ_PAGES - 1).min(total);
        for p in read(f, "", a, b, ocr)?.parts {
            match p {
                DocPart::Page(n) => out.push((n as usize, String::new(), false)),
                DocPart::Text(t) => push_text(&mut out, &t, false),
                DocPart::Image { text: Some(t), scan: true, .. } => push_text(&mut out, &t, true),
                DocPart::Image { .. } => {}
            }
        }
        a = b + 1;
    }
    Ok(out)
}

fn push_text(out: &mut [(usize, String, bool)], t: &str, ocr: bool) {
    if let Some(last) = out.last_mut() {
        if !last.1.is_empty() {
            last.1.push_str("\n\n");
        }
        last.1.push_str(t);
        last.2 |= ocr;
    }
}

/// Текст страниц без распознавания сканов.
pub fn pages(f: &Path) -> Result<Vec<(usize, String, bool)>, String> {
    pages_with(f, false)
}

/// Весь текст с разметкой страниц (сканы распознаются).
pub fn text(f: &Path) -> Result<String, String> {
    let pages = pages_with(f, true)?;
    if pages.iter().all(|(_, t, _)| t.trim().is_empty()) {
        return Err("в PDF нет текста и не удалось распознать страницы".into());
    }
    Ok(pages.iter().map(|(n, t, ocr)| format!("--- страница {n}{} ---\n{t}\n", if *ocr { " (скан, распознано)" } else { "" })).collect::<Vec<_>>().join("\n"))
}

/// Части документа — текстом с маркерами на месте картинок (`media` — по порядку).
pub fn render(parts: &[DocPart], media: &mut Vec<Media>) -> (String, usize) {
    let mut s = String::new();
    let mut skipped = 0;
    let mut shown = 0;
    for p in parts {
        match p {
            DocPart::Page(n) => s.push_str(&format!("\n--- страница {n} ---\n")),
            DocPart::Text(t) => {
                s.push_str(t);
                s.push('\n');
            }
            DocPart::Image { path, scan, text } => {
                if shown >= MAX_IMAGES {
                    skipped += 1;
                    s.push_str(&format!("[картинка: {path}]\n"));
                    continue;
                }
                shown += 1;
                media.push(Media { kind: MediaKind::Image, path: path.clone() });
                s.push_str(&format!("[{}: {path}]\n{MEDIA_MARK}\n", if *scan { "страница-скан" } else { "картинка" }));
                if let Some(t) = text {
                    s.push_str(&format!("(распознано: {t})\n"));
                }
            }
        }
    }
    (s, skipped)
}

/// Свёрнутое в процессе: история агента собирается на каждом шаге.
type Key = (PathBuf, u64, u64, String);
static MEMO: Mutex<Option<HashMap<Key, (String, Vec<Media>)>>> = Mutex::new(None);

fn key(f: &Path, model: &str) -> Key {
    let m = std::fs::metadata(f).ok();
    let mtime = m.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
    (f.to_path_buf(), m.map(|m| m.len()).unwrap_or(0), mtime, model.to_string())
}

/// Вложение PDF для модели `model`: целиком (если влезает в долю окна) или
/// обзор с подсказкой читать дальше инструментом `document`.
pub fn attachment(f: &Path, model: &str, media: &mut Vec<Media>) -> String {
    let k = key(f, model);
    if let Some((s, m)) = MEMO.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|c| c.get(&k)).cloned() {
        media.extend(m);
        return s;
    }
    let mut own = Vec::new();
    let s = match attachment_fresh(f, model, &mut own) {
        Ok(s) => s,
        Err(e) => {
            own.clear();
            format!("\n\nДокумент {} — не прочитан: {e}.", f.display())
        }
    };
    MEMO.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(k, (s.clone(), own.clone()));
    media.extend(own);
    s
}

fn attachment_fresh(f: &Path, model: &str, media: &mut Vec<Media>) -> Result<String, String> {
    let o = overview(f, model)?;
    let window = if o.window > 0 { o.window } else { DEFAULT_WINDOW };
    let need = o.tokens + o.images.min(MAX_IMAGES) * IMAGE_TOKENS;
    let budget = (window as f64 * DOC_SHARE) as usize;
    let head = format!(
        "\n\nДокумент {} — {} стр., {} КБ, текст ≈{} токенов{}{}.",
        f.display(),
        o.pages,
        o.bytes / 1024,
        o.tokens,
        if o.scan_pages.is_empty() { String::new() } else { format!(", страниц-сканов {} ({})", o.scan_pages.len(), pages_list(&o.scan_pages)) },
        if o.images > 0 { format!(", картинок {}", o.images) } else { String::new() }
    );
    if need <= budget && o.pages <= MAX_WHOLE_PAGES {
        // целиком: по порядку документа, картинки на своих местах
        let mut s = format!("{head} Целиком, по порядку, как в PDF: текст страниц и картинки на своих местах.\n");
        let mut a = 1;
        let mut skipped = 0;
        while a <= o.pages {
            let b = (a + READ_PAGES - 1).min(o.pages);
            let r = read(f, model, a, b, false)?;
            let (t, sk) = render(&r.parts, media);
            s.push_str(&t);
            skipped += sk;
            a = b + 1;
        }
        if skipped > 0 {
            s.push_str(&format!("[ещё {skipped} картинок — путями; посмотреть: view_media]\n"));
        }
        return Ok(s);
    }
    // большой: обзор и чтение по запросу
    let mut s = format!(
        "{head} Документ большой для одного сообщения (нужно ≈{need} токенов при окне {window} модели {}): ниже обзор и первые страницы; остальное читай инструментом document — {{path, pages: \"a-b\"}} (до {READ_PAGES} страниц за раз, сканы распознаются только у прочитанных страниц) или {{path, search: \"<слова>\"}}.\n",
        if o.model.is_empty() { "агента" } else { o.model.as_str() }
    );
    if !o.outline.is_empty() {
        s.push_str("Оглавление (заголовки и страницы):\n");
        for (p, h) in o.outline.iter().take(80) {
            s.push_str(&format!("- стр. {p}: {h}\n"));
        }
        if o.outline.len() > 80 {
            s.push_str(&format!("- … ещё {} заголовков\n", o.outline.len() - 80));
        }
    }
    // первые страницы — сколько влезет в треть бюджета (до 5)
    let mut used = 0;
    let mut last = 0;
    for (i, t) in o.page_tokens.iter().enumerate() {
        if used + t > budget / 3 && i > 0 {
            break;
        }
        used += t;
        last = i as u32 + 1;
        if last >= 5 {
            break;
        }
    }
    if last > 0 {
        let r = read(f, model, 1, last, false)?;
        let (t, _) = render(&r.parts, media);
        s.push_str(&format!("Первые страницы (1–{last}):\n{t}"));
    }
    Ok(s)
}

/// «1, 2, 5–9, 12» из списка номеров.
pub fn pages_list(v: &[u32]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < v.len() {
        let mut j = i;
        while j + 1 < v.len() && v[j + 1] == v[j] + 1 {
            j += 1;
        }
        out.push(if j > i { format!("{}–{}", v[i], v[j]) } else { v[i].to_string() });
        i = j + 1;
        if out.len() >= 12 {
            out.push("…".into());
            break;
        }
    }
    out.join(", ")
}

/// Разбор «a-b», «a–b», «a».
pub fn parse_range(s: &str) -> Option<(u32, u32)> {
    let s = s.trim();
    let (a, b) = s.split_once(['-', '–', '—']).map(|(a, b)| (a.trim(), b.trim())).unwrap_or((s, s));
    let (a, b): (u32, u32) = (a.parse().ok()?, b.parse().ok()?);
    (a >= 1 && b >= a).then_some((a, b))
}

#[cfg(test)]
mod tests {
    #[test]
    fn ranges_and_lists() {
        assert_eq!(super::parse_range("3-7"), Some((3, 7)));
        assert_eq!(super::parse_range(" 5 "), Some((5, 5)));
        assert_eq!(super::parse_range("7-3"), None);
        assert_eq!(super::pages_list(&[1, 2, 3, 7, 9, 10]), "1–3, 7, 9–10");
    }

    #[test]
    fn render_marks_images_in_order() {
        let parts = vec![synaptixd::DocPart::Page(1), synaptixd::DocPart::Text("до".into()), synaptixd::DocPart::Image { path: "/x.png".into(), scan: false, text: None }, synaptixd::DocPart::Text("после".into())];
        let mut media = Vec::new();
        let (t, _) = super::render(&parts, &mut media);
        assert_eq!(media.len(), 1);
        let (a, m, b) = (t.find("до").unwrap(), t.find(synaptixd::MEDIA_MARK).unwrap(), t.find("после").unwrap());
        assert!(a < m && m < b);
    }
}
