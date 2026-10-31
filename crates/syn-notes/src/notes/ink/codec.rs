//! Файл рукописного документа (`notes/objects/<id>.ink`):
//!
//! ```text
//! "SYNINK" · версия u16 LE · deflate(postcard(InkDoc))
//! ```
//!
//! Точки штрихов упакованы отдельно ([`packed_points`]): координаты в 1/16
//! единицы разностями (zigzag-varint), давление байтом, наклон байтами со
//! знаком, время разностями в мс. Типичная точка — 5–6 байт до сжатия.

use std::io::{Read, Write};

use super::model::{InkDoc, InkPoint};

pub const MAGIC: &[u8; 6] = b"SYNINK";
pub const VERSION: u16 = 1;

#[derive(Debug)]
pub enum CodecError {
    NotInk,
    /// Файл более новой версии.
    TooNew(u16),
    Corrupt(String),
}

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodecError::NotInk => write!(f, "не рукописный документ"),
            CodecError::TooNew(v) => write!(f, "документ новой версии {v}"),
            CodecError::Corrupt(e) => write!(f, "повреждён: {e}"),
        }
    }
}

impl std::error::Error for CodecError {}

/// Документ → байты файла.
pub fn save(doc: &InkDoc) -> Vec<u8> {
    let raw = postcard::to_allocvec(doc).unwrap_or_default();
    let mut out = Vec::with_capacity(raw.len() / 2 + 16);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    let mut enc = flate2::write::DeflateEncoder::new(out, flate2::Compression::new(6));
    let _ = enc.write_all(&raw);
    enc.finish().unwrap_or_default()
}

/// Байты файла → документ.
pub fn load(bytes: &[u8]) -> Result<InkDoc, CodecError> {
    if bytes.len() < 8 || &bytes[..6] != MAGIC {
        return Err(CodecError::NotInk);
    }
    let ver = u16::from_le_bytes([bytes[6], bytes[7]]);
    if ver > VERSION {
        return Err(CodecError::TooNew(ver));
    }
    let mut raw = Vec::new();
    flate2::read::DeflateDecoder::new(&bytes[8..])
        .read_to_end(&mut raw)
        .map_err(|e| CodecError::Corrupt(e.to_string()))?;
    let mut doc: InkDoc = postcard::from_bytes(&raw).map_err(|e| CodecError::Corrupt(e.to_string()))?;
    // Счётчик id не меньше максимального (на случай ручной правки).
    let mut max = 0;
    for p in &doc.pages {
        max = max.max(p.id);
        for l in &p.layers {
            max = max.max(l.id);
            for i in &l.items {
                max = max.max(i.id());
            }
        }
    }
    doc.next_id = doc.next_id.max(max + 1);
    if doc.pages.is_empty() {
        let p = doc.blank_page();
        doc.pages.push(p);
    }
    for p in &mut doc.pages {
        if p.layers.is_empty() {
            let id = max + 1;
            max += 1;
            p.layers.push(super::model::Layer::new(id, super::model::default_layer_name(1)));
        }
    }
    doc.next_id = doc.next_id.max(max + 1);
    Ok(doc)
}

const Q: f32 = 16.0;

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

fn get_varint(b: &[u8], pos: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    let mut shift = 0;
    loop {
        let byte = *b.get(*pos)?;
        *pos += 1;
        v |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some(v);
        }
        shift += 7;
        if shift > 63 {
            return None;
        }
    }
}

fn zig(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

fn unzig(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

/// Упаковать точки в байты.
pub fn pack_points(pts: &[InkPoint]) -> Vec<u8> {
    let mut out = Vec::with_capacity(pts.len() * 6 + 4);
    put_varint(&mut out, pts.len() as u64);
    // Есть ли наклон (у мыши/пальца нет — экономим 2 байта на точку).
    let tilt = pts.iter().any(|p| p.tx != 0.0 || p.ty != 0.0);
    out.push(tilt as u8);
    let (mut px, mut py, mut pt) = (0i64, 0i64, 0i64);
    for p in pts {
        let x = (p.x * Q).round() as i64;
        let y = (p.y * Q).round() as i64;
        put_varint(&mut out, zig(x - px));
        put_varint(&mut out, zig(y - py));
        out.push((p.p.clamp(0.0, 1.0) * 255.0).round() as u8);
        if tilt {
            out.push(p.tx.clamp(-90.0, 90.0).round() as i8 as u8);
            out.push(p.ty.clamp(-90.0, 90.0).round() as i8 as u8);
        }
        let t = p.t as i64;
        put_varint(&mut out, zig(t - pt));
        px = x;
        py = y;
        pt = t;
    }
    out
}

pub fn unpack_points(b: &[u8]) -> Option<Vec<InkPoint>> {
    let mut pos = 0;
    let n = get_varint(b, &mut pos)? as usize;
    let tilt = *b.get(pos)? != 0;
    pos += 1;
    let mut out = Vec::with_capacity(n.min(1 << 20));
    let (mut x, mut y, mut t) = (0i64, 0i64, 0i64);
    for _ in 0..n {
        x += unzig(get_varint(b, &mut pos)?);
        y += unzig(get_varint(b, &mut pos)?);
        let p = *b.get(pos)? as f32 / 255.0;
        pos += 1;
        let (mut tx, mut ty) = (0.0, 0.0);
        if tilt {
            tx = *b.get(pos)? as i8 as f32;
            ty = *b.get(pos + 1)? as i8 as f32;
            pos += 2;
        }
        t += unzig(get_varint(b, &mut pos)?);
        out.push(InkPoint { x: x as f32 / Q, y: y as f32 / Q, p, tx, ty, t: t.max(0) as u32 });
    }
    Some(out)
}

/// serde-обёртка для `Stroke::points`.
pub mod packed_points {
    use super::{pack_points, unpack_points};
    use crate::notes::ink::model::InkPoint;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[InkPoint], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(&pack_points(v))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<InkPoint>, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Vec<InkPoint>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("packed points")
            }
            fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<Self::Value, E> {
                unpack_points(v).ok_or_else(|| E::custom("bad points"))
            }
            fn visit_byte_buf<E: serde::de::Error>(self, v: Vec<u8>) -> Result<Self::Value, E> {
                self.visit_bytes(&v)
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut a: A) -> Result<Self::Value, A::Error> {
                let mut buf = Vec::new();
                while let Some(b) = a.next_element::<u8>()? {
                    buf.push(b);
                }
                self.visit_bytes(&buf)
            }
        }
        d.deserialize_bytes(V)
    }
}
