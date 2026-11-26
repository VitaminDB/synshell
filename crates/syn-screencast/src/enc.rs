use std::ffi::{c_char, c_int, c_void, CString};
use std::path::Path;
use std::ptr::null_mut;

use anyhow::{bail, Result};
use ffmpeg_next::ffi;

pub fn averr(r: c_int) -> String {
    let mut buf = [0 as c_char; 160];
    unsafe {
        ffi::av_strerror(r, buf.as_mut_ptr(), buf.len());
        std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Nvenc,
    Vaapi,
    X264,
}

impl Kind {
    pub fn order(pref: &str) -> Vec<Kind> {
        let all = [Kind::Nvenc, Kind::Vaapi, Kind::X264];
        let first = match pref {
            "nvenc" => Some(Kind::Nvenc),
            "vaapi" => Some(Kind::Vaapi),
            "x264" | "software" | "libx264" => Some(Kind::X264),
            _ => None,
        };
        match first {
            Some(f) => std::iter::once(f).chain(all.into_iter().filter(|k| *k != f)).collect(),
            None => all.to_vec(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct DrmObj {
    fd: c_int,
    size: usize,
    modifier: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct DrmPlane {
    object_index: c_int,
    offset: isize,
    pitch: isize,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct DrmLayer {
    format: u32,
    nb_planes: c_int,
    planes: [DrmPlane; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct DrmDesc {
    nb_objects: c_int,
    objects: [DrmObj; 4],
    nb_layers: c_int,
    layers: [DrmLayer; 4],
}

pub struct Src {
    pub fd: c_int,
    pub size: usize,
    pub modifier: Option<u64>,
    pub map: *mut u8,
    pub stride: u32,
    pub offset: u32,
    pub full: (u32, u32),
    pub crop: [u32; 4],
    pub release: Box<dyn FnOnce() + Send>,
}

struct Release(Option<Box<dyn FnOnce() + Send>>, #[allow(dead_code)] Option<Box<DrmDesc>>);

extern "C" fn release_cb(opaque: *mut c_void, _data: *mut u8) {
    let mut r = unsafe { Box::from_raw(opaque as *mut Release) };
    if let Some(f) = r.0.take() {
        f();
    }
}

fn wrap(data: *mut u8, size: usize, rel: Release) -> *mut ffi::AVBufferRef {
    let opaque = Box::into_raw(Box::new(rel)) as *mut c_void;
    let b = unsafe { ffi::av_buffer_create(data, size, Some(release_cb), opaque, ffi::AV_BUFFER_FLAG_READONLY as c_int) };
    if b.is_null() {
        release_cb(opaque, null_mut());
    }
    b
}

fn quality(kind: Kind, q: &str) -> i64 {
    let i = match q {
        "low" | "small" => 2,
        "medium" | "normal" => 1,
        _ => 0,
    };
    match kind {
        Kind::Nvenc => [21, 25, 30][i],
        Kind::Vaapi => [21, 25, 30][i],
        Kind::X264 => [19, 23, 28][i],
    }
}

pub struct Video {
    pub kind: Kind,
    pub name: String,
    pub ctx: *mut ffi::AVCodecContext,
    pub w: u32,
    pub h: u32,
    graph: *mut ffi::AVFilterGraph,
    bsrc: *mut ffi::AVFilterContext,
    bsink: *mut ffi::AVFilterContext,
    drm_frames: *mut ffi::AVBufferRef,
    sws: *mut ffi::SwsContext,
    yuv: *mut ffi::AVFrame,
    pkt: *mut ffi::AVPacket,
}

unsafe impl Send for Video {}

fn set(obj: *mut c_void, k: &str, v: &str) {
    let (k, v) = (CString::new(k).unwrap(), CString::new(v).unwrap());
    unsafe { ffi::av_opt_set(obj, k.as_ptr(), v.as_ptr(), ffi::AV_OPT_SEARCH_CHILDREN as c_int) };
}

pub struct VideoOpts<'a> {
    pub w: u32,
    pub h: u32,
    pub full: (u32, u32),
    pub fps: u32,
    pub hevc: bool,
    pub quality: &'a str,
    pub dmabuf: bool,
    pub drm_node: &'a str,
}

impl Video {
    pub fn open(kind: Kind, o: &VideoOpts) -> Result<Video> {
        if kind == Kind::Vaapi && !o.dmabuf {
            bail!("VAAPI: нет DMA-BUF");
        }
        if kind == Kind::Nvenc && !Path::new("/dev/nvidiactl").exists() {
            bail!("NVENC: нет /dev/nvidiactl");
        }
        let name = match (kind, o.hevc) {
            (Kind::Nvenc, false) => "h264_nvenc",
            (Kind::Nvenc, true) => "hevc_nvenc",
            (Kind::Vaapi, false) => "h264_vaapi",
            (Kind::Vaapi, true) => "hevc_vaapi",
            (Kind::X264, false) => "libx264",
            (Kind::X264, true) => "libx265",
        };
        let cname = CString::new(name)?;
        let mut v = Video { kind, name: name.into(), ctx: null_mut(), w: o.w, h: o.h, graph: null_mut(), bsrc: null_mut(), bsink: null_mut(), drm_frames: null_mut(), sws: null_mut(), yuv: null_mut(), pkt: null_mut() };
        unsafe {
            let codec = ffi::avcodec_find_encoder_by_name(cname.as_ptr());
            if codec.is_null() {
                bail!("нет кодера {name}");
            }
            v.ctx = ffi::avcodec_alloc_context3(codec);
            let c = v.ctx;
            (*c).width = o.w as c_int;
            (*c).height = o.h as c_int;
            (*c).time_base = ffi::AVRational { num: 1, den: o.fps as c_int };
            (*c).framerate = ffi::AVRational { num: o.fps as c_int, den: 1 };
            (*c).gop_size = (o.fps * 2) as c_int;
            (*c).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as c_int;
            (*c).color_range = ffi::AVColorRange::AVCOL_RANGE_MPEG;
            let q = quality(kind, o.quality);
            let pd = (*c).priv_data;
            match kind {
                Kind::Nvenc => {
                    (*c).pix_fmt = ffi::AVPixelFormat::AV_PIX_FMT_BGR0;
                    (*c).bit_rate = 0;
                    (*c).max_b_frames = 2;
                    (*c).colorspace = ffi::AVColorSpace::AVCOL_SPC_BT470BG;
                    (*c).color_primaries = ffi::AVColorPrimaries::AVCOL_PRI_BT709;
                    (*c).color_trc = ffi::AVColorTransferCharacteristic::AVCOL_TRC_BT709;
                    set(pd, "preset", "p5");
                    set(pd, "tune", "hq");
                    set(pd, "rc", "vbr");
                    set(pd, "cq", &q.to_string());
                    set(pd, "spatial-aq", "1");
                    set(pd, "profile", if o.hevc { "main" } else { "high" });
                }
                Kind::X264 => {
                    (*c).pix_fmt = ffi::AVPixelFormat::AV_PIX_FMT_YUV420P;
                    (*c).thread_count = 0;
                    (*c).colorspace = ffi::AVColorSpace::AVCOL_SPC_BT709;
                    (*c).color_primaries = ffi::AVColorPrimaries::AVCOL_PRI_BT709;
                    (*c).color_trc = ffi::AVColorTransferCharacteristic::AVCOL_TRC_BT709;
                    set(pd, "preset", if o.fps > 30 { "superfast" } else { "veryfast" });
                    set(pd, "crf", &q.to_string());
                    v.sws = ffi::sws_getContext(
                        o.w as c_int,
                        o.h as c_int,
                        ffi::AVPixelFormat::AV_PIX_FMT_BGR0,
                        o.w as c_int,
                        o.h as c_int,
                        ffi::AVPixelFormat::AV_PIX_FMT_YUV420P,
                        ffi::SwsFlags::SWS_POINT as c_int,
                        null_mut(),
                        null_mut(),
                        std::ptr::null(),
                    );
                    if v.sws.is_null() {
                        bail!("swscale");
                    }
                    let coefs = ffi::sws_getCoefficients(ffi::SWS_CS_ITU709 as c_int);
                    ffi::sws_setColorspaceDetails(v.sws, coefs, 1, coefs, 0, 0, 1 << 16, 1 << 16);
                    v.yuv = ffi::av_frame_alloc();
                    (*v.yuv).format = ffi::AVPixelFormat::AV_PIX_FMT_YUV420P as c_int;
                    (*v.yuv).width = o.w as c_int;
                    (*v.yuv).height = o.h as c_int;
                    let r = ffi::av_frame_get_buffer(v.yuv, 0);
                    if r < 0 {
                        bail!("кадр YUV: {}", averr(r));
                    }
                }
                Kind::Vaapi => {
                    (*c).pix_fmt = ffi::AVPixelFormat::AV_PIX_FMT_VAAPI;
                    (*c).colorspace = ffi::AVColorSpace::AVCOL_SPC_BT470BG;
                    (*c).color_primaries = ffi::AVColorPrimaries::AVCOL_PRI_BT709;
                    (*c).color_trc = ffi::AVColorTransferCharacteristic::AVCOL_TRC_BT709;
                    (*c).max_b_frames = 0;
                    set(pd, "rc_mode", "CQP");
                    (*c).global_quality = q as c_int;
                    set(pd, "qp", &q.to_string());
                    v.vaapi_graph(o)?;
                    let hw = ffi::av_buffersink_get_hw_frames_ctx(v.bsink);
                    if hw.is_null() {
                        bail!("VAAPI: нет кадров у фильтра");
                    }
                    (*c).hw_frames_ctx = ffi::av_buffer_ref(hw);
                }
            }
            let r = ffi::avcodec_open2(c, codec, null_mut());
            if r < 0 {
                bail!("{name}: {}", averr(r));
            }
            v.pkt = ffi::av_packet_alloc();
        }
        Ok(v)
    }

    unsafe fn vaapi_graph(&mut self, o: &VideoOpts) -> Result<()> {
        let node = CString::new(o.drm_node)?;
        let mut drm_dev: *mut ffi::AVBufferRef = null_mut();
        let r = ffi::av_hwdevice_ctx_create(&mut drm_dev, ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_DRM, node.as_ptr(), null_mut(), 0);
        if r < 0 {
            bail!("DRM {}: {}", o.drm_node, averr(r));
        }
        let frames = ffi::av_hwframe_ctx_alloc(drm_dev);
        ffi::av_buffer_unref(&mut drm_dev);
        if frames.is_null() {
            bail!("DRM: кадры");
        }
        let fc = (*frames).data as *mut ffi::AVHWFramesContext;
        (*fc).format = ffi::AVPixelFormat::AV_PIX_FMT_DRM_PRIME;
        (*fc).sw_format = ffi::AVPixelFormat::AV_PIX_FMT_BGR0;
        (*fc).width = o.full.0 as c_int;
        (*fc).height = o.full.1 as c_int;
        let r = ffi::av_hwframe_ctx_init(frames);
        if r < 0 {
            let mut f = frames;
            ffi::av_buffer_unref(&mut f);
            bail!("DRM: кадры: {}", averr(r));
        }
        self.drm_frames = frames;
        self.graph = ffi::avfilter_graph_alloc();
        let src = ffi::avfilter_graph_alloc_filter(self.graph, ffi::avfilter_get_by_name(c"buffer".as_ptr()), c"in".as_ptr());
        let par = ffi::av_buffersrc_parameters_alloc();
        (*par).format = ffi::AVPixelFormat::AV_PIX_FMT_DRM_PRIME as c_int;
        (*par).width = o.full.0 as c_int;
        (*par).height = o.full.1 as c_int;
        (*par).time_base = ffi::AVRational { num: 1, den: o.fps as c_int };
        (*par).frame_rate = ffi::AVRational { num: o.fps as c_int, den: 1 };
        (*par).sample_aspect_ratio = ffi::AVRational { num: 1, den: 1 };
        (*par).hw_frames_ctx = frames;
        let r = ffi::av_buffersrc_parameters_set(src, par);
        ffi::av_free(par as *mut c_void);
        if r < 0 {
            bail!("buffersrc: {}", averr(r));
        }
        let r = ffi::avfilter_init_str(src, std::ptr::null());
        if r < 0 {
            bail!("buffersrc: {}", averr(r));
        }
        let mut sink: *mut ffi::AVFilterContext = null_mut();
        let r = ffi::avfilter_graph_create_filter(&mut sink, ffi::avfilter_get_by_name(c"buffersink".as_ptr()), c"out".as_ptr(), std::ptr::null(), null_mut(), self.graph);
        if r < 0 {
            bail!("buffersink: {}", averr(r));
        }
        let mut outs = ffi::avfilter_inout_alloc();
        let mut ins = ffi::avfilter_inout_alloc();
        (*outs).name = ffi::av_strdup(c"in".as_ptr());
        (*outs).filter_ctx = src;
        (*outs).pad_idx = 0;
        (*outs).next = null_mut();
        (*ins).name = ffi::av_strdup(c"out".as_ptr());
        (*ins).filter_ctx = sink;
        (*ins).pad_idx = 0;
        (*ins).next = null_mut();
        let desc = CString::new(format!(
            "hwmap=mode=read:derive_device=vaapi,scale_vaapi=w={}:h={}:format=nv12:out_range=tv",
            o.w, o.h
        ))?;
        let r = ffi::avfilter_graph_parse_ptr(self.graph, desc.as_ptr(), &mut ins, &mut outs, null_mut());
        ffi::avfilter_inout_free(&mut ins);
        ffi::avfilter_inout_free(&mut outs);
        if r < 0 {
            bail!("фильтр VAAPI: {}", averr(r));
        }
        let r = ffi::avfilter_graph_config(self.graph, null_mut());
        if r < 0 {
            bail!("фильтр VAAPI: {}", averr(r));
        }
        self.bsrc = src;
        self.bsink = sink;
        Ok(())
    }

    pub fn encode(&mut self, src: Src, pts: i64, out: &mut dyn FnMut(*mut ffi::AVPacket) -> Result<()>) -> Result<()> {
        unsafe {
            let frame = ffi::av_frame_alloc();
            let r = match self.kind {
                Kind::Nvenc => {
                    let off = src.offset as usize + src.crop[1] as usize * src.stride as usize + src.crop[0] as usize * 4;
                    dma_sync(src.fd, src.modifier.is_some(), true);
                    let fd = src.fd;
                    let dma = src.modifier.is_some();
                    let rel = src.release;
                    let buf = wrap(src.map.add(off), src.size.saturating_sub(off), Release(Some(Box::new(move || {
                        dma_sync(fd, dma, false);
                        rel();
                    })), None));
                    (*frame).buf[0] = buf;
                    (*frame).data[0] = src.map.add(off);
                    (*frame).linesize[0] = src.stride as c_int;
                    (*frame).format = ffi::AVPixelFormat::AV_PIX_FMT_BGR0 as c_int;
                    (*frame).width = self.w as c_int;
                    (*frame).height = self.h as c_int;
                    (*frame).pts = pts;
                    let r = ffi::avcodec_send_frame(self.ctx, frame);
                    ffi::av_frame_free(&mut (frame as *mut _));
                    r
                }
                Kind::X264 => {
                    let off = src.offset as usize + src.crop[1] as usize * src.stride as usize + src.crop[0] as usize * 4;
                    dma_sync(src.fd, src.modifier.is_some(), true);
                    let ptrs = [src.map.add(off) as *const u8, std::ptr::null(), std::ptr::null(), std::ptr::null()];
                    let strides = [src.stride as c_int, 0, 0, 0];
                    let wr = ffi::av_frame_make_writable(self.yuv);
                    if wr >= 0 {
                        ffi::sws_scale(self.sws, ptrs.as_ptr(), strides.as_ptr(), 0, self.h as c_int, (*self.yuv).data.as_ptr() as *const *mut u8, (*self.yuv).linesize.as_ptr());
                    }
                    dma_sync(src.fd, src.modifier.is_some(), false);
                    (src.release)();
                    ffi::av_frame_free(&mut (frame as *mut _));
                    if wr < 0 {
                        wr
                    } else {
                        (*self.yuv).pts = pts;
                        ffi::avcodec_send_frame(self.ctx, self.yuv)
                    }
                }
                Kind::Vaapi => {
                    let mut d = Box::new(DrmDesc::default());
                    d.nb_objects = 1;
                    d.objects[0] = DrmObj { fd: src.fd, size: src.size, modifier: src.modifier.unwrap_or(0) };
                    d.nb_layers = 1;
                    d.layers[0].format = u32::from_le_bytes(*b"XR24");
                    d.layers[0].nb_planes = 1;
                    d.layers[0].planes[0] = DrmPlane { object_index: 0, offset: src.offset as isize, pitch: src.stride as isize };
                    let dp = &mut *d as *mut DrmDesc as *mut u8;
                    let buf = wrap(dp, std::mem::size_of::<DrmDesc>(), Release(Some(src.release), Some(d)));
                    (*frame).buf[0] = buf;
                    (*frame).data[0] = dp;
                    (*frame).format = ffi::AVPixelFormat::AV_PIX_FMT_DRM_PRIME as c_int;
                    (*frame).width = src.full.0 as c_int;
                    (*frame).height = src.full.1 as c_int;
                    (*frame).crop_left = src.crop[0] as usize;
                    (*frame).crop_top = src.crop[1] as usize;
                    (*frame).crop_right = src.full.0.saturating_sub(src.crop[0] + src.crop[2]) as usize;
                    (*frame).crop_bottom = src.full.1.saturating_sub(src.crop[1] + src.crop[3]) as usize;
                    (*frame).hw_frames_ctx = ffi::av_buffer_ref(self.drm_frames);
                    (*frame).pts = pts;
                    let mut r = ffi::av_buffersrc_add_frame_flags(self.bsrc, frame, 0);
                    ffi::av_frame_free(&mut (frame as *mut _));
                    if r >= 0 {
                        loop {
                            let f = ffi::av_frame_alloc();
                            let g = ffi::av_buffersink_get_frame(self.bsink, f);
                            if g < 0 {
                                ffi::av_frame_free(&mut (f as *mut _));
                                if g != ffi::AVERROR(ffi::EAGAIN) && g != ffi::AVERROR_EOF {
                                    r = g;
                                }
                                break;
                            }
                            (*f).pts = pts;
                            r = ffi::avcodec_send_frame(self.ctx, f);
                            ffi::av_frame_free(&mut (f as *mut _));
                            if r < 0 {
                                break;
                            }
                            self.drain(out)?;
                        }
                    }
                    r
                }
            };
            if r < 0 && r != ffi::AVERROR(ffi::EAGAIN) {
                bail!("{}: {}", self.name, averr(r));
            }
        }
        self.drain(out)
    }

    fn drain(&mut self, out: &mut dyn FnMut(*mut ffi::AVPacket) -> Result<()>) -> Result<()> {
        unsafe {
            loop {
                let r = ffi::avcodec_receive_packet(self.ctx, self.pkt);
                if r == ffi::AVERROR(ffi::EAGAIN) || r == ffi::AVERROR_EOF {
                    return Ok(());
                }
                if r < 0 {
                    bail!("{}: {}", self.name, averr(r));
                }
                let res = out(self.pkt);
                ffi::av_packet_unref(self.pkt);
                res?;
            }
        }
    }

    pub fn finish(&mut self, out: &mut dyn FnMut(*mut ffi::AVPacket) -> Result<()>) -> Result<()> {
        unsafe { ffi::avcodec_send_frame(self.ctx, std::ptr::null()) };
        self.drain(out)
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        unsafe {
            if !self.pkt.is_null() {
                ffi::av_packet_free(&mut self.pkt);
            }
            if !self.ctx.is_null() {
                ffi::avcodec_free_context(&mut self.ctx);
            }
            if !self.graph.is_null() {
                ffi::avfilter_graph_free(&mut self.graph);
            }
            if !self.drm_frames.is_null() {
                ffi::av_buffer_unref(&mut self.drm_frames);
            }
            if !self.sws.is_null() {
                ffi::sws_freeContext(self.sws);
            }
            if !self.yuv.is_null() {
                ffi::av_frame_free(&mut self.yuv);
            }
        }
    }
}

#[repr(C)]
struct DmaBufSync {
    flags: u64,
}

pub fn dma_sync(fd: c_int, dma: bool, start: bool) {
    if !dma {
        return;
    }
    const READ: u64 = 1;
    const END: u64 = 1 << 2;
    let s = DmaBufSync { flags: READ | if start { 0 } else { END } };
    let req: libc::c_ulong = (1 << 30) | ((std::mem::size_of::<DmaBufSync>() as libc::c_ulong) << 16) | ((b'b' as libc::c_ulong) << 8);
    unsafe { libc::ioctl(fd, req as _, &s) };
}

pub struct Audio {
    pub ctx: *mut ffi::AVCodecContext,
    frame: *mut ffi::AVFrame,
    pkt: *mut ffi::AVPacket,
    buf: Vec<f32>,
    pub pts: i64,
}

unsafe impl Send for Audio {}

pub const RATE: i64 = 48000;

impl Audio {
    pub fn open() -> Result<Audio> {
        unsafe {
            let codec = ffi::avcodec_find_encoder(ffi::AVCodecID::AV_CODEC_ID_AAC);
            if codec.is_null() {
                bail!("нет кодера AAC");
            }
            let c = ffi::avcodec_alloc_context3(codec);
            (*c).sample_rate = RATE as c_int;
            (*c).sample_fmt = ffi::AVSampleFormat::AV_SAMPLE_FMT_FLTP;
            ffi::av_channel_layout_default(&mut (*c).ch_layout, 2);
            (*c).bit_rate = 192_000;
            (*c).time_base = ffi::AVRational { num: 1, den: RATE as c_int };
            (*c).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as c_int;
            let r = ffi::avcodec_open2(c, codec, null_mut());
            if r < 0 {
                let mut c = c;
                ffi::avcodec_free_context(&mut c);
                bail!("AAC: {}", averr(r));
            }
            let fs = (*c).frame_size.max(1024);
            let frame = ffi::av_frame_alloc();
            (*frame).nb_samples = fs;
            (*frame).format = ffi::AVSampleFormat::AV_SAMPLE_FMT_FLTP as c_int;
            (*frame).sample_rate = RATE as c_int;
            ffi::av_channel_layout_copy(&mut (*frame).ch_layout, &(*c).ch_layout);
            let r = ffi::av_frame_get_buffer(frame, 0);
            if r < 0 {
                bail!("AAC: {}", averr(r));
            }
            Ok(Audio { ctx: c, frame, pkt: ffi::av_packet_alloc(), buf: Vec::new(), pts: 0 })
        }
    }

    pub fn push(&mut self, interleaved: &[f32], out: &mut dyn FnMut(*mut ffi::AVPacket) -> Result<()>) -> Result<()> {
        self.buf.extend_from_slice(interleaved);
        let fs = unsafe { (*self.frame).nb_samples } as usize;
        while self.buf.len() >= fs * 2 {
            unsafe {
                ffi::av_frame_make_writable(self.frame);
                let l = (*self.frame).data[0] as *mut f32;
                let r = (*self.frame).data[1] as *mut f32;
                for i in 0..fs {
                    *l.add(i) = self.buf[i * 2];
                    *r.add(i) = self.buf[i * 2 + 1];
                }
                (*self.frame).pts = self.pts;
                self.pts += fs as i64;
                self.buf.drain(..fs * 2);
                let e = ffi::avcodec_send_frame(self.ctx, self.frame);
                if e < 0 {
                    bail!("AAC: {}", averr(e));
                }
            }
            self.drain(out)?;
        }
        Ok(())
    }

    fn drain(&mut self, out: &mut dyn FnMut(*mut ffi::AVPacket) -> Result<()>) -> Result<()> {
        unsafe {
            loop {
                let r = ffi::avcodec_receive_packet(self.ctx, self.pkt);
                if r == ffi::AVERROR(ffi::EAGAIN) || r == ffi::AVERROR_EOF {
                    return Ok(());
                }
                if r < 0 {
                    bail!("AAC: {}", averr(r));
                }
                let res = out(self.pkt);
                ffi::av_packet_unref(self.pkt);
                res?;
            }
        }
    }

    pub fn finish(&mut self, out: &mut dyn FnMut(*mut ffi::AVPacket) -> Result<()>) -> Result<()> {
        let fs = unsafe { (*self.frame).nb_samples } as usize;
        if !self.buf.is_empty() {
            let pad = fs * 2 - self.buf.len() % (fs * 2);
            if pad < fs * 2 {
                self.buf.extend(std::iter::repeat_n(0.0, pad));
            }
            let b = std::mem::take(&mut self.buf);
            self.push(&b, out)?;
        }
        unsafe { ffi::avcodec_send_frame(self.ctx, std::ptr::null()) };
        self.drain(out)
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        unsafe {
            ffi::av_packet_free(&mut self.pkt);
            ffi::av_frame_free(&mut self.frame);
            ffi::avcodec_free_context(&mut self.ctx);
        }
    }
}

pub struct Mux {
    ctx: *mut ffi::AVFormatContext,
    pub vst: c_int,
    pub ast: c_int,
    vtb: ffi::AVRational,
    atb: ffi::AVRational,
    header: bool,
    pub bytes: u64,
}

unsafe impl Send for Mux {}

impl Mux {
    pub fn new(path: &Path, v: &Video, a: Option<&Audio>, hevc: bool) -> Result<Mux> {
        let cpath = CString::new(path.to_string_lossy().as_bytes())?;
        unsafe {
            let mut ctx: *mut ffi::AVFormatContext = null_mut();
            let r = ffi::avformat_alloc_output_context2(&mut ctx, std::ptr::null(), c"mp4".as_ptr(), cpath.as_ptr());
            if r < 0 || ctx.is_null() {
                bail!("mp4: {}", averr(r));
            }
            let mut m = Mux { ctx, vst: -1, ast: -1, vtb: (*v.ctx).time_base, atb: ffi::AVRational { num: 1, den: RATE as c_int }, header: false, bytes: 0 };
            let st = ffi::avformat_new_stream(ctx, std::ptr::null());
            let r = ffi::avcodec_parameters_from_context((*st).codecpar, v.ctx);
            if r < 0 {
                bail!("mp4: {}", averr(r));
            }
            if hevc {
                (*(*st).codecpar).codec_tag = u32::from_le_bytes(*b"hvc1");
            }
            (*st).time_base = (*v.ctx).time_base;
            (*st).avg_frame_rate = (*v.ctx).framerate;
            (*st).r_frame_rate = (*v.ctx).framerate;
            m.vst = (*st).index;
            if let Some(a) = a {
                let st = ffi::avformat_new_stream(ctx, std::ptr::null());
                let r = ffi::avcodec_parameters_from_context((*st).codecpar, a.ctx);
                if r < 0 {
                    bail!("mp4: {}", averr(r));
                }
                (*st).time_base = (*a.ctx).time_base;
                m.atb = (*a.ctx).time_base;
                m.ast = (*st).index;
            }
            let r = ffi::avio_open(&mut (*ctx).pb, cpath.as_ptr(), ffi::AVIO_FLAG_WRITE as c_int);
            if r < 0 {
                bail!("{}: {}", path.display(), averr(r));
            }
            let mut opts: *mut ffi::AVDictionary = null_mut();
            ffi::av_dict_set(&mut opts, c"movflags".as_ptr(), c"+faststart".as_ptr(), 0);
            let r = ffi::avformat_write_header(ctx, &mut opts);
            ffi::av_dict_free(&mut opts);
            if r < 0 {
                bail!("mp4: {}", averr(r));
            }
            m.header = true;
            Ok(m)
        }
    }

    pub fn write(&mut self, pkt: *mut ffi::AVPacket, video: bool) -> Result<()> {
        unsafe {
            let (idx, tb) = if video { (self.vst, self.vtb) } else { (self.ast, self.atb) };
            let st = *(*self.ctx).streams.add(idx as usize);
            ffi::av_packet_rescale_ts(pkt, tb, (*st).time_base);
            (*pkt).stream_index = idx;
            self.bytes += (*pkt).size as u64;
            let r = ffi::av_interleaved_write_frame(self.ctx, pkt);
            if r < 0 {
                bail!("mp4: {}", averr(r));
            }
        }
        Ok(())
    }

    pub fn finish(&mut self) -> Result<()> {
        if !self.header {
            return Ok(());
        }
        self.header = false;
        unsafe {
            let r = ffi::av_write_trailer(self.ctx);
            if r < 0 {
                bail!("mp4: {}", averr(r));
            }
        }
        Ok(())
    }
}

impl Drop for Mux {
    fn drop(&mut self) {
        unsafe {
            if !(*self.ctx).pb.is_null() {
                ffi::avio_closep(&mut (*self.ctx).pb);
            }
            ffi::avformat_free_context(self.ctx);
        }
    }
}
