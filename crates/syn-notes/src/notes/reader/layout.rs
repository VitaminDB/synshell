//! Вёрстка перекомпоновываемой книги в страницы. Раздел книги верстается
//! целиком и независимо от других (раздел всегда с новой страницы), поэтому
//! открыть книгу на любой позиции — это вёрстка одного раздела, а общее
//! число страниц считается в фоне ([`Paginator::count_pages`]).
//!
//! Ширины слов меряет [`Measure`] (в приложении — `TextMeasure` дерева
//! syngui, в тестах — моноширинная заглушка).

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use syngui::core::{Point, Rect, Size};

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::new(Point::new(x, y), Size::new(w, h))
}

use super::hyph::hyphen_points;
use super::model::*;

pub trait Measure: Send + Sync {
    fn width(&self, text: &str, size: f32, bold: bool, family: Option<&str>) -> f32;
}

/// `TextMeasure` дерева syngui как [`Measure`].
pub struct TmMeasure(pub Arc<dyn syngui::widget::context::TextMeasure>);

impl Measure for TmMeasure {
    fn width(&self, text: &str, size: f32, bold: bool, family: Option<&str>) -> f32 {
        self.0.measure_text_width_styled(text, size, text.chars().count(), bold, family)
    }
}

/// Настройки текста (меню «Аа»).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextSettings {
    /// Кегль основного текста, логические px.
    pub font_size: f32,
    /// Межстрочный интервал — множитель кегля.
    pub line_height: f32,
    /// Поля страницы, px.
    pub margin: f32,
    /// Абзацный отступ, em.
    pub indent: f32,
    /// Отбивка между абзацами, em.
    pub para_spacing: f32,
    pub justify: bool,
    pub hyphenate: bool,
    /// Гарнитура; `None` — шрифт системы.
    pub family: Option<String>,
    /// Жирнее основной текст (на E Ink тонкие шрифты бледные).
    pub bold: bool,
}

impl Default for TextSettings {
    fn default() -> Self {
        Self {
            font_size: 20.0,
            line_height: 1.45,
            margin: 28.0,
            indent: 1.5,
            para_spacing: 0.25,
            justify: true,
            hyphenate: true,
            family: None,
            bold: false,
        }
    }
}

impl TextSettings {
    fn key(&self) -> String {
        format!("{self:?}")
    }
}

/// Отрезок текста на странице.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub rect: Rect,
    pub text: String,
    pub size: f32,
    pub style: u8,
    pub bold: bool,
    pub mono: bool,
    pub block: u32,
    /// Символы блока `[ch0, ch1)`; у отрезка с дефисом переноса дефис не
    /// входит в диапазон.
    pub ch0: u32,
    pub ch1: u32,
    pub link: Option<Arc<str>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Text(Run),
    Image { id: String, rect: Rect, block: u32 },
    Rule { rect: Rect },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub items: Vec<Item>,
    pub start: Pos,
}

impl Page {
    pub fn runs(&self) -> impl Iterator<Item = &Run> {
        self.items.iter().filter_map(|i| match i {
            Item::Text(r) => Some(r),
            _ => None,
        })
    }
}

// ─────────────────────────── Вёрстка раздела ───────────────────────────

#[derive(Clone, Debug)]
enum Atom {
    Word { text: String, style: u8, link: Option<Arc<str>>, ch: u32, w: f32 },
    Glue,
    Break,
}

struct BlockStyle {
    size: f32,
    bold: bool,
    italic: bool,
    mono: bool,
    left: f32,
    first: f32,
    center: bool,
    right: bool,
    justify: bool,
    before: f32,
    after: f32,
    marker: Option<String>,
}

fn block_style(b: &Block, s: &TextSettings, width: f32) -> BlockStyle {
    let fs = s.font_size;
    let mut st = BlockStyle {
        size: fs,
        bold: s.bold,
        italic: false,
        mono: false,
        left: 0.0,
        first: s.indent * fs,
        center: b.center,
        right: false,
        justify: s.justify,
        before: s.para_spacing * fs,
        after: 0.0,
        marker: None,
    };
    match &b.kind {
        BlockKind::Heading(lv) => {
            st.size = fs * match lv {
                1 => 1.6,
                2 => 1.4,
                3 => 1.2,
                _ => 1.1,
            };
            st.bold = true;
            st.first = 0.0;
            st.justify = false;
            st.before = fs * 1.2;
            st.after = fs * 0.6;
        }
        BlockKind::Subtitle => {
            st.bold = true;
            st.center = true;
            st.first = 0.0;
            st.justify = false;
            st.before = fs * 0.6;
            st.after = fs * 0.3;
        }
        BlockKind::Quote => {
            st.left = fs * 1.5;
            st.first = 0.0;
        }
        BlockKind::Epigraph | BlockKind::TextAuthor => {
            st.left = width * 0.35;
            st.first = 0.0;
            st.italic = true;
            st.size = fs * 0.92;
            st.justify = false;
            st.right = matches!(b.kind, BlockKind::TextAuthor);
        }
        BlockKind::Verse => {
            st.left = fs * 2.0;
            st.first = 0.0;
            st.justify = false;
            st.before = 0.0;
        }
        BlockKind::ListItem { ordered, depth } => {
            st.left = fs * 1.6 * (*depth as f32 + 1.0);
            st.first = 0.0;
            st.marker = Some(ordered.map_or("•".to_string(), |n| format!("{n}.")));
        }
        BlockKind::Code => {
            st.mono = true;
            st.size = fs * 0.85;
            st.first = 0.0;
            st.justify = false;
        }
        _ => {}
    }
    if st.center {
        st.first = 0.0;
        st.justify = false;
    }
    st
}

/// Кэш ширин слов на время вёрстки.
struct Widths<'a> {
    m: &'a dyn Measure,
    family: Option<&'a str>,
    cache: HashMap<(String, u32, bool, bool), f32>,
}

impl Widths<'_> {
    fn w(&mut self, text: &str, size: f32, bold: bool, mono: bool) -> f32 {
        let key = (text.to_string(), size.to_bits(), bold, mono);
        if let Some(&w) = self.cache.get(&key) {
            return w;
        }
        let fam = if mono { Some("monospace") } else { self.family };
        let w = self.m.width(text, size, bold, fam);
        self.cache.insert(key, w);
        w
    }
}

fn span_size(style: u8, base: f32) -> f32 {
    if style & (SUP | SUB) != 0 { base * 0.65 } else { base }
}

fn atoms(b: &Block, st: &BlockStyle, ws: &mut Widths) -> Vec<Atom> {
    let mut out = Vec::new();
    let mut ch = 0u32;
    for sp in &b.spans {
        let style = sp.style | if st.italic { ITALIC } else { 0 };
        let size = span_size(style, st.size);
        let bold = st.bold || style & BOLD != 0;
        let mono = st.mono || style & CODE != 0;
        let mut word = String::new();
        let mut wch = ch;
        let flush = |out: &mut Vec<Atom>, word: &mut String, wch: u32, ws: &mut Widths| {
            if !word.is_empty() {
                let w = ws.w(word, size, bold, mono);
                out.push(Atom::Word { text: std::mem::take(word), style, link: sp.link.clone(), ch: wch, w });
            }
        };
        for c in sp.text.chars() {
            if c == '\n' {
                flush(&mut out, &mut word, wch, ws);
                out.push(Atom::Break);
            } else if c == ' ' || c == '\t' || (c.is_whitespace() && c != '\u{a0}') {
                flush(&mut out, &mut word, wch, ws);
                out.push(Atom::Glue);
            } else if c == '\u{ad}' {
                // Мягкий перенос книги — не печатается.
            } else {
                if word.is_empty() {
                    wch = ch;
                }
                word.push(c);
            }
            ch += 1;
        }
        flush(&mut out, &mut word, wch, ws);
    }
    out
}

struct Line {
    runs: Vec<Run>,
    height: f32,
    natural: f32,
    glues: usize,
    last: bool,
    indent: f32,
}

/// Разбить блок на строки ширины `width`.
fn break_lines(b: &Block, bi: u32, st: &BlockStyle, s: &TextSettings, width: f32, ws: &mut Widths) -> Vec<Line> {
    let atoms = atoms(b, st, ws);
    let space = ws.w(" ", st.size, st.bold, st.mono).max(st.size * 0.25);
    let lh = st.size * s.line_height;
    let mut lines: Vec<Line> = Vec::new();
    let mut cur = Line { runs: Vec::new(), height: lh, natural: 0.0, glues: 0, last: false, indent: st.first };
    let mut x = st.first;
    let mut pending_glue = false;
    let avail = |indent: f32| (width - st.left).max(st.size * 4.0) - indent + st.first * 0.0;
    // Слово целиком: подряд идущие Word без Glue между ними.
    let mut i = 0;
    let push_line = |lines: &mut Vec<Line>, cur: &mut Line, last: bool| {
        cur.last = last;
        let done = std::mem::replace(cur, Line { runs: Vec::new(), height: lh, natural: 0.0, glues: 0, last: false, indent: 0.0 });
        lines.push(done);
    };
    while i < atoms.len() {
        match &atoms[i] {
            Atom::Break => {
                push_line(&mut lines, &mut cur, true);
                x = 0.0;
                pending_glue = false;
                i += 1;
                continue;
            }
            Atom::Glue => {
                pending_glue = !cur.runs.is_empty();
                i += 1;
                continue;
            }
            Atom::Word { .. } => {}
        }
        let mut j = i;
        let mut ww = 0.0;
        while let Some(Atom::Word { w, .. }) = atoms.get(j) {
            ww += w;
            j += 1;
        }
        let line_w = avail(cur.indent);
        let gap = if pending_glue { space } else { 0.0 };
        let fits = x + gap + ww <= line_w + 0.5;
        if !fits {
            // Перенос по слогам: только слово из одного отрезка.
            let mut placed = false;
            if s.hyphenate && j == i + 1 && !cur.runs.is_empty() {
                if let Atom::Word { text, style, link, ch, .. } = &atoms[i] {
                    let size = span_size(*style, st.size);
                    let bold = st.bold || style & BOLD != 0;
                    let mono = st.mono || style & CODE != 0;
                    let chars: Vec<char> = text.chars().collect();
                    let lead = chars.iter().take_while(|c| !c.is_alphabetic()).count();
                    let core_end = chars.len() - chars.iter().rev().take_while(|c| !c.is_alphabetic()).count();
                    if core_end > lead + 3 {
                        let core: String = chars[lead..core_end].iter().collect();
                        for p in hyphen_points(&core).into_iter().rev() {
                            let head: String = chars[..lead + p].iter().collect::<String>() + "-";
                            let hw = ws.w(&head, size, bold, mono);
                            if x + gap + hw <= line_w + 0.5 {
                                if gap > 0.0 {
                                    cur.glues += 1;
                                }
                                cur.runs.push(Run {
                                    rect: rect(st.left + cur.indent + x + gap, 0.0, hw, size),
                                    text: head,
                                    size,
                                    style: *style,
                                    bold,
                                    mono,
                                    block: bi,
                                    ch0: *ch,
                                    ch1: *ch + (lead + p) as u32,
                                    link: link.clone(),
                                });
                                cur.natural = x + gap + hw;
                                push_line(&mut lines, &mut cur, false);
                                let tail: String = chars[lead + p..].iter().collect();
                                let tw = ws.w(&tail, size, bold, mono);
                                cur.runs.push(Run {
                                    rect: rect(st.left, 0.0, tw, size),
                                    text: tail,
                                    size,
                                    style: *style,
                                    bold,
                                    mono,
                                    block: bi,
                                    ch0: *ch + (lead + p) as u32,
                                    ch1: *ch + chars.len() as u32,
                                    link: link.clone(),
                                });
                                x = tw;
                                cur.natural = x;
                                placed = true;
                                break;
                            }
                        }
                    }
                }
            }
            if placed {
                pending_glue = false;
                i = j;
                continue;
            }
            if !cur.runs.is_empty() {
                push_line(&mut lines, &mut cur, false);
                x = 0.0;
            }
        }
        let gap = if cur.runs.is_empty() { 0.0 } else if pending_glue { space } else { 0.0 };
        if gap > 0.0 {
            cur.glues += 1;
        }
        x += gap;
        for a in &atoms[i..j] {
            let Atom::Word { text, style, link, ch, w } = a else { continue };
            let size = span_size(*style, st.size);
            let bold = st.bold || style & BOLD != 0;
            let mono = st.mono || style & CODE != 0;
            let line_w = avail(cur.indent);
            if *w > line_w && x <= 0.5 {
                // Слово длиннее строки — режем по символам.
                let chars: Vec<char> = text.chars().collect();
                let mut k = 0;
                while k < chars.len() {
                    let mut n = 1;
                    while k + n < chars.len() {
                        let piece: String = chars[k..k + n + 1].iter().collect();
                        if ws.w(&piece, size, bold, mono) > avail(cur.indent) {
                            break;
                        }
                        n += 1;
                    }
                    let piece: String = chars[k..k + n].iter().collect();
                    let pw = ws.w(&piece, size, bold, mono);
                    cur.runs.push(Run {
                        rect: rect(st.left + cur.indent + x, 0.0, pw, size),
                        text: piece,
                        size,
                        style: *style,
                        bold,
                        mono,
                        block: bi,
                        ch0: ch + k as u32,
                        ch1: ch + (k + n) as u32,
                        link: link.clone(),
                    });
                    x += pw;
                    cur.natural = x;
                    k += n;
                    if k < chars.len() {
                        push_line(&mut lines, &mut cur, false);
                        x = 0.0;
                    }
                }
                continue;
            }
            cur.runs.push(Run {
                rect: rect(st.left + cur.indent + x, 0.0, *w, size),
                text: text.clone(),
                size,
                style: *style,
                bold,
                mono,
                block: bi,
                ch0: *ch,
                ch1: *ch + text.chars().count() as u32,
                link: link.clone(),
            });
            x += w;
            cur.natural = x;
        }
        pending_glue = false;
        i = j;
    }
    if !cur.runs.is_empty() || lines.is_empty() {
        push_line(&mut lines, &mut cur, true);
    }
    // Выравнивание: по ширине (кроме последней строки), по центру, вправо.
    for line in &mut lines {
        let line_w = avail(line.indent);
        let free = (line_w - line.natural).max(0.0);
        if st.center || st.right {
            let dx = if st.center { free / 2.0 } else { free };
            for r in &mut line.runs {
                r.rect.origin.x += dx;
            }
        } else if st.justify && !line.last && line.glues > 0 && free < line_w * 0.35 {
            let per = free / line.glues as f32;
            let mut shift = 0.0;
            let mut prev_end: Option<(u32, u32)> = None;
            for r in &mut line.runs {
                // Новый «просвет» — когда символы не продолжают предыдущий отрезок.
                if let Some((b, e)) = prev_end {
                    if b != r.block || e != r.ch0 {
                        shift += per;
                    }
                }
                prev_end = Some((r.block, r.ch1));
                r.rect.origin.x += shift;
            }
        }
        let max_size = line.runs.iter().map(|r| r.size).fold(st.size, f32::max);
        line.height = max_size * s.line_height;
    }
    lines
}

/// Сверстать раздел в страницы размера `size` (поля внутри).
pub fn layout_section(doc: &FlowDoc, section: usize, s: &TextSettings, size: (f32, f32), m: &dyn Measure) -> Vec<Page> {
    let (start, end) = doc.section_range(section);
    let mut ws = Widths { m, family: s.family.as_deref(), cache: HashMap::new() };
    let x0 = s.margin;
    let top = s.margin * 0.75;
    let width = (size.0 - 2.0 * s.margin).max(s.font_size * 6.0);
    let bottom = (size.1 - s.margin * 0.5).max(top + s.font_size * 3.0);
    let mut pages = vec![Page { items: Vec::new(), start: Pos::new(start, 0) }];
    let mut y = top;
    let new_page = |pages: &mut Vec<Page>, y: &mut f32, at: Pos| {
        pages.push(Page { items: Vec::new(), start: at });
        *y = top;
    };
    for bi in start..end {
        let b = &doc.blocks[bi];
        let st = block_style(b, s, width);
        let page_empty = pages.last().is_some_and(|p| p.items.is_empty());
        if !page_empty {
            y += st.before;
        }
        match &b.kind {
            BlockKind::Image { id, .. } => {
                let (iw, ih) = doc.images.get(id).and_then(|i| i.size).unwrap_or((200, 150));
                let max_h = bottom - top;
                let scale = (width / iw as f32).min(max_h / ih as f32).min(2.0);
                let (w, h) = (iw as f32 * scale, ih as f32 * scale);
                if y + h > bottom && !page_empty {
                    new_page(&mut pages, &mut y, Pos::new(bi, 0));
                }
                let rect = rect(x0 + (width - w) / 2.0, y, w, h);
                pages.last_mut().unwrap().items.push(Item::Image { id: id.clone(), rect, block: bi as u32 });
                y += h + s.font_size * 0.5;
                continue;
            }
            BlockKind::Rule => {
                let h = s.font_size;
                if y + h > bottom {
                    new_page(&mut pages, &mut y, Pos::new(bi, 0));
                }
                pages.last_mut().unwrap().items.push(Item::Rule { rect: rect(x0 + width * 0.3, y + h / 2.0, width * 0.4, 1.0) });
                y += h;
                continue;
            }
            BlockKind::Empty => {
                y += s.font_size * s.line_height * 0.6;
                continue;
            }
            _ => {}
        }
        let lines = break_lines(b, bi as u32, &st, s, width, &mut ws);
        // Заголовок не остаётся внизу страницы без текста под ним.
        if matches!(b.kind, BlockKind::Heading(_) | BlockKind::Subtitle) {
            let need: f32 = lines.iter().map(|l| l.height).sum::<f32>() + st.after + s.font_size * s.line_height * 2.0;
            if y + need > bottom && !pages.last().unwrap().items.is_empty() {
                new_page(&mut pages, &mut y, Pos::new(bi, 0));
            }
        }
        for (li, line) in lines.into_iter().enumerate() {
            if y + line.height > bottom + 0.5 && !pages.last().unwrap().items.is_empty() {
                let at = line.runs.first().map_or(Pos::new(bi, 0), |r| Pos { block: bi as u32, ch: r.ch0 });
                new_page(&mut pages, &mut y, at);
            }
            if li == 0 {
                if let Some(mk) = &st.marker {
                    let mw = ws.w(mk, st.size, st.bold, false);
                    let r = Run {
                        rect: rect(x0 + st.left - mw - st.size * 0.4, y + (line.height - st.size * 1.2) / 2.0, mw, st.size * 1.2),
                        text: mk.clone(),
                        size: st.size,
                        style: 0,
                        bold: st.bold,
                        mono: false,
                        block: bi as u32,
                        ch0: 0,
                        ch1: 0,
                        link: None,
                    };
                    pages.last_mut().unwrap().items.push(Item::Text(r));
                }
            }
            let lh = line.height;
            for mut r in line.runs {
                let fh = r.size * 1.2;
                let mut ry = y + (lh - fh) / 2.0;
                if r.style & SUP != 0 {
                    ry -= st.size * 0.3;
                } else if r.style & SUB != 0 {
                    ry += st.size * 0.2;
                }
                r.rect.origin.x += x0;
                r.rect.origin.y = ry;
                r.rect.size.height = fh;
                pages.last_mut().unwrap().items.push(Item::Text(r));
            }
            y += lh;
        }
        y += st.after;
    }
    // Пустая последняя страница (только отступ) не нужна.
    if pages.len() > 1 && pages.last().is_some_and(|p| p.items.is_empty()) {
        pages.pop();
    }
    pages
}

// ─────────────────────────── Книга целиком ───────────────────────────

/// Страницы книги при текущих настройках и размере: вёрстка по разделам
/// по требованию, с кэшем.
pub struct Paginator {
    pub doc: Arc<FlowDoc>,
    pub settings: TextSettings,
    pub size: (f32, f32),
    measure: Arc<dyn Measure>,
    cache: HashMap<usize, Arc<Vec<Page>>>,
    /// Число страниц каждого раздела (после фонового подсчёта).
    pub counts: Option<Arc<Vec<usize>>>,
    key: String,
}

impl Paginator {
    pub fn new(doc: Arc<FlowDoc>, settings: TextSettings, size: (f32, f32), measure: Arc<dyn Measure>) -> Self {
        let key = format!("{}|{:?}", settings.key(), size);
        Self { doc, settings, size, measure, cache: HashMap::new(), counts: None, key }
    }

    /// Ключ раскладки: настройки + размер (для сверки фонового подсчёта).
    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn measure(&self) -> Arc<dyn Measure> {
        self.measure.clone()
    }

    pub fn section_pages(&mut self, section: usize) -> Arc<Vec<Page>> {
        if let Some(p) = self.cache.get(&section) {
            return p.clone();
        }
        let pages = Arc::new(layout_section(&self.doc, section, &self.settings, self.size, &*self.measure));
        if self.cache.len() > 12 {
            self.cache.clear();
        }
        self.cache.insert(section, pages.clone());
        pages
    }

    /// Раздел и индекс страницы, на которой лежит позиция.
    pub fn locate(&mut self, pos: Pos) -> (usize, usize, Arc<Vec<Page>>) {
        let block = (pos.block as usize).min(self.doc.blocks.len().saturating_sub(1));
        let sec = self.doc.section_of(block);
        let pages = self.section_pages(sec);
        let p = Pos { block: block as u32, ch: pos.ch };
        let idx = pages.iter().rposition(|pg| pg.start <= p).unwrap_or(0);
        (sec, idx, pages)
    }

    /// Начало страницы, на которой лежит позиция (выравнивание позиции).
    pub fn page_start(&mut self, pos: Pos) -> Pos {
        let (_, idx, pages) = self.locate(pos);
        pages[idx].start
    }

    pub fn page(&mut self, pos: Pos) -> Page {
        let (_, idx, pages) = self.locate(pos);
        pages[idx].clone()
    }

    pub fn next(&mut self, pos: Pos) -> Option<Pos> {
        let (sec, idx, pages) = self.locate(pos);
        if idx + 1 < pages.len() {
            return Some(pages[idx + 1].start);
        }
        (sec + 1 < self.doc.sections.len()).then(|| self.section_pages(sec + 1)[0].start)
    }

    pub fn prev(&mut self, pos: Pos) -> Option<Pos> {
        let (sec, idx, pages) = self.locate(pos);
        if idx > 0 {
            return Some(pages[idx - 1].start);
        }
        if sec == 0 {
            return None;
        }
        let prev = self.section_pages(sec - 1);
        prev.last().map(|p| p.start)
    }

    /// Номер страницы (с 0) и всего страниц — когда подсчёт готов.
    pub fn page_number(&mut self, pos: Pos) -> Option<(usize, usize)> {
        let counts = self.counts.clone()?;
        let (sec, idx, _) = self.locate(pos);
        let before: usize = counts.iter().take(sec).sum();
        Some((before + idx, counts.iter().sum()))
    }

    /// Начало страницы с номером `n` (нужен подсчёт).
    pub fn pos_of_page(&mut self, n: usize) -> Option<Pos> {
        let counts = self.counts.clone()?;
        let mut acc = 0;
        for (sec, c) in counts.iter().enumerate() {
            if n < acc + c {
                return self.section_pages(sec).get(n - acc).map(|p| p.start);
            }
            acc += c;
        }
        None
    }

    /// Подсчёт страниц всех разделов (долго — вызывать в фоне).
    pub fn count_pages(doc: &FlowDoc, s: &TextSettings, size: (f32, f32), m: &dyn Measure) -> Vec<usize> {
        (0..doc.sections.len()).map(|sec| layout_section(doc, sec, s, size, m).len()).collect()
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::notes::reader::formats::parse_txt;

    /// Моноширинная мера: символ = 0,5 кегля.
    pub struct Mono;
    impl Measure for Mono {
        fn width(&self, text: &str, size: f32, bold: bool, _f: Option<&str>) -> f32 {
            text.chars().count() as f32 * size * if bold { 0.55 } else { 0.5 }
        }
    }

    fn long_book() -> FlowDoc {
        let mut t = String::new();
        for ch in 1..=3 {
            t.push_str(&format!("Глава {ch}\n\n"));
            for p in 0..40 {
                t.push_str(&format!("Абзац {p} главы {ch}: электронная книга проверяет перенос строк, выравнивание и разбиение на страницы при разных размерах шрифта.\n\n"));
            }
        }
        parse_txt(&t)
    }

    fn all_runs_within(pages: &[Page], size: (f32, f32)) -> bool {
        pages.iter().flat_map(|p| p.runs()).all(|r| r.rect.origin.x >= 0.0 && r.rect.origin.x + r.rect.size.width <= size.0 + 1.0 && r.rect.origin.y + r.rect.size.height <= size.1)
    }

    #[test]
    fn pages_cover_all_text_in_order() {
        let doc = long_book();
        let s = TextSettings::default();
        let size = (600.0, 800.0);
        let pages = layout_section(&doc, 1, &s, size, &Mono);
        assert!(pages.len() > 2);
        assert!(all_runs_within(&pages, size));
        // Каждый символ раздела попадает ровно в один отрезок (кроме пробелов).
        let (a, b) = doc.section_range(1);
        for bi in a..b {
            let text: Vec<char> = doc.blocks[bi].text().chars().collect();
            let mut covered = vec![false; text.len()];
            for r in pages.iter().flat_map(|p| p.runs()).filter(|r| r.block as usize == bi && r.ch1 > r.ch0) {
                for c in r.ch0..r.ch1 {
                    assert!(!covered[c as usize], "символ {c} дважды");
                    covered[c as usize] = true;
                }
            }
            for (i, c) in text.iter().enumerate() {
                assert!(covered[i] || c.is_whitespace(), "символ {i} {c:?} блока {bi} не на странице");
            }
        }
        // Начала страниц возрастают.
        assert!(pages.windows(2).all(|w| w[0].start < w[1].start));
    }

    #[test]
    fn position_survives_font_change() {
        let doc = Arc::new(long_book());
        let mut p = Paginator::new(doc.clone(), TextSettings::default(), (600.0, 800.0), Arc::new(Mono));
        let mut pos = Pos::START;
        for _ in 0..7 {
            pos = p.next(pos).unwrap();
        }
        let text_before = p.page(pos).runs().next().unwrap().text.clone();
        let progress = doc.progress(pos);
        // Крупнее шрифт: та же позиция — на странице, которая её содержит.
        let mut big = Paginator::new(doc.clone(), TextSettings { font_size: 30.0, ..Default::default() }, (600.0, 800.0), Arc::new(Mono));
        let start = big.page_start(pos);
        assert!(start <= pos);
        let page = big.page(pos);
        let contains = page.runs().any(|r| r.block == pos.block && r.ch0 <= pos.ch && pos.ch < r.ch1.max(r.ch0 + 1))
            || page.runs().any(|r| r.block == pos.block && r.ch0 >= pos.ch);
        assert!(contains, "позиция {pos:?} потеряна (было «{text_before}»)");
        assert!((doc.progress(start) - progress).abs() < 0.02);
        // Листание вперёд-назад возвращает на ту же страницу.
        let n = big.next(start).unwrap();
        assert_eq!(big.prev(n), Some(start));
    }

    #[test]
    fn counts_and_page_numbers() {
        let doc = Arc::new(long_book());
        let s = TextSettings::default();
        let mut p = Paginator::new(doc.clone(), s.clone(), (600.0, 800.0), Arc::new(Mono));
        p.counts = Some(Arc::new(Paginator::count_pages(&doc, &s, (600.0, 800.0), &Mono)));
        let (n, total) = p.page_number(Pos::START).unwrap();
        assert_eq!(n, 0);
        let mut pos = Pos::START;
        let mut k = 0;
        while let Some(nx) = p.next(pos) {
            pos = nx;
            k += 1;
        }
        assert_eq!(k + 1, total);
        assert_eq!(p.page_number(pos).unwrap().0, total - 1);
        assert_eq!(p.pos_of_page(total - 1), Some(pos));
    }

    #[test]
    fn justify_fills_lines() {
        let doc = long_book();
        let s = TextSettings { hyphenate: true, ..Default::default() };
        let size = (500.0, 900.0);
        let pages = layout_section(&doc, 0, &s, size, &Mono);
        // В выровненной строке (не последней) правый край ≈ ширине набора.
        let right = size.0 - s.margin;
        let mut lines: HashMap<i32, f32> = HashMap::new();
        for r in pages[0].runs() {
            let e = lines.entry(r.rect.origin.y as i32).or_insert(0.0);
            *e = e.max(r.rect.origin.x + r.rect.size.width);
        }
        let full = lines.values().filter(|&&x| (x - right).abs() < 1.0).count();
        assert!(full >= lines.len() / 2, "выровнено {full} из {}", lines.len());
    }
}
