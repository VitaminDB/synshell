//! Тестовые картинки — RGBA в памяти, без файлов.
//!
//! Мелкие узоры (дизеринг, линии в пиксель) считаются в физических пикселях экрана: картинка
//! строится с [`PX`] пикселями на логический и показывается в рамке ровно `w/PX × h/PX`
//! логических — на BOOX Note Air 4C (масштаб 2) пиксель картинки ложится на пиксель панели.
//! Каждая картинка строится один раз за процесс (общие пиксели, [`Pic`]).

use std::sync::{Arc, OnceLock};

use syngui::n_;

/// Пикселей картинки на логический пиксель (масштаб экрана BOOX Note Air 4C).
pub const PX: u32 = 2;

/// Готовая картинка: ключ кэша текстур, размер в пикселях, общие RGBA.
#[derive(Clone)]
pub struct Pic {
    pub key: &'static str,
    pub w: u32,
    pub h: u32,
    pub rgba: Arc<Vec<u8>>,
}

impl Pic {
    /// Размер рамки в логических пикселях (1:1 с панелью при масштабе [`PX`]).
    pub fn logical(&self) -> (f32, f32) {
        (self.w as f32 / PX as f32, self.h as f32 / PX as f32)
    }
}

fn build(key: &'static str, w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 3]) -> Pic {
    let mut v = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let [r, g, b] = f(x, y);
            v.extend_from_slice(&[r, g, b, 255]);
        }
    }
    Pic { key, w, h, rgba: Arc::new(v) }
}

macro_rules! cached {
    ($(#[$m:meta])* $name:ident, $body:expr) => {
        $(#[$m])*
        pub fn $name() -> Pic {
            static P: OnceLock<Pic> = OnceLock::new();
            P.get_or_init(|| $body).clone()
        }
    };
}

fn gray(v: u8) -> [u8; 3] {
    [v, v, v]
}

/// sRGB → линейная яркость и обратно (градиенты «по восприятию» и пиксельное смешение).
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn clamp8(v: f32) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
}

/// HSV → RGB (h 0..360, s и v 0..1).
pub fn hsv(h: f32, s: f32, v: f32) -> [u8; 3] {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [clamp8((r + m) * 255.0), clamp8((g + m) * 255.0), clamp8((b + m) * 255.0)]
}

// ── Серое ──────────────────────────────────────────────────────────────────────────────────

cached!(
/// Плавный градиент чёрный → белый (сверху) и 16 ступеней тем же размахом (снизу) для сравнения.
gray_ramp, build("epd-gray-ramp", 1800, 120, |x, y| {
    let t = x as f32 / 1799.0;
    if y < 80 {
        gray(clamp8(t * 255.0))
    } else {
        gray(((t * 16.0).floor().min(15.0) as u8) * 17)
    }
}));

/// Матрица Байера 8×8.
fn bayer8(x: u32, y: u32) -> f32 {
    const M: [[u8; 8]; 8] = [
        [0, 32, 8, 40, 2, 34, 10, 42],
        [48, 16, 56, 24, 50, 18, 58, 26],
        [12, 44, 4, 36, 14, 46, 6, 38],
        [60, 28, 52, 20, 62, 30, 54, 22],
        [3, 35, 11, 43, 1, 33, 9, 41],
        [51, 19, 59, 27, 49, 17, 57, 25],
        [15, 47, 7, 39, 13, 45, 5, 37],
        [63, 31, 55, 23, 61, 29, 53, 21],
    ];
    (M[(y % 8) as usize][(x % 8) as usize] as f32 + 0.5) / 64.0
}

/// Мелкий дизеринг в пикселях панели: ч/б градиент Байером, Флойд — Стейнбергом,
/// шахматка 1 и 2 пикселя, линии через пиксель — горизонтальные и вертикальные.
pub const DITHER_BANDS: [&str; 6] = [n_!("Байер 8×8, ч/б"), n_!("Флойд — Стейнберг, ч/б"), n_!("Шахматка 1 px"), n_!("Шахматка 2 px"), n_!("Линии 1 px, горизонтальные"), n_!("Линии 1 px, вертикальные")];
cached!(dither, {
    let (w, band) = (1800u32, 64u32);
    let h = band * DITHER_BANDS.len() as u32;
    // Флойд — Стейнберг: ошибка по строкам полосы
    let mut fs = vec![0u8; (w * band) as usize];
    let mut err = vec![0f32; ((w + 2) * (band + 1)) as usize];
    let idx = |x: u32, y: u32| (y * (w + 2) + x + 1) as usize;
    for y in 0..band {
        for x in 0..w {
            let v = x as f32 / (w - 1) as f32 + err[idx(x, y)];
            let o = if v >= 0.5 { 1.0 } else { 0.0 };
            fs[(y * w + x) as usize] = if o > 0.5 { 255 } else { 0 };
            let e = v - o;
            err[idx(x + 1, y)] += e * 7.0 / 16.0;
            if x > 0 {
                err[idx(x - 1, y + 1)] += e * 3.0 / 16.0;
            }
            err[idx(x, y + 1)] += e * 5.0 / 16.0;
            err[idx(x + 1, y + 1)] += e * 1.0 / 16.0;
        }
    }
    build("epd-dither", w, h, move |x, y| {
        let (b, yy) = (y / band, y % band);
        // тонкий белый зазор между полосами
        if yy >= band - 4 {
            return gray(255);
        }
        let t = x as f32 / (w - 1) as f32;
        let on = match b {
            0 => t > bayer8(x, y),
            1 => fs[(yy * w + x) as usize] > 0,
            2 => (x + y) % 2 == 0,
            3 => (x / 2 + y / 2) % 2 == 0,
            4 => y % 2 == 0,
            _ => x % 2 == 0,
        };
        gray(if on { 255 } else { 0 })
    })
});

// ── Цвет ───────────────────────────────────────────────────────────────────────────────────

cached!(
/// Оттенок по горизонтали, сверху — к белому (пастель), снизу — к чёрному.
hue_field, build("epd-hue-field", 1800, 360, |x, y| {
    let h = x as f32 / 1800.0 * 360.0;
    let t = y as f32 / 359.0;
    let [r, g, b] = hsv(h, 1.0, 1.0);
    let mix = |c: u8| {
        let c = c as f32;
        if t < 0.5 {
            lerp(255.0, c, t * 2.0)
        } else {
            lerp(c, 0.0, (t - 0.5) * 2.0)
        }
    };
    [clamp8(mix(r)), clamp8(mix(g)), clamp8(mix(b))]
}));

cached!(
/// Плавные переходы между цветами (как на фотографии: небо, кожа, зелень).
color_ramps, {
    const STOPS: [[[u8; 3]; 2]; 5] = [
        [[0x1e, 0x4f, 0xa8], [0xf2, 0xc0, 0x8a]], // небо → закат
        [[0x5a, 0x2e, 0x16], [0xf6, 0xd8, 0xc0]], // коричневый → кожа
        [[0x0f, 0x3d, 0x1c], [0xd8, 0xe8, 0x6a]], // хвоя → молодая зелень
        [[0xc6, 0x16, 0x2c], [0x1d, 0x6f, 0xd8]], // красный → синий
        [[0x00, 0x00, 0x00], [0xff, 0xff, 0xff]], // серое для сравнения
    ];
    build("epd-color-ramps", 1800, 5 * 72, |x, y| {
        let (b, yy) = ((y / 72) as usize, y % 72);
        if yy >= 66 {
            return gray(255);
        }
        let t = x as f32 / 1799.0;
        let [a, c] = STOPS[b];
        [clamp8(lerp(a[0] as f32, c[0] as f32, t)), clamp8(lerp(a[1] as f32, c[1] as f32, t)), clamp8(lerp(a[2] as f32, c[2] as f32, t))]
    })
});

cached!(
/// «Фотография» без файла: небо с солнцем, горы, луг, озеро — мягкие цветовые переходы.
landscape, build("epd-landscape", 960, 600, |x, y| {
    let (w, h) = (960.0f32, 600.0f32);
    let (fx, fy) = (x as f32 / w, y as f32 / h);
    let ridge = |base: f32, amp: f32, f1: f32, f2: f32| base - amp * ((fx * f1).sin() * 0.6 + (fx * f2 + 1.3).sin() * 0.4);
    let far = ridge(0.48, 0.10, 7.0, 17.0);
    let near = ridge(0.62, 0.07, 4.0, 11.0);
    let sun = ((fx - 0.72).powi(2) * (w / h).powi(2) + (fy - 0.24).powi(2)).sqrt();
    if fy < far {
        // небо: голубое сверху, тёплое к горизонту, солнце с ореолом
        let t = fy / far;
        let mut c = [lerp(70.0, 250.0, t), lerp(130.0, 205.0, t), lerp(215.0, 160.0, t)];
        if sun < 0.07 {
            c = [255.0, 236.0, 150.0];
        } else if sun < 0.2 {
            let k = 1.0 - (sun - 0.07) / 0.13;
            c = [lerp(c[0], 255.0, k * 0.7), lerp(c[1], 225.0, k * 0.7), lerp(c[2], 150.0, k * 0.7)];
        }
        return [clamp8(c[0]), clamp8(c[1]), clamp8(c[2])];
    }
    if fy < near {
        // дальние горы: сине-фиолетовые, снег на вершинах
        let t = (fy - far) / (near - far);
        if t < 0.08 && far < 0.44 {
            return [240, 244, 250];
        }
        return [clamp8(lerp(110.0, 70.0, t)), clamp8(lerp(105.0, 80.0, t)), clamp8(lerp(150.0, 120.0, t))];
    }
    if fy > 0.82 && (fx - 0.3).abs() < 0.26 - (fy - 0.82) * 0.6 {
        // озеро — отражение неба
        let t = (fy - 0.82) / 0.18;
        return [clamp8(lerp(120.0, 60.0, t)), clamp8(lerp(170.0, 110.0, t)), clamp8(lerp(220.0, 170.0, t))];
    }
    // луг: от жёлто-зелёного к тёмной зелени, полоса цветов
    let t = (fy - near) / (1.0 - near);
    let flowers = ((x * 7919 + y * 104729) % 97) < 3 && t > 0.3;
    if flowers {
        return [[230, 60, 70], [250, 210, 60], [240, 240, 240], [180, 90, 200]][((x / 3 + y) % 4) as usize];
    }
    [clamp8(lerp(150.0, 40.0, t)), clamp8(lerp(190.0, 110.0, t)), clamp8(lerp(70.0, 40.0, t))]
}));

// ── Текст и линии ──────────────────────────────────────────────────────────────────────────

cached!(
/// Линии в пикселях панели: толщина 1, 2, 3, 4 px — горизонтальные, вертикальные и наклонная.
hairlines, build("epd-hairlines", 1200, 240, |x, y| {
    // слева — горизонтальные пачки, справа — вертикальные
    let ink = if x < 560 {
        let (g, yy) = (y / 60, y % 60);
        let t = g + 1;
        yy % (t * 4) < t && yy < 48
    } else if x < 1120 {
        let xx = x - 560;
        let (g, x2) = (xx / 140, xx % 140);
        let t = g + 1;
        x2 % (t * 4) < t && x2 < 120
    } else {
        // наклонная в 1 px
        (x - 1120) == y / 3
    };
    gray(if ink { 0 } else { 255 })
}));

// ── Прокрутка ──────────────────────────────────────────────────────────────────────────────

/// Миниатюра строки списка: свой оттенок и простая фигура (8 разных на весь список).
pub fn thumb(i: usize) -> Pic {
    static P: OnceLock<Vec<Pic>> = OnceLock::new();
    const KEYS: [&str; 8] = ["epd-th0", "epd-th1", "epd-th2", "epd-th3", "epd-th4", "epd-th5", "epd-th6", "epd-th7"];
    P.get_or_init(|| {
        (0..8)
            .map(|k| {
                let hue = k as f32 * 45.0;
                build(KEYS[k], 192, 128, move |x, y| {
                    let (fx, fy) = (x as f32 / 192.0, y as f32 / 128.0);
                    let d = ((fx - 0.5).powi(2) * 2.25 + (fy - 0.5).powi(2)).sqrt();
                    if d < 0.28 {
                        hsv(hue + 180.0, 0.6, 0.95)
                    } else {
                        hsv(hue, lerp(0.25, 0.9, fy), lerp(1.0, 0.7, fx))
                    }
                })
            })
            .collect()
    })[i % 8]
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_match_rgba() {
        for p in [gray_ramp(), dither(), hue_field(), color_ramps(), landscape(), hairlines(), thumb(3)] {
            assert_eq!(p.rgba.len(), (p.w * p.h * 4) as usize, "{}", p.key);
        }
    }

    #[test]
    fn dither_halftone_is_fifty_percent() {
        let p = dither();
        // шахматка 1 px (третья полоса): ровно половина белых
        let y0 = 2 * 64;
        let white = (0..p.w).filter(|&x| p.rgba[((y0 * p.w + x) * 4) as usize] == 255).count();
        assert_eq!(white as u32, p.w / 2);
        assert_eq!(hsv(0.0, 1.0, 1.0), [255, 0, 0]);
        assert_eq!(hsv(240.0, 1.0, 1.0), [0, 0, 255]);
    }
}
