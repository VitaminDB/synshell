//! Установщики NSIS: данные после «firstheader» (сигнатура 0xDEADBEEF «NullsoftInst») — общий
//! поток LZMA/deflate (solid) или блоки, сжатые по отдельности. Из потока берутся блоки
//! `[u32 длина][данные]`: первый — сценарий установщика, дальше — файлы (имена не нужны:
//! DLL узнаются по PE).

use std::io::Read;

const SIG: &[u8] = b"\xEF\xBE\xAD\xDENullsoftInst";
const MAX_OUT: usize = 1 << 30;

pub fn extract(data: &[u8]) -> Option<Vec<Vec<u8>>> {
    if !crate::pe::looks_like_pe(data) {
        return None;
    }
    // firstheader выровнен по 512 байтам
    let mut at = None;
    let mut off = 0;
    while let Some(i) = crate::unpack::find(&data[off..], SIG) {
        let pos = off + i;
        if pos >= 4 && (pos - 4) % 512 == 0 {
            at = Some(pos - 4);
            break;
        }
        off = pos + 1;
    }
    let fh = at?;
    let u32_at = |o: usize| data.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let total = u32_at(fh + 24)? as usize;
    let end = (fh + total).min(data.len());
    let body = data.get(fh + 28..end)?;
    if body.len() < 8 {
        return None;
    }
    // solid: сразу поток LZMA (свойства 0x5D + словарь) или deflate; иначе — блоки, у первого
    // (сценария) в длине старший бит «сжат». LZMA проверяется первым: его байты 5D 00 00 80 тоже
    // читаются как длина со старшим битом.
    let lzma_at = |b: &[u8]| b.len() > 5 && b[0] == 0x5D && u32::from_le_bytes([b[1], b[2], b[3], b[4]]).is_power_of_two();
    let stream = if lzma_at(body) {
        lzma(body)?
    } else if body[0] <= 1 && lzma_at(&body[1..]) {
        // NSIS 3: байт фильтра перед LZMA (1 — BCJ x86, его не снимаем: DLL внутри будут испорчены)
        if body[0] == 1 {
            return None;
        }
        lzma(&body[1..])?
    } else if u32_at(fh + 28)? & 0x8000_0000 != 0 {
        return Some(non_solid(body));
    } else {
        inflate(body)?
    };
    Some(blocks(&stream))
}

fn lzma(input: &[u8]) -> Option<Vec<u8>> {
    let opts = lzma_rs::decompress::Options {
        unpacked_size: lzma_rs::decompress::UnpackedSize::UseProvided(None),
        memlimit: Some(MAX_OUT),
        allow_incomplete: true,
    };
    // за потоком у NSIS — CRC32 установщика: lzma-rs не принимает байты после маркера конца
    for tail in [0, 4] {
        let Some(part) = input.len().checked_sub(tail).map(|n| &input[..n]) else { break };
        let mut out = Vec::new();
        if lzma_rs::lzma_decompress_with_options(&mut std::io::BufReader::new(part), &mut out, &opts).is_ok() && !out.is_empty() {
            return Some(out);
        }
    }
    None
}

fn inflate(input: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let _ = flate2::read::DeflateDecoder::new(input).take(MAX_OUT as u64).read_to_end(&mut out);
    (out.len() > 4).then_some(out)
}

/// Распакованный поток: подряд `[u32 длина][данные]`.
fn blocks(s: &[u8]) -> Vec<Vec<u8>> {
    let mut v = Vec::new();
    let mut o = 0;
    while o + 4 <= s.len() {
        let n = u32::from_le_bytes([s[o], s[o + 1], s[o + 2], s[o + 3]]) as usize;
        o += 4;
        if n == 0 || o + n > s.len() {
            break;
        }
        v.push(s[o..o + n].to_vec());
        o += n;
    }
    v
}

/// Блоки по отдельности: старший бит длины — блок сжат (тем же методом, что весь установщик).
fn non_solid(s: &[u8]) -> Vec<Vec<u8>> {
    let mut v = Vec::new();
    let mut o = 0;
    while o + 4 <= s.len() {
        let raw = u32::from_le_bytes([s[o], s[o + 1], s[o + 2], s[o + 3]]);
        o += 4;
        let n = (raw & 0x7fff_ffff) as usize;
        if n == 0 || o + n > s.len() {
            break;
        }
        let b = &s[o..o + n];
        o += n;
        if raw & 0x8000_0000 == 0 {
            v.push(b.to_vec());
        } else if let Some(d) = if b.first() == Some(&0x5D) { lzma(b) } else { inflate(b) } {
            v.push(d);
        }
    }
    v
}
