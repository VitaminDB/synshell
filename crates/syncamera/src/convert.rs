//! Преобразование кадров камеры (NV21/NV12, полный диапазон BT.601 — JFIF).
//!
//! - превью: RGBA с поворотом на четверти оборота, зеркалом (фронтальная) и прореживанием (шаг 1–4) — в
//!   несколько потоков; целочисленная арифметика;
//! - запись: NV12 с поворотом в буфер кодера (строка и число строк Y — его).

/// Описание кадра-источника.
#[derive(Clone, Copy, Debug)]
pub struct Src<'a> {
    pub data: &'a [u8],
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub scanlines: usize,
    /// Пара цветности — V,U (NV21), иначе U,V (NV12).
    pub vu: bool,
}

impl Src<'_> {
    fn uv_off(&self) -> usize {
        self.stride * self.scanlines
    }
}

/// Размер результата для поворота `rot` (градусы по часовой) и шага `step`.
pub fn out_size(w: usize, h: usize, rot: u32, step: usize) -> (usize, usize) {
    let (w, h) = (w / step, h / step);
    if rot % 180 == 90 {
        (h, w)
    } else {
        (w, h)
    }
}

// Точка результата (ox, oy) → точка источника (в шагах) при повороте по часовой на rot.
#[inline(always)]
fn src_xy(ox: usize, oy: usize, sw: usize, sh: usize, rot: u32) -> (usize, usize) {
    match rot {
        90 => (oy, sh - 1 - ox),
        180 => (sw - 1 - ox, sh - 1 - oy),
        270 => (sw - 1 - oy, ox),
        _ => (ox, oy),
    }
}

#[inline(always)]
fn clamp(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

/// NV21/NV12 → RGBA: поворот по часовой `rot`, зеркало по горизонтали результата, шаг прореживания.
/// `out` — `ow * oh * 4` байт (`out_size`).
pub fn to_rgba(src: &Src, rot: u32, mirror: bool, step: usize, out: &mut [u8]) {
    let (ow, oh) = out_size(src.width, src.height, rot, step);
    let (sw, sh) = (src.width / step, src.height / step);
    if ow == 0 || oh == 0 || out.len() < ow * oh * 4 {
        return;
    }
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).clamp(1, 4);
    let rows_per = oh.div_ceil(threads);
    let s = *src;
    std::thread::scope(|scope| {
        for (i, chunk) in out[..ow * oh * 4].chunks_mut(rows_per * ow * 4).enumerate() {
            let y0 = i * rows_per;
            scope.spawn(move || {
                let uv0 = s.uv_off();
                let (ui, vi) = if s.vu { (1, 0) } else { (0, 1) };
                for (r, row) in chunk.chunks_mut(ow * 4).enumerate() {
                    let oy = y0 + r;
                    for ox in 0..ow {
                        let mx = if mirror { ow - 1 - ox } else { ox };
                        let (x, y) = src_xy(mx, oy, sw, sh, rot);
                        let (x, y) = (x * step, y * step);
                        let yv = s.data[y * s.stride + x] as i32;
                        let c = uv0 + (y / 2) * s.stride + (x & !1);
                        let u = s.data[c + ui] as i32 - 128;
                        let v = s.data[c + vi] as i32 - 128;
                        // JFIF: R = Y + 1.402 V, G = Y − 0.344 U − 0.714 V, B = Y + 1.772 U (×1024)
                        let px = &mut row[ox * 4..ox * 4 + 4];
                        px[0] = clamp(yv + ((1436 * v) >> 10));
                        px[1] = clamp(yv - ((352 * u + 731 * v) >> 10));
                        px[2] = clamp(yv + ((1815 * u) >> 10));
                        px[3] = 255;
                    }
                }
            });
        }
    });
}

/// Кадр → NV12 в буфер кодера с поворотом по часовой `rot`: строка `dst_stride`, `dst_scan` строк Y.
/// Размер результата — `out_size(…, rot, 1)`.
pub fn to_nv12(src: &Src, rot: u32, dst: &mut [u8], dst_stride: usize, dst_scan: usize) {
    let (ow, oh) = out_size(src.width, src.height, rot, 1);
    let (sw, sh) = (src.width, src.height);
    let uv_dst = dst_stride * dst_scan;
    if dst.len() < uv_dst + dst_stride * oh.div_ceil(2) {
        return;
    }
    let (ui, vi) = if src.vu { (1, 0) } else { (0, 1) };
    let uv0 = src.uv_off();
    if rot == 0 {
        for y in 0..oh {
            dst[y * dst_stride..y * dst_stride + ow].copy_from_slice(&src.data[y * src.stride..y * src.stride + ow]);
        }
        for y in 0..oh / 2 {
            let s = &src.data[uv0 + y * src.stride..uv0 + y * src.stride + ow];
            let d = &mut dst[uv_dst + y * dst_stride..uv_dst + y * dst_stride + ow];
            if src.vu {
                for (dp, sp) in d.chunks_exact_mut(2).zip(s.chunks_exact(2)) {
                    dp[0] = sp[1];
                    dp[1] = sp[0];
                }
            } else {
                d.copy_from_slice(s);
            }
        }
        return;
    }
    // Поворот плитками 32×32 (кэш): яркость, затем цветность (вдвое меньше, пары байтов).
    const T: usize = 32;
    let (data, stride) = (src.data, src.stride);
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).clamp(1, 4);
    {
        let (yplane, _) = dst.split_at_mut(uv_dst);
        let rows_per = oh.div_ceil(threads).div_ceil(T) * T;
        std::thread::scope(|scope| {
            for (i, chunk) in yplane.chunks_mut(rows_per * dst_stride).enumerate() {
                let y0 = i * rows_per;
                scope.spawn(move || {
                    let rows = (chunk.len() / dst_stride).min(oh.saturating_sub(y0));
                    for ty in (0..rows).step_by(T) {
                        for tx in (0..ow).step_by(T) {
                            for oy in ty..(ty + T).min(rows) {
                                let d = &mut chunk[oy * dst_stride..];
                                for ox in tx..(tx + T).min(ow) {
                                    let (x, y) = src_xy(ox, y0 + oy, sw, sh, rot);
                                    d[ox] = data[y * stride + x];
                                }
                            }
                        }
                    }
                });
            }
        });
    }
    let (cw, ch) = (ow / 2, oh / 2);
    let (scw, sch) = (sw / 2, sh / 2);
    let uvp = &mut dst[uv_dst..];
    for ty in (0..ch).step_by(T) {
        for tx in (0..cw).step_by(T) {
            for oy in ty..(ty + T).min(ch) {
                for ox in tx..(tx + T).min(cw) {
                    let (x, y) = src_xy(ox, oy, scw, sch, rot);
                    let s = uv0 + y * stride + x * 2;
                    let d = oy * dst_stride + ox * 2;
                    uvp[d] = data[s + ui];
                    uvp[d + 1] = data[s + vi];
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: usize, h: usize) -> Vec<u8> {
        let mut v = vec![0u8; w * h * 3 / 2];
        for y in 0..h {
            for x in 0..w {
                v[y * w + x] = (y * w + x) as u8;
            }
        }
        for i in 0..w * h / 2 {
            v[w * h + i] = 128;
        }
        v
    }

    #[test]
    fn rotate_rgba_90() {
        let (w, h) = (4, 2);
        let d = frame(w, h);
        let s = Src { data: &d, width: w, height: h, stride: w, scanlines: h, vu: true };
        let (ow, oh) = out_size(w, h, 90, 1);
        assert_eq!((ow, oh), (2, 4));
        let mut out = vec![0u8; ow * oh * 4];
        to_rgba(&s, 90, false, 1, &mut out);
        // по часовой: левый верхний результата — левый нижний источника
        assert_eq!(out[0], d[(h - 1) * w]);
        assert_eq!(out[4], d[0]);
    }

    #[test]
    fn rotate_nv12_matches_rgba() {
        let (w, h) = (64, 32);
        let d = frame(w, h);
        let s = Src { data: &d, width: w, height: h, stride: w, scanlines: h, vu: true };
        for rot in [0, 90, 180, 270] {
            let (ow, oh) = out_size(w, h, rot, 1);
            let mut nv = vec![0u8; ow * oh * 3 / 2];
            to_nv12(&s, rot, &mut nv, ow, oh);
            let mut rgba = vec![0u8; ow * oh * 4];
            to_rgba(&s, rot, false, 1, &mut rgba);
            for i in 0..ow * oh {
                assert_eq!(nv[i], rgba[i * 4], "rot {rot} px {i}");
            }
        }
    }
}
