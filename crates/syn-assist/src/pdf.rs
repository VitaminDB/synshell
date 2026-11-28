//! PDF своим кодом, без внешних программ (pdftotext, poppler, python):
//! текстовый слой страниц — `pdf-extract` (разбор PDF на Rust), страница без
//! текста (скан) — её картинка из PDF (`lopdf`) распознаётся моделью службы
//! со зрением. Результат кэшируется на диске по пути, размеру и времени файла.

use std::path::{Path, PathBuf};

/// Страница считается сканом, если в её текстовом слое меньше стольких букв.
const SCAN_MIN_CHARS: usize = 16;

/// Текст документа по страницам: `(номер с 1, текст, распознана ли моделью)`.
pub fn pages(f: &Path) -> Result<Vec<(usize, String, bool)>, String> {
    let bytes = std::fs::read(f).map_err(|e| format!("{}: {e}", f.display()))?;
    // разбор чужих PDF может паниковать на битых файлах — не роняем программу
    let layer = std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem_by_pages(&bytes)).map_err(|_| "PDF не разобран (повреждён или необычный формат)".to_string())?;
    let layer: Vec<String> = match layer {
        Ok(p) => p,
        Err(e) => {
            // текстовый слой не читается — пробуем страницы как сканы
            let n = page_count(&bytes).ok_or_else(|| format!("PDF не разобран: {e}"))?;
            vec![String::new(); n]
        }
    };
    let mut out = Vec::with_capacity(layer.len());
    let mut doc: Option<lopdf::Document> = None;
    for (i, t) in layer.into_iter().enumerate() {
        let t = tidy(&t);
        if t.chars().filter(|c| c.is_alphanumeric()).count() >= SCAN_MIN_CHARS {
            out.push((i + 1, t, false));
            continue;
        }
        if doc.is_none() {
            doc = lopdf::Document::load_mem(&bytes).ok();
        }
        let ocr = doc.as_ref().and_then(|d| page_image(d, i + 1)).map(|img| ocr_image(&img));
        match ocr {
            Some(Ok(text)) if !text.trim().is_empty() => out.push((i + 1, text.trim().to_string(), true)),
            _ => out.push((i + 1, t, false)),
        }
    }
    Ok(out)
}

/// Весь текст документа с разметкой страниц (кэш на диске).
pub fn text(f: &Path) -> Result<String, String> {
    let cached = cache_file(f);
    if let Ok(t) = std::fs::read_to_string(&cached) {
        return Ok(t);
    }
    let pages = pages(f)?;
    let mut s = String::new();
    for (n, t, ocr) in &pages {
        s.push_str(&format!("--- страница {n}{} ---\n{t}\n\n", if *ocr { " (скан, распознано)" } else { "" }));
    }
    if pages.iter().all(|(_, t, _)| t.trim().is_empty()) {
        return Err("в PDF нет текста и не удалось распознать страницы (нет модели со зрением в службе?)".into());
    }
    if let Some(d) = cached.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&cached, &s);
    Ok(s)
}

/// Файл кэша текста PDF (агент может открыть его целиком).
pub fn cache_file(f: &Path) -> PathBuf {
    crate::attach::cache_root().join("pdf").join(format!("{}.txt", crate::attach::cache_key(f)))
}

/// Убрать мусор разбора: \r, хвостовые пробелы, больше двух пустых строк подряд.
fn tidy(t: &str) -> String {
    let mut out = String::with_capacity(t.len());
    let mut blank = 0;
    for l in t.replace('\r', "").lines() {
        let l = l.trim_end();
        if l.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(l);
        out.push('\n');
    }
    out.trim().to_string()
}

fn page_count(bytes: &[u8]) -> Option<usize> {
    lopdf::Document::load_mem(bytes).ok().map(|d| d.get_pages().len())
}

/// Картинка страницы для распознавания.
enum PageImage {
    Jpeg(Vec<u8>),
    Png(Vec<u8>),
}

/// Самая большая картинка страницы (у скана — сам лист): JPEG как есть,
/// несжатые/Flate-пиксели (серые или RGB, 8 бит) — в PNG.
fn page_image(doc: &lopdf::Document, page: usize) -> Option<PageImage> {
    let id = *doc.get_pages().get(&(page as u32))?;
    let (res, ids) = doc.get_page_resources(id).ok()?;
    let mut dicts: Vec<&lopdf::Dictionary> = res.into_iter().collect();
    dicts.extend(ids.into_iter().filter_map(|i| doc.get_dictionary(i).ok()));
    let mut best: Option<(u64, PageImage)> = None;
    for r in dicts {
        let Ok(xo) = r.get(b"XObject").and_then(|o| doc.dereference(o)).and_then(|(_, o)| o.as_dict()) else { continue };
        for (_, v) in xo.iter() {
            let Ok((_, obj)) = doc.dereference(v) else { continue };
            let Ok(stream) = obj.as_stream() else { continue };
            let d = &stream.dict;
            if d.get(b"Subtype").and_then(|s| s.as_name()).ok() != Some(b"Image".as_slice()) {
                continue;
            }
            let num = |k: &[u8]| d.get(k).and_then(|v| v.as_i64()).unwrap_or(0).max(0) as u64;
            let (w, h) = (num(b"Width"), num(b"Height"));
            let area = w * h;
            if best.as_ref().is_some_and(|(a, _)| *a >= area) {
                continue;
            }
            let filters: Vec<Vec<u8>> = match d.get(b"Filter") {
                Ok(lopdf::Object::Name(n)) => vec![n.clone()],
                Ok(lopdf::Object::Array(a)) => a.iter().filter_map(|o| o.as_name().ok().map(<[u8]>::to_vec)).collect(),
                _ => Vec::new(),
            };
            let img = if filters.iter().any(|f| f == b"DCTDecode") {
                Some(PageImage::Jpeg(stream.content.clone()))
            } else {
                let raw = if filters.is_empty() { Some(stream.content.clone()) } else { stream.decompressed_content().ok() };
                let cs = d.get(b"ColorSpace").and_then(|c| c.as_name()).ok().map(<[u8]>::to_vec).unwrap_or_default();
                let bpc = num(b"BitsPerComponent");
                let ch = match cs.as_slice() {
                    b"DeviceGray" => 1,
                    b"DeviceRGB" => 3,
                    _ => 0,
                };
                raw.filter(|r| bpc == 8 && ch > 0 && r.len() as u64 >= area * ch).and_then(|r| encode_png(&r, w as u32, h as u32, ch as u8)).map(PageImage::Png)
            };
            if let Some(img) = img {
                best = Some((area, img));
            }
        }
    }
    best.map(|(_, i)| i)
}

fn encode_png(raw: &[u8], w: u32, h: u32, ch: u8) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, w, h);
        e.set_color(if ch == 1 { png::ColorType::Grayscale } else { png::ColorType::Rgb });
        e.set_depth(png::BitDepth::Eight);
        let mut wr = e.write_header().ok()?;
        wr.write_image_data(&raw[..(w * h * ch as u32) as usize]).ok()?;
    }
    Some(out)
}

/// Распознать текст страницы моделью службы со зрением (самой лёгкой).
fn ocr_image(img: &PageImage) -> Result<String, String> {
    let (bytes, ext) = match img {
        PageImage::Jpeg(b) => (b, "jpg"),
        PageImage::Png(b) => (b, "png"),
    };
    let path = std::env::temp_dir().join(format!("syn-assist-pdf-{}-{}.{ext}", std::process::id(), bytes.len()));
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    let r = (|| {
        let mut c = synaptixd::Client::connect().map_err(|e| e.to_string())?;
        let mut list: Vec<_> = c.list().map_err(|e| e.to_string())?.into_iter().filter(|m| m.kind == synaptixd::ModelKind::Llm).collect();
        list.sort_by_key(|m| m.disk_bytes);
        let model = list.into_iter().map(|m| m.id).find(|id| c.llm_info(id).map(|i| i.media).unwrap_or(false)).ok_or("в службе нет модели со зрением")?;
        let mut msg = synaptixd::Message::user(format!(
            "{}\nЭто страница документа. Перепиши весь её текст дословно, на языке оригинала, сохраняя порядок строк и абзацев, заголовки и имена говорящих. Ничего не добавляй и не пересказывай; только текст страницы.",
            synaptixd::MEDIA_MARK
        ));
        msg.media.push(synaptixd::Media { kind: synaptixd::MediaKind::Image, path: path.to_string_lossy().into_owned() });
        let params = synaptixd::Params::new().with("temperature", 0.0).with("thinking", false).with("max_tokens", 4096i64);
        c.chat(&model, vec![msg], Vec::new(), &params, |_| {}).map(|g| g.text).map_err(|e| e.to_string())
    })();
    let _ = std::fs::remove_file(&path);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tidy_squeezes_blank_lines() {
        assert_eq!(tidy("a  \r\n\n\n\nb\n"), "a\n\nb");
    }

    /// Настоящий PDF: текстовый слой читается своим кодом, страницы размечены.
    #[test]
    fn reads_text_layer_of_real_pdf() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.pdf");
        std::fs::write(&p, minimal_pdf("INT. KITCHEN - NIGHT. GRYMZA stirs the soup.")).unwrap();
        let pages = pages(&p).unwrap();
        assert_eq!(pages.len(), 1);
        assert!(pages[0].1.contains("KITCHEN"), "{:?}", pages[0].1);
        assert!(!pages[0].2);
    }

    /// Одностраничный PDF с текстом Helvetica (смещения xref считаются).
    fn minimal_pdf(text: &str) -> Vec<u8> {
        let content = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET");
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string(),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offs = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offs.push(out.len());
            out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).bytes());
        for o in offs {
            out.extend(format!("{o:010} 00000 n \n").bytes());
        }
        out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).bytes());
        out
    }
}

#[cfg(test)]
mod real {
    /// Ручная проверка на своём PDF: `SYN_PDF=файл.pdf cargo test -p syn-assist real_pdf -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_pdf() {
        let p = std::path::PathBuf::from(std::env::var("SYN_PDF").unwrap());
        let pages = super::pages(&p).unwrap();
        for (n, t, ocr) in &pages {
            println!("== стр. {n} ocr={ocr} знаков {}", t.chars().count());
        }
        println!("{}", super::text(&p).unwrap());
    }
}
