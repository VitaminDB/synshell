//! Видеокодеры записи. Телефон — аппаратный кодер V4L2 M2M (`venc.c`: буферы входа — dma-heap, кадр
//! копируется в них в NV12, несколько кадров в работе). Компьютер — libavcodec: NVENC (кадр NV12 из
//! памяти), VAAPI (кадр загружается в VA-поверхность), libx264 (программный, запасной).
//! Выход — поток Annex B (параметры SPS/PPS в начале первого пакета), метки времени — мкс.
//!
//! Выбор: `Kind::Auto` пробует V4L2 M2M, NVENC, VAAPI, libx264 по порядку; переменная окружения
//! `SYNMEDIA_ENCODER` (`v4l2`, `nvenc`, `vaapi`, `x264`) задаёт кодер явно.

use std::ffi::{c_char, c_int, c_long, CString};
use std::os::fd::RawFd;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use ffmpeg_next::ffi;

use crate::convert::{self, Src};
use crate::mux::{averr, has_vcl};
use crate::syncamd;

/// Выход кодера: (данные, метка, мкс, ключевой).
pub type Out<'a> = dyn FnMut(&[u8], u64, bool) -> Result<()> + 'a;

pub trait VideoEncoder: Send {
    /// Имя для журнала (`v4l2m2m`, `h264_nvenc`…).
    fn name(&self) -> &str;
    /// Отдать кадр (поворот — матрицей MP4, не здесь). `Ok(false)` — кодер занят, кадр пропущен.
    fn encode(&mut self, src: &Src, pts_us: u64, key: bool, out: &mut Out) -> Result<bool>;
    /// Забрать готовые пакеты (ждать первого не дольше `wait_ms`).
    fn poll(&mut self, wait_ms: i32, out: &mut Out) -> Result<()>;
    /// Дописать кадры, ещё идущие в кодере.
    fn finish(&mut self, out: &mut Out) -> Result<()>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Auto,
    V4l2,
    Nvenc,
    Vaapi,
    X264,
}

impl Kind {
    fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "auto" => Kind::Auto,
            "v4l2" | "v4l2m2m" => Kind::V4l2,
            "nvenc" => Kind::Nvenc,
            "vaapi" => Kind::Vaapi,
            "x264" | "libx264" | "software" => Kind::X264,
            _ => return None,
        })
    }
}

/// Открыть кодер `w × h` (чётные), битрейт бит/с.
pub fn open(kind: Kind, w: u32, h: u32, hevc: bool, bitrate: u32, fps: u32) -> Result<Box<dyn VideoEncoder>> {
    let kind = match std::env::var("SYNMEDIA_ENCODER").ok().and_then(|v| Kind::parse(&v)) {
        Some(k) if k != Kind::Auto => k,
        _ => kind,
    };
    let order: &[Kind] = match kind {
        Kind::Auto => &[Kind::V4l2, Kind::Nvenc, Kind::Vaapi, Kind::X264],
        Kind::V4l2 => &[Kind::V4l2],
        Kind::Nvenc => &[Kind::Nvenc],
        Kind::Vaapi => &[Kind::Vaapi],
        Kind::X264 => &[Kind::X264],
    };
    let mut errs = Vec::new();
    for k in order {
        let r: Result<Box<dyn VideoEncoder>> = match k {
            Kind::V4l2 => Venc::open(w, h, hevc, bitrate, fps).map(|e| Box::new(e) as _),
            Kind::Nvenc => FfEnc::open(if hevc { "hevc_nvenc" } else { "h264_nvenc" }, false, w, h, bitrate, fps).map(|e| Box::new(e) as _),
            Kind::Vaapi => FfEnc::open(if hevc { "hevc_vaapi" } else { "h264_vaapi" }, true, w, h, bitrate, fps).map(|e| Box::new(e) as _),
            Kind::X264 => FfEnc::open(if hevc { "libx265" } else { "libx264" }, false, w, h, bitrate, fps).map(|e| Box::new(e) as _),
            Kind::Auto => continue,
        };
        match r {
            Ok(e) => {
                tracing::info!("видеокодер {} {w}×{h}", e.name());
                return Ok(e);
            }
            Err(e) => errs.push(format!("{k:?}: {e:#}")),
        }
    }
    bail!("нет видеокодера ({})", errs.join("; "))
}

// ─── V4L2 M2M (телефон) ─────────────────────────────────────────────────────

#[repr(C)]
#[derive(Default, Clone, Copy, Debug)]
struct VencInfo {
    width: u32,
    height: u32,
    stride: u32,
    y_scanlines: u32,
    in_size: u32,
    in_count: u32,
}

#[repr(C)]
struct VencC {
    _p: [u8; 0],
}

extern "C" {
    fn venc_open(
        dev: *const c_char,
        w: u32,
        h: u32,
        hevc: c_int,
        bitrate: u32,
        fps: u32,
        gop: u32,
        info: *mut VencInfo,
        err: *mut c_char,
        errlen: c_int,
    ) -> *mut VencC;
    fn venc_close(e: *mut VencC);
    fn venc_in_fd(e: *mut VencC, i: u32) -> c_int;
    fn venc_free_input(e: *mut VencC) -> c_int;
    fn venc_queue(e: *mut VencC, i: u32, force_key: c_int, ts_us: u64) -> c_int;
    fn venc_dequeue(e: *mut VencC, timeout_ms: c_int, out: *mut *const u8, ts_us: *mut u64, key: *mut c_int) -> c_long;
}

struct Venc {
    enc: *mut VencC,
    vi: VencInfo,
    maps: Vec<Option<(*mut u8, usize)>>,
    in_flight: u32,
    hevc: bool,
}

// SAFETY: кодер используется одним потоком за раз (владение)
unsafe impl Send for Venc {}

impl Venc {
    fn open(w: u32, h: u32, hevc: bool, bitrate: u32, fps: u32) -> Result<Venc> {
        let mut vi = VencInfo::default();
        let mut err = [0 as c_char; 160];
        // SAFETY: FFI кодера; закрывается в Drop
        let enc = unsafe { venc_open(std::ptr::null(), w, h, hevc as c_int, bitrate, fps, fps.max(1), &mut vi, err.as_mut_ptr(), err.len() as c_int) };
        if enc.is_null() {
            let e = unsafe { std::ffi::CStr::from_ptr(err.as_ptr()) }.to_string_lossy().into_owned();
            bail!("{e}");
        }
        Ok(Venc { enc, vi, maps: vec![None; vi.in_count as usize], in_flight: 0, hevc })
    }
}

impl VideoEncoder for Venc {
    fn name(&self) -> &str {
        "v4l2m2m"
    }

    fn encode(&mut self, src: &Src, pts_us: u64, key: bool, out: &mut Out) -> Result<bool> {
        let max_flight = self.vi.in_count.saturating_sub(1).max(1);
        // SAFETY: FFI кодера
        let mut idx = if self.in_flight < max_flight { unsafe { venc_free_input(self.enc) } } else { -1 };
        if idx < 0 {
            self.poll(40, out)?;
            idx = unsafe { venc_free_input(self.enc) };
        }
        if idx < 0 {
            return Ok(false);
        }
        let idx = idx as usize;
        let fd: RawFd = unsafe { venc_in_fd(self.enc, idx as u32) };
        if self.maps[idx].is_none() {
            // SAFETY: отображение буфера dma-heap кодера на запись
            let p = unsafe { libc::mmap(std::ptr::null_mut(), self.vi.in_size as usize, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd, 0) };
            if p == libc::MAP_FAILED {
                bail!("mmap входа кодера: {}", std::io::Error::last_os_error());
            }
            self.maps[idx] = Some((p as *mut u8, self.vi.in_size as usize));
        }
        let (p, len) = self.maps[idx].unwrap();
        // SAFETY: отображение живо, пока жив кодер
        let dst = unsafe { std::slice::from_raw_parts_mut(p, len) };
        syncamd::dma_sync(fd, true, true);
        convert::to_nv12(src, 0, dst, self.vi.stride as usize, self.vi.y_scanlines as usize);
        syncamd::dma_sync(fd, false, true);
        // SAFETY: FFI кодера
        let r = unsafe { venc_queue(self.enc, idx as u32, key as c_int, pts_us) };
        if r < 0 {
            bail!("видеокодер: {}", std::io::Error::from_raw_os_error(-r));
        }
        self.in_flight += 1;
        Ok(true)
    }

    fn poll(&mut self, wait_ms: i32, out: &mut Out) -> Result<()> {
        let mut wait = wait_ms;
        loop {
            let mut p: *const u8 = std::ptr::null();
            let (mut ts, mut key) = (0u64, 0 as c_int);
            // SAFETY: FFI кодера; данные действительны до следующего вызова
            let n = unsafe { venc_dequeue(self.enc, wait, &mut p, &mut ts, &mut key) };
            if n < 0 {
                bail!("видеокодер: {}", std::io::Error::from_raw_os_error(-n as i32));
            }
            if n == 0 {
                return Ok(());
            }
            wait = 0;
            let data = unsafe { std::slice::from_raw_parts(p, n as usize) };
            if has_vcl(data, self.hevc) {
                self.in_flight = self.in_flight.saturating_sub(1);
            }
            out(data, ts, key != 0)?;
        }
    }

    fn finish(&mut self, out: &mut Out) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.in_flight > 0 && Instant::now() < deadline {
            self.poll(50, out)?;
        }
        Ok(())
    }
}

impl Drop for Venc {
    fn drop(&mut self) {
        for (p, len) in self.maps.drain(..).flatten() {
            // SAFETY: отображения из encode
            unsafe { libc::munmap(p as *mut libc::c_void, len) };
        }
        // SAFETY: кодер из venc_open
        unsafe { venc_close(self.enc) };
    }
}

// ─── libavcodec (компьютер) ─────────────────────────────────────────────────

struct FfEnc {
    name: String,
    ctx: *mut ffi::AVCodecContext,
    frame: *mut ffi::AVFrame,
    pkt: *mut ffi::AVPacket,
    hw_frames: *mut ffi::AVBufferRef,
    hw_dev: *mut ffi::AVBufferRef,
}

// SAFETY: контексты FFmpeg используются одним потоком за раз (владение)
unsafe impl Send for FfEnc {}

fn set_opt(d: &mut *mut ffi::AVDictionary, k: &str, v: &str) {
    let (k, v) = (CString::new(k).unwrap(), CString::new(v).unwrap());
    // SAFETY: FFI FFmpeg
    unsafe { ffi::av_dict_set(d, k.as_ptr(), v.as_ptr(), 0) };
}

impl FfEnc {
    fn open(name: &str, vaapi: bool, w: u32, h: u32, bitrate: u32, fps: u32) -> Result<FfEnc> {
        let cname = CString::new(name)?;
        // SAFETY: FFI FFmpeg; при ошибке всё освобождает Drop
        unsafe {
            let codec = ffi::avcodec_find_encoder_by_name(cname.as_ptr());
            if codec.is_null() {
                bail!("{name}: нет в ffmpeg");
            }
            let mut e = FfEnc {
                name: name.to_string(),
                ctx: ffi::avcodec_alloc_context3(codec),
                frame: ffi::av_frame_alloc(),
                pkt: ffi::av_packet_alloc(),
                hw_frames: std::ptr::null_mut(),
                hw_dev: std::ptr::null_mut(),
            };
            let c = e.ctx;
            (*c).width = w as c_int;
            (*c).height = h as c_int;
            (*c).time_base = ffi::AVRational { num: 1, den: 1_000_000 };
            (*c).framerate = ffi::AVRational { num: fps.max(1) as c_int, den: 1 };
            (*c).gop_size = fps.max(1) as c_int;
            (*c).max_b_frames = 0;
            (*c).bit_rate = bitrate as i64;
            (*c).color_range = ffi::AVColorRange::AVCOL_RANGE_JPEG;
            (*c).pix_fmt = ffi::AVPixelFormat::AV_PIX_FMT_NV12;
            let mut opts: *mut ffi::AVDictionary = std::ptr::null_mut();
            if name.contains("nvenc") {
                set_opt(&mut opts, "preset", "p4");
                set_opt(&mut opts, "tune", "ll");
            } else if name.starts_with("libx26") {
                set_opt(&mut opts, "preset", "veryfast");
                set_opt(&mut opts, "tune", "zerolatency");
            }
            if vaapi {
                let mut last = String::from("нет /dev/dri/renderD*");
                for n in 128..136 {
                    let dev = CString::new(format!("/dev/dri/renderD{n}"))?;
                    if !std::path::Path::new(dev.to_str()?).exists() {
                        continue;
                    }
                    let r = ffi::av_hwdevice_ctx_create(&mut e.hw_dev, ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI, dev.as_ptr(), std::ptr::null_mut(), 0);
                    if r >= 0 {
                        break;
                    }
                    last = format!("{}: {}", dev.to_string_lossy(), averr(r));
                }
                if e.hw_dev.is_null() {
                    ffi::av_dict_free(&mut opts);
                    bail!("VAAPI: {last}");
                }
                e.hw_frames = ffi::av_hwframe_ctx_alloc(e.hw_dev);
                let fc = (*e.hw_frames).data as *mut ffi::AVHWFramesContext;
                (*fc).format = ffi::AVPixelFormat::AV_PIX_FMT_VAAPI;
                (*fc).sw_format = ffi::AVPixelFormat::AV_PIX_FMT_NV12;
                (*fc).width = w as c_int;
                (*fc).height = h as c_int;
                (*fc).initial_pool_size = 8;
                let r = ffi::av_hwframe_ctx_init(e.hw_frames);
                if r < 0 {
                    ffi::av_dict_free(&mut opts);
                    bail!("VAAPI кадры: {}", averr(r));
                }
                (*c).pix_fmt = ffi::AVPixelFormat::AV_PIX_FMT_VAAPI;
                (*c).hw_frames_ctx = ffi::av_buffer_ref(e.hw_frames);
            }
            let r = ffi::avcodec_open2(c, codec, &mut opts);
            ffi::av_dict_free(&mut opts);
            if r < 0 {
                bail!("{name}: {}", averr(r));
            }
            let f = e.frame;
            (*f).format = ffi::AVPixelFormat::AV_PIX_FMT_NV12 as c_int;
            (*f).width = w as c_int;
            (*f).height = h as c_int;
            (*f).color_range = ffi::AVColorRange::AVCOL_RANGE_JPEG;
            let r = ffi::av_frame_get_buffer(f, 32);
            if r < 0 {
                bail!("кадр кодера: {}", averr(r));
            }
            Ok(e)
        }
    }

    fn receive(&mut self, out: &mut Out) -> Result<()> {
        // SAFETY: FFI FFmpeg; пакет освобождается после каждой отдачи
        unsafe {
            loop {
                let r = ffi::avcodec_receive_packet(self.ctx, self.pkt);
                if r < 0 {
                    return Ok(());
                }
                let p = self.pkt;
                let data = std::slice::from_raw_parts((*p).data, (*p).size as usize);
                let res = out(data, (*p).pts.max(0) as u64, (*p).flags & ffi::AV_PKT_FLAG_KEY != 0);
                ffi::av_packet_unref(p);
                res?;
            }
        }
    }
}

impl VideoEncoder for FfEnc {
    fn name(&self) -> &str {
        &self.name
    }

    fn encode(&mut self, src: &Src, pts_us: u64, key: bool, out: &mut Out) -> Result<bool> {
        // SAFETY: FFI FFmpeg; кадр NV12 нашего размера
        unsafe {
            let f = self.frame;
            if ffi::av_frame_make_writable(f) < 0 {
                bail!("кадр кодера занят");
            }
            let (w, h) = (((*f).width as usize).min(src.width), ((*f).height as usize).min(src.height));
            let (ls0, ls1) = ((*f).linesize[0] as usize, (*f).linesize[1] as usize);
            let (y, uv) = ((*f).data[0], (*f).data[1]);
            let uv0 = src.stride * src.scanlines;
            for r in 0..h {
                std::ptr::copy_nonoverlapping(src.data[r * src.stride..].as_ptr(), y.add(r * ls0), w);
            }
            for r in 0..h / 2 {
                let s = &src.data[uv0 + r * src.stride..uv0 + r * src.stride + w];
                let d = std::slice::from_raw_parts_mut(uv.add(r * ls1), w);
                if src.vu {
                    for (dp, sp) in d.chunks_exact_mut(2).zip(s.chunks_exact(2)) {
                        dp[0] = sp[1];
                        dp[1] = sp[0];
                    }
                } else {
                    d.copy_from_slice(s);
                }
            }
            (*f).pts = pts_us as i64;
            (*f).pict_type = if key { ffi::AVPictureType::AV_PICTURE_TYPE_I } else { ffi::AVPictureType::AV_PICTURE_TYPE_NONE };
            let r = if self.hw_frames.is_null() {
                ffi::avcodec_send_frame(self.ctx, f)
            } else {
                let mut hw = ffi::av_frame_alloc();
                let mut r = ffi::av_hwframe_get_buffer(self.hw_frames, hw, 0);
                if r >= 0 {
                    r = ffi::av_hwframe_transfer_data(hw, f, 0);
                }
                if r >= 0 {
                    (*hw).pts = (*f).pts;
                    (*hw).pict_type = (*f).pict_type;
                    r = ffi::avcodec_send_frame(self.ctx, hw);
                }
                ffi::av_frame_free(&mut hw);
                r
            };
            if r < 0 && r != ffi::AVERROR(libc::EAGAIN) {
                return Err(anyhow!("{}: {}", self.name, averr(r)));
            }
            self.receive(out)?;
            Ok(r >= 0)
        }
    }

    fn poll(&mut self, _wait_ms: i32, out: &mut Out) -> Result<()> {
        self.receive(out)
    }

    fn finish(&mut self, out: &mut Out) -> Result<()> {
        // SAFETY: FFI FFmpeg: конец потока
        unsafe { ffi::avcodec_send_frame(self.ctx, std::ptr::null()) };
        self.receive(out)
    }
}

impl Drop for FfEnc {
    fn drop(&mut self) {
        // SAFETY: объекты из open (null допустим)
        unsafe {
            ffi::av_frame_free(&mut self.frame);
            ffi::av_packet_free(&mut self.pkt);
            ffi::avcodec_free_context(&mut self.ctx);
            ffi::av_buffer_unref(&mut self.hw_frames);
            ffi::av_buffer_unref(&mut self.hw_dev);
        }
    }
}
