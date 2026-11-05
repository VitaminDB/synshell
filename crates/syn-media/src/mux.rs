//! MP4 (libavformat): поток видео (H.264/HEVC Annex B от кодера) и звука (AAC); заголовок пишется по
//! первому пакету видео (параметры SPS/PPS — extradata), звук до него копится.

use std::ffi::{c_char, c_int, CString};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Mutex;

use anyhow::{bail, Result};
use ffmpeg_next::ffi;

use crate::recorder::Shared;

pub(crate) fn averr(r: c_int) -> String {
    let mut buf = [0 as c_char; 128];
    // SAFETY: буфер достаточной длины
    unsafe {
        ffi::av_strerror(r, buf.as_mut_ptr(), buf.len());
        std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
    }
}

/// MP4: поток видео и, возможно, звука; заголовок пишется по первому пакету видео (SPS/PPS — extradata).
pub(crate) struct Mux {
    ctx: *mut ffi::AVFormatContext,
    pub(crate) vst: c_int,
    pub(crate) ast: c_int,
    pub(crate) header: bool,
    /// Пакеты звука до заголовка: (данные, pts в отсчётах).
    pub(crate) pending: Vec<(Vec<u8>, i64, i64)>,
    audio_tb: ffi::AVRational,
    last_vdts: i64,
    finished: bool,
}

// SAFETY: доступ только под Mutex
unsafe impl Send for Mux {}

impl Mux {
    pub(crate) fn new(path: &Path, w: u32, h: u32, hevc: bool, rot: u32) -> Result<Mux> {
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
                bail!(syngui::t!("mp4: поток видео"));
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
            if !rot.is_multiple_of(360) {
                // матрица отображения (tkhd)
                let sd = ffi::av_packet_side_data_new(
                    &mut (*p).coded_side_data,
                    &mut (*p).nb_coded_side_data,
                    ffi::AVPacketSideDataType::AV_PKT_DATA_DISPLAYMATRIX,
                    9 * 4,
                    0,
                );
                if !sd.is_null() {
                    // set — по часовой (get отдаёт против часовой)
                    ffi::av_display_rotation_set((*sd).data as *mut i32, rot as f64);
                }
            }
            m.vst = (*st).index;
            let r = ffi::avio_open(&mut (*ctx).pb, cpath.as_ptr(), ffi::AVIO_FLAG_WRITE);
            if r < 0 {
                bail!("{}: {}", path.display(), averr(r));
            }
            Ok(m)
        }
    }

    /// Поток звука по открытому кодеру.
    pub(crate) fn add_audio(&mut self, enc: *mut ffi::AVCodecContext) -> Result<()> {
        // SAFETY: FFI FFmpeg, enc открыт
        unsafe {
            let st = ffi::avformat_new_stream(self.ctx, std::ptr::null());
            if st.is_null() {
                bail!(syngui::t!("mp4: поток звука"));
            }
            let r = ffi::avcodec_parameters_from_context((*st).codecpar, enc);
            if r < 0 {
                bail!(syngui::t!("mp4: параметры звука: {err}", err = averr(r)));
            }
            (*st).time_base = (*enc).time_base;
            self.audio_tb = (*enc).time_base;
            self.ast = (*st).index;
        }
        Ok(())
    }

    pub(crate) fn write(&mut self, stream: c_int, data: &[u8], pts: i64, tb: ffi::AVRational, key: bool, duration: i64) -> Result<()> {
        // SAFETY: FFI FFmpeg; пакет освобождается здесь же
        unsafe {
            let pkt = ffi::av_packet_alloc();
            if ffi::av_new_packet(pkt, data.len() as c_int) < 0 {
                ffi::av_packet_free(&mut (pkt as *mut _));
                bail!(syngui::t!("пакет"));
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
                bail!(syngui::t!("запись mp4: {err}", err = averr(r)));
            }
        }
        Ok(())
    }

    /// Первый пакет видео: extradata (параметры до первого кадра), заголовок, отложенный звук.
    pub(crate) fn start(&mut self, first: &[u8], hevc: bool) -> Result<()> {
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
                bail!(syngui::t!("заголовок mp4: {err}", err = averr(r)));
            }
        }
        self.header = true;
        let tb = self.audio_tb;
        for (d, pts, dur) in std::mem::take(&mut self.pending) {
            self.write(self.ast, &d, pts, tb, true, dur)?;
        }
        Ok(())
    }

    pub(crate) fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        // SAFETY: FFI FFmpeg
        unsafe {
            if self.header {
                let r = ffi::av_write_trailer(self.ctx);
                if r < 0 {
                    bail!(syngui::t!("завершение mp4: {err}", err = averr(r)));
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

/// Пакет потока из буферов кодера: параметры (SPS/PPS) могут прийти отдельным буфером — копятся до
/// первого буфера с кадром (VCL NAL).
pub(crate) fn has_vcl(data: &[u8], hevc: bool) -> bool {
    let mut i = 0;
    while i + 3 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            let h = data[i + 3];
            if if hevc { ((h >> 1) & 0x3F) < 32 } else { (1..=5).contains(&(h & 0x1F)) } {
                return true;
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    false
}

/// Пакеты кодера → MP4: параметры (SPS/PPS) могут прийти отдельным буфером — копятся до буфера с кадром.
pub(crate) struct Writer<'a> {
    pub mux: &'a Mutex<Mux>,
    pub sh: &'a Shared,
    pub hevc: bool,
    pub frame_us: i64,
    pub started: bool,
    pending: Vec<u8>,
    pending_key: bool,
}

impl<'a> Writer<'a> {
    pub fn new(mux: &'a Mutex<Mux>, sh: &'a Shared, hevc: bool, frame_us: i64) -> Self {
        Writer { mux, sh, hevc, frame_us, started: false, pending: Vec::new(), pending_key: false }
    }

    /// Буфер выхода кодера → MP4.
    pub fn put(&mut self, data: &[u8], ts_us: u64, key: bool) -> Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        self.pending.extend_from_slice(data);
        self.pending_key |= key;
        if !has_vcl(&self.pending, self.hevc) {
            return Ok(());
        }
        let pkt = std::mem::take(&mut self.pending);
        let key = std::mem::take(&mut self.pending_key);
        let mut m = self.mux.lock().unwrap();
        if !self.started {
            m.start(&pkt, self.hevc)?;
            self.started = true;
        }
        let vst = m.vst;
        m.write(vst, &pkt, ts_us as i64, ffi::AVRational { num: 1, den: 1_000_000 }, key, self.frame_us)?;
        drop(m);
        self.sh.duration_us.fetch_max(ts_us, Ordering::Relaxed);
        self.sh.bytes.fetch_add(pkt.len() as u64, Ordering::Relaxed);
        Ok(())
    }
}
