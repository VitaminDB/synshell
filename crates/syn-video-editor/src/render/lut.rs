//! LUT `.cube` (3D, Resolve/Adobe): разбор и применение к RGBA с
//! трилинейной интерполяцией, силой `strength` (0–1). Разобранные таблицы
//! кэшируются по пути.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use rayon::prelude::*;

pub struct Lut {
    pub size: usize,
    min: [f32; 3],
    max: [f32; 3],
    /// RGB, индекс r + g·n + b·n² (как в .cube: красный меняется быстрее всего).
    data: Vec<[f32; 3]>,
}

pub fn parse(text: &str) -> Result<Lut, String> {
    let mut size = 0usize;
    let mut min = [0.0f32; 3];
    let mut max = [1.0f32; 3];
    let mut data = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let mut it = l.split_whitespace();
        let head = it.next().unwrap_or_default();
        match head {
            "LUT_3D_SIZE" => size = it.next().and_then(|v| v.parse().ok()).ok_or("LUT_3D_SIZE")?,
            "LUT_1D_SIZE" => return Err("1D LUT не поддерживается".into()),
            "DOMAIN_MIN" | "DOMAIN_MAX" => {
                let v: Vec<f32> = it.filter_map(|x| x.parse().ok()).collect();
                if v.len() == 3 {
                    if head == "DOMAIN_MIN" {
                        min = [v[0], v[1], v[2]];
                    } else {
                        max = [v[0], v[1], v[2]];
                    }
                }
            }
            "TITLE" | "LUT_3D_INPUT_RANGE" | "LUT_IN_VIDEO_RANGE" | "LUT_OUT_VIDEO_RANGE" => {}
            _ => {
                let v: Vec<f32> = l.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                if v.len() == 3 {
                    data.push([v[0], v[1], v[2]]);
                }
            }
        }
    }
    if size < 2 || data.len() != size * size * size {
        return Err(format!("неверный .cube: размер {size}, строк {}", data.len()));
    }
    Ok(Lut { size, min, max, data })
}

/// Таблица по пути (кэш).
pub fn load(path: &str) -> Result<Arc<Lut>, String> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<Lut>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(l) = cache.lock().unwrap().get(path) {
        return Ok(l.clone());
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let lut = Arc::new(parse(&text).map_err(|e| format!("{path}: {e}"))?);
    cache.lock().unwrap().insert(path.to_string(), lut.clone());
    Ok(lut)
}

impl Lut {
    fn at(&self, r: usize, g: usize, b: usize) -> [f32; 3] {
        self.data[r + g * self.size + b * self.size * self.size]
    }

    /// Цвет 0–1 → цвет 0–1.
    pub fn map(&self, c: [f32; 3]) -> [f32; 3] {
        let n = (self.size - 1) as f32;
        let mut idx = [0usize; 3];
        let mut frac = [0f32; 3];
        for i in 0..3 {
            let v = ((c[i] - self.min[i]) / (self.max[i] - self.min[i]).max(1e-6)).clamp(0.0, 1.0) * n;
            let f = v.floor().min(n - 1.0);
            idx[i] = f as usize;
            frac[i] = v - f;
        }
        let [r, g, b] = idx;
        let [fr, fg, fb] = frac;
        let lerp = |a: [f32; 3], b: [f32; 3], t: f32| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
        let c00 = lerp(self.at(r, g, b), self.at(r + 1, g, b), fr);
        let c10 = lerp(self.at(r, g + 1, b), self.at(r + 1, g + 1, b), fr);
        let c01 = lerp(self.at(r, g, b + 1), self.at(r + 1, g, b + 1), fr);
        let c11 = lerp(self.at(r, g + 1, b + 1), self.at(r + 1, g + 1, b + 1), fr);
        lerp(lerp(c00, c10, fg), lerp(c01, c11, fg), fb)
    }

    /// Применить к RGBA на месте (`strength` 0–1 — смешение с исходным).
    pub fn apply(&self, rgba: &mut [u8], strength: f32) {
        let s = strength.clamp(0.0, 1.0);
        rgba.par_chunks_mut(4 * 1024).for_each(|chunk| {
            for px in chunk.chunks_exact_mut(4) {
                let c = [px[0] as f32 / 255.0, px[1] as f32 / 255.0, px[2] as f32 / 255.0];
                let m = self.map(c);
                for i in 0..3 {
                    let v = c[i] + (m[i] - c[i]) * s;
                    px[i] = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_and_invert() {
        // 2×2×2: тождественная и инверсия
        let id = "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
        let l = parse(id).unwrap();
        let mut px = vec![10u8, 128, 250, 255];
        l.apply(&mut px, 1.0);
        assert_eq!(px, vec![10, 128, 250, 255]);
        let inv = "LUT_3D_SIZE 2\n1 1 1\n0 1 1\n1 0 1\n0 0 1\n1 1 0\n0 1 0\n1 0 0\n0 0 0\n";
        let l = parse(inv).unwrap();
        let mut px = vec![0u8, 255, 100, 255];
        l.apply(&mut px, 1.0);
        assert_eq!(px, vec![255, 0, 155, 255]);
    }
}
