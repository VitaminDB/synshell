//! Цветовая математика редактора: OKLCH (перцептивно ровные оттенки),
//! контраст WCAG, генерация палитры из акцента, светлый вариант из тёмного
//! и цвета из обоев.

use synshell_common::config::Rgba;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Oklch {
    pub l: f32,
    pub c: f32,
    /// Градусы 0..360.
    pub h: f32,
}

fn to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn from_linear(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

fn oklab(c: Rgba) -> (f32, f32, f32) {
    let (r, g, b) = (to_linear(c.r), to_linear(c.g), to_linear(c.b));
    let l = (0.412_221_47 * r + 0.536_332_55 * g + 0.051_445_99 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    (
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    )
}

/// Линейный sRGB из Oklab (может выходить за 0..1).
fn oklab_to_linear(l: f32, a: f32, b: f32) -> (f32, f32, f32) {
    let l_ = (l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m_ = (l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s_ = (l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    (
        4.076_741_7 * l_ - 3.307_711_6 * m_ + 0.230_969_94 * s_,
        -1.268_438 * l_ + 2.609_757_4 * m_ - 0.341_319_38 * s_,
        -0.004_196_086_3 * l_ - 0.703_418_6 * m_ + 1.707_614_7 * s_,
    )
}

pub fn to_oklch(c: Rgba) -> Oklch {
    let (l, a, b) = oklab(c);
    let h = b.atan2(a).to_degrees();
    Oklch { l, c: (a * a + b * b).sqrt(), h: if h < 0.0 { h + 360.0 } else { h } }
}

/// Цвет из OKLCH; вне охвата sRGB насыщенность снижается, светлота и оттенок
/// сохраняются.
pub fn from_oklch(o: Oklch) -> Rgba {
    let l = o.l.clamp(0.0, 1.0);
    let (hs, hc) = o.h.to_radians().sin_cos();
    let in_gamut = |c: f32| {
        let (r, g, b) = oklab_to_linear(l, c * hc, c * hs);
        let ok = |x: f32| (-1e-4..=1.0001).contains(&x);
        ok(r) && ok(g) && ok(b)
    };
    let mut c = o.c.max(0.0);
    if !in_gamut(c) {
        let (mut lo, mut hi) = (0.0f32, c);
        for _ in 0..20 {
            let mid = (lo + hi) / 2.0;
            if in_gamut(mid) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        c = lo;
    }
    let (r, g, b) = oklab_to_linear(l, c * hc, c * hs);
    let q = |x: f32| (from_linear(x.clamp(0.0, 1.0)) * 255.0).round() as u8;
    Rgba::rgb(q(r), q(g), q(b))
}

/// Относительная яркость WCAG.
pub fn luminance(c: Rgba) -> f32 {
    0.2126 * to_linear(c.r) + 0.7152 * to_linear(c.g) + 0.0722 * to_linear(c.b)
}

/// Наложить полупрозрачный `fg` на непрозрачный `bg`.
pub fn over(fg: Rgba, bg: Rgba) -> Rgba {
    let a = fg.a as f32 / 255.0;
    let m = |f: u8, b: u8| (f as f32 * a + b as f32 * (1.0 - a)).round() as u8;
    Rgba::rgb(m(fg.r, bg.r), m(fg.g, bg.g), m(fg.b, bg.b))
}

/// Контраст WCAG 1..21 (`fg` поверх `bg` с учётом прозрачности).
pub fn contrast(fg: Rgba, bg: Rgba) -> f32 {
    let bg = Rgba { a: 255, ..bg };
    let (a, b) = (luminance(over(fg, bg)), luminance(bg));
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    (hi + 0.05) / (lo + 0.05)
}

/// Оценка контраста текста: (подпись, класс для значка).
pub fn contrast_grade(ratio: f32) -> (&'static str, &'static str) {
    if ratio >= 7.0 {
        ("AAA", "te-grade-good")
    } else if ratio >= 4.5 {
        ("AA", "te-grade-good")
    } else if ratio >= 3.0 {
        ("AA+", "te-grade-mid")
    } else {
        ("!", "te-grade-bad")
    }
}

/// Параметры генератора палитры.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GenParams {
    pub accent: Rgba,
    pub dark: bool,
    /// Насколько фоны окрашены в оттенок (0 — нейтрально-серые, 1 — заметно).
    pub tint: f32,
    /// Оттенок фонов в градусах; `None` — оттенок акцента.
    pub hue: Option<f32>,
    /// Контраст: 0 — мягкий, 1 — жёсткий.
    pub contrast: f32,
}

/// Сгенерированная палитра варианта: ключи `theme.toml` → значения.
pub fn generate(p: GenParams) -> Vec<(&'static str, String)> {
    let acc = to_oklch(p.accent);
    let h = p.hue.unwrap_or(acc.h);
    let n = 0.004 + p.tint.clamp(0.0, 1.0) * 0.045;
    let k = p.contrast.clamp(0.0, 1.0);
    let nc = |l: f32, c: f32| from_oklch(Oklch { l, c, h });
    let mut v: Vec<(&'static str, String)> = Vec::new();
    let (bg, surface, alt, border, fg, muted, accent, status_l, status_c);
    if p.dark {
        bg = nc(0.19 - 0.05 * k, n);
        surface = nc(0.23 - 0.04 * k, n);
        alt = nc(0.28 - 0.03 * k, n * 1.1);
        border = nc(0.33 - 0.02 * k, n * 1.1);
        fg = nc(0.90 + 0.07 * k, n * 0.35);
        muted = nc(0.68 + 0.06 * k, n * 0.6);
        accent = from_oklch(Oklch { l: acc.l.clamp(0.58, 0.85), ..acc });
        status_l = 0.70;
        status_c = 0.15;
    } else {
        bg = nc(0.955 - 0.02 * k, n * 0.7);
        surface = nc(0.99, n * 0.3);
        alt = nc(0.925 - 0.02 * k, n * 0.8);
        border = nc(0.86 - 0.04 * k, n);
        fg = nc(0.27 - 0.10 * k, n * 0.6);
        muted = nc(0.50 - 0.06 * k, n * 0.8);
        accent = from_oklch(Oklch { l: acc.l.clamp(0.40, 0.62), ..acc });
        status_l = 0.55;
        status_c = 0.17;
    }
    let status = |hue: f32| from_oklch(Oklch { l: status_l, c: status_c, h: hue }).hex();
    let wall_top = nc(if p.dark { 0.14 } else { 0.97 }, n);
    let wall_mid = from_oklch(Oklch { l: if p.dark { 0.24 } else { 0.88 }, c: acc.c * 0.35, h: acc.h });
    let wall_bot = from_oklch(Oklch { l: if p.dark { 0.38 } else { 0.78 }, c: acc.c * 0.7, h: (acc.h + 25.0) % 360.0 });
    v.push(("bg", bg.hex()));
    v.push(("surface", surface.hex()));
    v.push(("surface_alt", alt.hex()));
    v.push(("fg", fg.hex()));
    v.push(("muted", muted.hex()));
    v.push(("border", border.hex()));
    v.push(("accent", accent.hex()));
    v.push(("accent_fg", accent.contrast_fg().hex()));
    v.push(("danger", status(25.0)));
    v.push(("success", status(150.0)));
    v.push(("warning", from_oklch(Oklch { l: status_l + 0.08, c: status_c, h: 75.0 }).hex()));
    v.push(("wallpaper", format!("linear-gradient(170deg, {} 0%, {} 55%, {} 100%)", wall_top.hex(), wall_mid.hex(), wall_bot.hex())));
    v.push(("wallpaper_color", bg.hex()));
    v.push(("titlebar", surface.hex()));
    v.push(("titlebar_inactive", bg.hex()));
    v.push(("shadow", if p.dark { "#00000080".into() } else { nc(0.2, n).with_alpha(0.16).hex() }));
    v
}

/// Перевернуть светлоту цвета для другой схемы (тёмная ↔ светлая):
/// нейтральные цвета зеркалятся, насыщенные — сдвигаются к читаемой светлоте.
pub fn flip(c: Rgba, to_dark: bool) -> Rgba {
    // Светлота нейтральных: тёмный фон 0.2 ↔ светлый 0.96, светлый текст
    // 0.92 ↔ тёмный 0.25 — линейно между ними.
    const A: f32 = 1.157;
    const B: f32 = 0.986;
    let o = to_oklch(c);
    let l = if o.c < 0.06 {
        if to_dark {
            (A - o.l) / B
        } else {
            A - B * o.l
        }
    } else if to_dark {
        o.l.max(0.62)
    } else {
        o.l.min(0.58)
    };
    let out = from_oklch(Oklch { l: l.clamp(0.0, 1.0), ..o });
    Rgba { a: c.a, ..out }
}

/// Основные цвета картинки (k-средних в Oklab): (цвет, доля 0..1) по
/// убыванию доли.
pub fn image_colors(path: &std::path::Path) -> Result<Vec<(Rgba, f32)>, String> {
    let img = image::open(path).map_err(|e| e.to_string())?.thumbnail(72, 72).to_rgb8();
    let px: Vec<(f32, f32, f32)> = img.pixels().map(|p| oklab(Rgba::rgb(p[0], p[1], p[2]))).collect();
    if px.is_empty() {
        return Err("пустая картинка".into());
    }
    const K: usize = 6;
    let mut centers: Vec<(f32, f32, f32)> = (0..K).map(|i| px[i * px.len() / K]).collect();
    let mut assign = vec![0usize; px.len()];
    for _ in 0..12 {
        for (i, p) in px.iter().enumerate() {
            let d = |c: &(f32, f32, f32)| (p.0 - c.0).powi(2) + (p.1 - c.1).powi(2) + (p.2 - c.2).powi(2);
            assign[i] = (0..K).min_by(|a, b| d(&centers[*a]).total_cmp(&d(&centers[*b]))).unwrap();
        }
        for (k, c) in centers.iter_mut().enumerate() {
            let (mut s, mut n) = ((0.0, 0.0, 0.0), 0.0);
            for (i, p) in px.iter().enumerate() {
                if assign[i] == k {
                    s = (s.0 + p.0, s.1 + p.1, s.2 + p.2);
                    n += 1.0;
                }
            }
            if n > 0.0 {
                *c = (s.0 / n, s.1 / n, s.2 / n);
            }
        }
    }
    let total = px.len() as f32;
    let mut out: Vec<(Rgba, f32)> = centers
        .iter()
        .enumerate()
        .map(|(k, c)| {
            let share = assign.iter().filter(|a| **a == k).count() as f32 / total;
            let ch = (c.1 * c.1 + c.2 * c.2).sqrt();
            let h = c.2.atan2(c.1).to_degrees();
            (from_oklch(Oklch { l: c.0, c: ch, h: if h < 0.0 { h + 360.0 } else { h } }), share)
        })
        .filter(|(_, s)| *s > 0.0)
        .collect();
    out.sort_by(|a, b| b.1.total_cmp(&a.1));
    Ok(out)
}

/// Параметры генератора по цветам обоев: акцент — самый насыщенный заметный
/// цвет, оттенок и окраска фонов — по преобладающему цвету.
pub fn params_from_image(colors: &[(Rgba, f32)], dark: bool) -> Option<GenParams> {
    let main = to_oklch(colors.first()?.0);
    let accent = colors
        .iter()
        .map(|(c, s)| (to_oklch(*c), *c, *s))
        .filter(|(o, _, _)| o.l > 0.3 && o.l < 0.92)
        .max_by(|a, b| (a.0.c * a.2.sqrt()).total_cmp(&(b.0.c * b.2.sqrt())))
        .map(|(o, _, _)| from_oklch(Oklch { c: o.c.max(0.12), ..o }))
        .unwrap_or_else(|| from_oklch(Oklch { l: 0.65, c: 0.14, h: main.h }));
    Some(GenParams { accent, dark, tint: (main.c / 0.08).clamp(0.0, 1.0), hue: Some(main.h), contrast: 0.5 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oklch_roundtrip() {
        for c in [Rgba::rgb(0x3d, 0x8b, 0xfd), Rgba::rgb(10, 200, 30), Rgba::rgb(255, 255, 255), Rgba::rgb(0, 0, 0)] {
            let back = from_oklch(to_oklch(c));
            for (a, b) in [(c.r, back.r), (c.g, back.g), (c.b, back.b)] {
                assert!((a as i32 - b as i32).abs() <= 1, "{c:?} → {back:?}");
            }
        }
    }

    #[test]
    fn contrast_bounds() {
        let w = Rgba::rgb(255, 255, 255);
        let b = Rgba::rgb(0, 0, 0);
        assert!((contrast(w, b) - 21.0).abs() < 0.1);
        assert!((contrast(w, w) - 1.0).abs() < 0.01);
    }

    #[test]
    fn generated_text_is_readable() {
        for dark in [true, false] {
            for contrast_k in [0.0, 0.5, 1.0] {
                let v = generate(GenParams { accent: Rgba::rgb(0x3d, 0x8b, 0xfd), dark, tint: 0.5, hue: None, contrast: contrast_k });
                let get = |k: &str| Rgba::parse(&v.iter().find(|(n, _)| *n == k).unwrap().1).unwrap();
                assert!(contrast(get("fg"), get("bg")) >= 7.0, "fg/bg dark={dark}");
                assert!(contrast(get("muted"), get("surface")) >= 3.0, "muted dark={dark}");
                assert!(contrast(get("accent_fg"), get("accent")) >= 3.0, "accent_fg dark={dark}");
            }
        }
    }
}
