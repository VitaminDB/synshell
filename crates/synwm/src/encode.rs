//! Видео для потока кадров: кадр потока (текстура GLES) → NV12 на GPU →
//! аппаратный кодер V4L2 (H.264/HEVC, `v4l2enc.c`).
//!
//! Буферы NV12 выделяет кодер (dma-heap), GPU рисует в них напрямую: плоскость
//! Y — как R8, UV — как GR88 вдвое меньше (два прохода шейдера по той же
//! текстуре), копий через CPU нет. Цвет — BT.709, полный диапазон (так же
//! читает synlink-view).

use std::ffi::{c_char, c_int, c_long, CStr};
use std::os::fd::{BorrowedFd, OwnedFd};

use anyhow::{bail, Context, Result};
use smithay::backend::{
    allocator::{
        dmabuf::{Dmabuf, DmabufFlags},
        Fourcc, Modifier,
    },
    renderer::{
        gles::{GlesRenderer, GlesTexProgram, GlesTexture},
        Bind, Frame, Renderer, Texture,
    },
};
use smithay::utils::{Rectangle, Size, Transform};

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
        info: *mut VencInfo,
        err: *mut c_char,
        errlen: c_int,
    ) -> *mut Venc;
    fn venc_close(e: *mut Venc);
    fn venc_in_fd(e: *mut Venc, i: u32) -> c_int;
    fn venc_free_input(e: *mut Venc) -> c_int;
    fn venc_set_bitrate(e: *mut Venc, bitrate: u32);
    fn venc_encode(e: *mut Venc, i: u32, force_key: c_int, ts_us: u64, timeout_ms: c_int, out: *mut *const u8, key: *mut c_int) -> c_long;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    H264,
    Hevc,
}

impl Codec {
    pub fn name(self) -> &'static str {
        match self {
            Codec::H264 => "h264",
            Codec::Hevc => "hevc",
        }
    }
}

const Y_SHADER: &str = r#"#version 100
//_DEFINES_
#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif
precision highp float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif
uniform float alpha;
varying vec2 v_coords;
void main() {
    vec3 c = texture2D(tex, v_coords).rgb;
    gl_FragColor = vec4(dot(c, vec3(0.2126, 0.7152, 0.0722)), 0.0, 0.0, 1.0);
}
"#;

// Цветность — по центру блока 2×2: линейная выборка на половинном размере
// берёт среднее четырёх пикселей.
const UV_SHADER: &str = r#"#version 100
//_DEFINES_
#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif
precision highp float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif
uniform float alpha;
varying vec2 v_coords;
void main() {
    vec3 c = texture2D(tex, v_coords).rgb;
    float y = dot(c, vec3(0.2126, 0.7152, 0.0722));
    gl_FragColor = vec4((c.b - y) / 1.8556 + 0.5, (c.r - y) / 1.5748 + 0.5, 0.0, 1.0);
}
"#;

struct Input {
    y: Dmabuf,
    uv: Dmabuf,
}

pub struct Encoder {
    raw: *mut Venc,
    inputs: Vec<Input>,
    y_prog: GlesTexProgram,
    uv_prog: GlesTexProgram,
    pub codec: Codec,
    pub size: (i32, i32),
    bitrate: u32,
    start: std::time::Instant,
}

impl Drop for Encoder {
    fn drop(&mut self) {
        // Сначала GL-образы буферов (Dmabuf держат копии fd), потом кодер.
        self.inputs.clear();
        unsafe { venc_close(self.raw) };
    }
}

fn plane(fd: BorrowedFd<'_>, fourcc: Fourcc, size: (i32, i32), offset: u32, stride: u32) -> Result<Dmabuf> {
    let owned: OwnedFd = fd.try_clone_to_owned()?;
    let mut b = Dmabuf::builder(size, fourcc, Modifier::Linear, DmabufFlags::empty());
    b.add_plane(owned, 0, offset, stride);
    b.build().context("dma-buf плоскости")
}

impl Encoder {
    /// Открыть кодер для кадров `size`; `bitrate` — бит/с.
    pub fn new(gles: &mut GlesRenderer, size: (i32, i32), codec: Codec, bitrate: u32) -> Result<Self> {
        let (w, h) = size;
        if w <= 0 || h <= 0 || w % 2 != 0 || h % 2 != 0 {
            bail!("размер кадра {w}x{h} не подходит кодеру");
        }
        let mut info = VencInfo::default();
        let mut err = [0 as c_char; 256];
        let dev = std::env::var("SYNSHELL_VIDEO_ENCODER").ok().and_then(|d| std::ffi::CString::new(d).ok());
        let raw = unsafe {
            venc_open(
                dev.as_ref().map_or(std::ptr::null(), |d| d.as_ptr()),
                w as u32,
                h as u32,
                (codec == Codec::Hevc) as c_int,
                bitrate,
                60,
                &mut info,
                err.as_mut_ptr(),
                err.len() as c_int,
            )
        };
        if raw.is_null() {
            let msg = unsafe { CStr::from_ptr(err.as_ptr()) }.to_string_lossy().into_owned();
            bail!("кодер {}: {msg}", codec.name());
        }
        let mut inputs = Vec::new();
        for i in 0..info.in_count {
            let fd = unsafe { venc_in_fd(raw, i) };
            let fd = unsafe { BorrowedFd::borrow_raw(fd) };
            let y = plane(fd, Fourcc::R8, (w, h), 0, info.stride)?;
            let uv = plane(fd, Fourcc::Gr88, (w / 2, h / 2), info.stride * info.y_scanlines, info.stride)?;
            inputs.push(Input { y, uv });
        }
        let y_prog = gles.compile_custom_texture_shader(Y_SHADER, &[]).context("шейдер Y")?;
        let uv_prog = gles.compile_custom_texture_shader(UV_SHADER, &[]).context("шейдер UV")?;
        tracing::info!(codec = codec.name(), w, h, bitrate, stride = info.stride, buffers = info.in_count, "видеокодер открыт");
        Ok(Self { raw, inputs, y_prog, uv_prog, codec, size, bitrate, start: std::time::Instant::now() })
    }

    pub fn set_bitrate(&mut self, bitrate: u32) {
        if bitrate != self.bitrate {
            self.bitrate = bitrate;
            unsafe { venc_set_bitrate(self.raw, bitrate) };
        }
    }

    /// Закодировать текстуру кадра: (ключевой ли кадр, пакет).
    pub fn encode(&mut self, gles: &mut GlesRenderer, tex: &GlesTexture, force_key: bool) -> Result<(bool, Vec<u8>)> {
        let i = unsafe { venc_free_input(self.raw) };
        if i < 0 {
            bail!("кодер занят: нет свободного входного буфера");
        }
        let (w, h) = self.size;
        let src = Rectangle::from_size(tex.size().to_f64());
        for (plane, prog, size) in [(0usize, &self.y_prog, (w, h)), (1, &self.uv_prog, (w / 2, h / 2))] {
            let input = &mut self.inputs[i as usize];
            let target = if plane == 0 { &mut input.y } else { &mut input.uv };
            let mut fb = gles.bind(target).context("буфер кодера как цель GL")?;
            let out: Size<i32, smithay::utils::Physical> = size.into();
            let dst = Rectangle::from_size(out);
            let mut frame = gles.render(&mut fb, out, Transform::Normal)?;
            frame.render_texture_from_to(tex, src, dst, &[dst], &[dst], Transform::Normal, 1.0, Some(prog), &[])?;
            let sync = frame.finish()?;
            // Кодер читает буфер сразу после QBUF — GPU должен закончить.
            let _ = sync.wait();
        }
        let ts = self.start.elapsed().as_micros() as u64;
        let mut out: *const u8 = std::ptr::null();
        let mut key: c_int = 0;
        let n = unsafe { venc_encode(self.raw, i as u32, force_key as c_int, ts, 200, &mut out, &mut key) };
        if n < 0 {
            bail!("кодирование: {}", std::io::Error::from_raw_os_error(-n as i32));
        }
        let data = unsafe { std::slice::from_raw_parts(out, n as usize) }.to_vec();
        Ok((key != 0, data))
    }
}

/// Кадр потока на GLES: с видео, если его просили и кодер открылся.
pub fn stream_render(
    core: &mut crate::state::Core,
    gles: &mut GlesRenderer,
    output: &smithay::output::Output,
    slot: &mut Option<Box<dyn std::any::Any>>,
    cursor: bool,
    opts: &mut crate::stream::StreamOpts,
) -> Result<crate::stream::Rendered> {
    use smithay::backend::renderer::Offscreen;
    // `auto` — HEVC (лучше качество на тот же битрейт), нет — H.264.
    let want: Option<(Vec<Codec>, u32)> = opts.video.as_ref().map(|v| {
        let codecs = match v.codec.as_str() {
            "hevc" => vec![Codec::Hevc],
            "h264" => vec![Codec::H264],
            _ => vec![Codec::Hevc, Codec::H264],
        };
        (codecs, v.bitrate)
    });
    let mut enc: Option<Box<dyn std::any::Any>> = opts.encoder.take();
    let mut codec_name = opts.codec;
    let mut encode = |r: &mut GlesRenderer, tex: &mut GlesTexture, size: (i32, i32), key: bool| -> Result<(bool, Vec<u8>)> {
        let (codecs, bitrate) = want.as_ref().context("видео не просили")?;
        let bitrate = *bitrate;
        // Новый размер (поворот экрана) — новый кодер.
        let reuse = enc.as_ref().and_then(|e| e.downcast_ref::<Encoder>()).is_some_and(|e| e.size == size && codecs.contains(&e.codec));
        if !reuse {
            enc = None;
            let mut last = None;
            for &c in codecs {
                match Encoder::new(r, size, c, bitrate) {
                    Ok(e) => {
                        enc = Some(Box::new(e));
                        break;
                    }
                    Err(e) => {
                        tracing::info!(codec = c.name(), ?e, "кодер не открылся");
                        last = Some(e);
                    }
                }
            }
            if enc.is_none() {
                return Err(last.unwrap_or_else(|| anyhow::anyhow!("нет кодеков")));
            }
        }
        let e = enc.as_mut().and_then(|e| e.downcast_mut::<Encoder>()).expect("кодер");
        e.set_bitrate(bitrate);
        codec_name = e.codec.name();
        // Новый кодер начинает с ключевого кадра сам, но просим явно.
        e.encode(r, tex, key || !reuse)
    };
    let res = crate::stream::render(
        core,
        gles,
        output,
        slot,
        cursor,
        opts,
        |r, size| Ok(<GlesRenderer as Offscreen<GlesTexture>>::create_buffer(r, Fourcc::Abgr8888, size)?),
        if want.is_some() { Some(&mut encode) } else { None },
    );
    // Кодер сломался — `render` выключил видео, кодер не нужен.
    if opts.video.is_some() {
        opts.encoder = enc;
        opts.codec = codec_name;
    }
    res
}
