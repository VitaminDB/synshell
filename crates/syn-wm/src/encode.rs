//! Видео для потока кадров: кадр потока (текстура GLES) → NV12 на GPU →
//! аппаратный кодер H.264/HEVC. Кодеры (`[link] screen_encoder`):
//! - V4L2 (`v4l2enc.c`) — телефоны (Qualcomm msm_vidc);
//! - VAAPI (`ffenc.c`, libavcodec) — Intel/AMD компьютера;
//! - NVENC (`ffenc.c`) — NVIDIA.
//!
//! У V4L2 и VAAPI буферы NV12 выделяет кодер (dma-heap / VA-поверхности,
//! экспортированные как dma-buf), GPU рисует в них напрямую: плоскость Y —
//! как R8, UV — как GR88 вдвое меньше (два прохода шейдера по той же
//! текстуре), копий через CPU нет. Для NVENC (экран рисует другой GPU) шейдер
//! пакует NV12 в RGBA (4 яркости или 2 пары цветности на пиксель), кадр
//! читается в память одной выгрузкой. Цвет — BT.709, полный диапазон (так же
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

#[repr(C)]
#[derive(Default, Clone, Copy, Debug)]
struct FencPlanes {
    fd: [c_int; 2],
    offset: [u32; 2],
    pitch: [u32; 2],
    modifier: u64,
}

#[repr(C)]
struct Fenc {
    _p: [u8; 0],
}

extern "C" {
    fn fenc_open(
        kind: c_int,
        dev: *const c_char,
        w: u32,
        h: u32,
        hevc: c_int,
        bitrate: u32,
        fps: u32,
        n_inputs: *mut u32,
        err: *mut c_char,
        errlen: c_int,
    ) -> *mut Fenc;
    fn fenc_close(e: *mut Fenc);
    fn fenc_input(e: *mut Fenc, i: u32, p: *mut FencPlanes) -> c_int;
    fn fenc_next_input(e: *mut Fenc) -> c_int;
    fn fenc_set_bitrate(e: *mut Fenc, bitrate: u32);
    fn fenc_encode(
        e: *mut Fenc,
        i: u32,
        y: *const u8,
        uv: *const u8,
        stride: u32,
        force_key: c_int,
        ts_us: u64,
        out: *mut *const u8,
        key: *mut c_int,
    ) -> c_long;
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

/// Аппаратный кодер.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    V4l2,
    Vaapi,
    Nvenc,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::V4l2 => "v4l2",
            Kind::Vaapi => "vaapi",
            Kind::Nvenc => "nvenc",
        }
    }

    /// Порядок проб по `[link] screen_encoder`: выбранный первым, остальные —
    /// запасом (`auto`: V4L2 телефона, VAAPI — Intel/AMD, NVENC).
    pub fn order(pref: &str) -> Vec<Kind> {
        let first = match pref.trim().to_lowercase().as_str() {
            "nvenc" | "nvidia" => Some(Kind::Nvenc),
            "vaapi" | "intel" | "amd" => Some(Kind::Vaapi),
            "v4l2" => Some(Kind::V4l2),
            _ => None,
        };
        let mut v: Vec<Kind> = first.into_iter().collect();
        for k in [Kind::V4l2, Kind::Vaapi, Kind::Nvenc] {
            if !v.contains(&k) {
                v.push(k);
            }
        }
        v
    }
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

// NVENC: NV12 упакован в RGBA для выгрузки — пиксель цели = 4 яркости подряд
// (`dst_w` = ширина/4 с округлением вверх).
const Y4_SHADER: &str = r#"#version 100
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
uniform float src_w;
uniform float dst_w;
varying vec2 v_coords;
float luma(float x) {
    vec3 c = texture2D(tex, vec2(x / src_w, v_coords.y)).rgb;
    return dot(c, vec3(0.2126, 0.7152, 0.0722));
}
void main() {
    float x = floor(v_coords.x * dst_w) * 4.0 + 0.5;
    gl_FragColor = vec4(luma(x), luma(x + 1.0), luma(x + 2.0), luma(x + 3.0));
}
"#;

// …и 2 пары цветности (U, V) — каждая по центру своего блока 2×2.
const UV4_SHADER: &str = r#"#version 100
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
uniform float src_w;
uniform float dst_w;
varying vec2 v_coords;
vec2 chroma(float x) {
    vec3 c = texture2D(tex, vec2(x / src_w, v_coords.y)).rgb;
    float y = dot(c, vec3(0.2126, 0.7152, 0.0722));
    return vec2((c.b - y) / 1.8556 + 0.5, (c.r - y) / 1.5748 + 0.5);
}
void main() {
    float x = floor(v_coords.x * dst_w) * 4.0 + 1.0;
    gl_FragColor = vec4(chroma(x), chroma(x + 2.0));
}
"#;

struct Input {
    y: Dmabuf,
    uv: Dmabuf,
}

enum Backend {
    V4l2(*mut Venc),
    Ff(*mut Fenc),
}

/// NVENC: упакованные плоскости в RGBA-текстурах для выгрузки.
struct Readback {
    y: GlesTexture,
    uv: GlesTexture,
    w4: i32,
    y_prog: GlesTexProgram,
    uv_prog: GlesTexProgram,
}

pub struct Encoder {
    backend: Backend,
    inputs: Vec<Input>,
    readback: Option<Readback>,
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
        match self.backend {
            Backend::V4l2(raw) => unsafe { venc_close(raw) },
            Backend::Ff(raw) => unsafe { fenc_close(raw) },
        }
    }
}

fn plane(fd: BorrowedFd<'_>, fourcc: Fourcc, size: (i32, i32), offset: u32, stride: u32) -> Result<Dmabuf> {
    plane_mod(fd, fourcc, size, offset, stride, Modifier::Linear)
}

fn plane_mod(fd: BorrowedFd<'_>, fourcc: Fourcc, size: (i32, i32), offset: u32, stride: u32, modifier: Modifier) -> Result<Dmabuf> {
    let owned: OwnedFd = fd.try_clone_to_owned()?;
    let mut b = Dmabuf::builder(size, fourcc, modifier, DmabufFlags::empty());
    b.add_plane(owned, 0, offset, stride);
    b.build().context("dma-buf плоскости")
}

impl Encoder {
    /// Открыть кодер `kind` для кадров `size`; `bitrate` — бит/с.
    pub fn new(gles: &mut GlesRenderer, size: (i32, i32), codec: Codec, bitrate: u32, kind: Kind) -> Result<Self> {
        let (w, h) = size;
        if w <= 0 || h <= 0 || w % 2 != 0 || h % 2 != 0 {
            bail!("размер кадра {w}x{h} не подходит кодеру");
        }
        let mut err = [0 as c_char; 256];
        let errmsg = |err: &[c_char]| unsafe { CStr::from_ptr(err.as_ptr()) }.to_string_lossy().into_owned();
        let mut inputs = Vec::new();
        let mut readback = None;
        let backend = match kind {
            Kind::V4l2 => {
                let mut info = VencInfo::default();
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
                    bail!("кодер {} {}: {}", kind.name(), codec.name(), errmsg(&err));
                }
                let backend = Backend::V4l2(raw);
                for i in 0..info.in_count {
                    let fd = unsafe { BorrowedFd::borrow_raw(venc_in_fd(raw, i)) };
                    let y = plane(fd, Fourcc::R8, (w, h), 0, info.stride)?;
                    let uv = plane(fd, Fourcc::Gr88, (w / 2, h / 2), info.stride * info.y_scanlines, info.stride)?;
                    inputs.push(Input { y, uv });
                }
                backend
            }
            Kind::Vaapi | Kind::Nvenc => {
                // VAAPI — на том GPU, где рисует композитор (буферы общие).
                let dev = if kind == Kind::Vaapi {
                    let node = smithay::backend::egl::EGLDevice::device_for_display(gles.egl_context().display())
                        .ok()
                        .and_then(|d| d.try_get_render_node().ok().flatten())
                        .and_then(|n| n.dev_path())
                        .context("render node GPU композитора")?;
                    Some(std::ffi::CString::new(node.to_string_lossy().as_bytes())?)
                } else {
                    None
                };
                let mut n = 0u32;
                let raw = unsafe {
                    fenc_open(
                        (kind == Kind::Nvenc) as c_int,
                        dev.as_ref().map_or(std::ptr::null(), |d| d.as_ptr()),
                        w as u32,
                        h as u32,
                        (codec == Codec::Hevc) as c_int,
                        bitrate,
                        60,
                        &mut n,
                        err.as_mut_ptr(),
                        err.len() as c_int,
                    )
                };
                if raw.is_null() {
                    bail!("кодер {} {}: {}", kind.name(), codec.name(), errmsg(&err));
                }
                let backend = Backend::Ff(raw);
                if kind == Kind::Vaapi {
                    for i in 0..n {
                        let mut p = FencPlanes::default();
                        if unsafe { fenc_input(raw, i, &mut p) } != 0 {
                            unsafe { fenc_close(raw) };
                            bail!("VA-поверхность {i}: неизвестная раскладка dma-buf");
                        }
                        let m = Modifier::from(p.modifier);
                        let fd0 = unsafe { BorrowedFd::borrow_raw(p.fd[0]) };
                        let fd1 = unsafe { BorrowedFd::borrow_raw(p.fd[1]) };
                        let made = plane_mod(fd0, Fourcc::R8, (w, h), p.offset[0], p.pitch[0], m)
                            .and_then(|y| Ok((y, plane_mod(fd1, Fourcc::Gr88, (w / 2, h / 2), p.offset[1], p.pitch[1], m)?)));
                        match made {
                            Ok((y, uv)) => inputs.push(Input { y, uv }),
                            Err(e) => {
                                inputs.clear();
                                unsafe { fenc_close(raw) };
                                return Err(e);
                            }
                        }
                    }
                } else {
                    use smithay::backend::renderer::Offscreen;
                    use smithay::backend::renderer::gles::{UniformName, UniformType};
                    let w4 = (w + 3) / 4;
                    let mk = |g: &mut GlesRenderer, hh: i32| {
                        <GlesRenderer as Offscreen<GlesTexture>>::create_buffer(g, Fourcc::Abgr8888, (w4, hh).into())
                    };
                    let names = [UniformName::new("src_w", UniformType::_1f), UniformName::new("dst_w", UniformType::_1f)];
                    let rb = (|| -> Result<Readback> {
                        Ok(Readback {
                            y: mk(gles, h)?,
                            uv: mk(gles, h / 2)?,
                            w4,
                            y_prog: gles.compile_custom_texture_shader(Y4_SHADER, &names).context("шейдер Y4")?,
                            uv_prog: gles.compile_custom_texture_shader(UV4_SHADER, &names).context("шейдер UV4")?,
                        })
                    })();
                    match rb {
                        Ok(rb) => readback = Some(rb),
                        Err(e) => {
                            unsafe { fenc_close(raw) };
                            return Err(e);
                        }
                    }
                }
                backend
            }
        };
        let y_prog = gles.compile_custom_texture_shader(Y_SHADER, &[]).context("шейдер Y")?;
        let uv_prog = gles.compile_custom_texture_shader(UV_SHADER, &[]).context("шейдер UV")?;
        tracing::info!(encoder = kind.name(), codec = codec.name(), w, h, bitrate, buffers = inputs.len(), "видеокодер открыт");
        Ok(Self { backend, inputs, readback, y_prog, uv_prog, codec, size, bitrate, start: std::time::Instant::now() })
    }

    pub fn set_bitrate(&mut self, bitrate: u32) {
        if bitrate != self.bitrate {
            self.bitrate = bitrate;
            match self.backend {
                Backend::V4l2(raw) => unsafe { venc_set_bitrate(raw, bitrate) },
                Backend::Ff(raw) => unsafe { fenc_set_bitrate(raw, bitrate) },
            }
        }
    }

    /// Закодировать текстуру кадра: (ключевой ли кадр, пакет).
    pub fn encode(&mut self, gles: &mut GlesRenderer, tex: &GlesTexture, force_key: bool) -> Result<(bool, Vec<u8>)> {
        let (w, h) = self.size;
        let src = Rectangle::from_size(tex.size().to_f64());
        let ts = self.start.elapsed().as_micros() as u64;
        let mut out: *const u8 = std::ptr::null();
        let mut key: c_int = 0;
        if let Some(rb) = self.readback.as_mut() {
            // NVENC: упакованный NV12 → память → кодер.
            use smithay::backend::renderer::{gles::Uniform, ExportMem};
            let uniforms = [Uniform::new("src_w", w as f32), Uniform::new("dst_w", rb.w4 as f32)];
            let mut planes: Vec<Vec<u8>> = Vec::with_capacity(2);
            for (target, prog, hh) in [(&mut rb.y, &rb.y_prog, h), (&mut rb.uv, &rb.uv_prog, h / 2)] {
                let out_size: Size<i32, smithay::utils::Physical> = (rb.w4, hh).into();
                let dst = Rectangle::from_size(out_size);
                let mut fb = gles.bind(target).context("цель выгрузки")?;
                {
                    let mut frame = gles.render(&mut fb, out_size, Transform::Normal)?;
                    frame.render_texture_from_to(tex, src, dst, &[dst], &[dst], Transform::Normal, 1.0, Some(prog), &uniforms)?;
                    let _ = frame.finish()?;
                }
                let region = Rectangle::<i32, smithay::utils::Buffer>::from_size((rb.w4, hh).into());
                let mapping = gles.copy_framebuffer(&fb, region, Fourcc::Abgr8888)?;
                planes.push(gles.map_texture(&mapping)?.to_vec());
            }
            let Backend::Ff(raw) = self.backend else { bail!("выгрузка без кодера ffmpeg") };
            let n = unsafe {
                fenc_encode(raw, 0, planes[0].as_ptr(), planes[1].as_ptr(), (rb.w4 * 4) as u32, force_key as c_int, ts, &mut out, &mut key)
            };
            return finish(n, out, key);
        }
        let i = match self.backend {
            Backend::V4l2(raw) => unsafe { venc_free_input(raw) },
            Backend::Ff(raw) => unsafe { fenc_next_input(raw) },
        };
        if i < 0 {
            bail!("кодер занят: нет свободного входного буфера");
        }
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
        let n = match self.backend {
            Backend::V4l2(raw) => unsafe { venc_encode(raw, i as u32, force_key as c_int, ts, 200, &mut out, &mut key) },
            Backend::Ff(raw) => unsafe {
                fenc_encode(raw, i as u32, std::ptr::null(), std::ptr::null(), 0, force_key as c_int, ts, &mut out, &mut key)
            },
        };
        finish(n, out, key)
    }
}

fn finish(n: c_long, out: *const u8, key: c_int) -> Result<(bool, Vec<u8>)> {
    if n < 0 {
        bail!("кодирование: код {n} ({})", std::io::Error::from_raw_os_error((-n) as i32));
    }
    if n == 0 {
        bail!("кодер не отдал пакет кадра");
    }
    let data = unsafe { std::slice::from_raw_parts(out, n as usize) }.to_vec();
    Ok((key != 0, data))
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
    let kinds = Kind::order(&core.config.link.screen_encoder);
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
            'open: for &k in &kinds {
                for &c in codecs {
                    match Encoder::new(r, size, c, bitrate, k) {
                        Ok(e) => {
                            enc = Some(Box::new(e));
                            break 'open;
                        }
                        Err(e) => {
                            tracing::info!(encoder = k.name(), codec = c.name(), ?e, "кодер не открылся");
                            last = Some(e);
                        }
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
