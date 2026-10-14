//! Рекурсивный обход контейнеров без внешних программ: CAB (и вшитые в самораспаковывающиеся exe —
//! IExpress, MSSFX, контейнеры WiX Burn), MSI и прочие OLE-файлы, ZIP, NSIS. Каждый найденный файл
//! отдаётся обходчику с путём `внешний!внутренний!…`; что из этого нужно, решает он сам.

use std::io::{Cursor, Read};

/// Глубже — уже не установщики, а мусор (и защита от «бомб»).
const MAX_DEPTH: u32 = 8;
/// Файлы больше этого внутри контейнеров не распаковываются.
const MAX_MEMBER: u64 = 512 << 20;

pub enum Flow {
    Continue,
    Stop,
}

pub struct Walker<'a> {
    visit: &'a mut dyn FnMut(&str, &[u8]) -> Flow,
    stopped: bool,
}

pub fn walk(name: &str, data: &[u8], visit: &mut dyn FnMut(&str, &[u8]) -> Flow) {
    let mut w = Walker { visit, stopped: false };
    w.blob(name, data, 0);
}

impl Walker<'_> {
    fn blob(&mut self, path: &str, data: &[u8], depth: u32) {
        if self.stopped {
            return;
        }
        if let Flow::Stop = (self.visit)(path, data) {
            self.stopped = true;
            return;
        }
        if depth >= MAX_DEPTH || data.len() < 64 {
            return;
        }
        if data.starts_with(b"PK\x03\x04") {
            self.zip(path, data, depth);
            return;
        }
        if data.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]) {
            self.cfb(path, data, depth);
            return;
        }
        self.cabs(path, data, depth);
        if let Some(blocks) = crate::nsis::extract(data) {
            for (i, b) in blocks.iter().enumerate() {
                self.blob(&format!("{path}!nsis{i}"), b, depth + 1);
            }
        }
    }

    fn zip(&mut self, path: &str, data: &[u8], depth: u32) {
        let Ok(mut z) = zip::ZipArchive::new(Cursor::new(data)) else { return };
        for i in 0..z.len() {
            let Ok(mut f) = z.by_index(i) else { continue };
            if f.is_dir() || f.size() > MAX_MEMBER {
                continue;
            }
            let name = f.name().to_string();
            let mut buf = Vec::with_capacity(f.size() as usize);
            if f.read_to_end(&mut buf).is_ok() {
                drop(f);
                self.blob(&format!("{path}!{name}"), &buf, depth + 1);
            }
        }
    }

    fn cfb(&mut self, path: &str, data: &[u8], depth: u32) {
        let Ok(mut comp) = cfb::CompoundFile::open(Cursor::new(data)) else { return };
        let streams: Vec<_> = comp.walk().filter(|e| e.is_stream() && e.len() <= MAX_MEMBER).map(|e| e.path().to_path_buf()).collect();
        for s in streams {
            let mut buf = Vec::new();
            if let Ok(mut r) = comp.open_stream(&s) {
                if r.read_to_end(&mut buf).is_ok() {
                    // имена потоков MSI закодированы — для пути хватит номера
                    self.blob(&format!("{path}!{}", s.display()), &buf, depth + 1);
                }
            }
        }
    }

    /// Все CAB внутри данных: с начала файла и вшитые (по сигнатуре MSCF с проверкой заголовка).
    fn cabs(&mut self, path: &str, data: &[u8], depth: u32) {
        let mut off = 0;
        while let Some(i) = find(&data[off..], b"MSCF\0\0\0\0") {
            let at = off + i;
            match cab_len(&data[at..]) {
                Some(len) => {
                    self.cab(&format!("{path}@{at:x}"), &data[at..at + len], depth);
                    off = at + len;
                }
                None => off = at + 4,
            }
            if self.stopped {
                return;
            }
        }
    }

    fn cab(&mut self, path: &str, data: &[u8], depth: u32) {
        let Some(members) = crate::cab::read(data, MAX_MEMBER as usize) else { return };
        for m in members {
            self.blob(&format!("{path}!{}", m.name), &m.data, depth + 1);
            if self.stopped {
                return;
            }
        }
    }
}

/// Длина CAB по заголовку, если он правдоподобен.
fn cab_len(d: &[u8]) -> Option<usize> {
    if d.len() < 36 {
        return None;
    }
    let u32_at = |o: usize| u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) as usize;
    let len = u32_at(8);
    let files_off = u32_at(16);
    if u32_at(12) != 0 || u32_at(20) != 0 || d[24] != 3 || d[25] != 1 {
        return None;
    }
    if len < 36 || len > d.len() || files_off >= len {
        return None;
    }
    Some(len)
}

pub fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    let first = needle[0];
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        match hay[i..hay.len() - needle.len() + 1].iter().position(|&b| b == first) {
            Some(p) => {
                i += p;
                if &hay[i..i + needle.len()] == needle {
                    return Some(i);
                }
                i += 1;
            }
            None => return None,
        }
    }
    None
}
