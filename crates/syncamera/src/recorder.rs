//! Запись видео: кадры камеры → NV12 с поворотом → аппаратный кодер V4L2 (`venc.c`, H.264/HEVC) → MP4
//! (libavformat); звук — микрофон (cpal → PipeWire) → AAC (кодер FFmpeg).
//!
//! Кадр камеры копируется в буфер кодера в потоке кодера и сразу возвращается syncamd. Метки времени
//! видео — время экспозиции кадров (CLOCK_BOOTTIME), паузы вырезаются; звук идёт непрерывным счётчиком
//! отсчётов от первого кадра (паузы отбрасывают его отсчёты). Таймлапс — каждый N-й кадр с шагом 1/fps,
//! без звука.

use std::collections::VecDeque;
use std::ffi::{c_char, c_int, c_long, CString};
use std::os::fd::RawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use ffmpeg_next::ffi;

use crate::convert::{self, Src};
use crate::proto::{self, Stream};

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
struct Venc {
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
    ) -> *mut Venc;
    fn venc_close(e: *mut Venc);
    fn venc_in_fd(e: *mut Venc, i: u32) -> c_int;
    fn venc_free_input(e: *mut Venc) -> c_int;
    fn venc_encode(e: *mut Venc, i: u32, force_key: c_int, ts_us: u64, timeout_ms: c_int, out: *mut *const u8, key: *mut c_int) -> c_long;
}

/// Параметры записи.
#[derive(Clone, Debug)]
pub struct Settings {
    pub path: PathBuf,
    /// Поворот кадра по часовой (градусы) — видео пишется уже повёрнутым.
    pub rot: u32,
    pub fps: u32,
    pub bitrate: u32,
    pub hevc: bool,
    pub audio: bool,
    /// Таймлапс: брать каждый N-й кадр (1 — обычная запись).
    pub timelapse: u32,
}

/// Общее состояние записи (поток кодера ↔ интерфейс).
#[derive(Default)]
pub struct Shared {
    pub paused: AtomicBool,
    resumed: AtomicBool,
    /// Длительность записанного, мкс.
    pub duration_us: AtomicU64,
    pub bytes: AtomicU64,
    pub error: Mutex<Option<String>>,
}

enum Job {
    Frame(proto::FrameRef),
    Stop,
}

pub struct Recorder {
    tx: SyncSender<Job>,
    pub shared: Arc<Shared>,
    thread: Option<JoinHandle<Result<PathBuf>>>,
    every: u32,
    count: u32,
}

struct SendPtr<T>(*mut T);
// SAFETY: кодер и контекст FFmpeg используются из одного потока за раз (Mutex / владение потоком)
unsafe impl<T> Send for SendPtr<T> {}

fn averr(r: c_int) -> String {
    let mut buf = [0 as c_char; 128];
    // SAFETY: буфер достаточной длины
    unsafe {
        ffi::av_strerror(r, buf.as_mut_ptr(), buf.len());
        std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
    }
}

/// MP4: поток видео и, возможно, звука; заголовок пишется по первому пакету видео (SPS/PPS — extradata).
struct Mux {
    ctx: *mut ffi::AVFormatContext,
    vst: c_int,
    ast: c_int,
    header: bool,
    /// Пакеты звука до заголовка: (данные, pts в отсчётах).
    pending: Vec<(Vec<u8>, i64, i64)>,
    audio_tb: ffi::AVRational,
    last_vdts: i64,
    finished: bool,
}

// SAFETY: доступ только под Mutex
unsafe impl Send for Mux {}

impl Mux {
    fn new(path: &Path, w: u32, h: u32, hevc: bool) -> Result<Mux> {
        let cpath = CString::new(path.to_string_lossy().as_bytes())?;
        let mut ctx: *mut ffi::AVFormatContext = std::ptr::null_mut();
        // SAFETY: FFI FFmpeg; ошибки проверяются, ресурсы освобождает Drop
        unsafe {
            let r = ffi::avformat_alloc_output_context2(&mut ctx, std::ptr::null(), c"mp4".as_ptr(), cpath.as_ptr());
            if r < 0 || ctx.is_null() {
                bail!("mp4: {}", averr(r));
            }
            let mut m = Mux { ctx, vst: -1, ast: -1, header: false, pending: Vec::new(), audio_tb: ffi::AVRational { num: 1, den: 48000 }, last_vdts: -1, finished: false };
            let st = ffi::avformat_new_stream(ctx, std::ptr::null());
            if st.is_null() {
                bail!("mp4: поток видео");
            }
            let p = (*st).codecpar;
            (*p).codec_type = ffi::AVMediaType::AVMEDIA_TYPE_VIDEO;
            (*p).codec_id = if hevc { ffi::AVCodecID::AV_CODEC_ID_HEVC } else { ffi::AVCodecID::AV_CODEC_ID_H264 };
            if hevc {
                (*p).codec_tag = u32::from_le_bytes(*b"hvc1");
            }
            (*p).width = w as c_int;
            (*p).height = h as c_int;
            (*p).format = ffi::AVPixelFormat::AV_PIX_FMT_YUVJ420P as c_int;
            (*p).color_range = ffi::AVColorRange::AVCOL_RANGE_JPEG;
            (*st).time_base = ffi::AVRational { num: 1, den: 90000 };
            m.vst = (*st).index;
            let r = ffi::avio_open(&mut (*ctx).pb, cpath.as_ptr(), ffi::AVIO_FLAG_WRITE);
            if r < 0 {
                bail!("{}: {}", path.display(), averr(r));
            }
            Ok(m)
        }
    }

    /// Поток звука по открытому кодеру.
    fn add_audio(&mut self, enc: *mut ffi::AVCodecContext) -> Result<()> {
        // SAFETY: FFI FFmpeg, enc открыт
        unsafe {
            let st = ffi::avformat_new_stream(self.ctx, std::ptr::null());
            if st.is_null() {
                bail!("mp4: поток звука");
            }
            let r = ffi::avcodec_parameters_from_context((*st).codecpar, enc);
            if r < 0 {
                bail!("mp4: параметры звука: {}", averr(r));
            }
            (*st).time_base = (*enc).time_base;
            self.audio_tb = (*enc).time_base;
            self.ast = (*st).index;
        }
        Ok(())
    }

    fn write(&mut self, stream: c_int, data: &[u8], pts: i64, tb: ffi::AVRational, key: bool, duration: i64) -> Result<()> {
        // SAFETY: FFI FFmpeg; пакет освобождается здесь же
        unsafe {
            let pkt = ffi::av_packet_alloc();
            if ffi::av_new_packet(pkt, data.len() as c_int) < 0 {
                ffi::av_packet_free(&mut (pkt as *mut _));
                bail!("пакет");
            }
            std::ptr::copy_nonoverlapping(data.as_ptr(), (*pkt).data, data.len());
            let st = *(*self.ctx).streams.add(stream as usize);
            let stb = (*st).time_base;
            (*pkt).pts = ffi::av_rescale_q(pts, tb, stb);
            (*pkt).dts = (*pkt).pts;
            (*pkt).duration = ffi::av_rescale_q(duration, tb, stb);
            (*pkt).stream_index = stream;
            if key {
                (*pkt).flags |= ffi::AV_PKT_FLAG_KEY;
            }
            if stream == self.vst {
                if (*pkt).dts <= self.last_vdts {
                    (*pkt).dts = self.last_vdts + 1;
                    (*pkt).pts = (*pkt).dts;
                }
                self.last_vdts = (*pkt).dts;
            }
            let mut p = pkt;
            let r = ffi::av_interleaved_write_frame(self.ctx, p);
            ffi::av_packet_free(&mut p);
            if r < 0 {
                bail!("запись mp4: {}", averr(r));
            }
        }
        Ok(())
    }

    /// Первый пакет видео: extradata (параметры до первого кадра), заголовок, отложенный звук.
    fn start(&mut self, first: &[u8], hevc: bool) -> Result<()> {
        let extra = params_prefix(first, hevc);
        // SAFETY: FFI FFmpeg; extradata выделяется av_malloc с отступом
        unsafe {
            let st = *(*self.ctx).streams.add(self.vst as usize);
            let p = (*st).codecpar;
            if !extra.is_empty() {
                let pad = ffi::AV_INPUT_BUFFER_PADDING_SIZE as usize;
                let b = ffi::av_mallocz(extra.len() + pad) as *mut u8;
                std::ptr::copy_nonoverlapping(extra.as_ptr(), b, extra.len());
                (*p).extradata = b;
                (*p).extradata_size = extra.len() as c_int;
            }
            let r = ffi::avformat_write_header(self.ctx, std::ptr::null_mut());
            if r < 0 {
                bail!("заголовок mp4: {}", averr(r));
            }
        }
        self.header = true;
        let tb = self.audio_tb;
        for (d, pts, dur) in std::mem::take(&mut self.pending) {
            self.write(self.ast, &d, pts, tb, true, dur)?;
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        // SAFETY: FFI FFmpeg
        unsafe {
            if self.header {
                let r = ffi::av_write_trailer(self.ctx);
                if r < 0 {
                    bail!("завершение mp4: {}", averr(r));
                }
            }
        }
        Ok(())
    }
}

impl Drop for Mux {
    fn drop(&mut self) {
        // SAFETY: контекст из new
        unsafe {
            if !(*self.ctx).pb.is_null() {
                ffi::avio_closep(&mut (*self.ctx).pb);
            }
            ffi::avformat_free_context(self.ctx);
        }
    }
}

/// Начало потока до первого кадра (VCL NAL): VPS/SPS/PPS в Annex B — extradata для MP4.
fn params_prefix(data: &[u8], hevc: bool) -> Vec<u8> {
    let mut i = 0;
    while i + 3 < data.len() {
        let sc = if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            3
        } else if i + 4 < data.len() && data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 0 && data[i + 3] == 1 {
            4
        } else {
            i += 1;
            continue;
        };
        let h = data[i + sc];
        let vcl = if hevc { ((h >> 1) & 0x3F) < 32 } else { (1..=5).contains(&(h & 0x1F)) };
        if vcl {
            return data[..i].to_vec();
        }
        i += sc;
    }
    Vec::new()
}

fn boottime_ns() -> u64 {
    let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: запись в живую структуру
    unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut t) };
    t.tv_sec as u64 * 1_000_000_000 + t.tv_nsec as u64
}

impl Recorder {
    /// Начать запись кадров потока `stream`.
    pub fn start(stream: Arc<Stream>, s: Settings) -> Result<Recorder> {
        let info = stream.info;
        let (w, h) = convert::out_size(info.width as usize, info.height as usize, s.rot, 1);
        let (w, h) = (w as u32 & !1, h as u32 & !1);
        let mut vi = VencInfo::default();
        let mut err = [0 as c_char; 160];
        // SAFETY: FFI кодера; указатель живёт в потоке кодера и закрывается им
        let enc = unsafe { venc_open(std::ptr::null(), w, h, s.hevc as c_int, s.bitrate, s.fps, s.fps.max(1), &mut vi, err.as_mut_ptr(), err.len() as c_int) };
        if enc.is_null() {
            let e = unsafe { std::ffi::CStr::from_ptr(err.as_ptr()) }.to_string_lossy().into_owned();
            bail!("видеокодер: {e}");
        }
        let mux = match Mux::new(&s.path, w, h, s.hevc) {
            Ok(m) => m,
            Err(e) => {
                unsafe { venc_close(enc) };
                return Err(e);
            }
        };
        let mux = Arc::new(Mutex::new(mux));
        let shared = Arc::new(Shared::default());
        let t0 = Arc::new(AtomicU64::new(0));
        let audio = if s.audio && s.timelapse <= 1 {
            match Audio::start(mux.clone(), shared.clone(), t0.clone()) {
                Ok(a) => Some(a),
                Err(e) => {
                    tracing::warn!("звук для видео: {e:#}");
                    None
                }
            }
        } else {
            None
        };
        let (tx, rx) = sync_channel::<Job>(3);
        let sh = shared.clone();
        let enc = SendPtr(enc);
        let every = s.timelapse.max(1);
        let thread = std::thread::Builder::new()
            .name("video-enc".into())
            .spawn(move || {
                let enc = enc;
                let r = encode_loop(enc.0, vi, &stream, rx, &mux, &sh, &s, w, h, &t0, audio);
                // SAFETY: кодер из venc_open
                unsafe { venc_close(enc.0) };
                if let Err(e) = &r {
                    *sh.error.lock().unwrap() = Some(format!("{e:#}"));
                    let _ = std::fs::remove_file(&s.path);
                }
                r.map(|_| s.path.clone())
            })?;
        Ok(Recorder { tx, shared, thread: Some(thread), every, count: 0 })
    }

    /// Кадр для записи. false — не взят (пауза, очередь полна, пропуск таймлапса): буфер вернуть самому.
    pub fn push(&mut self, f: proto::FrameRef) -> bool {
        if self.shared.paused.load(Ordering::Relaxed) {
            return false;
        }
        self.count += 1;
        if (self.count - 1) % self.every != 0 {
            return false;
        }
        match self.tx.try_send(Job::Frame(f)) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => false,
        }
    }

    pub fn set_paused(&self, on: bool) {
        self.shared.paused.store(on, Ordering::SeqCst);
        if !on {
            self.shared.resumed.store(true, Ordering::SeqCst);
        }
    }

    pub fn failed(&self) -> Option<String> {
        self.shared.error.lock().unwrap().clone()
    }

    /// Остановить и дописать файл; путь готового видео.
    pub fn stop(mut self) -> Result<PathBuf> {
        let _ = self.tx.send(Job::Stop);
        match self.thread.take().map(|t| t.join()) {
            Some(Ok(r)) => r,
            _ => Err(anyhow!("поток записи упал")),
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if let Some(t) = self.thread.take() {
            let _ = self.tx.send(Job::Stop);
            let _ = t.join();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn encode_loop(
    enc: *mut Venc,
    vi: VencInfo,
    stream: &Stream,
    rx: Receiver<Job>,
    mux: &Mutex<Mux>,
    sh: &Shared,
    s: &Settings,
    w: u32,
    h: u32,
    t0: &AtomicU64,
    audio: Option<Audio>,
) -> Result<()> {
    let info = stream.info;
    let mut maps: Vec<Option<(*mut u8, usize)>> = vec![None; vi.in_count as usize];
    let frame_ns = 1_000_000_000 / s.fps.max(1) as u64;
    let (mut first_ts, mut last_ts, mut paused_ns, mut n) = (0u64, 0u64, 0u64, 0u64);
    let mut started = false;
    let mut result = Ok(());
    while let Ok(Job::Frame(f)) = rx.recv() {
        // SAFETY: FFI кодера
        let idx = unsafe { venc_free_input(enc) };
        if idx < 0 {
            stream.release(f.buf);
            continue;
        }
        let idx = idx as usize;
        let fd: RawFd = unsafe { venc_in_fd(enc, idx as u32) };
        if maps[idx].is_none() {
            // SAFETY: отображение буфера dma-heap кодера на запись
            let p = unsafe { libc::mmap(std::ptr::null_mut(), vi.in_size as usize, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd, 0) };
            if p == libc::MAP_FAILED {
                stream.release(f.buf);
                result = Err(anyhow!("mmap входа кодера: {}", std::io::Error::last_os_error()));
                break;
            }
            maps[idx] = Some((p as *mut u8, vi.in_size as usize));
        }
        let (p, len) = maps[idx].unwrap();
        // SAFETY: отображение живо до конца функции
        let dst = unsafe { std::slice::from_raw_parts_mut(p, len) };
        stream.begin(f.buf);
        let src = Src {
            data: stream.data(f.buf),
            width: info.width as usize,
            height: info.height as usize,
            stride: info.stride as usize,
            scanlines: info.scanlines as usize,
            vu: info.format == proto::FMT_NV21,
        };
        proto::dma_sync(fd, true, true);
        convert::to_nv12(&src, s.rot, dst, vi.stride as usize, vi.y_scanlines as usize);
        proto::dma_sync(fd, false, true);
        stream.end(f.buf);
        stream.release(f.buf);
        // метка времени: время экспозиции без пауз; таймлапс — ровный шаг
        if !started {
            first_ts = f.ts_ns;
            last_ts = f.ts_ns;
            t0.store(boottime_ns().max(1), Ordering::SeqCst);
        }
        if sh.resumed.swap(false, Ordering::SeqCst) && f.ts_ns > last_ts + frame_ns {
            paused_ns += f.ts_ns - last_ts - frame_ns;
        }
        let pts_us = if s.timelapse > 1 { n * frame_ns / 1000 } else { f.ts_ns.saturating_sub(first_ts + paused_ns) / 1000 };
        last_ts = f.ts_ns;
        let mut out: *const u8 = std::ptr::null();
        let mut key: c_int = 0;
        // SAFETY: FFI кодера; пакет действителен до следующего вызова
        let len = unsafe { venc_encode(enc, idx as u32, (!started) as c_int, pts_us, 2000, &mut out, &mut key) };
        if len < 0 {
            result = Err(anyhow!("видеокодер: {}", std::io::Error::from_raw_os_error(-len as i32)));
            break;
        }
        if len == 0 {
            continue;
        }
        let pkt = unsafe { std::slice::from_raw_parts(out, len as usize) };
        let mut m = mux.lock().unwrap();
        if !started {
            if let Err(e) = m.start(pkt, s.hevc) {
                result = Err(e);
                break;
            }
            started = true;
        }
        let us = ffi::AVRational { num: 1, den: 1_000_000 };
        let vst = m.vst;
        if let Err(e) = m.write(vst, pkt, pts_us as i64, us, key != 0, (frame_ns / 1000) as i64) {
            result = Err(e);
            break;
        }
        drop(m);
        n += 1;
        sh.duration_us.store(pts_us, Ordering::Relaxed);
        sh.bytes.fetch_add(len as u64, Ordering::Relaxed);
    }
    // кадры, оставшиеся в очереди, — вернуть
    while let Ok(j) = rx.try_recv() {
        if let Job::Frame(f) = j {
            stream.release(f.buf);
        }
    }
    for (p, len) in maps.into_iter().flatten() {
        // SAFETY: отображения выше
        unsafe { libc::munmap(p as *mut libc::c_void, len) };
    }
    if let Some(a) = audio {
        a.stop();
    }
    let mut m = mux.lock().unwrap();
    if !started && result.is_ok() {
        result = Err(anyhow!("не записано ни одного кадра"));
    }
    let fin = m.finish();
    let _ = (w, h);
    result.and(fin)
}

// ─── Звук ───────────────────────────────────────────────────────────────────

struct Audio {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl Audio {
    fn start(mux: Arc<Mutex<Mux>>, sh: Arc<Shared>, t0: Arc<AtomicU64>) -> Result<Audio> {
        use cpal::traits::{DeviceTrait, HostTrait};
        let host = cpal::default_host();
        let dev = host.default_input_device().context("нет микрофона")?;
        let def = dev.default_input_config().context("настройки микрофона")?;
        let rate = 48000u32;
        let channels = def.channels().clamp(1, 2);
        // кодер AAC и поток mp4 — до заголовка
        let enc = open_aac(rate, channels)?;
        if let Err(e) = mux.lock().unwrap().add_audio(enc) {
            // SAFETY: кодер из open_aac
            unsafe { ffi::avcodec_free_context(&mut (enc as *mut _)) };
            return Err(e);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let st = stop.clone();
        let enc = SendPtr(enc);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();
        let thread = std::thread::Builder::new().name("video-audio".into()).spawn(move || {
            let enc = enc;
            let buf: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::new()));
            let b2 = buf.clone();
            let cfg = cpal::StreamConfig { channels, sample_rate: cpal::SampleRate(rate), buffer_size: cpal::BufferSize::Default };
            let err_fn = |e| tracing::warn!("микрофон: {e}");
            let stream = match def.sample_format() {
                cpal::SampleFormat::I16 => dev.build_input_stream(&cfg, move |d: &[i16], _: &_| b2.lock().unwrap().extend(d.iter().map(|&v| v as f32 / 32768.0)), err_fn, None),
                _ => dev.build_input_stream(&cfg, move |d: &[f32], _: &_| b2.lock().unwrap().extend(d.iter().copied()), err_fn, None),
            };
            let stream = match stream {
                Ok(s) => s,
                Err(e) => {
                    let _ = ready_tx.send(Err(anyhow!("микрофон: {e}")));
                    unsafe { ffi::avcodec_free_context(&mut (enc.0 as *mut _)) };
                    return;
                }
            };
            use cpal::traits::StreamTrait;
            if let Err(e) = stream.play() {
                let _ = ready_tx.send(Err(anyhow!("микрофон: {e}")));
                unsafe { ffi::avcodec_free_context(&mut (enc.0 as *mut _)) };
                return;
            }
            let _ = ready_tx.send(Ok(()));
            audio_loop(enc.0, &buf, &mux, &sh, &t0, &st, channels as usize);
            drop(stream);
            unsafe { ffi::avcodec_free_context(&mut (enc.0 as *mut _)) };
        })?;
        match ready_rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(())) => Ok(Audio { stop, thread }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                stop.store(true, Ordering::SeqCst);
                Err(anyhow!("микрофон не отвечает"))
            }
        }
    }

    fn stop(self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.thread.join();
    }
}

fn open_aac(rate: u32, channels: u16) -> Result<*mut ffi::AVCodecContext> {
    // SAFETY: FFI FFmpeg; при ошибке контекст освобождается
    unsafe {
        let codec = ffi::avcodec_find_encoder(ffi::AVCodecID::AV_CODEC_ID_AAC);
        if codec.is_null() {
            bail!("нет кодера AAC");
        }
        let c = ffi::avcodec_alloc_context3(codec);
        (*c).sample_rate = rate as c_int;
        (*c).sample_fmt = ffi::AVSampleFormat::AV_SAMPLE_FMT_FLTP;
        ffi::av_channel_layout_default(&mut (*c).ch_layout, channels as c_int);
        (*c).bit_rate = if channels > 1 { 192_000 } else { 128_000 };
        (*c).time_base = ffi::AVRational { num: 1, den: rate as c_int };
        (*c).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as c_int;
        let r = ffi::avcodec_open2(c, codec, std::ptr::null_mut());
        if r < 0 {
            let mut c = c;
            ffi::avcodec_free_context(&mut c);
            bail!("кодер AAC: {}", averr(r));
        }
        Ok(c)
    }
}

fn audio_loop(enc: *mut ffi::AVCodecContext, buf: &Mutex<VecDeque<f32>>, mux: &Mutex<Mux>, sh: &Shared, t0: &AtomicU64, stop: &AtomicBool, ch: usize) {
    // SAFETY: FFI FFmpeg; кадр и пакет освобождаются в конце
    unsafe {
        let fs = ((*enc).frame_size.max(1024)) as usize;
        let tb = (*enc).time_base;
        let frame = ffi::av_frame_alloc();
        (*frame).nb_samples = fs as c_int;
        (*frame).format = ffi::AVSampleFormat::AV_SAMPLE_FMT_FLTP as c_int;
        ffi::av_channel_layout_copy(&mut (*frame).ch_layout, &(*enc).ch_layout);
        (*frame).sample_rate = (*enc).sample_rate;
        ffi::av_frame_get_buffer(frame, 0);
        let pkt = ffi::av_packet_alloc();
        let mut pts: i64 = 0;
        let drain = |pkt: *mut ffi::AVPacket| {
            while ffi::avcodec_receive_packet(enc, pkt) >= 0 {
                let d = std::slice::from_raw_parts((*pkt).data, (*pkt).size as usize);
                let mut m = mux.lock().unwrap();
                if m.header {
                    let ast = m.ast;
                    if let Err(e) = m.write(ast, d, (*pkt).pts, tb, true, (*pkt).duration) {
                        tracing::warn!("звук: {e:#}");
                    }
                } else {
                    m.pending.push((d.to_vec(), (*pkt).pts, (*pkt).duration));
                }
                drop(m);
                ffi::av_packet_unref(pkt);
            }
        };
        loop {
            let stopping = stop.load(Ordering::SeqCst);
            // до первого кадра видео и на паузе — отсчёты выбрасываются
            let take: Option<Vec<f32>> = {
                let mut b = buf.lock().unwrap();
                if t0.load(Ordering::SeqCst) == 0 || sh.paused.load(Ordering::Relaxed) {
                    b.clear();
                    None
                } else if b.len() >= fs * ch {
                    Some(b.drain(..fs * ch).collect())
                } else {
                    None
                }
            };
            match take {
                Some(s) => {
                    ffi::av_frame_make_writable(frame);
                    for c in 0..ch {
                        let plane = std::slice::from_raw_parts_mut((*frame).data[c] as *mut f32, fs);
                        for (i, v) in plane.iter_mut().enumerate() {
                            *v = s[i * ch + c];
                        }
                    }
                    (*frame).pts = pts;
                    pts += fs as i64;
                    if ffi::avcodec_send_frame(enc, frame) >= 0 {
                        drain(pkt);
                    }
                }
                None if stopping => break,
                None => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        ffi::avcodec_send_frame(enc, std::ptr::null());
        drain(pkt);
        let mut f = frame;
        ffi::av_frame_free(&mut f);
        let mut p = pkt;
        ffi::av_packet_free(&mut p);
    }
}
