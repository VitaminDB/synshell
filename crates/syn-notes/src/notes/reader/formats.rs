//! Разбор перекомпоновываемых форматов в [`FlowDoc`]: EPUB (2/3), FB2 и
//! FB2 в zip, TXT (UTF-8/UTF-16/CP1251/KOI8-R), Markdown.

use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;

use super::dom::{self, Elem, Node};
use super::model::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Pdf,
    Epub,
    Fb2,
    Fb2Zip,
    Txt,
    Markdown,
}

impl Format {
    /// По содержимому, затем по имени файла.
    pub fn detect(name: &str, data: &[u8]) -> Format {
        let lower = name.to_lowercase();
        if data.starts_with(b"%PDF") || lower.ends_with(".pdf") {
            return Format::Pdf;
        }
        if data.starts_with(b"PK") {
            if let Ok(z) = zip::ZipArchive::new(std::io::Cursor::new(data)) {
                if z.file_names().any(|n| n.eq_ignore_ascii_case("META-INF/container.xml")) {
                    return Format::Epub;
                }
                if z.file_names().any(|n| n.to_lowercase().ends_with(".fb2")) {
                    return Format::Fb2Zip;
                }
            }
            if lower.ends_with(".epub") {
                return Format::Epub;
            }
        }
        let head = String::from_utf8_lossy(&data[..data.len().min(1024)]);
        if head.contains("<FictionBook") || lower.ends_with(".fb2") {
            return Format::Fb2;
        }
        if lower.ends_with(".md") || lower.ends_with(".markdown") {
            return Format::Markdown;
        }
        Format::Txt
    }

    pub fn ext(self) -> &'static str {
        match self {
            Format::Pdf => "pdf",
            Format::Epub => "epub",
            Format::Fb2 => "fb2",
            Format::Fb2Zip => "fb2.zip",
            Format::Txt => "txt",
            Format::Markdown => "md",
        }
    }
}

pub fn parse_flow(format: Format, data: &[u8]) -> Result<FlowDoc, String> {
    match format {
        Format::Epub => parse_epub(data),
        Format::Fb2 => Ok(parse_fb2(&decode_xml(data))),
        Format::Fb2Zip => {
            let mut z = zip::ZipArchive::new(std::io::Cursor::new(data)).map_err(|e| e.to_string())?;
            let name = z.file_names().find(|n| n.to_lowercase().ends_with(".fb2")).map(str::to_string).ok_or("нет .fb2 в архиве")?;
            let mut buf = Vec::new();
            z.by_name(&name).map_err(|e| e.to_string())?.read_to_end(&mut buf).map_err(|e| e.to_string())?;
            Ok(parse_fb2(&decode_xml(&buf)))
        }
        Format::Txt => Ok(parse_txt(&decode_text(data))),
        Format::Markdown => Ok(parse_md(&decode_text(data))),
        Format::Pdf => Err("PDF — не перекомпоновываемый формат".into()),
    }
}

// ─────────────────────────── Кодировки ───────────────────────────

/// Текст неизвестной кодировки: BOM, UTF-8, иначе CP1251 или KOI8-R —
/// что даёт больше частых русских букв.
pub fn decode_text(data: &[u8]) -> String {
    if let Some((enc, bom)) = encoding_rs::Encoding::for_bom(data) {
        return enc.decode_without_bom_handling(&data[bom..]).0.into_owned();
    }
    if let Ok(s) = std::str::from_utf8(data) {
        return s.to_string();
    }
    let score = |s: &str| -> i64 {
        s.chars()
            .map(|c| match c {
                'о' | 'е' | 'а' | 'и' | 'н' | 'т' | 'с' | 'р' | 'в' | 'л' => 3,
                'а'..='я' | 'ё' => 1,
                'А'..='Я' | 'Ё' => 0,
                c if c.is_ascii() => 0,
                _ => -2,
            })
            .sum()
    };
    let cp = encoding_rs::WINDOWS_1251.decode_without_bom_handling(data).0;
    let koi = encoding_rs::KOI8_R.decode_without_bom_handling(data).0;
    if score(&koi) > score(&cp) { koi.into_owned() } else { cp.into_owned() }
}

/// XML с кодировкой из пролога (`encoding="windows-1251"`).
pub fn decode_xml(data: &[u8]) -> String {
    if encoding_rs::Encoding::for_bom(data).is_none() && std::str::from_utf8(data).is_err() {
        let head = String::from_utf8_lossy(&data[..data.len().min(200)]).to_lowercase();
        if let Some(p) = head.find("encoding=") {
            let rest = &head[p + 9..];
            let label: String = rest.trim_start_matches(['"', '\'']).chars().take_while(|c| *c != '"' && *c != '\'').collect();
            if let Some(enc) = encoding_rs::Encoding::for_label(label.trim().as_bytes()) {
                return enc.decode_without_bom_handling(data).0.into_owned();
            }
        }
    }
    decode_text(data)
}

pub fn base64_decode(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => continue,
        } as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

// ─────────────────────────── TXT ───────────────────────────

fn is_chapter_line(l: &str) -> bool {
    let t = l.trim();
    if t.is_empty() || t.chars().count() > 60 {
        return false;
    }
    let low = t.to_lowercase();
    ["глава ", "часть ", "chapter ", "part ", "пролог", "эпилог", "prologue", "epilogue"]
        .iter()
        .any(|p| low.starts_with(p))
        || (t.chars().count() <= 6 && t.chars().all(|c| "IVXLC.".contains(c)) && t.len() > 0)
}

pub fn parse_txt(text: &str) -> FlowDoc {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = text.lines().collect();
    let blank = lines.iter().filter(|l| l.trim().is_empty()).count();
    // Абзацы через пустые строки, если их заметное число; иначе строка = абзац.
    let by_blank = blank * 8 >= lines.len().max(1) && blank > 0;
    let mut b = Builder::new();
    let mut first = true;
    let mut para = String::new();
    let flush = |b: &mut Builder, para: &mut String| {
        if !para.trim().is_empty() {
            b.start_block(BlockKind::Para);
            b.text(para.trim());
            b.end_block();
        }
        para.clear();
    };
    for l in lines {
        if is_chapter_line(l) {
            flush(&mut b, &mut para);
            if !first {
                b.section();
            }
            b.toc(l.trim().to_string(), 1);
            b.start_block(BlockKind::Heading(2));
            b.text(l.trim());
            b.end_block();
            first = false;
            continue;
        }
        if by_blank {
            if l.trim().is_empty() {
                flush(&mut b, &mut para);
            } else {
                // Строка с отступом — новый абзац и в «слитном» тексте.
                if l.starts_with("  ") || l.starts_with('\t') {
                    flush(&mut b, &mut para);
                }
                if !para.is_empty() {
                    para.push(' ');
                }
                para.push_str(l.trim());
            }
        } else {
            para.push_str(l.trim());
            flush(&mut b, &mut para);
        }
        if !l.trim().is_empty() {
            first = false;
        }
    }
    flush(&mut b, &mut para);
    b.finish()
}

// ─────────────────────────── Markdown ───────────────────────────

pub fn parse_md(text: &str) -> FlowDoc {
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
    let mut b = Builder::new();
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut quote = 0;
    let mut heading: Option<(u8, String)> = None;
    let mut first_heading = true;
    for ev in Parser::new_ext(text, Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES) {
        match ev {
            Event::Start(Tag::Heading { level, .. }) => {
                let lv = level as u8;
                if lv == 1 && !first_heading {
                    b.section();
                }
                first_heading = false;
                heading = Some((lv, String::new()));
                b.end_block();
                let pos = Pos::new(b.doc.blocks.len(), 0);
                b.doc.toc.push(TocEntry { title: String::new(), level: lv, pos });
                b.start_block(BlockKind::Heading(lv));
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((_, t)) = heading.take() {
                    if let Some(e) = b.doc.toc.last_mut() {
                        e.title = t.trim().to_string();
                    }
                }
                b.end_block();
            }
            Event::Start(Tag::Paragraph) => {
                if lists.is_empty() || !b.has_block() {
                    b.start_block(if quote > 0 { BlockKind::Quote } else { BlockKind::Para });
                }
            }
            Event::End(TagEnd::Paragraph) => {
                if lists.is_empty() {
                    b.end_block();
                }
            }
            Event::Start(Tag::BlockQuote(_)) => quote += 1,
            Event::End(TagEnd::BlockQuote(_)) => quote -= 1,
            Event::Start(Tag::List(start)) => lists.push(start),
            Event::End(TagEnd::List(_)) => {
                lists.pop();
                b.end_block();
            }
            Event::Start(Tag::Item) => {
                let depth = lists.len().saturating_sub(1) as u8;
                let ordered = lists.last_mut().and_then(|s| {
                    s.map(|n| {
                        *s = Some(n + 1);
                        n as u32
                    })
                });
                b.start_block(BlockKind::ListItem { ordered, depth });
            }
            Event::End(TagEnd::Item) => b.end_block(),
            Event::Start(Tag::CodeBlock(_)) => {
                b.start_block(BlockKind::Code);
                b.preserve = true;
            }
            Event::End(TagEnd::CodeBlock) => {
                b.end_block();
                b.preserve = false;
            }
            Event::Start(Tag::Emphasis) => b.push_style(ITALIC),
            Event::Start(Tag::Strong) => b.push_style(BOLD),
            Event::End(TagEnd::Emphasis) | Event::End(TagEnd::Strong) => b.pop_style(),
            Event::Start(Tag::Link { dest_url, .. }) => b.push_link(Some(&dest_url)),
            Event::End(TagEnd::Link) => b.pop_link(),
            Event::Start(Tag::Image { dest_url, .. }) => b.image(dest_url.to_string(), String::new()),
            Event::Text(t) => {
                if let Some((_, h)) = &mut heading {
                    h.push_str(&t);
                }
                b.text(&t);
            }
            Event::Code(t) => {
                b.push_style(CODE);
                b.text(&t);
                b.pop_style();
            }
            Event::SoftBreak => b.text(" "),
            Event::HardBreak => b.line_break(),
            Event::Rule => b.simple(BlockKind::Rule),
            Event::Start(Tag::TableRow) | Event::Start(Tag::TableHead) => b.start_block(BlockKind::Para),
            Event::End(TagEnd::TableCell) => b.text(" | "),
            Event::End(TagEnd::TableRow) | Event::End(TagEnd::TableHead) => b.end_block(),
            _ => {}
        }
    }
    let mut d = b.finish();
    d.toc.retain(|e| !e.title.is_empty());
    d
}

// ─────────────────────────── FB2 ───────────────────────────

pub fn parse_fb2(xml: &str) -> FlowDoc {
    let root = dom::parse(xml);
    let fb = root.find("fictionbook").unwrap_or(&root);
    let mut b = Builder::new();
    if let Some(ti) = fb.find("title-info") {
        b.doc.meta.title = ti.child("book-title").map(|e| e.text().trim().to_string()).unwrap_or_default();
        let authors: Vec<String> = ti
            .elems()
            .filter(|e| e.name == "author")
            .map(|a| {
                ["first-name", "middle-name", "last-name"]
                    .iter()
                    .filter_map(|n| a.child(n).map(|e| e.text().trim().to_string()))
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .filter(|s| !s.is_empty())
            .collect();
        b.doc.meta.author = authors.join(", ");
        b.doc.meta.lang = ti.child("lang").map(|e| e.text().trim().to_string()).unwrap_or_default();
        if let Some(img) = ti.child("coverpage").and_then(|c| c.child("image")) {
            b.doc.meta.cover = img.attr("href").map(|h| h.trim_start_matches('#').to_string());
        }
    }
    for bin in fb.elems().filter(|e| e.name == "binary") {
        if let Some(id) = bin.attr("id") {
            let data = base64_decode(&bin.text());
            b.doc.images.insert(id.to_string(), BookImage { data: Arc::new(data), size: None });
        }
    }
    let mut first = true;
    for body in fb.elems().filter(|e| e.name == "body") {
        let notes = body.attr("name").is_some();
        if !first {
            b.section();
            if notes {
                let title = body.child("title").map(|t| t.text().trim().to_string()).filter(|s| !s.is_empty());
                let title = title.unwrap_or_else(|| "Примечания".to_string());
                b.toc(title.clone(), 1);
            }
        }
        first = false;
        fb2_container(&mut b, body, 0, notes);
    }
    b.finish()
}

fn fb2_title_text(e: &Elem) -> String {
    e.elems().map(|p| p.text().split_whitespace().collect::<Vec<_>>().join(" ")).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(". ")
}

fn fb2_container(b: &mut Builder, e: &Elem, depth: u8, notes: bool) {
    let mut first_section = true;
    for c in e.elems() {
        if let Some(id) = c.attr("id") {
            b.anchor(id);
        }
        match c.name.as_str() {
            "section" => {
                if depth == 0 && !first_section && !notes {
                    b.section();
                }
                first_section = false;
                if let Some(id) = c.attr("id") {
                    b.anchor(id);
                }
                if let Some(t) = c.child("title") {
                    let tt = fb2_title_text(t);
                    if !notes || depth > 0 {
                        b.toc(tt, depth + 1);
                    }
                }
                fb2_container(b, c, depth + 1, notes);
            }
            "title" => {
                b.start_block(BlockKind::Heading((depth + 1).min(6)));
                b.center();
                let mut firstp = true;
                for p in c.elems() {
                    if !firstp {
                        b.line_break();
                    }
                    firstp = false;
                    fb2_inline(b, p);
                }
                b.end_block();
            }
            "epigraph" => fb2_kind(b, c, BlockKind::Epigraph),
            "cite" => fb2_kind(b, c, BlockKind::Quote),
            "annotation" => fb2_kind(b, c, BlockKind::Quote),
            "poem" => {
                for s in c.elems() {
                    match s.name.as_str() {
                        "title" => {
                            b.start_block(BlockKind::Subtitle);
                            b.text(&fb2_title_text(s));
                            b.end_block();
                        }
                        "stanza" => {
                            for v in s.elems() {
                                b.start_block(if v.name == "v" { BlockKind::Verse } else { BlockKind::Subtitle });
                                fb2_inline(b, v);
                                b.end_block();
                            }
                            b.simple(BlockKind::Empty);
                        }
                        "text-author" => {
                            b.start_block(BlockKind::TextAuthor);
                            fb2_inline(b, s);
                            b.end_block();
                        }
                        _ => {}
                    }
                }
            }
            "image" => {
                let id = c.attr("href").unwrap_or("").trim_start_matches('#').to_string();
                if !id.is_empty() {
                    b.image(id, c.attr("alt").unwrap_or("").to_string());
                }
            }
            "empty-line" => b.simple(BlockKind::Empty),
            "subtitle" => {
                b.start_block(BlockKind::Subtitle);
                b.center();
                fb2_inline(b, c);
                b.end_block();
            }
            "text-author" => {
                b.start_block(BlockKind::TextAuthor);
                fb2_inline(b, c);
                b.end_block();
            }
            "table" => {
                for tr in c.elems() {
                    b.start_block(BlockKind::Para);
                    let cells: Vec<String> = tr.elems().map(|td| td.text().trim().to_string()).collect();
                    b.text(&cells.join(" | "));
                    b.end_block();
                }
            }
            "p" | "v" => {
                b.start_block(BlockKind::Para);
                fb2_inline(b, c);
                b.end_block();
            }
            _ => {}
        }
    }
}

fn fb2_kind(b: &mut Builder, e: &Elem, kind: BlockKind) {
    for c in e.elems() {
        match c.name.as_str() {
            "text-author" => {
                b.start_block(BlockKind::TextAuthor);
                fb2_inline(b, c);
                b.end_block();
            }
            "p" | "v" | "subtitle" => {
                b.start_block(kind.clone());
                fb2_inline(b, c);
                b.end_block();
            }
            "poem" | "empty-line" | "image" | "table" => {
                let wrap = Elem { name: String::new(), attrs: vec![], children: vec![Node::Elem(c.clone())] };
                fb2_container(b, &wrap, 1, false);
            }
            _ => {}
        }
    }
}

fn fb2_inline(b: &mut Builder, e: &Elem) {
    for n in &e.children {
        match n {
            Node::Text(t) => b.text(t),
            Node::Elem(c) => {
                let style = match c.name.as_str() {
                    "emphasis" => ITALIC,
                    "strong" => BOLD,
                    "code" => CODE,
                    "sup" => SUP,
                    "sub" => SUB,
                    _ => 0,
                };
                if c.name == "a" {
                    let href = c.attr("href").unwrap_or("");
                    b.push_link(Some(href));
                    fb2_inline(b, c);
                    b.pop_link();
                } else if c.name == "image" {
                    // Строчная картинка — отдельным блоком после абзаца не
                    // ставим; пропускаем.
                } else {
                    b.push_style(style);
                    fb2_inline(b, c);
                    b.pop_style();
                }
            }
        }
    }
}

// ─────────────────────────── EPUB ───────────────────────────

fn zip_read(z: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>, path: &str) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    let found = z.by_name(path).ok().map(|mut f| f.read_to_end(&mut buf));
    if found.is_none() {
        // Регистр и %-кодирование имён в ссылках не всегда совпадают.
        let dec = percent_decode(path);
        let name = z.file_names().find(|n| n.eq_ignore_ascii_case(&dec) || n.eq_ignore_ascii_case(path))?.to_string();
        z.by_name(&name).ok()?.read_to_end(&mut buf).ok()?;
    }
    Some(buf)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `dir/` + `../x.html#a` → нормализованный путь в архиве.
pub fn join_path(base_dir: &str, rel: &str) -> String {
    if rel.contains("://") {
        return rel.to_string();
    }
    let rel = percent_decode(rel);
    let mut parts: Vec<&str> = if rel.starts_with('/') { Vec::new() } else { base_dir.split('/').filter(|s| !s.is_empty()).collect() };
    for p in rel.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(p),
        }
    }
    parts.join("/")
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// Стили классов CSS: жирный/курсив/по центру.
#[derive(Default)]
struct Css {
    classes: HashMap<String, (u8, bool)>,
}

impl Css {
    fn parse_into(&mut self, src: &str) {
        let mut rest = src;
        while let Some(open) = rest.find('{') {
            let sel = &rest[..open];
            let Some(close) = rest[open..].find('}') else { break };
            let body = &rest[open + 1..open + close];
            let props = parse_decl(body);
            if props != (0, false) {
                for s in sel.split(',') {
                    let s = s.trim();
                    if let Some(cls) = s.rsplit('.').next().filter(|_| s.contains('.')) {
                        let cls = cls.split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_').next().unwrap_or("");
                        if !cls.is_empty() {
                            let e = self.classes.entry(cls.to_string()).or_default();
                            e.0 |= props.0;
                            e.1 |= props.1;
                        }
                    }
                }
            }
            rest = &rest[open + close + 1..];
        }
    }
}

fn parse_decl(body: &str) -> (u8, bool) {
    let mut style = 0;
    let mut center = false;
    for d in body.split(';') {
        let Some((k, v)) = d.split_once(':') else { continue };
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_ascii_lowercase());
        match k.as_str() {
            "font-weight" if v.starts_with("bold") || v.parse::<u32>().is_ok_and(|w| w >= 600) => style |= BOLD,
            "font-style" if v.starts_with("italic") || v.starts_with("oblique") => style |= ITALIC,
            "text-align" if v.starts_with("center") => center = true,
            _ => {}
        }
    }
    (style, center)
}

struct Html<'a, 'z, 'd> {
    css: &'a Css,
    file: String,
    dir: String,
    images: &'a mut HashMap<String, BookImage>,
    zip: &'z mut zip::ZipArchive<std::io::Cursor<&'d [u8]>>,
    lists: Vec<Option<u32>>,
    quote: u32,
}

const BLOCK_TAGS: &[&str] = &[
    "p", "div", "h1", "h2", "h3", "h4", "h5", "h6", "li", "ul", "ol", "blockquote", "pre", "table", "tr", "section",
    "article", "body", "header", "footer", "aside", "figure", "figcaption", "dl", "dt", "dd", "nav", "hr",
];

impl Html<'_, '_, '_> {
    fn class_style(&self, e: &Elem) -> (u8, bool) {
        let mut st = (0, false);
        if let Some(cls) = e.attr("class") {
            for c in cls.split_whitespace() {
                if let Some(&(s, cen)) = self.css.classes.get(c) {
                    st.0 |= s;
                    st.1 |= cen;
                }
            }
        }
        if let Some(s) = e.attr("style") {
            let (s2, c2) = parse_decl(s);
            st.0 |= s2;
            st.1 |= c2;
        }
        st
    }

    fn image(&mut self, b: &mut Builder, src: &str, alt: &str) {
        let path = join_path(&self.dir, src);
        if !self.images.contains_key(&path) {
            let Some(data) = zip_read(self.zip, &path) else { return };
            self.images.insert(path.clone(), BookImage { data: Arc::new(data), size: None });
        }
        b.image(path, alt.to_string());
    }

    fn walk(&mut self, b: &mut Builder, e: &Elem) {
        for n in &e.children {
            match n {
                Node::Text(t) => b.text(t),
                Node::Elem(c) => self.elem(b, c),
            }
        }
    }

    fn elem(&mut self, b: &mut Builder, c: &Elem) {
        let name = c.name.as_str();
        if matches!(name, "head" | "script" | "style" | "title") {
            return;
        }
        if let Some(id) = c.attr("id") {
            b.anchor(&format!("{}#{}", self.file, id));
        }
        let (cls_style, center) = self.class_style(c);
        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let lv = name[1..].parse().unwrap_or(1);
                b.start_block(BlockKind::Heading(lv));
                b.center();
                b.push_style(cls_style);
                self.walk(b, c);
                b.pop_style();
                b.end_block();
            }
            "p" => {
                b.start_block(if self.quote > 0 { BlockKind::Quote } else { BlockKind::Para });
                if center {
                    b.center();
                }
                b.push_style(cls_style);
                self.walk(b, c);
                b.pop_style();
                b.end_block();
            }
            "li" => {
                let depth = self.lists.len().saturating_sub(1) as u8;
                let ordered = self.lists.last_mut().and_then(|s| {
                    s.map(|n| {
                        *s = Some(n + 1);
                        n
                    })
                });
                b.start_block(BlockKind::ListItem { ordered, depth });
                self.walk(b, c);
                b.end_block();
            }
            "ul" | "ol" => {
                b.end_block();
                self.lists.push((name == "ol").then(|| c.attr("start").and_then(|s| s.parse().ok()).unwrap_or(1)));
                self.walk(b, c);
                self.lists.pop();
                b.end_block();
            }
            "blockquote" => {
                b.end_block();
                self.quote += 1;
                self.walk(b, c);
                self.quote -= 1;
                b.end_block();
            }
            "pre" => {
                b.start_block(BlockKind::Code);
                b.preserve = true;
                b.text(&c.text());
                b.end_block();
                b.preserve = false;
            }
            "br" => b.line_break(),
            "hr" => b.simple(BlockKind::Rule),
            "img" => {
                if let Some(src) = c.attr("src") {
                    let alt = c.attr("alt").unwrap_or("").to_string();
                    self.image(b, src, &alt);
                }
            }
            "image" => {
                if let Some(src) = c.attr("href") {
                    self.image(b, src, "");
                }
            }
            "tr" => {
                b.start_block(BlockKind::Para);
                let mut first = true;
                for td in c.elems() {
                    if !first {
                        b.text(" | ");
                    }
                    first = false;
                    self.walk(b, td);
                }
                b.end_block();
            }
            "a" => {
                let href = c.attr("href").map(|h| {
                    if h.contains("://") || h.starts_with("mailto:") {
                        h.to_string()
                    } else if let Some(frag) = h.strip_prefix('#') {
                        format!("{}#{}", self.file, frag)
                    } else {
                        join_path(&self.dir, h)
                    }
                });
                b.push_link(href.as_deref());
                b.push_style(cls_style);
                self.walk(b, c);
                b.pop_style();
                b.pop_link();
            }
            _ => {
                let inline = match name {
                    "b" | "strong" => BOLD,
                    "i" | "em" | "cite" | "dfn" | "var" => ITALIC,
                    "code" | "tt" | "kbd" | "samp" => CODE,
                    "sup" => SUP,
                    "sub" => SUB,
                    _ => 0,
                } | cls_style;
                let block = BLOCK_TAGS.contains(&name);
                if block {
                    b.end_block();
                }
                b.push_style(inline);
                self.walk(b, c);
                b.pop_style();
                if block {
                    if center {
                        b.center();
                    }
                    b.end_block();
                }
            }
        }
    }
}

pub fn parse_epub(data: &[u8]) -> Result<FlowDoc, String> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(data)).map_err(|e| format!("EPUB: {e}"))?;
    let container = zip_read(&mut z, "META-INF/container.xml").ok_or("EPUB: нет container.xml")?;
    let container = dom::parse(&decode_xml(&container));
    let opf_path = container.find("rootfile").and_then(|r| r.attr("full-path")).ok_or("EPUB: нет rootfile")?.to_string();
    let opf = dom::parse(&decode_xml(&zip_read(&mut z, &opf_path).ok_or("EPUB: нет OPF")?));
    let opf_dir = dir_of(&opf_path).to_string();
    let mut b = Builder::new();
    if let Some(meta) = opf.find("metadata") {
        b.doc.meta.title = meta.child("title").map(|e| e.text().trim().to_string()).unwrap_or_default();
        let mut creators = Vec::new();
        meta.find_all("creator", &mut creators);
        b.doc.meta.author = creators.iter().map(|e| e.text().trim().to_string()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ");
        b.doc.meta.lang = meta.child("language").map(|e| e.text().trim().to_string()).unwrap_or_default();
    }
    // Манифест: id → (путь, тип, свойства).
    let mut manifest: HashMap<String, (String, String, String)> = HashMap::new();
    let mut cover_id = None;
    if let Some(m) = opf.find("manifest") {
        for it in m.elems().filter(|e| e.name == "item") {
            let (Some(id), Some(href)) = (it.attr("id"), it.attr("href")) else { continue };
            let props = it.attr("properties").unwrap_or("").to_string();
            if props.contains("cover-image") {
                cover_id = Some(id.to_string());
            }
            manifest.insert(id.to_string(), (join_path(&opf_dir, href), it.attr("media-type").unwrap_or("").to_string(), props));
        }
    }
    if cover_id.is_none() {
        let mut metas = Vec::new();
        opf.find_all("meta", &mut metas);
        cover_id = metas.iter().find(|m| m.attr("name") == Some("cover")).and_then(|m| m.attr("content")).map(str::to_string);
    }
    let spine = opf.find("spine");
    let ncx_id = spine.and_then(|s| s.attr("toc")).map(str::to_string);
    let spine_items: Vec<String> = spine
        .map(|s| s.elems().filter(|e| e.name == "itemref").filter_map(|e| e.attr("idref").map(str::to_string)).collect())
        .unwrap_or_default();

    // Стили всех CSS книги.
    let mut css = Css::default();
    for (path, mt, _) in manifest.values() {
        if mt == "text/css" {
            if let Some(src) = zip_read(&mut z, path) {
                css.parse_into(&String::from_utf8_lossy(&src));
            }
        }
    }

    let mut images = HashMap::new();
    let mut first = true;
    for idref in &spine_items {
        let Some((path, mt, _)) = manifest.get(idref).cloned() else { continue };
        if !(mt.contains("html") || mt.contains("xml") || path.ends_with("htm") || path.ends_with("html")) {
            continue;
        }
        let Some(src) = zip_read(&mut z, &path) else { continue };
        let doc = dom::parse(&decode_xml(&src));
        if !first {
            b.section();
        }
        first = false;
        b.anchor(&path);
        let mut inline_css = Css::default();
        let mut styles = Vec::new();
        doc.find_all("style", &mut styles);
        for s in styles {
            inline_css.parse_into(&s.text());
        }
        for (k, v) in &css.classes {
            inline_css.classes.entry(k.clone()).or_insert(*v);
        }
        let body = doc.find("body").unwrap_or(&doc);
        let mut h = Html { css: &inline_css, file: path.clone(), dir: dir_of(&path).to_string(), images: &mut images, zip: &mut z, lists: Vec::new(), quote: 0 };
        h.walk(&mut b, body);
        b.end_block();
    }
    b.doc.images = images;
    if let Some(cid) = cover_id.and_then(|id| manifest.get(&id).cloned()) {
        if !b.doc.images.contains_key(&cid.0) {
            if let Some(data) = zip_read(&mut z, &cid.0) {
                b.doc.images.insert(cid.0.clone(), BookImage { data: Arc::new(data), size: None });
            }
        }
        b.doc.meta.cover = Some(cid.0);
    }

    // Оглавление: nav (EPUB 3), иначе NCX; ссылки — по якорям.
    let mut raw_toc: Vec<(String, u8, String)> = Vec::new();
    if let Some((nav_path, _, _)) = manifest.values().find(|(_, _, p)| p.split_whitespace().any(|x| x == "nav")).cloned() {
        if let Some(src) = zip_read(&mut z, &nav_path) {
            let doc = dom::parse(&decode_xml(&src));
            let mut navs = Vec::new();
            doc.find_all("nav", &mut navs);
            let toc_nav = navs.iter().find(|n| n.attr("type").is_some_and(|t| t.contains("toc"))).or(navs.first());
            if let Some(ol) = toc_nav.and_then(|n| n.find("ol")) {
                nav_list(ol, 1, dir_of(&nav_path), &mut raw_toc);
            }
        }
    }
    if raw_toc.is_empty() {
        if let Some((ncx_path, _, _)) = ncx_id.and_then(|id| manifest.get(&id).cloned()) {
            if let Some(src) = zip_read(&mut z, &ncx_path) {
                let doc = dom::parse(&decode_xml(&src));
                if let Some(map) = doc.find("navmap") {
                    ncx_points(map, 1, dir_of(&ncx_path), &mut raw_toc);
                }
            }
        }
    }
    let mut d = b.finish();
    for (title, level, href) in raw_toc {
        let block = d.anchors.get(&href).or_else(|| d.anchors.get(href.split('#').next().unwrap_or(""))).copied();
        if let Some(block) = block {
            d.toc.push(TocEntry { title, level, pos: Pos::new(block, 0) });
        }
    }
    if d.toc.is_empty() {
        d.toc = headings_toc(&d);
    }
    Ok(d)
}

fn nav_list(ol: &Elem, level: u8, dir: &str, out: &mut Vec<(String, u8, String)>) {
    for li in ol.elems().filter(|e| e.name == "li") {
        if let Some(a) = li.child("a").or_else(|| li.find("a")) {
            let title = a.text().split_whitespace().collect::<Vec<_>>().join(" ");
            if let Some(href) = a.attr("href") {
                out.push((title, level, join_path(dir, href)));
            }
        }
        if let Some(sub) = li.child("ol") {
            nav_list(sub, level + 1, dir, out);
        }
    }
}

fn ncx_points(e: &Elem, level: u8, dir: &str, out: &mut Vec<(String, u8, String)>) {
    for np in e.elems().filter(|e| e.name == "navpoint") {
        let title = np.child("navlabel").map(|l| l.text().split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default();
        if let Some(src) = np.child("content").and_then(|c| c.attr("src")) {
            out.push((title, level, join_path(dir, src)));
        }
        ncx_points(np, level + 1, dir, out);
    }
}

/// Оглавление по заголовкам, когда в книге своего нет.
pub fn headings_toc(d: &FlowDoc) -> Vec<TocEntry> {
    d.blocks
        .iter()
        .enumerate()
        .filter_map(|(i, b)| match b.kind {
            BlockKind::Heading(lv) if lv <= 3 => Some(TocEntry {
                title: b.text().replace('\n', " ").split_whitespace().collect::<Vec<_>>().join(" "),
                level: lv,
                pos: Pos::new(i, 0),
            }),
            _ => None,
        })
        .filter(|e| !e.title.is_empty())
        .collect()
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::io::Write;

    pub fn sample_epub() -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            z.start_file("mimetype", o).unwrap();
            z.write_all(b"application/epub+zip").unwrap();
            z.start_file("META-INF/container.xml", o).unwrap();
            z.write_all(br#"<?xml version="1.0"?><container><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#).unwrap();
            z.start_file("OEBPS/content.opf", o).unwrap();
            z.write_all(
                r#"<?xml version="1.0"?><package xmlns:dc="http://purl.org/dc/elements/1.1/"><metadata><dc:title>Тестовая книга</dc:title><dc:creator>Иван Петров</dc:creator><dc:language>ru</dc:language></metadata>
<manifest><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/><item id="c2" href="text/ch2.xhtml" media-type="application/xhtml+xml"/><item id="css" href="style.css" media-type="text/css"/><item id="img" href="img/pic.png" media-type="image/png"/></manifest>
<spine><itemref idref="c1"/><itemref idref="c2"/></spine></package>"#
                    .as_bytes(),
            )
            .unwrap();
            z.start_file("OEBPS/style.css", o).unwrap();
            z.write_all(b".em { font-style: italic } .c { text-align: center }").unwrap();
            z.start_file("OEBPS/nav.xhtml", o).unwrap();
            z.write_all(r#"<html><body><nav epub:type="toc"><ol><li><a href="text/ch1.xhtml">Глава первая</a></li><li><a href="text/ch2.xhtml#s2">Глава вторая</a></li></ol></nav></body></html>"#.as_bytes()).unwrap();
            z.start_file("OEBPS/text/ch1.xhtml", o).unwrap();
            z.write_all(r#"<html><head><title>x</title></head><body><h1>Глава первая</h1><p>Первый&nbsp;абзац <span class="em">курсивом</span> и <b>жирным</b>.</p><p>Сноска<a href="ch2.xhtml#n1"><sup>1</sup></a></p><img src="../img/pic.png" alt="рис"/></body></html>"#.as_bytes()).unwrap();
            z.start_file("OEBPS/text/ch2.xhtml", o).unwrap();
            z.write_all(r#"<html><body><h2 id="s2">Глава вторая</h2><ul><li>раз</li><li>два</li></ul><blockquote><p>Цитата</p></blockquote><p id="n1">Текст сноски</p></body></html>"#.as_bytes()).unwrap();
            z.start_file("OEBPS/img/pic.png", o).unwrap();
            let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
            png.extend(64u32.to_be_bytes());
            png.extend(32u32.to_be_bytes());
            png.extend([0; 8]);
            z.write_all(&png).unwrap();
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    pub const SAMPLE_FB2: &str = r##"<?xml version="1.0" encoding="utf-8"?>
<FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0" xmlns:l="http://www.w3.org/1999/xlink">
<description><title-info><author><first-name>Лев</first-name><last-name>Толстой</last-name></author><book-title>Рассказ</book-title><lang>ru</lang></title-info></description>
<body><title><p>Рассказ</p></title>
<section id="s1"><title><p>Глава 1</p><p>Начало</p></title><epigraph><p>Эпиграф</p><text-author>Кто-то</text-author></epigraph>
<p>Первый <emphasis>абзац</emphasis> со <strong>сноской</strong><a l:href="#n1" type="note">[1]</a>.</p><empty-line/>
<poem><stanza><v>Строка стиха один</v><v>Строка стиха два</v></stanza></poem><image l:href="#pic"/></section>
<section><title><p>Глава 2</p></title><p>Второй раздел.</p></section></body>
<body name="notes"><section id="n1"><title><p>1</p></title><p>Текст примечания.</p></section></body>
<binary id="pic" content-type="image/png">iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==</binary>
</FictionBook>"##;

    #[test]
    fn epub_structure() {
        let d = parse_epub(&sample_epub()).unwrap();
        assert_eq!(d.meta.title, "Тестовая книга");
        assert_eq!(d.meta.author, "Иван Петров");
        assert_eq!(d.sections.len(), 2);
        assert_eq!(d.toc.len(), 2);
        assert_eq!(d.toc[0].title, "Глава первая");
        let p = &d.blocks[1];
        assert_eq!(p.text(), "Первый\u{a0}абзац курсивом и жирным.");
        assert!(p.spans.iter().any(|s| s.text == "курсивом" && s.style & ITALIC != 0));
        assert!(p.spans.iter().any(|s| s.text == "жирным" && s.style & BOLD != 0));
        let img = d.blocks.iter().find(|b| matches!(b.kind, BlockKind::Image { .. })).unwrap();
        let BlockKind::Image { id, .. } = &img.kind else { unreachable!() };
        assert_eq!(d.images[id].size, Some((64, 32)));
        // Ссылка на сноску ведёт к абзацу во второй главе.
        let link = d.blocks[2].spans.iter().find_map(|s| s.link.clone()).unwrap();
        let target = d.resolve_link(&link).unwrap();
        assert_eq!(d.blocks[target].text(), "Текст сноски");
        let toc2 = d.toc[1].pos.block as usize;
        assert_eq!(d.blocks[toc2].text(), "Глава вторая");
        assert!(d.blocks.iter().any(|b| matches!(b.kind, BlockKind::ListItem { .. }) && b.text() == "два"));
        assert!(d.blocks.iter().any(|b| b.kind == BlockKind::Quote));
    }

    #[test]
    fn fb2_structure_and_cp1251() {
        let d = parse_fb2(SAMPLE_FB2);
        assert_eq!(d.meta.title, "Рассказ");
        assert_eq!(d.meta.author, "Лев Толстой");
        assert!(d.images.contains_key("pic"));
        assert_eq!(d.images["pic"].size, Some((1, 1)));
        assert!(d.toc.iter().any(|t| t.title == "Глава 1. Начало"));
        assert!(d.blocks.iter().any(|b| b.kind == BlockKind::Verse));
        assert!(d.blocks.iter().any(|b| b.kind == BlockKind::Epigraph));
        let link = d.blocks.iter().flat_map(|b| b.spans.iter()).find_map(|s| s.link.clone()).unwrap();
        let t = d.resolve_link(&link).unwrap();
        assert!(d.blocks[t..].iter().any(|b| b.text() == "Текст примечания."));
        // Та же книга в windows-1251 с объявленной кодировкой.
        let src = SAMPLE_FB2.replace("utf-8", "windows-1251");
        let (bytes, _, _) = encoding_rs::WINDOWS_1251.encode(&src);
        let d2 = parse_fb2(&decode_xml(&bytes));
        assert_eq!(d2.meta.author, "Лев Толстой");
    }

    #[test]
    fn txt_encodings_and_chapters() {
        let text = "Глава 1\n\nПервый абзац\nпродолжение.\n\nВторой абзац.\n\nГлава 2\n\nТретий.\n";
        for bytes in [
            text.as_bytes().to_vec(),
            encoding_rs::WINDOWS_1251.encode(text).0.into_owned(),
            encoding_rs::KOI8_R.encode(text).0.into_owned(),
        ] {
            let s = decode_text(&bytes);
            assert_eq!(s, text);
        }
        let d = parse_txt(text);
        assert_eq!(d.toc.len(), 2);
        assert_eq!(d.sections.len(), 2);
        assert!(d.blocks.iter().any(|b| b.text() == "Первый абзац продолжение."));
    }

    #[test]
    fn markdown() {
        let d = parse_md("# Заголовок\n\nТекст *курсив* **жирный**.\n\n- раз\n- два\n\n# Второй\n\n> цитата\n");
        assert_eq!(d.toc.len(), 2);
        assert_eq!(d.sections.len(), 2);
        assert!(d.blocks.iter().any(|b| b.kind == BlockKind::Quote));
        assert_eq!(d.blocks.iter().filter(|b| matches!(b.kind, BlockKind::ListItem { .. })).count(), 2);
    }

    #[test]
    fn detect_formats() {
        assert_eq!(Format::detect("a.epub", &sample_epub()), Format::Epub);
        assert_eq!(Format::detect("x", SAMPLE_FB2.as_bytes()), Format::Fb2);
        assert_eq!(Format::detect("x.pdf", b"%PDF-1.4"), Format::Pdf);
        assert_eq!(Format::detect("x.md", b"# hi"), Format::Markdown);
        assert_eq!(Format::detect("x.txt", b"hi"), Format::Txt);
    }
}
