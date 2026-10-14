//! Значок программы Windows (.exe, .dll, .ocx, .scr, .cpl) из её ресурсов:
//! первая группа значков (`RT_GROUP_ICON`), из неё — самый крупный кадр
//! (`RT_ICON`), собранный в однокадровый ICO. Кадр может быть BMP или PNG —
//! декодирует `image`.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const RT_ICON: u32 = 3;
const RT_GROUP_ICON: u32 = 14;
/// Больше — не PE-ресурсы, а мусор: не читаем.
const MAX_RESOURCES: u32 = 64 << 20;
const MAX_ICON: u32 = 4 << 20;

fn u16le(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn read_at(f: &mut File, off: u64, len: usize) -> Option<Vec<u8>> {
    f.seek(SeekFrom::Start(off)).ok()?;
    let mut buf = vec![0; len];
    f.read_exact(&mut buf).ok()?;
    Some(buf)
}

struct Section {
    va: u32,
    size: u32,
    raw: u32,
}

/// Ресурсы PE: секция с каталогом ресурсов и её адрес в памяти.
struct Resources {
    data: Vec<u8>,
    /// RVA начала `data`.
    rva: u32,
}

impl Resources {
    fn at(&self, rva: u32, len: u32) -> Option<&[u8]> {
        let start = rva.checked_sub(self.rva)? as usize;
        self.data.get(start..start + len as usize)
    }

    /// Записи каталога: (id или имя со старшим битом, смещение со старшим битом подкаталога).
    fn entries(&self, dir: u32) -> Vec<(u32, u32)> {
        let d = dir as usize;
        let (Some(named), Some(ids)) = (u16le(&self.data, d + 12), u16le(&self.data, d + 14)) else { return vec![] };
        (0..named as usize + ids as usize)
            .filter_map(|i| {
                let e = d + 16 + i * 8;
                Some((u32le(&self.data, e)?, u32le(&self.data, e + 4)?))
            })
            .collect()
    }

    /// Подкаталог записи с числовым `id` (`None` — первая запись).
    fn subdir(&self, dir: u32, id: Option<u32>) -> Option<u32> {
        let (_, off) = self.entries(dir).into_iter().find(|(name, off)| off & 0x8000_0000 != 0 && id.map_or(true, |id| *name == id))?;
        Some(off & 0x7fff_ffff)
    }

    /// Данные первого языка ресурса `kind`/`id`.
    fn resource(&self, kind: u32, id: Option<u32>) -> Option<&[u8]> {
        let names = self.subdir(0, Some(kind))?;
        let langs = self.subdir(names, id)?;
        let (_, leaf) = *self.entries(langs).first()?;
        if leaf & 0x8000_0000 != 0 {
            return None;
        }
        let rva = u32le(&self.data, leaf as usize)?;
        let size = u32le(&self.data, leaf as usize + 4)?;
        if size > MAX_ICON {
            return None;
        }
        self.at(rva, size)
    }
}

fn load_resources(path: &Path) -> Option<Resources> {
    let mut f = File::open(path).ok()?;
    let dos = read_at(&mut f, 0, 64)?;
    if &dos[..2] != b"MZ" {
        return None;
    }
    let pe = u32le(&dos, 0x3c)? as u64;
    let head = read_at(&mut f, pe, 24)?;
    if &head[..4] != b"PE\0\0" {
        return None;
    }
    let sections = u16le(&head, 6)? as usize;
    let opt_size = u16le(&head, 20)? as usize;
    let opt = read_at(&mut f, pe + 24, opt_size)?;
    // Каталог данных №2 — ресурсы.
    let dirs = match u16le(&opt, 0)? {
        0x10b => 96,
        0x20b => 112,
        _ => return None,
    };
    let res_rva = u32le(&opt, dirs + 2 * 8)?;
    let res_size = u32le(&opt, dirs + 2 * 8 + 4)?;
    if res_rva == 0 || res_size == 0 || res_size > MAX_RESOURCES {
        return None;
    }
    let table = read_at(&mut f, pe + 24 + opt_size as u64, sections.min(96) * 40)?;
    let sec = (0..sections.min(96))
        .filter_map(|i| {
            let s = &table[i * 40..i * 40 + 40];
            Some(Section { size: u32le(s, 8)?.max(u32le(s, 16)?), va: u32le(s, 12)?, raw: u32le(s, 20)? })
        })
        .find(|s| res_rva >= s.va && res_rva < s.va.saturating_add(s.size))?;
    // Значки обычно лежат в той же секции, что и каталог, — читаем её целиком.
    let len = sec.size.min(MAX_RESOURCES);
    let mut data = read_at(&mut f, sec.raw as u64, len as usize).or_else(|| {
        let file_len = f.metadata().ok()?.len();
        let avail = file_len.checked_sub(sec.raw as u64)? as usize;
        read_at(&mut f, sec.raw as u64, avail.min(len as usize))
    })?;
    // Каталог ресурсов — с начала `data`.
    let skip = (res_rva - sec.va) as usize;
    if skip >= data.len() {
        return None;
    }
    data.drain(..skip);
    Some(Resources { data, rva: res_rva })
}

/// Однокадровый ICO с самым крупным значком программы.
pub fn extract_ico(path: &Path) -> Option<Vec<u8>> {
    let res = load_resources(path)?;
    let group = res.resource(RT_GROUP_ICON, None)?;
    let count = u16le(group, 4)? as usize;
    // Крупнее, затем больше цветов. Ширина 0 — это 256.
    let best = (0..count)
        .filter_map(|i| {
            let e = group.get(6 + i * 14..6 + i * 14 + 14)?;
            let w = if e[0] == 0 { 256 } else { e[0] as u32 };
            Some((w, u16le(e, 6)?, e, u16le(e, 12)?))
        })
        .max_by_key(|(w, bits, _, _)| (*w, *bits))?;
    let (_, _, entry, id) = best;
    let img = res.resource(RT_ICON, Some(id as u32))?;
    let mut ico = Vec::with_capacity(22 + img.len());
    ico.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
    ico.extend_from_slice(&entry[..8]);
    ico.extend_from_slice(&(img.len() as u32).to_le_bytes());
    ico.extend_from_slice(&22u32.to_le_bytes());
    ico.extend_from_slice(img);
    Some(ico)
}

/// Значок программы картинкой.
pub fn extract(path: &Path) -> Option<image::DynamicImage> {
    let ico = extract_ico(path)?;
    image::load_from_memory_with_format(&ico, image::ImageFormat::Ico).ok()
}

/// Тип файла — программа Windows (у неё бывает значок).
pub fn is_pe(mime: &str) -> bool {
    crate::mime::is_a(mime, "application/x-msdownload")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Минимальный PE32 с одной группой из одного значка 16×16 (PNG).
    fn tiny_pe() -> Vec<u8> {
        let mut png = Vec::new();
        image::DynamicImage::new_rgba8(16, 16).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        // Ресурсы с RVA 0x1000, в файле с 0x200.
        let base = 0x1000u32;
        let mut r = vec![0u8; 0x200];
        let dir = |r: &mut Vec<u8>, at: usize, ids: u16| r[at + 14..at + 16].copy_from_slice(&ids.to_le_bytes());
        let ent = |r: &mut Vec<u8>, at: usize, id: u32, off: u32| {
            r[at..at + 4].copy_from_slice(&id.to_le_bytes());
            r[at + 4..at + 8].copy_from_slice(&off.to_le_bytes());
        };
        // корень: RT_ICON → 0x20, RT_GROUP_ICON → 0x40
        dir(&mut r, 0, 2);
        ent(&mut r, 16, RT_ICON, 0x8000_0020);
        ent(&mut r, 24, RT_GROUP_ICON, 0x8000_0040);
        // RT_ICON: id 1 → языки 0x60; RT_GROUP_ICON: id 7 → языки 0x80
        dir(&mut r, 0x20, 1);
        ent(&mut r, 0x30, 1, 0x8000_0060);
        dir(&mut r, 0x40, 1);
        ent(&mut r, 0x50, 7, 0x8000_0080);
        dir(&mut r, 0x60, 1);
        ent(&mut r, 0x70, 0x409, 0xa0);
        dir(&mut r, 0x80, 1);
        ent(&mut r, 0x90, 0x409, 0xb0);
        // данные: значок с 0x100, группа с 0xc0
        let icon_off = 0x100u32;
        r[0xa0..0xa4].copy_from_slice(&(base + icon_off).to_le_bytes());
        r[0xa4..0xa8].copy_from_slice(&(png.len() as u32).to_le_bytes());
        r[0xb0..0xb4].copy_from_slice(&(base + 0xc0).to_le_bytes());
        r[0xb4..0xb8].copy_from_slice(&20u32.to_le_bytes());
        let g = [0u8, 0, 1, 0, 1, 0, 16, 16, 0, 0, 1, 0, 32, 0];
        r[0xc0..0xce].copy_from_slice(&g);
        r[0xce..0xd2].copy_from_slice(&(png.len() as u32).to_le_bytes());
        r[0xd2..0xd4].copy_from_slice(&1u16.to_le_bytes());
        r.truncate(icon_off as usize);
        r.extend_from_slice(&png);

        let mut f = vec![0u8; 0x200];
        f[..2].copy_from_slice(b"MZ");
        f[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
        f[0x40..0x44].copy_from_slice(b"PE\0\0");
        f[0x46..0x48].copy_from_slice(&1u16.to_le_bytes()); // секций
        f[0x54..0x56].copy_from_slice(&224u16.to_le_bytes()); // размер необязательного заголовка
        let opt = 0x58;
        f[opt..opt + 2].copy_from_slice(&0x10bu16.to_le_bytes());
        f[opt + 112..opt + 116].copy_from_slice(&base.to_le_bytes());
        f[opt + 116..opt + 120].copy_from_slice(&(r.len() as u32).to_le_bytes());
        let sec = opt + 224;
        f[sec + 8..sec + 12].copy_from_slice(&(r.len() as u32).to_le_bytes());
        f[sec + 12..sec + 16].copy_from_slice(&base.to_le_bytes());
        f[sec + 16..sec + 20].copy_from_slice(&(r.len() as u32).to_le_bytes());
        f[sec + 20..sec + 24].copy_from_slice(&0x200u32.to_le_bytes());
        f.extend_from_slice(&r);
        f
    }

    #[test]
    fn icon_from_synthetic_pe() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.exe");
        std::fs::write(&p, tiny_pe()).unwrap();
        let img = extract(&p).expect("значок");
        assert_eq!((img.width(), img.height()), (16, 16));
    }

    #[test]
    fn not_pe() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.exe");
        std::fs::write(&p, b"MZ but nothing else").unwrap();
        assert!(extract(&p).is_none());
    }

    /// Свой файл: `SYN_PE_ICON=путь.exe cargo test … pe_icon -- --ignored`.
    #[test]
    #[ignore]
    fn real_file() {
        let p = std::env::var("SYN_PE_ICON").unwrap();
        let img = extract(Path::new(&p)).expect("значок");
        eprintln!("{}x{}", img.width(), img.height());
    }
}
