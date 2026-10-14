//! Microsoft Cabinet: каждая папка (общий сжатый поток её файлов) распаковывается один раз, файлы
//! вырезаются из неё по смещениям. Крейт `cab` для каждого файла распаковывает папку с начала —
//! на установщиках с тысячами файлов (VC++ 2022, VS .NET SP1) это минуты вместо секунд.
//! Сжатие: без сжатия, MSZIP (deflate со словарём из прошлого блока), LZX; Quantum не встречается.

use miniz_oxide::inflate::core::{decompress, inflate_flags, DecompressorOxide};
use miniz_oxide::inflate::TINFLStatus;

pub struct Member {
    pub name: String,
    pub data: Vec<u8>,
}

struct Folder {
    start: usize,
    blocks: u16,
    kind: u16,
}

struct Entry {
    size: usize,
    offset: usize,
    folder: u16,
    name: String,
}

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    d.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
}

fn u32_at(d: &[u8], o: usize) -> Option<usize> {
    d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
}

fn cstr(d: &[u8], o: usize) -> Option<(String, usize)> {
    let end = o + d.get(o..)?.iter().position(|&b| b == 0)?;
    Some((String::from_utf8_lossy(&d[o..end]).into_owned(), end + 1))
}

/// Все файлы CAB (`max_member` — больше этого файлы пропускаются).
pub fn read(d: &[u8], max_member: usize) -> Option<Vec<Member>> {
    if !d.starts_with(b"MSCF") {
        return None;
    }
    let files_off = u32_at(d, 16)?;
    let n_folders = u16_at(d, 26)?;
    let n_files = u16_at(d, 28)?;
    let flags = u16_at(d, 30)?;
    let mut o = 36;
    let (mut folder_res, mut data_res) = (0usize, 0usize);
    if flags & 4 != 0 {
        let header_res = u16_at(d, o)? as usize;
        folder_res = *d.get(o + 2)? as usize;
        data_res = *d.get(o + 3)? as usize;
        o += 4 + header_res;
    }
    // имена соседних томов многотомного CAB
    for bit in [1u16, 2] {
        if flags & bit != 0 {
            o = cstr(d, o)?.1;
            o = cstr(d, o)?.1;
        }
    }
    let mut folders = Vec::new();
    for _ in 0..n_folders {
        folders.push(Folder { start: u32_at(d, o)?, blocks: u16_at(d, o + 4)?, kind: u16_at(d, o + 6)? });
        o += 8 + folder_res;
    }
    let mut entries = Vec::new();
    let mut o = files_off;
    for _ in 0..n_files {
        let size = u32_at(d, o)?;
        let offset = u32_at(d, o + 4)?;
        let folder = u16_at(d, o + 8)?;
        let (name, next) = cstr(d, o + 16)?;
        entries.push(Entry { size, offset, folder, name: name.replace('\\', "/") });
        o = next;
    }
    let mut out = Vec::new();
    for (i, f) in folders.iter().enumerate() {
        let mine: Vec<&Entry> = entries.iter().filter(|e| e.folder as usize == i).collect();
        if mine.is_empty() {
            continue;
        }
        // распаковывать только до конца последнего нужного файла
        let need = mine.iter().filter(|e| e.size <= max_member).map(|e| e.offset + e.size).max().unwrap_or(0);
        let Some(data) = folder_data(d, f, data_res, need) else { continue };
        for e in mine {
            if e.size <= max_member && e.offset + e.size <= data.len() {
                out.push(Member { name: e.name.clone(), data: data[e.offset..e.offset + e.size].to_vec() });
            }
        }
    }
    Some(out)
}

fn folder_data(d: &[u8], f: &Folder, data_res: usize, need: usize) -> Option<Vec<u8>> {
    let mut out: Vec<u8> = Vec::new();
    let mut o = f.start;
    let kind = f.kind & 0x0f;
    let mut lzx = if kind == 3 {
        let bits = (f.kind >> 8) & 0x1f;
        let ws = match bits {
            15 => lzxd::WindowSize::KB32,
            16 => lzxd::WindowSize::KB64,
            17 => lzxd::WindowSize::KB128,
            18 => lzxd::WindowSize::KB256,
            19 => lzxd::WindowSize::KB512,
            20 => lzxd::WindowSize::MB1,
            21 => lzxd::WindowSize::MB2,
            _ => return None,
        };
        Some(lzxd::Lzxd::new(ws))
    } else {
        None
    };
    for _ in 0..f.blocks {
        if out.len() >= need {
            break;
        }
        let packed = u16_at(d, o + 4)? as usize;
        let size = u16_at(d, o + 6)? as usize;
        let start = o + 8 + data_res;
        let chunk = d.get(start..start + packed)?;
        o = start + packed;
        match kind {
            0 => out.extend_from_slice(chunk.get(..size)?),
            1 => {
                if !chunk.starts_with(b"CK") {
                    return None;
                }
                let pos = out.len();
                out.resize(pos + size, 0);
                let mut r = DecompressorOxide::new();
                let (st, _, n) = decompress(&mut r, &chunk[2..], &mut out, pos, inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF);
                // блок — отдельный поток deflate (обычно с BFINAL), история — из прошлых блоков в `out`
                if n != size || matches!(st, TINFLStatus::Failed | TINFLStatus::FailedCannotMakeProgress | TINFLStatus::BadParam | TINFLStatus::Adler32Mismatch) {
                    return None;
                }
            }
            3 => {
                let part = lzx.as_mut()?.decompress_next(chunk, size).ok()?;
                out.extend_from_slice(part);
            }
            _ => return None,
        }
    }
    Some(out)
}
