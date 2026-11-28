//! PDF своим кодом, без внешних программ (pdftotext, poppler, python).
//!
//! Документ — последовательность в порядке чтения: страница за страницей,
//! внутри страницы текстовые блоки и картинки сверху вниз по их месту на
//! странице. Картинки находятся проходом по потоку содержимого (`lopdf`) с
//! матрицей преобразования (q/Q/cm, вложенные Form), их вызовы убираются, и
//! очищенный документ отдаётся разбору текста (`pdf-extract`) со своим
//! приёмником символов — строки получают координаты. Мелкие украшения
//! (линии, значки, логотипы) отбрасываются по размеру. Страница-скан — сама
//! картинка на своём месте; для потребителей только текста (база знаний)
//! она распознаётся моделью службы со зрением. Всё кэшируется на диске.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, ObjectId};
use serde::{Deserialize, Serialize};

/// Страница считается сканом, если в её текстовом слое меньше стольких букв.
const SCAN_MIN_CHARS: usize = 16;
/// Картинка меньше этой доли страницы (по площади) — украшение.
const MIN_AREA_FRACTION: f64 = 0.02;
/// …или уже/ниже стольких пунктов на странице.
const MIN_SIDE_PT: f64 = 40.0;
/// …или меньше стольких пикселей по стороне.
const MIN_SIDE_PX: i64 = 48;
/// …или меньше стольких пикселей всего (логотипы, значки).
const MIN_PIXELS: i64 = 160 * 160;
/// …или вытянута сильнее (рамки, полосы орнамента, линейки, баннеры).
const MAX_ASPECT: f64 = 4.0;

/// Кусок документа в порядке чтения.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Part {
    /// Начало страницы (номер с 1).
    Page(usize),
    /// Текстовый блок.
    Text(String),
    /// Картинка (файл в кэше); `scan` — это вся страница-скан.
    Image { path: PathBuf, scan: bool },
}

/// Документ последовательностью (кэш на диске: текст и картинки).
pub fn sequence(f: &Path) -> Result<Vec<Part>, String> {
    let dir = cache_dir(f);
    let index = dir.join("parts.json");
    if let Some(parts) = std::fs::read(&index).ok().and_then(|b| serde_json::from_slice::<Vec<Part>>(&b).ok()) {
        if parts.iter().all(|p| !matches!(p, Part::Image { path, .. } if !path.is_file())) {
            return Ok(parts);
        }
    }
    let bytes = std::fs::read(f).map_err(|e| format!("{}: {e}", f.display()))?;
    let _ = std::fs::create_dir_all(&dir);
    let parts = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| layout(&bytes, &dir))).map_err(|_| "PDF не разобран (повреждён или необычный формат)".to_string())??;
    let _ = std::fs::write(&index, serde_json::to_vec(&parts).unwrap_or_default());
    Ok(parts)
}

/// Текст документа по страницам: `(номер с 1, текст, распознана ли моделью)`.
pub fn pages(f: &Path) -> Result<Vec<(usize, String, bool)>, String> {
    let parts = sequence(f)?;
    let mut out: Vec<(usize, String, bool)> = Vec::new();
    let mut scans: BTreeMap<usize, PathBuf> = BTreeMap::new();
    for p in parts {
        match p {
            Part::Page(n) => out.push((n, String::new(), false)),
            Part::Text(t) => {
                if let Some(last) = out.last_mut() {
                    if !last.1.is_empty() {
                        last.1.push_str("\n\n");
                    }
                    last.1.push_str(&t);
                }
            }
            Part::Image { path, scan: true } => {
                if let Some(last) = out.last() {
                    scans.insert(last.0, path);
                }
            }
            Part::Image { .. } => {}
        }
    }
    // страница без текстового слоя — распознать её картинку
    for page in out.iter_mut() {
        if page.1.chars().filter(|c| c.is_alphanumeric()).count() >= SCAN_MIN_CHARS {
            continue;
        }
        if let Some(img) = scans.get(&page.0) {
            if let Ok(t) = ocr_image(img) {
                if !t.trim().is_empty() {
                    page.1 = t.trim().to_string();
                    page.2 = true;
                }
            }
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

fn cache_dir(f: &Path) -> PathBuf {
    crate::attach::cache_root().join("pdf").join(crate::attach::cache_key(f))
}

// ---------------------------------------------------------------- раскладка

/// Матрица PDF `[a b c d e f]`.
type M = [f64; 6];
const ID: M = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `m × n` (сначала m, потом n) — как `cm` применяется к текущей CTM.
fn mul(m: &M, n: &M) -> M {
    [
        m[0] * n[0] + m[1] * n[2],
        m[0] * n[1] + m[1] * n[3],
        m[2] * n[0] + m[3] * n[2],
        m[2] * n[1] + m[3] * n[3],
        m[4] * n[0] + m[5] * n[2] + n[4],
        m[4] * n[1] + m[5] * n[3] + n[5],
    ]
}

fn num(o: &Object) -> f64 {
    match o {
        Object::Integer(i) => *i as f64,
        Object::Real(r) => *r as f64,
        _ => 0.0,
    }
}

fn matrix(ops: &[Object]) -> Option<M> {
    (ops.len() == 6).then(|| [num(&ops[0]), num(&ops[1]), num(&ops[2]), num(&ops[3]), num(&ops[4]), num(&ops[5])])
}

/// Картинка на странице: объект и рамка в пунктах (`x0, y0, x1, y1`, PDF-координаты).
struct Placed {
    id: ObjectId,
    rect: (f64, f64, f64, f64),
}

/// XObject по имени из ресурсов: `(id, это картинка?)`.
fn xobject(doc: &Document, res: &[&lopdf::Dictionary], name: &[u8]) -> Option<(ObjectId, bool)> {
    for r in res {
        let Ok(xo) = r.get(b"XObject").and_then(|o| doc.dereference(o)).and_then(|(_, o)| o.as_dict()) else { continue };
        let Ok(Object::Reference(id)) = xo.get(name) else { continue };
        let Ok(stream) = doc.get_object(*id).and_then(Object::as_stream) else { continue };
        let image = stream.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Image".as_slice());
        return Some((*id, image));
    }
    None
}

/// Пройти поток содержимого: найти картинки (с местом), убрать их вызовы.
/// Возвращает новый поток (без `Do` картинок) и найденные картинки; формы
/// (`Form`) обходятся рекурсивно и правятся в `forms`.
fn walk(doc: &Document, content: &[u8], res: &[&lopdf::Dictionary], ctm0: M, depth: usize, found: &mut Vec<Placed>, forms: &mut BTreeMap<ObjectId, Vec<u8>>, seen: &mut HashSet<ObjectId>) -> Option<Vec<u8>> {
    let content = Content::decode(content).ok()?;
    let mut stack: Vec<M> = Vec::new();
    let mut ctm = ctm0;
    let mut kept: Vec<Operation> = Vec::with_capacity(content.operations.len());
    let mut changed = false;
    for op in content.operations {
        match op.operator.as_str() {
            "q" => stack.push(ctm),
            "Q" => ctm = stack.pop().unwrap_or(ctm0),
            "cm" => {
                if let Some(m) = matrix(&op.operands) {
                    ctm = mul(&m, &ctm);
                }
            }
            "Do" => {
                let name = op.operands.first().and_then(|o| o.as_name().ok()).map(<[u8]>::to_vec).unwrap_or_default();
                match xobject(doc, res, &name) {
                    Some((id, true)) => {
                        let xs = [ctm[4], ctm[0] + ctm[4], ctm[2] + ctm[4], ctm[0] + ctm[2] + ctm[4]];
                        let ys = [ctm[5], ctm[1] + ctm[5], ctm[3] + ctm[5], ctm[1] + ctm[3] + ctm[5]];
                        let min = |v: &[f64; 4]| v.iter().cloned().fold(f64::INFINITY, f64::min);
                        let max = |v: &[f64; 4]| v.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                        found.push(Placed { id, rect: (min(&xs), min(&ys), max(&xs), max(&ys)) });
                        changed = true;
                        continue;
                    }
                    Some((id, false)) if depth < 4 => {
                        if let Ok(form) = doc.get_object(id).and_then(Object::as_stream) {
                            let fm = form.dict.get(b"Matrix").ok().and_then(|m| m.as_array().ok()).and_then(|a| matrix(a)).unwrap_or(ID);
                            let fres: Vec<&lopdf::Dictionary> = form.dict.get(b"Resources").ok().and_then(|r| doc.dereference(r).ok()).and_then(|(_, o)| o.as_dict().ok()).into_iter().chain(res.iter().copied()).collect();
                            let body = form.decompressed_content().unwrap_or_else(|_| form.content.clone());
                            if let Some(new) = walk(doc, &body, &fres, mul(&fm, &ctm), depth + 1, found, forms, seen) {
                                if seen.insert(id) {
                                    forms.insert(id, new);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        kept.push(op);
    }
    if !changed && depth > 0 {
        // форма без картинок — менять не нужно
        return None;
    }
    Content { operations: kept }.encode().ok()
}

/// Строка текста страницы: верх (пункты от верха страницы), левый край, текст, кегль.
#[derive(Clone, Debug)]
struct Line {
    top: f64,
    x: f64,
    text: String,
    size: f64,
}

/// Приёмник символов `pdf-extract`: строки с координатами по страницам.
#[derive(Default)]
struct Collector {
    pages: BTreeMap<u32, Vec<Line>>,
    page: u32,
    height: f64,
    cur: Option<Line>,
    last_end: f64,
    last_y: f64,
    first_char: bool,
}

impl Collector {
    fn flush(&mut self) {
        if let Some(l) = self.cur.take() {
            if !l.text.trim().is_empty() {
                self.pages.entry(self.page).or_default().push(l);
            }
        }
    }
}

impl pdf_extract::OutputDev for Collector {
    fn begin_page(&mut self, page_num: u32, media_box: &pdf_extract::MediaBox, _: Option<(f64, f64, f64, f64)>) -> Result<(), pdf_extract::OutputError> {
        self.flush();
        self.page = page_num;
        self.height = media_box.ury - media_box.lly;
        self.pages.entry(page_num).or_default();
        Ok(())
    }
    fn end_page(&mut self) -> Result<(), pdf_extract::OutputError> {
        self.flush();
        Ok(())
    }
    fn output_character(&mut self, trm: &pdf_extract::Transform, width: f64, _spacing: f64, font_size: f64, ch: &str) -> Result<(), pdf_extract::OutputError> {
        let (x, y) = (trm.m31, trm.m32);
        let fs = (font_size * trm.m11.hypot(trm.m12) * font_size * trm.m21.hypot(trm.m22)).abs().sqrt().max(1.0);
        let new_line = match &self.cur {
            None => true,
            Some(_) => (y - self.last_y).abs() > fs * 0.5 && (x < self.last_end || (y - self.last_y).abs() > fs * 1.5),
        };
        if new_line {
            self.flush();
            self.cur = Some(Line { top: self.height - y - fs, x, text: String::new(), size: fs });
        } else if self.first_char && x > self.last_end + fs * 0.1 {
            if let Some(l) = self.cur.as_mut() {
                l.text.push(' ');
            }
        }
        if let Some(l) = self.cur.as_mut() {
            l.text.push_str(ch);
        }
        self.first_char = false;
        self.last_y = y;
        self.last_end = x + width * fs;
        Ok(())
    }
    fn begin_word(&mut self) -> Result<(), pdf_extract::OutputError> {
        self.first_char = true;
        Ok(())
    }
    fn end_word(&mut self) -> Result<(), pdf_extract::OutputError> {
        Ok(())
    }
    fn end_line(&mut self) -> Result<(), pdf_extract::OutputError> {
        Ok(())
    }
}

/// Разложить документ: картинки сохраняются в `dir`.
fn layout(bytes: &[u8], dir: &Path) -> Result<Vec<Part>, String> {
    let mut doc = Document::load_mem(bytes).map_err(|e| format!("PDF не разобран: {e}"))?;
    let pages: Vec<(u32, ObjectId)> = doc.get_pages().into_iter().collect();
    // 1. картинки каждой страницы с местом; их вызовы — из потоков
    let mut images: BTreeMap<u32, Vec<Placed>> = BTreeMap::new();
    let mut boxes: BTreeMap<u32, (f64, f64)> = BTreeMap::new();
    let mut forms: BTreeMap<ObjectId, Vec<u8>> = BTreeMap::new();
    let mut seen = HashSet::new();
    let mut new_content: Vec<(ObjectId, Vec<u8>)> = Vec::new();
    for (n, id) in &pages {
        let mb = page_box(&doc, *id);
        boxes.insert(*n, mb);
        let Ok(content) = doc.get_page_content(*id) else { continue };
        let (own, ids) = doc.get_page_resources(*id).unwrap_or((None, Vec::new()));
        let mut res: Vec<&lopdf::Dictionary> = own.into_iter().collect();
        res.extend(ids.iter().filter_map(|i| doc.get_dictionary(*i).ok()));
        let mut found = Vec::new();
        if let Some(c) = walk(&doc, &content, &res, ID, 0, &mut found, &mut forms, &mut seen) {
            if !found.is_empty() {
                new_content.push((*id, c));
            }
        }
        images.insert(*n, found);
    }
    for (id, c) in new_content {
        let _ = doc.change_page_content(id, c);
    }
    for (id, c) in forms {
        if let Ok(Object::Stream(s)) = doc.get_object_mut(id) {
            s.set_plain_content(c);
        }
    }
    // 2. текст страниц со строками и координатами
    let mut col = Collector::default();
    let text_ok = pdf_extract::output_doc(&doc, &mut col).is_ok();
    col.flush();
    // 3. слить: сверху вниз
    let mut out = Vec::new();
    let mut k = 0usize;
    for (n, _) in &pages {
        out.push(Part::Page(*n as usize));
        let (w, h) = boxes.get(n).copied().unwrap_or((612.0, 792.0));
        let lines = if text_ok { col.pages.get(n).cloned().unwrap_or_default() } else { Vec::new() };
        let letters: usize = lines.iter().map(|l| l.text.chars().filter(|c| c.is_alphanumeric()).count()).sum();
        let mut items: Vec<(f64, f64, Option<Line>, Option<(PathBuf, bool)>)> = lines.into_iter().map(|l| (l.top, l.x, Some(l), None)).collect();
        let placed = images.remove(n).unwrap_or_default();
        let big = placed.iter().map(|p| (p.rect.2 - p.rect.0) * (p.rect.3 - p.rect.1)).fold(0.0, f64::max);
        for p in placed {
            let (pw, ph) = (p.rect.2 - p.rect.0, p.rect.3 - p.rect.1);
            let area = pw * ph;
            if area < w * h * MIN_AREA_FRACTION || pw < MIN_SIDE_PT || ph < MIN_SIDE_PT || pw.max(ph) > MAX_ASPECT * pw.min(ph) {
                continue;
            }
            let scan = letters < SCAN_MIN_CHARS && area >= big && area >= w * h * 0.5;
            let Some(file) = save_image(&doc, p.id, dir, *n, k) else { continue };
            k += 1;
            items.push((h - p.rect.3, p.rect.0, None, Some((file, scan))));
        }
        items.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        // строки подряд — в абзацы (пустая строка при большом зазоре)
        let mut block = String::new();
        let mut prev: Option<(f64, f64)> = None;
        for (_, _, line, img) in items {
            match (line, img) {
                (Some(l), _) => {
                    if let Some((top, size)) = prev {
                        block.push('\n');
                        if l.top - top > size.max(l.size) * 2.2 {
                            block.push('\n');
                        }
                    }
                    block.push_str(l.text.trim_end());
                    prev = Some((l.top, l.size));
                }
                (None, Some((path, scan))) => {
                    if !block.trim().is_empty() {
                        out.push(Part::Text(tidy(&block)));
                    }
                    block.clear();
                    prev = None;
                    out.push(Part::Image { path, scan });
                }
                _ => {}
            }
        }
        if !block.trim().is_empty() {
            out.push(Part::Text(tidy(&block)));
        }
    }
    Ok(out)
}

/// Размер страницы (ширина, высота) в пунктах.
fn page_box(doc: &Document, id: ObjectId) -> (f64, f64) {
    let mut cur = Some(id);
    for _ in 0..8 {
        let Some(i) = cur else { break };
        let Ok(d) = doc.get_dictionary(i) else { break };
        if let Ok(a) = d.get(b"MediaBox").and_then(|o| doc.dereference(o)).and_then(|(_, o)| o.as_array()) {
            if a.len() == 4 {
                return ((num(&a[2]) - num(&a[0])).abs(), (num(&a[3]) - num(&a[1])).abs());
            }
        }
        cur = d.get(b"Parent").ok().and_then(|p| p.as_reference().ok());
    }
    (612.0, 792.0)
}

/// Сохранить картинку объекта: JPEG как есть, серые/RGB 8 бит — в PNG.
fn save_image(doc: &Document, id: ObjectId, dir: &Path, page: u32, k: usize) -> Option<PathBuf> {
    let stream = doc.get_object(id).ok()?.as_stream().ok()?;
    let d = &stream.dict;
    let int = |k: &[u8]| d.get(k).and_then(|v| v.as_i64()).unwrap_or(0);
    let (w, h) = (int(b"Width"), int(b"Height"));
    if w < MIN_SIDE_PX || h < MIN_SIDE_PX || w * h < MIN_PIXELS || (w.max(h) as f64) > MAX_ASPECT * w.min(h) as f64 {
        return None;
    }
    let filters: Vec<Vec<u8>> = match d.get(b"Filter") {
        Ok(Object::Name(n)) => vec![n.clone()],
        Ok(Object::Array(a)) => a.iter().filter_map(|o| o.as_name().ok().map(<[u8]>::to_vec)).collect(),
        _ => Vec::new(),
    };
    let stem = format!("стр-{page:03}-{k:02}");
    if filters.iter().any(|f| f == b"DCTDecode") {
        let p = dir.join(format!("{stem}.jpg"));
        std::fs::write(&p, &stream.content).ok()?;
        return Some(p);
    }
    let raw = if filters.is_empty() { stream.content.clone() } else { stream.decompressed_content().ok()? };
    let cs = d.get(b"ColorSpace").and_then(|c| doc.dereference(c)).ok().and_then(|(_, o)| o.as_name().ok().map(<[u8]>::to_vec)).unwrap_or_default();
    let ch = match cs.as_slice() {
        b"DeviceGray" => 1u8,
        b"DeviceRGB" => 3,
        _ => return None,
    };
    if int(b"BitsPerComponent") != 8 || (raw.len() as i64) < w * h * ch as i64 {
        return None;
    }
    let png = encode_png(&raw, w as u32, h as u32, ch)?;
    let p = dir.join(format!("{stem}.png"));
    std::fs::write(&p, png).ok()?;
    Some(p)
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

/// Кириллица в старых PDF без таблицы Unicode у шрифта читается как
/// латиница: байты CP1251 — как Windows-1252 («Ëþäè»), Mac-кириллица — как
/// MacRoman («é·‡ÚËÚÂ»). Строка без кириллицы, где таких букв не меньше
/// половины, перекодируется обратно; берётся вариант, где вышла кириллица.
pub fn fix_cyrillic(line: &str) -> String {
    let letters = line.chars().filter(|c| c.is_alphabetic()).count();
    let cyr = line.chars().filter(|c| ('\u{400}'..='\u{4FF}').contains(c)).count();
    let odd = line.chars().filter(|c| ('\u{80}'..='\u{2122}').contains(c) && !c.is_ascii()).count();
    if letters == 0 || cyr > 0 || odd * 2 < letters {
        return line.to_string();
    }
    let mut best: Option<(f64, String)> = None;
    for (from, to) in [(encoding_rs::WINDOWS_1252, encoding_rs::WINDOWS_1251), (encoding_rs::MACINTOSH, encoding_rs::X_MAC_CYRILLIC)] {
        let (bytes, _, bad) = from.encode(line);
        if bad {
            continue;
        }
        let (text, _, bad) = to.decode(&bytes);
        if bad {
            continue;
        }
        // доля кириллицы среди не-ASCII знаков: у неверной пары остаются «‚‡ˆ»
        let l = text.chars().filter(|c| !c.is_ascii()).count().max(1);
        let share = text.chars().filter(|c| ('\u{400}'..='\u{4FF}').contains(c)).count() as f64 / l as f64;
        if share >= 0.6 && best.as_ref().is_none_or(|(s, _)| share > *s) {
            best = Some((share, text.into_owned()));
        }
    }
    best.map(|(_, t)| t).unwrap_or_else(|| line.to_string())
}

/// Убрать мусор разбора: \r, хвостовые пробелы, больше двух пустых строк подряд.
fn tidy(t: &str) -> String {
    let mut out = String::with_capacity(t.len());
    let mut blank = 0;
    for l in t.replace('\r', "").lines() {
        let l = fix_cyrillic(l.trim_end());
        let l = l.as_str();
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

/// Распознать текст страницы-скана моделью службы со зрением (самой лёгкой).
fn ocr_image(path: &Path) -> Result<String, String> {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_cyrillic_is_restored() {
        assert_eq!(fix_cyrillic("Ëþäè: Ëîðäåðîíñêèé Àëüÿíñ"), "Люди: Лордеронский Альянс");
        assert_eq!(fix_cyrillic("‚ÌËÏ‡ÌËÂ"), "внимание");
        // обычный текст не трогается
        assert_eq!(fix_cyrillic("Café au lait"), "Café au lait");
        assert_eq!(fix_cyrillic("Удар грома"), "Удар грома");
    }

    #[test]
    fn tidy_squeezes_blank_lines() {
        assert_eq!(tidy("a  \r\n\n\n\nb\n"), "a\n\nb");
    }

    /// Текст — картинка — текст на одной странице: порядок чтения сохраняется,
    /// мелкий значок отброшен, картинка сохранена файлом.
    #[test]
    fn text_and_images_in_reading_order() {
        let d = tempfile::tempdir().unwrap();
        let bytes = test_pdf();
        let parts = layout(&bytes, d.path()).unwrap();
        let kinds: Vec<String> = parts
            .iter()
            .map(|p| match p {
                Part::Page(n) => format!("page{n}"),
                Part::Text(t) => format!("text:{}", t.split_whitespace().next().unwrap_or("")),
                Part::Image { scan, .. } => format!("image{}", if *scan { ":scan" } else { "" }),
            })
            .collect();
        assert_eq!(kinds, vec!["page1", "text:BEFORE", "image", "text:AFTER"], "{parts:?}");
        let Part::Image { path, .. } = &parts[2] else { unreachable!() };
        assert!(path.is_file());
    }

    /// Вложение PDF в сообщении: маркеров столько же, сколько медиа, и
    /// картинка стоит между текстом «до» и «после» (служба ставит её на маркер).
    #[test]
    fn attachment_message_keeps_document_order() {
        let d = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_CACHE_HOME", d.path().join("cache"));
        let pdf = d.path().join("doc.pdf");
        std::fs::write(&pdf, test_pdf()).unwrap();
        let photo = d.path().join("photo.png");
        std::fs::write(&photo, b"x").unwrap();
        let (text, media) = crate::attach::describe(&[photo.clone(), pdf.clone()]);
        assert_eq!(text.matches(synaptixd::MEDIA_MARK).count(), media.len(), "{text}");
        assert_eq!(media.len(), 2);
        assert_eq!(media[0].path, photo.to_string_lossy());
        let (before, after, mark) = (text.find("BEFORE").unwrap(), text.find("AFTER").unwrap(), text.rfind(synaptixd::MEDIA_MARK).unwrap());
        assert!(before < mark && mark < after, "{text}");
    }

    /// Страница — одна картинка во весь лист без текста: скан на своём месте.
    #[test]
    fn scan_page_is_an_image() {
        let d = tempfile::tempdir().unwrap();
        let parts = layout(&scan_pdf(), d.path()).unwrap();
        assert!(matches!(parts.as_slice(), [Part::Page(1), Part::Image { scan: true, .. }]), "{parts:?}");
    }

    fn rgb(w: usize, h: usize) -> Vec<u8> {
        (0..w * h).flat_map(|i| [(i % 251) as u8, 120, 200]).collect()
    }

    /// PDF из объектов (смещения xref считаются); `objs[i]` — объект `i+1`.
    fn build(objs: Vec<Vec<u8>>) -> Vec<u8> {
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offs = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offs.push(out.len());
            out.extend(format!("{} 0 obj\n", i + 1).bytes());
            out.extend(o);
            out.extend(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).bytes());
        for o in offs {
            out.extend(format!("{o:010} 00000 n \n").bytes());
        }
        out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).bytes());
        out
    }

    fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
        let mut v = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
        v.extend(data);
        v.extend(b"\nendstream");
        v
    }

    fn test_pdf() -> Vec<u8> {
        // сверху текст, ниже большая картинка, под ней текст; в углу значок 10×10 pt
        let content = "BT /F1 14 Tf 72 740 Td (BEFORE the picture) Tj ET\nq 300 0 0 200 72 500 cm /Im1 Do Q\nq 10 0 0 10 560 780 cm /Im2 Do Q\nBT /F1 14 Tf 72 460 Td (AFTER the picture) Tj ET";
        build(vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> /XObject << /Im1 6 0 R /Im2 7 0 R >> >> >>".to_vec(),
            stream("", content.as_bytes()),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
            stream("/Type /XObject /Subtype /Image /Width 200 /Height 200 /ColorSpace /DeviceRGB /BitsPerComponent 8", &rgb(200, 200)),
            stream("/Type /XObject /Subtype /Image /Width 64 /Height 64 /ColorSpace /DeviceRGB /BitsPerComponent 8", &rgb(64, 64)),
        ])
    }

    fn scan_pdf() -> Vec<u8> {
        let content = "q 612 0 0 792 0 0 cm /Im1 Do Q";
        build(vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /XObject << /Im1 5 0 R >> >> >>".to_vec(),
            stream("", content.as_bytes()),
            stream("/Type /XObject /Subtype /Image /Width 200 /Height 260 /ColorSpace /DeviceRGB /BitsPerComponent 8", &rgb(200, 260)),
        ])
    }
}

#[cfg(test)]
mod real {
    /// Ручная проверка на своём PDF: `SYN_PDF=файл.pdf cargo test -p syn-assist real_pdf -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_pdf() {
        let p = std::path::PathBuf::from(std::env::var("SYN_PDF").unwrap());
        for part in super::sequence(&p).unwrap() {
            match part {
                super::Part::Page(n) => println!("== стр. {n}"),
                super::Part::Text(t) => println!("[текст {} зн.] {}", t.chars().count(), t.lines().next().unwrap_or("")),
                super::Part::Image { path, scan } => println!("[картинка{}] {}", if scan { " — скан" } else { "" }, path.display()),
            }
        }
    }
}
