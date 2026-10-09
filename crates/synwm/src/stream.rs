//! Поток кадров вывода для synlink (IPC `frame-stream` / `frame-next`).
//!
//! У каждого потока — свой постоянный буфер и `OutputDamageTracker`:
//! кадр дорисовывается только там, где что-то изменилось, и наружу уходят
//! только эти прямоугольники. Пиксели пишутся в файл потока в
//! `$XDG_RUNTIME_DIR` (tmpfs, отображён в память), клиенту уходит JSON с
//! прямоугольниками — как у `capture`, без передачи fd.
//!
//! С `video` (GLES + аппаратный кодер V4L2, `encode.rs`) крупные изменения
//! (прокрутка, анимации, видео) уходят видеокадром H.264/HEVC — кадр
//! переводится в NV12 на GPU прямо в буфер кодера. Места, прошедшие через
//! видео, досылаются без потерь, когда экран замрёт ([`REFINE_DELAY`]):
//! при движении — плавно и мало трафика, в покое — пиксель в пиксель.
//!
//! Поток тянущий: следующий кадр композитор готовит только после
//! `frame-next`, поэтому медленный клиент (Wi-Fi) сам снижает частоту, а
//! ненужные кадры не рисуются вовсе.

use std::any::Any;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{damage::OutputDamageTracker, Bind, ExportMem, ImportAll, ImportMem, Renderer, Texture},
    },
    output::Output,
    utils::{Buffer as BufferCoords, Physical, Rectangle, Size, Transform},
};
use synshell_common::ipc::{FrameInfo, FrameVideo, VideoRequest};

use crate::state::{Core, State};
use synshell_tr::t;

/// Через сколько после последнего видеокадра дослать его места без потерь.
pub const REFINE_DELAY: Duration = Duration::from_millis(150);
/// Смешанный режим: изменения меньше этой доли экрана — без потерь.
const MIXED_LOSSLESS_SHARE: f64 = 0.04;

type Rect = Rectangle<i32, Physical>;

/// Результат отрисовки кадра потока: размер, куски RGBA без потерь, видеопакет.
pub struct Rendered {
    pub width: i32,
    pub height: i32,
    pub full: bool,
    pub rects: Vec<([i32; 4], Vec<u8>)>,
    /// (ключевой, пакет, где кадр изменился).
    pub video: Option<(bool, Vec<u8>, Vec<[i32; 4]>)>,
}

/// Настройки и состояние видео потока (между кадрами).
#[derive(Default)]
pub struct StreamOpts {
    pub video: Option<VideoRequest>,
    /// Кодер бэкенда (`encode::Encoder` у GLES).
    pub encoder: Option<Box<dyn Any>>,
    pub force_key: bool,
    /// Места, ушедшие видео, — дослать без потерь.
    pub lossy: Vec<Rect>,
    /// Этот кадр — досылка: прочитать `lossy` без потерь.
    pub refine: bool,
    pub video_error: Option<String>,
    pub last_video: Option<Instant>,
    /// Кодек открытого кодера (`h264`/`hevc`).
    pub codec: &'static str,
}

/// Кодер кадра потока: (рендерер, буфер кадра, размер, ключевой?) → (ключевой, пакет).
pub type EncodeFn<'a, R, B> = &'a mut dyn FnMut(&mut R, &mut B, (i32, i32), bool) -> anyhow::Result<(bool, Vec<u8>)>;

struct Target<B> {
    buf: B,
    tracker: OutputDamageTracker,
    size: Size<i32, BufferCoords>,
    scale: f64,
    fresh: bool,
}

/// Отрисовать кадр потока в постоянный буфер `slot` (создаётся заново,
/// если вывод сменил размер), выгрузить изменения без потерь и/или видео.
#[allow(clippy::too_many_arguments)]
pub fn render<R, B>(
    core: &mut Core,
    r: &mut R,
    output: &Output,
    slot: &mut Option<Box<dyn Any>>,
    cursor: bool,
    opts: &mut StreamOpts,
    create: impl FnOnce(&mut R, Size<i32, BufferCoords>) -> anyhow::Result<B>,
    encode: Option<EncodeFn<'_, R, B>>,
) -> anyhow::Result<Rendered>
where
    R: Renderer + ImportAll + ImportMem + Bind<B> + ExportMem + crate::rotate_anim::RotateDraw,
    R::TextureId: Texture + Clone + Send + 'static,
    R::Error: Send + Sync + 'static,
    B: 'static,
{
    let mode = output.current_mode().map(|m| m.size).unwrap_or_default();
    let transformed = output.current_transform().transform_size(mode);
    let size: Size<i32, BufferCoords> = Size::from((transformed.w, transformed.h));
    let scale = output.current_scale().fractional_scale();
    let reuse = slot
        .as_ref()
        .and_then(|s| s.downcast_ref::<Target<B>>())
        .is_some_and(|t| t.size == size && t.scale == scale);
    if !reuse {
        let buf = create(r, size)?;
        *slot = Some(Box::new(Target {
            buf,
            tracker: OutputDamageTracker::new(transformed, scale, Transform::Normal),
            size,
            scale,
            fresh: true,
        }));
    }
    let t = slot.as_mut().and_then(|s| s.downcast_mut::<Target<B>>()).expect("цель потока");
    let (elements, clear) = crate::render::output_elements(core, r, output, cursor);
    let age = if t.fresh { 0 } else { 1 };
    let damage: Vec<Rect> = {
        let mut fb = r.bind(&mut t.buf)?;
        let res = t.tracker.render_output(r, &mut fb, age, &elements, clear).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        res.damage.cloned().unwrap_or_default()
    };
    drop(elements);
    let full = t.fresh;
    t.fresh = false;
    let bounds = Rect::from_size((size.w, size.h).into());
    let damage = merge(damage.into_iter().filter_map(|d| d.intersection(bounds)).collect());

    // Что уходит без потерь, а что видео.
    let can_video = opts.video.is_some() && encode.is_some();
    let mut lossless: Vec<Rect> = Vec::new();
    let mut video_rects: Vec<Rect> = Vec::new();
    let mut encode_now = false;
    if full {
        // Первый кадр (или новый размер) — целиком без потерь; видео —
        // ключевой кадр для декодера зрителя.
        lossless.push(bounds);
        opts.lossy.clear();
        encode_now = can_video;
        opts.force_key = true;
    } else if !damage.is_empty() {
        let area: i64 = damage.iter().map(|d| d.size.w as i64 * d.size.h as i64).sum();
        let share = area as f64 / (size.w as f64 * size.h as f64).max(1.0);
        let mixed = opts.video.as_ref().is_some_and(|v| v.mixed);
        if can_video && !(mixed && share < MIXED_LOSSLESS_SHARE) {
            video_rects = damage.clone();
            encode_now = true;
        } else {
            lossless.extend(damage.iter().copied());
        }
    }
    if opts.refine {
        opts.refine = false;
        lossless.extend(std::mem::take(&mut opts.lossy));
        lossless = merge(lossless);
    }

    let mut video = None;
    if encode_now {
        let enc = encode.expect("кодер");
        let key = std::mem::take(&mut opts.force_key);
        match enc(r, &mut t.buf, (size.w, size.h), key) {
            Ok((key, data)) => {
                opts.lossy.extend(video_rects.iter().copied());
                opts.lossy = merge(std::mem::take(&mut opts.lossy));
                opts.last_video = Some(Instant::now());
                let rects = video_rects.iter().map(|r| [r.loc.x, r.loc.y, r.size.w, r.size.h]).collect();
                video = Some((key, data, rects));
            }
            Err(e) => {
                // Кодера нет или он сломался — дальше без потерь.
                tracing::warn!(?e, "видео потока выключено");
                opts.video_error = Some(format!("{e:#}"));
                opts.video = None;
                opts.encoder = None;
                lossless.extend(video_rects);
                lossless = merge(lossless);
            }
        }
    }

    let fb = r.bind(&mut t.buf)?;
    let mut out = Vec::with_capacity(lossless.len());
    for rc in lossless {
        if rc.size.w <= 0 || rc.size.h <= 0 {
            continue;
        }
        let region = Rectangle::<i32, BufferCoords>::new((rc.loc.x, rc.loc.y).into(), (rc.size.w, rc.size.h).into());
        let mapping = r.copy_framebuffer(&fb, region, Fourcc::Abgr8888)?;
        let data = r.map_texture(&mapping)?.to_vec();
        out.push(([rc.loc.x, rc.loc.y, rc.size.w, rc.size.h], data));
    }
    Ok(Rendered { width: size.w, height: size.h, full, rects: out, video })
}

/// Слить прямоугольники повреждений: пересекающиеся и соседние — в один;
/// если кусков много, чтение каждого дороже общего — берём охват.
fn merge(mut v: Vec<Rect>) -> Vec<Rect> {
    let mut changed = true;
    while changed {
        changed = false;
        'outer: for i in 0..v.len() {
            for j in i + 1..v.len() {
                let a = v[i];
                let b = v[j];
                let grown = Rectangle::new((a.loc.x - 16, a.loc.y - 16).into(), (a.size.w + 32, a.size.h + 32).into());
                if grown.overlaps(b) {
                    v[i] = a.merge(b);
                    v.swap_remove(j);
                    changed = true;
                    break 'outer;
                }
            }
        }
    }
    if v.len() > 12 {
        let first = v[0];
        return vec![v.into_iter().fold(first, |acc, r| acc.merge(r))];
    }
    v
}

/// Файл потока, отображённый в память: RGBA кадра, за ним — видеопакет.
struct Mapped {
    file: std::fs::File,
    ptr: *mut u8,
    len: usize,
}

impl Mapped {
    fn ensure(&mut self, len: usize) -> std::io::Result<()> {
        if len <= self.len {
            return Ok(());
        }
        let len = len.next_multiple_of(1 << 20);
        self.file.set_len(len as u64)?;
        if !self.ptr.is_null() {
            unsafe { libc::munmap(self.ptr.cast(), self.len) };
            self.ptr = std::ptr::null_mut();
            self.len = 0;
        }
        use std::os::fd::AsRawFd;
        let p = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, self.file.as_raw_fd(), 0) };
        if p == libc::MAP_FAILED {
            return Err(std::io::Error::last_os_error());
        }
        self.ptr = p.cast();
        self.len = len;
        Ok(())
    }

    fn bytes(&mut self) -> &mut [u8] {
        // SAFETY: отображение живо, пока жив `self`; длина — `self.len`.
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}

impl Drop for Mapped {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { libc::munmap(self.ptr.cast(), self.len) };
        }
    }
}

/// Поток кадров одного IPC-соединения.
pub struct FrameConn {
    pub output: String,
    pub cursor: bool,
    pub seq: u64,
    pub path: PathBuf,
    map: Mapped,
    /// Клиент ждёт кадр (`frame-next` без накопленных изменений).
    pub waiting: bool,
    /// На выводе были изменения после последнего кадра.
    pub dirty: bool,
    pub target: Option<Box<dyn Any>>,
    pub opts: StreamOpts,
    /// Заведён таймер досылки без потерь.
    pub refine_timer: bool,
}

// Отображение файла принадлежит одному потоку композитора.
unsafe impl Send for Mapped {}

impl FrameConn {
    pub fn new(id: u64, output: String, cursor: bool, video: Option<VideoRequest>) -> anyhow::Result<Self> {
        let dir = synshell_common::paths::runtime_dir().join("synwm-frames");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}-{id}.rgba", std::process::id()));
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new().create(true).truncate(true).read(true).write(true).mode(0o600).open(&path)?;
        Ok(Self {
            output,
            cursor,
            seq: 0,
            path,
            map: Mapped { file, ptr: std::ptr::null_mut(), len: 0 },
            waiting: false,
            dirty: true,
            target: None,
            opts: StreamOpts { video, ..Default::default() },
            refine_timer: false,
        })
    }

    /// Записать куски кадра и видеопакет в файл потока; смещение пакета.
    fn write(&mut self, r: &Rendered) -> std::io::Result<Option<u64>> {
        let stride = r.width as usize * 4;
        let frame = stride * r.height as usize;
        let video_len = r.video.as_ref().map_or(0, |v| v.1.len());
        self.map.ensure(frame + video_len)?;
        let buf = self.map.bytes();
        for ([x, y, w, _h], data) in &r.rects {
            let row = *w as usize * 4;
            for (i, line) in data.chunks_exact(row).enumerate() {
                let off = (*y as usize + i) * stride + *x as usize * 4;
                let dst = &mut buf[off..off + row];
                dst.copy_from_slice(line);
                // Альфа у кадра потока не нужна: непрозрачно.
                for px in dst.chunks_exact_mut(4) {
                    px[3] = 255;
                }
            }
        }
        Ok(match &r.video {
            Some((_, data, _)) => {
                buf[frame..frame + data.len()].copy_from_slice(data);
                Some(frame as u64)
            }
            None => None,
        })
    }
}

impl Drop for FrameConn {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl State {
    /// Нарисовать и записать кадр потока; ответ для клиента.
    pub fn stream_capture(&mut self, fc: &mut FrameConn) -> Result<FrameInfo, String> {
        let output = self
            .core
            .output_by_name(&fc.output)
            .or_else(|| self.core.space.outputs().next().cloned())
            .ok_or(t!("нет выводов"))?;
        if output.name() != fc.output {
            fc.output = output.name();
            fc.target = None;
        }
        let rendered = self
            .backend
            .stream_frame(&mut self.core, &output, &mut fc.target, fc.cursor, &mut fc.opts)
            .map_err(|e| t!("кадр потока: {e}", e = format!("{:#}", e)))?;
        let video_offset = fc.write(&rendered).map_err(|e| t!("файл кадра: {e}", e = e))?;
        if rendered.full {
            fc.seq = 0;
        } else {
            fc.seq += 1;
        }
        fc.dirty = false;
        fc.waiting = false;
        let scale = output.current_scale().fractional_scale();
        let geo = self.core.space.output_geometry(&output).unwrap_or_default();
        let p = self.core.pointer.current_location();
        let pointer = geo.to_f64().contains(p).then(|| [(p.x - geo.loc.x as f64) * scale, (p.y - geo.loc.y as f64) * scale]);
        let codec = fc.opts.encoder_codec();
        Ok(FrameInfo {
            output: output.name(),
            width: rendered.width as u32,
            height: rendered.height as u32,
            scale,
            seq: fc.seq,
            path: fc.path.to_string_lossy().into_owned(),
            rects: rendered.rects.iter().map(|(r, _)| *r).collect(),
            pointer,
            video: rendered.video.as_ref().zip(video_offset).map(|((key, data, rects), offset)| FrameVideo {
                codec: codec.clone(),
                key: *key,
                offset,
                len: data.len() as u64,
                rects: rects.clone(),
            }),
            video_error: fc.opts.video_error.take(),
        })
    }
}

impl StreamOpts {
    fn encoder_codec(&self) -> String {
        self.codec.to_string()
    }
}
