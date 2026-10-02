//! «Ночь» — снимок из серии кадров (как Night Sight): N кадров YUV полного размера с потока камеры →
//! выравнивание по сдвигу (пирамида яркости: 1/8 → 1/2 → полный размер, сумма модулей разностей по
//! сетке точек) → слияние с отбраковкой несовпавших пикселей (движение в кадре не «двоится») →
//! тоновая кривая для тёмных сцен → JPEG (кодер MJPEG FFmpeg, YUV без перевода в RGB) с EXIF.
//! Шум падает примерно как √N.

use std::ffi::c_int;

use anyhow::{bail, Result};
use ffmpeg_next::ffi;

/// Кадр в памяти: яркость `w × h`, цветность — пары (V,U для NV21) `w × h/2`, строки без отступов.
pub struct Frame {
    pub w: usize,
    pub h: usize,
    pub y: Vec<u8>,
    pub uv: Vec<u8>,
}

impl Frame {
    /// Скопировать кадр потока (строка `stride`, `scanlines` строк Y).
    pub fn copy_from(data: &[u8], w: usize, h: usize, stride: usize, scanlines: usize) -> Frame {
        let (w, h) = (w & !1, h & !1);
        let mut y = vec![0u8; w * h];
        let mut uv = vec![0u8; w * h / 2];
        for r in 0..h {
            y[r * w..(r + 1) * w].copy_from_slice(&data[r * stride..r * stride + w]);
        }
        let uv0 = stride * scanlines;
        for r in 0..h / 2 {
            uv[r * w..(r + 1) * w].copy_from_slice(&data[uv0 + r * stride..uv0 + r * stride + w]);
        }
        Frame { w, h, y, uv }
    }
}

/// Уменьшенная яркость (среднее блоков k×k).
fn downscale(y: &[u8], w: usize, h: usize, k: usize) -> (Vec<u16>, usize, usize) {
    let (dw, dh) = (w / k, h / k);
    let mut out = vec![0u16; dw * dh];
    for by in 0..dh {
        for bx in 0..dw {
            let mut s = 0u32;
            for yy in 0..k {
                let row = &y[(by * k + yy) * w + bx * k..(by * k + yy) * w + bx * k + k];
                s += row.iter().map(|&v| v as u32).sum::<u32>();
            }
            out[by * dw + bx] = (s / (k * k) as u32) as u16;
        }
    }
    (out, dw, dh)
}

/// Лучший сдвиг (dx, dy) кадра `b` относительно `a` в окне ±r вокруг (cx, cy) — по сетке с шагом `step`.
#[allow(clippy::too_many_arguments)]
fn search(a: &[u16], b: &[u16], w: usize, h: usize, cx: i32, cy: i32, r: i32, step: usize) -> (i32, i32) {
    let margin = (r.unsigned_abs() as usize + cx.unsigned_abs() as usize + cy.unsigned_abs() as usize + 2).min(w / 3).min(h / 3);
    let mut best = (u64::MAX, cx, cy);
    for dy in cy - r..=cy + r {
        for dx in cx - r..=cx + r {
            let mut sad = 0u64;
            let mut y = margin;
            while y < h - margin {
                let by = (y as i32 + dy) as usize;
                let mut x = margin;
                while x < w - margin {
                    let bx = (x as i32 + dx) as usize;
                    sad += (a[y * w + x] as i32 - b[by * w + bx] as i32).unsigned_abs() as u64;
                    x += step;
                }
                y += step;
            }
            // при почти равных — меньший сдвиг (повторяющиеся узоры: плитка, жалюзи)
            let sad = sad + sad / 64 * ((dx - cx).unsigned_abs() + (dy - cy).unsigned_abs()) as u64 / r.max(1) as u64;
            if sad < best.0 {
                best = (sad, dx, dy);
            }
        }
    }
    (best.1, best.2)
}

/// Сдвиг кадра `f` относительно опорного (в пикселях полного размера, чётный — из-за цветности 2×2).
fn align(r8: &(Vec<u16>, usize, usize), r2: &(Vec<u16>, usize, usize), f: &Frame) -> (i32, i32) {
    let f8 = downscale(&f.y, f.w, f.h, 8);
    let (dx, dy) = search(&r8.0, &f8.0, r8.1, r8.2, 0, 0, 12, 2);
    let f2 = downscale(&f.y, f.w, f.h, 2);
    let (dx, dy) = search(&r2.0, &f2.0, r2.1, r2.2, dx * 4, dy * 4, 3, 8);
    (dx * 2, dy * 2)
}

/// Резкость (энергия градиента яркости на уменьшенном кадре) — опорным берётся самый резкий.
fn sharpness(f: &Frame) -> u64 {
    let (d, w, h) = downscale(&f.y, f.w, f.h, 4);
    let mut s = 0u64;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let gx = d[y * w + x + 1] as i32 - d[y * w + x - 1] as i32;
            let gy = d[(y + 1) * w + x] as i32 - d[(y - 1) * w + x] as i32;
            s += (gx * gx + gy * gy) as u64;
        }
    }
    s
}

/// Слить серию в один кадр.
pub fn merge(mut frames: Vec<Frame>) -> Frame {
    if frames.len() <= 1 {
        return frames.pop().expect("кадр");
    }
    let ref_i = (0..frames.len()).max_by_key(|&i| sharpness(&frames[i])).unwrap_or(0);
    let reference = frames.swap_remove(ref_i);
    let (w, h) = (reference.w, reference.h);
    let r8 = downscale(&reference.y, w, h, 8);
    let r2 = downscale(&reference.y, w, h, 2);
    let shifts: Vec<(i32, i32)> = frames.iter().map(|f| align(&r8, &r2, f)).collect();
    tracing::info!("ночь: опорный {ref_i}, сдвиги {shifts:?}");
    // сумма и число принятых по каждому пикселю; яркость отличается от опорной больше порога — движение
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).clamp(1, 6);
    let rows = (h / 2).div_ceil(threads) * 2;
    let mut out = Frame { w, h, y: vec![0u8; w * h], uv: vec![0u8; w * h / 2] };
    {
        let (fr, sh, rf) = (&frames, &shifts, &reference);
        let ychunks = out.y.chunks_mut(rows * w);
        let uvchunks = out.uv.chunks_mut(rows / 2 * w);
        std::thread::scope(|scope| {
            for (i, (yc, uvc)) in ychunks.zip(uvchunks).enumerate() {
                scope.spawn(move || {
                    let y0 = i * rows;
                    let nrows = yc.len() / w;
                    const T: i32 = 28;
                    for r in 0..nrows {
                        let y = y0 + r;
                        for x in 0..w {
                            let base = rf.y[y * w + x] as i32;
                            let (mut sum, mut n) = (base, 1);
                            for (f, &(dx, dy)) in fr.iter().zip(sh.iter()) {
                                let (sx, sy) = (x as i32 + dx, y as i32 + dy);
                                if sx < 0 || sy < 0 || sx >= w as i32 || sy >= h as i32 {
                                    continue;
                                }
                                let v = f.y[sy as usize * w + sx as usize] as i32;
                                if (v - base).abs() < T {
                                    sum += v;
                                    n += 1;
                                }
                            }
                            yc[r * w + x] = ((sum + n / 2) / n) as u8;
                        }
                        // цветность: строка пар на каждые две строки яркости; решение — по яркости левого верхнего
                        if y % 2 == 0 {
                            let cr = r / 2;
                            for x in (0..w).step_by(2) {
                                let base_y = rf.y[y * w + x] as i32;
                                let (mut s0, mut s1, mut n) = (rf.uv[(y / 2) * w + x] as i32, rf.uv[(y / 2) * w + x + 1] as i32, 1);
                                for (f, &(dx, dy)) in fr.iter().zip(sh.iter()) {
                                    let (sx, sy) = (x as i32 + dx, y as i32 + dy);
                                    if sx < 0 || sy < 0 || sx + 1 >= w as i32 || sy >= h as i32 {
                                        continue;
                                    }
                                    if (f.y[sy as usize * w + sx as usize] as i32 - base_y).abs() >= T {
                                        continue;
                                    }
                                    let c = (sy as usize / 2) * w + (sx as usize & !1);
                                    s0 += f.uv[c] as i32;
                                    s1 += f.uv[c + 1] as i32;
                                    n += 1;
                                }
                                uvc[cr * w + x] = ((s0 + n / 2) / n) as u8;
                                uvc[cr * w + x + 1] = ((s1 + n / 2) / n) as u8;
                            }
                        }
                    }
                });
            }
        });
    }
    out
}

/// Тоновая кривая тёмной сцены: средняя яркость тянется к ~0,42 (тени поднимаются, света не выбиваются),
/// цветность чуть насыщается вслед за яркостью.
pub fn tone(f: &mut Frame) {
    let mean = f.y.iter().step_by(7).map(|&v| v as u64).sum::<u64>() as f32 / (f.y.len() / 7).max(1) as f32 / 255.0;
    let target = 0.42f32;
    if mean >= target * 0.9 || mean <= 0.0 {
        return;
    }
    // y' = 1 − (1 − y)^g: g подбирается, чтобы среднее стало целевым
    let g = ((1.0 - target).ln() / (1.0 - mean).ln()).clamp(1.0, 6.0);
    let lut: Vec<u8> = (0..256).map(|v| ((1.0 - (1.0 - v as f32 / 255.0).powf(g)) * 255.0).round().clamp(0.0, 255.0) as u8).collect();
    for v in f.y.iter_mut() {
        *v = lut[*v as usize];
    }
    let sat = (1.0 + (g - 1.0) * 0.15).min(1.4);
    for c in f.uv.iter_mut() {
        *c = (128.0 + (*c as f32 - 128.0) * sat).round().clamp(0.0, 255.0) as u8;
    }
    tracing::info!("ночь: средняя яркость {:.2} → кривая g={g:.2}", mean);
}

/// JPEG из кадра (NV21, если `vu`) — кодер MJPEG FFmpeg (полный диапазон, 4:2:0).
pub fn to_jpeg(f: &Frame, vu: bool, quality: u32) -> Result<Vec<u8>> {
    // SAFETY: FFI FFmpeg; контекст, кадр и пакет освобождаются в конце
    unsafe {
        let codec = ffi::avcodec_find_encoder(ffi::AVCodecID::AV_CODEC_ID_MJPEG);
        if codec.is_null() {
            bail!("нет кодера JPEG");
        }
        let mut c = ffi::avcodec_alloc_context3(codec);
        (*c).width = f.w as c_int;
        (*c).height = f.h as c_int;
        (*c).pix_fmt = ffi::AVPixelFormat::AV_PIX_FMT_YUVJ420P;
        (*c).color_range = ffi::AVColorRange::AVCOL_RANGE_JPEG;
        (*c).time_base = ffi::AVRational { num: 1, den: 1 };
        (*c).flags |= ffi::AV_CODEC_FLAG_QSCALE as c_int;
        // качество: q 2 ≈ 95, 3 ≈ 90
        let q = if quality >= 95 { 2 } else { 3 };
        (*c).qmin = q;
        (*c).qmax = q;
        (*c).thread_count = 4;
        let r = ffi::avcodec_open2(c, codec, std::ptr::null_mut());
        if r < 0 {
            ffi::avcodec_free_context(&mut c);
            bail!("кодер JPEG: {r}");
        }
        let mut fr = ffi::av_frame_alloc();
        (*fr).width = f.w as c_int;
        (*fr).height = f.h as c_int;
        (*fr).format = ffi::AVPixelFormat::AV_PIX_FMT_YUVJ420P as c_int;
        (*fr).quality = q * ffi::FF_QP2LAMBDA as c_int;
        ffi::av_frame_get_buffer(fr, 0);
        for y in 0..f.h {
            let d = (*fr).data[0].add(y * (*fr).linesize[0] as usize);
            std::ptr::copy_nonoverlapping(f.y.as_ptr().add(y * f.w), d, f.w);
        }
        let (ui, vi) = if vu { (1, 0) } else { (0, 1) };
        for y in 0..f.h / 2 {
            let du = (*fr).data[1].add(y * (*fr).linesize[1] as usize);
            let dv = (*fr).data[2].add(y * (*fr).linesize[2] as usize);
            let row = &f.uv[y * f.w..(y + 1) * f.w];
            for x in 0..f.w / 2 {
                *du.add(x) = row[x * 2 + ui];
                *dv.add(x) = row[x * 2 + vi];
            }
        }
        let mut out = Vec::new();
        let mut pkt = ffi::av_packet_alloc();
        if ffi::avcodec_send_frame(c, fr) >= 0 {
            ffi::avcodec_send_frame(c, std::ptr::null());
            while ffi::avcodec_receive_packet(c, pkt) >= 0 {
                out.extend_from_slice(std::slice::from_raw_parts((*pkt).data, (*pkt).size as usize));
                ffi::av_packet_unref(pkt);
            }
        }
        ffi::av_packet_free(&mut pkt);
        ffi::av_frame_free(&mut fr);
        ffi::avcodec_free_context(&mut c);
        if out.len() < 4 {
            bail!("JPEG не получился");
        }
        Ok(out)
    }
}

/// Производитель и модель — из PRETTY_HOSTNAME `/etc/machine-info` (как у syncamd).
fn make_model() -> (String, String) {
    let s = std::fs::read_to_string("/etc/machine-info").unwrap_or_default();
    let name = s
        .lines()
        .find_map(|l| l.strip_prefix("PRETTY_HOSTNAME="))
        .map(|v| v.trim().trim_matches('"').to_string())
        .unwrap_or_default();
    match name.split_once(' ') {
        Some((a, b)) => (a.to_string(), b.to_string()),
        None => (name.clone(), name),
    }
}

/// EXIF (APP1) сразу после SOI: производитель, модель, ориентация, дата съёмки.
pub fn add_exif(jpeg: &[u8], orientation: u16) -> Vec<u8> {
    let (make, model) = make_model();
    // SAFETY: localtime_r в живую структуру
    let date = unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        format!("{:04}:{:02}:{:02} {:02}:{:02}:{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec)
    };
    let soft = "synshell syncamera (ночь)";
    // TIFF (little endian): IFD0 — 6 записей, Exif IFD — 1 запись; строки — после таблиц
    let asciz = |s: &str| {
        let mut v = s.as_bytes().to_vec();
        v.push(0);
        v
    };
    let strings = [asciz(&make), asciz(&model), asciz(soft), asciz(&date), asciz(&date)];
    let ifd0_n = 6u16;
    let ifd0_off = 8u32;
    let ifd0_len = 2 + ifd0_n as u32 * 12 + 4;
    let exif_off = ifd0_off + ifd0_len;
    let exif_len = 2 + 12 + 4;
    let mut data_off = exif_off + exif_len;
    let mut t: Vec<u8> = Vec::new();
    t.extend_from_slice(b"II*\0");
    t.extend_from_slice(&ifd0_off.to_le_bytes());
    let mut offs = Vec::new();
    for s in &strings {
        offs.push(data_off);
        data_off += s.len() as u32;
    }
    let entry = |t: &mut Vec<u8>, tag: u16, ty: u16, count: u32, val: u32| {
        t.extend_from_slice(&tag.to_le_bytes());
        t.extend_from_slice(&ty.to_le_bytes());
        t.extend_from_slice(&count.to_le_bytes());
        t.extend_from_slice(&val.to_le_bytes());
    };
    t.extend_from_slice(&ifd0_n.to_le_bytes());
    entry(&mut t, 0x010F, 2, strings[0].len() as u32, offs[0]); // Make
    entry(&mut t, 0x0110, 2, strings[1].len() as u32, offs[1]); // Model
    entry(&mut t, 0x0112, 3, 1, orientation as u32); // Orientation (SHORT в младших байтах)
    entry(&mut t, 0x0131, 2, strings[2].len() as u32, offs[2]); // Software
    entry(&mut t, 0x0132, 2, strings[3].len() as u32, offs[3]); // DateTime
    entry(&mut t, 0x8769, 4, 1, exif_off); // Exif IFD
    t.extend_from_slice(&0u32.to_le_bytes());
    t.extend_from_slice(&1u16.to_le_bytes());
    entry(&mut t, 0x9003, 2, strings[4].len() as u32, offs[4]); // DateTimeOriginal
    t.extend_from_slice(&0u32.to_le_bytes());
    for s in &strings {
        t.extend_from_slice(s);
    }
    let mut app1 = vec![0xFF, 0xE1];
    let len = (2 + 6 + t.len()) as u16;
    app1.extend_from_slice(&len.to_be_bytes());
    app1.extend_from_slice(b"Exif\0\0");
    app1.extend_from_slice(&t);
    let mut out = Vec::with_capacity(jpeg.len() + app1.len());
    out.extend_from_slice(&jpeg[..2]);
    out.extend_from_slice(&app1);
    out.extend_from_slice(&jpeg[2..]);
    out
}

/// Ориентация EXIF для поворота по часовой.
pub fn exif_orientation(rot: u32) -> u16 {
    match rot % 360 {
        90 => 6,
        180 => 3,
        270 => 8,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Непериодичная гладкая картинка, сдвинутая на (dx, dy), + шум ±8.
    fn scene(x: f32, y: f32) -> f32 {
        110.0 + 40.0 * (x * 0.031).sin() * (y * 0.017).cos() + 30.0 * ((x + 2.0 * y) * 0.0123).sin() + 20.0 * (x * 0.0071 - y * 0.029).cos()
    }

    fn noisy(w: usize, h: usize, seed: u32, dx: i32, dy: i32) -> Frame {
        let mut s = seed;
        let mut y = vec![0u8; w * h];
        for r in 0..h {
            for c in 0..w {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                let v = scene((c as i32 + dx) as f32, (r as i32 + dy) as f32);
                y[r * w + c] = (v as i32 + (s % 17) as i32 - 8).clamp(0, 255) as u8;
            }
        }
        Frame { w, h, y, uv: vec![128; w * h / 2] }
    }

    #[test]
    fn merge_aligns_and_denoises() {
        let (w, h) = (512, 384);
        let shifts = [(0, 0), (6, -4), (-10, 2), (2, 8), (-4, -6), (12, 10)];
        let frames: Vec<Frame> = shifts.iter().enumerate().map(|(i, &(dx, dy))| noisy(w, h, 7 + i as u32, dx, dy)).collect();
        let m = merge(frames);
        // после слияния шум меньше: отклонение от чистой картинки (опорный кадр — любой из серии)
        let (mut best, mut worst1) = (f32::MAX, 0f32);
        for &(dx, dy) in &shifts {
            let mut err = 0f32;
            for r in 100..280 {
                for c in 100..400 {
                    err += (m.y[r * w + c] as f32 - scene((c as i32 + dx) as f32, (r as i32 + dy) as f32)).abs();
                }
            }
            best = best.min(err / (180.0 * 300.0));
        }
        let one = noisy(w, h, 99, 0, 0);
        for r in 100..280 {
            for c in 100..400 {
                worst1 += (one.y[r * w + c] as f32 - scene(c as f32, r as f32)).abs();
            }
        }
        let worst1 = worst1 / (180.0 * 300.0);
        assert!(best < worst1 * 0.6, "ошибка после слияния {best:.2}, одного кадра {worst1:.2}");
    }

    #[test]
    fn exif_has_orientation() {
        let jpeg = [0xFF, 0xD8, 0xFF, 0xD9];
        let j = add_exif(&jpeg, 6);
        assert_eq!(crate::media::exif_orientation(&j), 6);
    }
}
