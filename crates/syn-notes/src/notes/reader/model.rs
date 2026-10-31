//! Модель перекомпоновываемой книги (EPUB, FB2, TXT, Markdown): плоский
//! список блоков со стилевыми отрезками, разделы (с них страница всегда
//! начинается заново), оглавление, картинки и якоря ссылок.
//!
//! Позиция в книге — [`Pos`] (блок + символ внутри блока): она не зависит
//! от шрифта и размера экрана, поэтому переживает перевёрстку.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// Позиция в перекомпоновываемой книге: индекс блока и символ в нём.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Pos {
    pub block: u32,
    pub ch: u32,
}

impl Pos {
    pub const START: Pos = Pos { block: 0, ch: 0 };
    pub fn new(block: usize, ch: usize) -> Self {
        Self { block: block as u32, ch: ch as u32 }
    }
}

pub const BOLD: u8 = 1;
pub const ITALIC: u8 = 2;
pub const CODE: u8 = 4;
pub const SUP: u8 = 8;
pub const SUB: u8 = 16;
pub const LINK: u8 = 32;

/// Отрезок текста одного стиля.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Span {
    pub text: String,
    pub style: u8,
    /// Цель ссылки: `#якорь` (внутри книги) или внешний адрес.
    pub link: Option<Arc<str>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BlockKind {
    Para,
    /// Заголовок уровня 1…6.
    Heading(u8),
    Subtitle,
    Quote,
    /// Строка стиха.
    Verse,
    Epigraph,
    /// Подпись автора (эпиграф, цитата).
    TextAuthor,
    ListItem { ordered: Option<u32>, depth: u8 },
    Code,
    /// Картинка: ключ в [`FlowDoc::images`].
    Image { id: String, alt: String },
    Rule,
    /// Пустая строка (FB2 `empty-line`).
    Empty,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub kind: BlockKind,
    pub spans: Vec<Span>,
    /// Выравнивание по центру (CSS `text-align: center`).
    pub center: bool,
}

impl Block {
    pub fn new(kind: BlockKind) -> Self {
        Self { kind, spans: Vec::new(), center: false }
    }
    pub fn text(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }
    pub fn char_len(&self) -> usize {
        match self.kind {
            BlockKind::Image { .. } | BlockKind::Rule | BlockKind::Empty => 1,
            _ => self.spans.iter().map(|s| s.text.chars().count()).sum(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TocEntry {
    pub title: String,
    pub level: u8,
    pub pos: Pos,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BookMeta {
    pub title: String,
    pub author: String,
    pub lang: String,
    /// Обложка — ключ картинки.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<String>,
}

/// Картинка книги (сжатые байты как в файле).
#[derive(Clone, Debug)]
pub struct BookImage {
    pub data: Arc<Vec<u8>>,
    /// Размер в пикселях, если удалось узнать по заголовку.
    pub size: Option<(u32, u32)>,
}

#[derive(Clone, Debug, Default)]
pub struct FlowDoc {
    pub meta: BookMeta,
    pub blocks: Vec<Block>,
    /// Блоки, с которых начинается раздел (страница с новой). Всегда есть 0.
    pub sections: Vec<usize>,
    pub toc: Vec<TocEntry>,
    pub images: HashMap<String, BookImage>,
    /// Якорь (`id` элемента, `файл#id`) → блок.
    pub anchors: HashMap<String, usize>,
    /// Символов до начала блока (для процента прочитанного).
    pub char_offsets: Vec<usize>,
    pub total_chars: usize,
}

impl FlowDoc {
    /// Досчитать служебное после сборки: смещения, разделы, пустая книга.
    pub fn finish(mut self) -> Self {
        if self.blocks.is_empty() {
            let mut b = Block::new(BlockKind::Para);
            b.spans.push(Span { text: " ".into(), ..Default::default() });
            self.blocks.push(b);
        }
        self.sections.retain(|&s| s < self.blocks.len());
        self.sections.push(0);
        self.sections.sort_unstable();
        self.sections.dedup();
        let mut acc = 0;
        self.char_offsets = self
            .blocks
            .iter()
            .map(|b| {
                let at = acc;
                acc += b.char_len();
                at
            })
            .collect();
        self.total_chars = acc.max(1);
        for (_, img) in self.images.iter_mut() {
            if img.size.is_none() {
                img.size = image_size(&img.data);
            }
        }
        self
    }

    /// Раздел, в котором лежит блок.
    pub fn section_of(&self, block: usize) -> usize {
        match self.sections.binary_search(&block) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        }
    }

    /// Блоки раздела `[start, end)`.
    pub fn section_range(&self, section: usize) -> (usize, usize) {
        let start = self.sections.get(section).copied().unwrap_or(0);
        let end = self.sections.get(section + 1).copied().unwrap_or(self.blocks.len());
        (start, end)
    }

    /// Доля прочитанного 0…1 для позиции.
    pub fn progress(&self, pos: Pos) -> f32 {
        let b = (pos.block as usize).min(self.blocks.len().saturating_sub(1));
        let at = self.char_offsets.get(b).copied().unwrap_or(0) + pos.ch as usize;
        (at as f32 / self.total_chars as f32).clamp(0.0, 1.0)
    }

    /// Позиция по доле 0…1 (переход «к проценту»).
    pub fn pos_at_fraction(&self, f: f32) -> Pos {
        let target = (f.clamp(0.0, 1.0) * self.total_chars as f32) as usize;
        let b = match self.char_offsets.binary_search(&target) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let ch = target.saturating_sub(self.char_offsets.get(b).copied().unwrap_or(0));
        Pos::new(b, ch.min(self.blocks.get(b).map_or(0, |x| x.char_len().saturating_sub(1))))
    }

    /// Текст между позициями (цитата выделения).
    pub fn text_between(&self, a: Pos, b: Pos) -> String {
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let mut out = String::new();
        for bi in a.block as usize..=(b.block as usize).min(self.blocks.len().saturating_sub(1)) {
            let text = self.blocks[bi].text();
            let from = if bi == a.block as usize { a.ch as usize } else { 0 };
            let to = if bi == b.block as usize { b.ch as usize } else { usize::MAX };
            let piece: String = text.chars().skip(from).take(to.saturating_sub(from)).collect();
            if !out.is_empty() && !piece.is_empty() {
                out.push('\n');
            }
            out.push_str(&piece);
        }
        out
    }

    /// Поиск подстроки (без учёта регистра): позиции начала и конца.
    pub fn search(&self, query: &str, limit: usize) -> Vec<(Pos, Pos)> {
        let q: Vec<char> = query.to_lowercase().chars().collect();
        if q.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for (bi, b) in self.blocks.iter().enumerate() {
            let t: Vec<char> = b.text().to_lowercase().chars().collect();
            if t.len() < q.len() {
                continue;
            }
            let mut i = 0;
            while i + q.len() <= t.len() {
                if t[i..i + q.len()] == q[..] {
                    out.push((Pos::new(bi, i), Pos::new(bi, i + q.len())));
                    if out.len() >= limit {
                        return out;
                    }
                    i += q.len();
                } else {
                    i += 1;
                }
            }
        }
        out
    }

    /// Блок ссылки `#якорь` / `файл#якорь`.
    pub fn resolve_link(&self, link: &str) -> Option<usize> {
        let l = link.trim_start_matches('#');
        self.anchors
            .get(l)
            .or_else(|| l.split_once('#').and_then(|(file, id)| self.anchors.get(id).or_else(|| self.anchors.get(file))))
            .copied()
    }
}

/// Размер картинки по заголовку PNG/JPEG/GIF/WebP/BMP.
pub fn image_size(d: &[u8]) -> Option<(u32, u32)> {
    if d.len() > 24 && d.starts_with(b"\x89PNG") {
        let w = u32::from_be_bytes(d[16..20].try_into().ok()?);
        let h = u32::from_be_bytes(d[20..24].try_into().ok()?);
        return Some((w, h));
    }
    if d.len() > 10 && (d.starts_with(b"GIF87a") || d.starts_with(b"GIF89a")) {
        return Some((u16::from_le_bytes([d[6], d[7]]) as u32, u16::from_le_bytes([d[8], d[9]]) as u32));
    }
    if d.len() > 26 && d.starts_with(b"BM") {
        let w = i32::from_le_bytes(d[18..22].try_into().ok()?).unsigned_abs();
        let h = i32::from_le_bytes(d[22..26].try_into().ok()?).unsigned_abs();
        return Some((w, h));
    }
    if d.len() > 30 && &d[0..4] == b"RIFF" && &d[8..12] == b"WEBP" {
        return match &d[12..16] {
            b"VP8X" => Some((
                1 + u32::from_le_bytes([d[24], d[25], d[26], 0]),
                1 + u32::from_le_bytes([d[27], d[28], d[29], 0]),
            )),
            b"VP8 " => Some(((u16::from_le_bytes([d[26], d[27]]) & 0x3fff) as u32, (u16::from_le_bytes([d[28], d[29]]) & 0x3fff) as u32)),
            b"VP8L" => {
                let b = u32::from_le_bytes(d[21..25].try_into().ok()?);
                Some(((b & 0x3fff) + 1, ((b >> 14) & 0x3fff) + 1))
            }
            _ => None,
        };
    }
    if d.len() > 4 && d[0] == 0xff && d[1] == 0xd8 {
        let mut i = 2;
        while i + 9 < d.len() {
            if d[i] != 0xff {
                i += 1;
                continue;
            }
            let m = d[i + 1];
            if m == 0xd8 || m == 0x01 || (0xd0..=0xd7).contains(&m) || m == 0xff {
                i += if m == 0xff { 1 } else { 2 };
                continue;
            }
            let len = u16::from_be_bytes([d[i + 2], d[i + 3]]) as usize;
            if matches!(m, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
                let h = u16::from_be_bytes([d[i + 5], d[i + 6]]) as u32;
                let w = u16::from_be_bytes([d[i + 7], d[i + 8]]) as u32;
                return Some((w, h));
            }
            i += 2 + len;
        }
    }
    None
}

// ─────────────────────────── Построитель ───────────────────────────

/// Сборка блоков из потока «открыть блок / текст / закрыть» — общая для
/// HTML (EPUB), FB2, Markdown. Пробелы схлопываются как в HTML.
#[derive(Default)]
pub struct Builder {
    pub doc: FlowDoc,
    cur: Option<Block>,
    /// Последний символ текущего блока — пробел (для схлопывания).
    space: bool,
    style: Vec<u8>,
    link: Vec<Option<Arc<str>>>,
    pending_anchors: Vec<String>,
    /// Не схлопывать пробелы (`pre`, стихи TXT).
    pub preserve: bool,
}

impl Builder {
    pub fn new() -> Self {
        Self::default()
    }

    fn cur_style(&self) -> u8 {
        self.style.iter().fold(0, |a, s| a | s)
    }

    pub fn push_style(&mut self, s: u8) {
        self.style.push(s);
    }
    pub fn pop_style(&mut self) {
        self.style.pop();
    }
    pub fn push_link(&mut self, l: Option<&str>) {
        self.link.push(l.map(Arc::from));
        self.style.push(if l.is_some() { LINK } else { 0 });
    }
    pub fn pop_link(&mut self) {
        self.link.pop();
        self.style.pop();
    }

    /// Якорь на следующий блок (или текущий, если он уже начат).
    pub fn anchor(&mut self, id: &str) {
        if id.is_empty() {
            return;
        }
        if self.cur.is_some() {
            self.doc.anchors.insert(id.to_string(), self.doc.blocks.len());
        } else {
            self.pending_anchors.push(id.to_string());
        }
    }

    /// Начать раздел (следующий блок — с новой страницы).
    pub fn section(&mut self) {
        self.end_block();
        let at = self.doc.blocks.len();
        if self.doc.sections.last() != Some(&at) {
            self.doc.sections.push(at);
        }
    }

    pub fn start_block(&mut self, kind: BlockKind) {
        self.end_block();
        self.cur = Some(Block::new(kind));
        self.space = true;
    }

    pub fn center(&mut self) {
        if let Some(b) = &mut self.cur {
            b.center = true;
        }
    }

    pub fn has_block(&self) -> bool {
        self.cur.is_some()
    }

    pub fn current_kind(&self) -> Option<&BlockKind> {
        self.cur.as_ref().map(|b| &b.kind)
    }

    /// Закрыть блок; пустой текстовый блок выбрасывается.
    pub fn end_block(&mut self) {
        let Some(mut b) = self.cur.take() else { return };
        if let Some(last) = b.spans.last_mut() {
            let t = last.text.trim_end().len();
            if !self.preserve {
                last.text.truncate(t);
            }
        }
        b.spans.retain(|s| !s.text.is_empty());
        let keep = matches!(b.kind, BlockKind::Image { .. } | BlockKind::Rule | BlockKind::Empty)
            || b.spans.iter().any(|s| !s.text.trim().is_empty());
        if !keep {
            return;
        }
        let idx = self.doc.blocks.len();
        for a in self.pending_anchors.drain(..) {
            self.doc.anchors.insert(a, idx);
        }
        self.doc.blocks.push(b);
    }

    /// Текст в текущий блок (блок абзаца открывается сам).
    pub fn text(&mut self, t: &str) {
        if t.is_empty() {
            return;
        }
        if self.cur.is_none() {
            if t.trim().is_empty() {
                return;
            }
            self.start_block(BlockKind::Para);
        }
        let style = self.cur_style();
        let link = self.link.iter().rev().find_map(|l| l.clone());
        let mut add = String::with_capacity(t.len());
        if self.preserve {
            add.push_str(t);
        } else {
            for c in t.chars() {
                if c.is_whitespace() && c != '\u{a0}' {
                    if !self.space {
                        add.push(' ');
                        self.space = true;
                    }
                } else {
                    add.push(c);
                    self.space = false;
                }
            }
        }
        if add.is_empty() {
            return;
        }
        let b = self.cur.as_mut().unwrap();
        match b.spans.last_mut() {
            Some(s) if s.style == style && s.link == link => s.text.push_str(&add),
            _ => b.spans.push(Span { text: add, style, link }),
        }
    }

    /// Перевод строки внутри блока (`<br>`).
    pub fn line_break(&mut self) {
        if self.cur.is_some() {
            let style = self.cur_style();
            let b = self.cur.as_mut().unwrap();
            if let Some(s) = b.spans.last_mut() {
                let t = s.text.trim_end().len();
                s.text.truncate(t);
            }
            b.spans.push(Span { text: "\n".into(), style, link: None });
            self.space = true;
        }
    }

    pub fn image(&mut self, id: String, alt: String) {
        self.end_block();
        self.cur = Some(Block::new(BlockKind::Image { id, alt }));
        self.end_block();
    }

    pub fn simple(&mut self, kind: BlockKind) {
        self.end_block();
        self.cur = Some(Block::new(kind));
        self.end_block();
    }

    pub fn toc(&mut self, title: String, level: u8) {
        let t = title.split_whitespace().collect::<Vec<_>>().join(" ");
        if t.is_empty() {
            return;
        }
        let pos = Pos::new(self.doc.blocks.len(), 0);
        self.doc.toc.push(TocEntry { title: t, level, pos });
    }

    pub fn finish(mut self) -> FlowDoc {
        self.end_block();
        self.doc.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_collapses_spaces_and_drops_empty() {
        let mut b = Builder::new();
        b.start_block(BlockKind::Para);
        b.text("  Привет,\n   ");
        b.push_style(BOLD);
        b.text(" мир ");
        b.pop_style();
        b.text("!  ");
        b.start_block(BlockKind::Para);
        b.text("   ");
        let d = b.finish();
        assert_eq!(d.blocks.len(), 1);
        assert_eq!(d.blocks[0].text(), "Привет, мир !");
        assert_eq!(d.blocks[0].spans[1].style, BOLD);
    }

    #[test]
    fn progress_and_fraction_roundtrip() {
        let mut b = Builder::new();
        for i in 0..10 {
            b.start_block(BlockKind::Para);
            b.text(&format!("абзац номер {i} с текстом"));
        }
        let d = b.finish();
        let p = d.pos_at_fraction(0.5);
        assert!((d.progress(p) - 0.5).abs() < 0.05);
        assert_eq!(d.pos_at_fraction(0.0), Pos::START);
    }

    #[test]
    fn image_sizes() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend(200u32.to_be_bytes());
        png.extend(100u32.to_be_bytes());
        png.extend([0; 8]);
        assert_eq!(image_size(&png), Some((200, 100)));
    }
}
