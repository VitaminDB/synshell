//! Декодер видео трансляции: libavcodec + libswscale через `dlopen`.
//!
//! ffmpeg не нужен для сборки: нет библиотек — зритель просто не просит
//! видео и трансляция идёт без потерь. Цвет — BT.709 полного диапазона, как
//! пишет композитор (synwm `encode.rs`), независимо от меток в потоке.
//! Потоки декодера — по срезам: покадровые потоки добавили бы задержку.

use std::ffi::{c_char, c_int, c_void, CString};
use std::sync::OnceLock;

type P = *mut c_void;

struct Lib {
    find_decoder_by_name: unsafe extern "C" fn(*const c_char) -> P,
    alloc_context3: unsafe extern "C" fn(P) -> P,
    open2: unsafe extern "C" fn(P, P, *mut P) -> c_int,
    free_context: unsafe extern "C" fn(*mut P),
    send_packet: unsafe extern "C" fn(P, P) -> c_int,
    receive_frame: unsafe extern "C" fn(P, P) -> c_int,
    packet_alloc: unsafe extern "C" fn() -> P,
    packet_free: unsafe extern "C" fn(*mut P),
    packet_from_data: unsafe extern "C" fn(P, *mut u8, c_int) -> c_int,
    frame_alloc: unsafe extern "C" fn() -> P,
    frame_free: unsafe extern "C" fn(*mut P),
    malloc: unsafe extern "C" fn(usize) -> *mut u8,
    opt_set: unsafe extern "C" fn(P, *const c_char, *const c_char, c_int) -> c_int,
    sws_get_context: unsafe extern "C" fn(c_int, c_int, c_int, c_int, c_int, c_int, c_int, P, P, *const f64) -> P,
    sws_set_colorspace_details: unsafe extern "C" fn(P, *const c_int, c_int, *const c_int, c_int, c_int, c_int, c_int) -> c_int,
    sws_get_coefficients: unsafe extern "C" fn(c_int) -> *const c_int,
    sws_scale: unsafe extern "C" fn(P, *const *const u8, *const c_int, c_int, c_int, *const *mut u8, *const c_int) -> c_int,
    sws_free_context: unsafe extern "C" fn(P),
}

unsafe impl Send for Lib {}
unsafe impl Sync for Lib {}

fn open(names: &[&str]) -> Option<P> {
    for n in names {
        let c = CString::new(*n).ok()?;
        let h = unsafe { libc::dlopen(c.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if !h.is_null() {
            return Some(h);
        }
    }
    None
}

fn sym<T>(h: P, name: &str) -> Option<T> {
    let c = CString::new(name).ok()?;
    let p = unsafe { libc::dlsym(h, c.as_ptr()) };
    if p.is_null() {
        log::warn!("ffmpeg: нет {name}");
        return None;
    }
    // SAFETY: T — указатель на функцию с сигнатурой из заголовков ffmpeg.
    Some(unsafe { std::mem::transmute_copy::<P, T>(&p) })
}

fn lib() -> Option<&'static Lib> {
    static LIB: OnceLock<Option<Lib>> = OnceLock::new();
    LIB.get_or_init(|| {
        let av = open(&["libavcodec.so", "libavcodec.so.63", "libavcodec.so.62", "libavcodec.so.61", "libavcodec.so.60"])?;
        let au = open(&["libavutil.so", "libavutil.so.61", "libavutil.so.60", "libavutil.so.59", "libavutil.so.58"])?;
        let sw = open(&["libswscale.so", "libswscale.so.10", "libswscale.so.9", "libswscale.so.8", "libswscale.so.7"])?;
        Some(Lib {
            find_decoder_by_name: sym(av, "avcodec_find_decoder_by_name")?,
            alloc_context3: sym(av, "avcodec_alloc_context3")?,
            open2: sym(av, "avcodec_open2")?,
            free_context: sym(av, "avcodec_free_context")?,
            send_packet: sym(av, "avcodec_send_packet")?,
            receive_frame: sym(av, "avcodec_receive_frame")?,
            packet_alloc: sym(av, "av_packet_alloc")?,
            packet_free: sym(av, "av_packet_free")?,
            packet_from_data: sym(av, "av_packet_from_data")?,
            frame_alloc: sym(au, "av_frame_alloc")?,
            frame_free: sym(au, "av_frame_free")?,
            malloc: sym(au, "av_malloc")?,
            opt_set: sym(au, "av_opt_set")?,
            sws_get_context: sym(sw, "sws_getContext")?,
            sws_set_colorspace_details: sym(sw, "sws_setColorspaceDetails")?,
            sws_get_coefficients: sym(sw, "sws_getCoefficients")?,
            sws_scale: sym(sw, "sws_scale")?,
            sws_free_context: sym(sw, "sws_freeContext")?,
        })
    })
    .as_ref()
}

/// Есть ли декодер видео (ffmpeg установлен).
pub fn available() -> bool {
    lib().is_some()
}

// Поля AVFrame, стабильные много лет (libavutil/frame.h): data[8], linesize[8],
// extended_data, width, height, nb_samples, format.
const FRAME_LINESIZE: usize = 64;
const FRAME_WIDTH: usize = 104;
const FRAME_HEIGHT: usize = 108;
const FRAME_FORMAT: usize = 116;
const PIX_FMT_RGBA: c_int = 26;
const SWS_BICUBIC: c_int = 1 << 2;
const SWS_FULL_CHR_H_INT: c_int = 1 << 13;
const SWS_ACCURATE_RND: c_int = 1 << 18;
const SWS_CS_ITU709: c_int = 1;
const AV_INPUT_BUFFER_PADDING_SIZE: usize = 64;

pub struct Decoder {
    l: &'static Lib,
    ctx: P,
    pkt: P,
    frame: P,
    sws: P,
    sws_key: (c_int, c_int, c_int),
    pub codec: String,
    /// Кадр RGBA пути через swscale (форматы кроме 8-битного 4:2:0).
    rgba: Vec<u8>,
    pub size: (u32, u32),
}

unsafe impl Send for Decoder {}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            if !self.sws.is_null() {
                (self.l.sws_free_context)(self.sws);
            }
            (self.l.frame_free)(&mut self.frame);
            (self.l.packet_free)(&mut self.pkt);
            (self.l.free_context)(&mut self.ctx);
        }
    }
}

impl Decoder {
    pub fn new(codec: &str) -> Result<Self, String> {
        let l = lib().ok_or("нет ffmpeg (libavcodec)")?;
        let name = CString::new(match codec {
            "hevc" => "hevc",
            _ => "h264",
        })
        .unwrap();
        unsafe {
            let c = (l.find_decoder_by_name)(name.as_ptr());
            if c.is_null() {
                return Err(format!("нет декодера {codec}"));
            }
            let ctx = (l.alloc_context3)(c);
            let set = |k: &str, v: &str| {
                let (k, v) = (CString::new(k).unwrap(), CString::new(v).unwrap());
                (l.opt_set)(ctx, k.as_ptr(), v.as_ptr(), 0)
            };
            set("flags", "+low_delay");
            set("thread_type", "slice");
            set("threads", "auto");
            if (l.open2)(ctx, c, std::ptr::null_mut()) < 0 {
                let mut ctx = ctx;
                (l.free_context)(&mut ctx);
                return Err(format!("декодер {codec} не открылся"));
            }
            Ok(Self {
                l,
                ctx,
                pkt: (l.packet_alloc)(),
                frame: (l.frame_alloc)(),
                sws: std::ptr::null_mut(),
                sws_key: (0, 0, -1),
                codec: codec.to_string(),
                rgba: Vec::new(),
                size: (0, 0),
            })
        }
    }

    /// Декодировать пакет одного кадра и перевести в RGBA прямоугольники
    /// `rects` прямо в кадр зрителя `dst` (`dw`×`dh`). `Ok(false)` — кадра пока нет.
    pub fn decode(&mut self, data: &[u8], rects: &[[u32; 4]], dst: &mut [u8], dw: u32, dh: u32) -> Result<bool, String> {
        let l = self.l;
        let timing = std::env::var_os("SYNLINK_VIEW_TIMING").is_some();
        unsafe {
            let buf = (l.malloc)(data.len() + AV_INPUT_BUFFER_PADDING_SIZE);
            if buf.is_null() {
                return Err("нет памяти".into());
            }
            std::ptr::copy_nonoverlapping(data.as_ptr(), buf, data.len());
            std::ptr::write_bytes(buf.add(data.len()), 0, AV_INPUT_BUFFER_PADDING_SIZE);
            // Пакет забирает буфер себе.
            if (l.packet_from_data)(self.pkt, buf, data.len() as c_int) < 0 {
                return Err("пакет".into());
            }
            let t = std::time::Instant::now();
            let r = (l.send_packet)(self.ctx, self.pkt);
            if r < 0 {
                return Err(format!("send_packet: {r}"));
            }
            let mut got = false;
            loop {
                let r = (l.receive_frame)(self.ctx, self.frame);
                if r < 0 {
                    break;
                }
                let dt = t.elapsed();
                let t2 = std::time::Instant::now();
                self.convert(rects, dst, dw, dh)?;
                if timing {
                    eprintln!("  декодер {:.1} мс, цвет {:.1} мс", dt.as_secs_f64() * 1e3, t2.elapsed().as_secs_f64() * 1e3);
                }
                got = true;
            }
            Ok(got)
        }
    }

    unsafe fn convert(&mut self, rects: &[[u32; 4]], dst: &mut [u8], dw: u32, dh: u32) -> Result<(), String> {
        let f = self.frame.cast::<u8>();
        let w = f.add(FRAME_WIDTH).cast::<c_int>().read();
        let h = f.add(FRAME_HEIGHT).cast::<c_int>().read();
        let fmt = f.add(FRAME_FORMAT).cast::<c_int>().read();
        if w <= 0 || h <= 0 {
            return Err("пустой кадр".into());
        }
        self.size = (w as u32, h as u32);
        if self.size != (dw, dh) {
            return Ok(());
        }
        let data = f.cast::<*const u8>();
        let linesize = f.add(FRAME_LINESIZE).cast::<c_int>();
        if fmt == PIX_FMT_YUV420P || fmt == PIX_FMT_YUVJ420P {
            let planes = Yuv420 {
                y: data.read(),
                u: data.add(1).read(),
                v: data.add(2).read(),
                ys: linesize.read() as usize,
                us: linesize.add(1).read() as usize,
                vs: linesize.add(2).read() as usize,
                w: w as usize,
                h: h as usize,
            };
            yuv420_to_rgba(&planes, rects, dst, dw as usize);
            return Ok(());
        }
        // Другой формат (10 бит и т. п.) — swscale целиком, потом прямоугольники.
        self.swscale(w, h, fmt)?;
        for &r in rects {
            crate::blit(dst, dw, &self.rgba, w as u32, r);
        }
        Ok(())
    }

    unsafe fn swscale(&mut self, w: c_int, h: c_int, fmt: c_int) -> Result<(), String> {
        let l = self.l;
        let f = self.frame.cast::<u8>();
        if self.sws.is_null() || self.sws_key != (w, h, fmt) {
            if !self.sws.is_null() {
                (l.sws_free_context)(self.sws);
            }
            self.sws = (l.sws_get_context)(
                w,
                h,
                fmt,
                w,
                h,
                PIX_FMT_RGBA,
                SWS_BICUBIC | SWS_FULL_CHR_H_INT | SWS_ACCURATE_RND,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            if self.sws.is_null() {
                return Err(format!("swscale для формата {fmt}"));
            }
            let coefs = (l.sws_get_coefficients)(SWS_CS_ITU709);
            (l.sws_set_colorspace_details)(self.sws, coefs, 1, coefs, 1, 0, 1 << 16, 1 << 16);
            self.sws_key = (w, h, fmt);
        }
        self.rgba.resize(w as usize * h as usize * 4, 255);
        let data = f.cast::<*const u8>();
        let linesize = f.add(FRAME_LINESIZE).cast::<c_int>();
        let dst = [self.rgba.as_mut_ptr(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()];
        let dst_stride = [w * 4, 0, 0, 0];
        (l.sws_scale)(self.sws, data, linesize, 0, h, dst.as_ptr(), dst_stride.as_ptr());
        Ok(())
    }
}

const PIX_FMT_YUV420P: c_int = 0;
const PIX_FMT_YUVJ420P: c_int = 12;

struct Yuv420 {
    y: *const u8,
    u: *const u8,
    v: *const u8,
    ys: usize,
    us: usize,
    vs: usize,
    w: usize,
    h: usize,
}

unsafe impl Sync for Yuv420 {}

/// YUV 4:2:0 (BT.709, полный диапазон) → RGBA в прямоугольниках, по
/// полосам строк в потоках. Цветность — билинейно, отсчёты по центру блока
/// 2×2 (так их усредняет шейдер композитора).
fn yuv420_to_rgba(p: &Yuv420, rects: &[[u32; 4]], dst: &mut [u8], dw: usize) {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
    for &[rx, ry, rw, rh] in rects {
        let (x0, y0) = (rx as usize, ry as usize);
        let x1 = (x0 + rw as usize).min(p.w);
        let y1 = (y0 + rh as usize).min(p.h);
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        let rows = y1 - y0;
        let band = rows.div_ceil(threads).max(16);
        // Полосы строк кадра зрителя не пересекаются — каждая своему потоку.
        let stride = dw * 4;
        let region = &mut dst[y0 * stride..y1 * stride];
        std::thread::scope(|sc| {
            for (bi, chunk) in region.chunks_mut(band * stride).enumerate() {
                let yb = y0 + bi * band;
                sc.spawn(move || {
                    let n = chunk.len() / stride;
                    for i in 0..n {
                        convert_row(p, yb + i, x0, x1, &mut chunk[i * stride..(i + 1) * stride]);
                    }
                });
            }
        });
    }
}

/// Ближний (вес 3/4) и дальний (1/4) отсчёт цветности для пикселя `i`
/// при отсчётах по центру пар: чётный пиксель — левее/выше центра пары.
#[inline]
fn near_far(i: usize, n: usize) -> (usize, usize) {
    let c = (i / 2).min(n - 1);
    let far = if i % 2 == 0 { c.saturating_sub(1) } else { (c + 1).min(n - 1) };
    (c, far)
}

#[inline]
fn convert_row(p: &Yuv420, y: usize, x0: usize, x1: usize, out: &mut [u8]) {
    let cw = p.w.div_ceil(2);
    let ch = p.h.div_ceil(2);
    let (ra, rb) = near_far(y, ch);
    unsafe {
        let yrow = p.y.add(y * p.ys);
        let (ua, ub) = (p.u.add(ra * p.us), p.u.add(rb * p.us));
        let (va, vb) = (p.v.add(ra * p.vs), p.v.add(rb * p.vs));
        // По вертикали: 3·ближняя строка + дальняя.
        let col = |a: *const u8, b: *const u8, cx: usize| -> i32 { 3 * *a.add(cx) as i32 + *b.add(cx) as i32 };
        for x in x0..x1 {
            let (na, nb) = near_far(x, cw);
            // По горизонтали так же: сумма весов 16.
            let u = ((3 * col(ua, ub, na) + col(ua, ub, nb) + 8) >> 4) - 128;
            let v = ((3 * col(va, vb, na) + col(va, vb, nb) + 8) >> 4) - 128;
            let yy = *yrow.add(x) as i32;
            // BT.709 полный диапазон, коэффициенты ×65536.
            let r = yy + ((103206 * v + 32768) >> 16);
            let g = yy - ((12276 * u + 30679 * v + 32768) >> 16);
            let b = yy + ((121609 * u + 32768) >> 16);
            let o = x * 4;
            out[o] = r.clamp(0, 255) as u8;
            out[o + 1] = g.clamp(0, 255) as u8;
            out[o + 2] = b.clamp(0, 255) as u8;
            out[o + 3] = 255;
        }
    }
}
