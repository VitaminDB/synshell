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
        self.apply_from(rgba, 1024, 0, strength)
    }

    /// То же со столбца `from_x` строки шириной `width` (сравнение «до/после»).
    pub fn apply_from(&self, rgba: &mut [u8], width: usize, from_x: usize, strength: f32) {
        let s = strength.clamp(0.0, 1.0);
        rgba.par_chunks_mut(4 * width.max(1)).for_each(|chunk| {
            for px in chunk.chunks_exact_mut(4).skip(from_x) {
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

// ── библиотека LUT (C7) ─────────────────────────────────────────────────

/// Запись библиотеки: тема (верхняя папка), набор (подпапки), имя, путь.
#[derive(Clone, Debug)]
pub struct Entry {
    pub theme: String,
    pub set: String,
    pub name: String,
    pub path: std::path::PathBuf,
    /// Для поиска: всё в нижнем регистре.
    pub hay: String,
}

/// Папка LUT: `SYN_VIDEO_EDITOR_LUTS` или `LUT/` библиотеки шаблонов.
pub fn library_dir() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("SYN_VIDEO_EDITOR_LUTS") {
        return p.into();
    }
    crate::fusion::library().join("LUT")
}

/// Все `.cube` библиотеки (рекурсивно), по теме и имени.
pub fn scan(root: &std::path::Path) -> Vec<Entry> {
    fn walk(d: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("cube")) {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(root, &mut files);
    let mut v: Vec<Entry> = files
        .into_iter()
        .map(|p| {
            let rel = p.parent().and_then(|d| d.strip_prefix(root).ok()).map(|d| d.display().to_string()).unwrap_or_default();
            let (theme, set) = match rel.split_once('/') {
                Some((a, b)) => (a.to_string(), b.to_string()),
                None => (rel.clone(), String::new()),
            };
            let name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let hay = format!("{theme} {set} {name}").to_lowercase().replace(['_', '-'], " ");
            Entry { theme, set, name, path: p, hay }
        })
        .collect();
    v.sort_by(|a, b| (&a.theme, &a.set, &a.name).cmp(&(&b.theme, &b.set, &b.name)));
    v
}

/// Каталог библиотеки (один раз за запуск).
pub fn catalog() -> &'static [Entry] {
    static CAT: OnceLock<Vec<Entry>> = OnceLock::new();
    CAT.get_or_init(|| scan(&library_dir()))
}

/// Поиск: все слова запроса в теме/наборе/имени, тема и набор — точно (пусто — любые).
pub fn filter<'a>(all: &'a [Entry], query: &str, theme: &str, set: &str) -> Vec<&'a Entry> {
    let q = query.trim().to_lowercase().replace(['_', '-'], " ");
    let words: Vec<&str> = q.split_whitespace().collect();
    all.iter()
        .filter(|e| (theme.is_empty() || e.theme == theme) && (set.is_empty() || e.set == set) && words.iter().all(|w| e.hay.contains(w)))
        .collect()
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
