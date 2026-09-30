//! Поток кадров вывода для synlink (IPC `frame-stream` / `frame-next`).
//!
//! У каждого потока — свой постоянный буфер и `OutputDamageTracker`:
//! кадр дорисовывается только там, где что-то изменилось (у pixman на
//! телефоне это главное), и наружу уходят только эти прямоугольники.
//! Пиксели пишутся в файл потока в `$XDG_RUNTIME_DIR` (tmpfs), клиенту
//! уходит JSON с прямоугольниками — как у `capture`, без передачи fd.
//!
//! Поток тянущий: следующий кадр композитор готовит только после
//! `frame-next`, поэтому медленный клиент (Wi-Fi) сам снижает частоту, а
//! ненужные кадры не рисуются вовсе.

use std::any::Any;
use std::os::unix::fs::FileExt;
use std::path::PathBuf;

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{damage::OutputDamageTracker, Bind, ExportMem, ImportAll, ImportMem, Renderer, Texture},
    },
    output::Output,
    utils::{Buffer as BufferCoords, Physical, Rectangle, Size, Transform},
};
use synshell_common::ipc::FrameInfo;

use crate::state::{Core, State};

/// Результат отрисовки кадра потока: размер и изменившиеся куски RGBA.
pub struct Rendered {
    pub width: i32,
    pub height: i32,
    pub full: bool,
    pub rects: Vec<([i32; 4], Vec<u8>)>,
}

struct Target<B> {
    buf: B,
    tracker: OutputDamageTracker,
    size: Size<i32, BufferCoords>,
    scale: f64,
    fresh: bool,
}

/// Отрисовать кадр потока в постоянный буфер `slot` (создаётся заново,
/// если вывод сменил размер) и выгрузить изменения.
pub fn render<R, B>(
    core: &mut Core,
    r: &mut R,
    output: &Output,
    slot: &mut Option<Box<dyn Any>>,
    cursor: bool,
    create: impl FnOnce(&mut R, Size<i32, BufferCoords>) -> anyhow::Result<B>,
) -> anyhow::Result<Rendered>
where
    R: Renderer + ImportAll + ImportMem + Bind<B> + ExportMem,
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
    let damage: Vec<Rectangle<i32, Physical>> = {
        let mut fb = r.bind(&mut t.buf)?;
        let res = t.tracker.render_output(r, &mut fb, age, &elements, clear).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        res.damage.cloned().unwrap_or_default()
    };
    let full = t.fresh;
    t.fresh = false;
    let bounds = Rectangle::<i32, Physical>::from_size((size.w, size.h).into());
    let rects: Vec<Rectangle<i32, Physical>> =
        if full { vec![bounds] } else { merge(damage.into_iter().filter_map(|d| d.intersection(bounds)).collect()) };
    let fb = r.bind(&mut t.buf)?;
    let mut out = Vec::with_capacity(rects.len());
    for rc in rects {
        if rc.size.w <= 0 || rc.size.h <= 0 {
            continue;
        }
        let region = Rectangle::<i32, BufferCoords>::new((rc.loc.x, rc.loc.y).into(), (rc.size.w, rc.size.h).into());
        let mapping = r.copy_framebuffer(&fb, region, Fourcc::Abgr8888)?;
        let mut data = r.map_texture(&mapping)?.to_vec();
        for px in data.chunks_exact_mut(4) {
            px[3] = 255;
        }
        out.push(([rc.loc.x, rc.loc.y, rc.size.w, rc.size.h], data));
    }
    Ok(Rendered { width: size.w, height: size.h, full, rects: out })
}

/// Слить прямоугольники повреждений: пересекающиеся и соседние — в один;
/// если кусков много, чтение каждого дороже общего — берём охват.
fn merge(mut v: Vec<Rectangle<i32, Physical>>) -> Vec<Rectangle<i32, Physical>> {
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

/// Поток кадров одного IPC-соединения.
pub struct FrameConn {
    pub output: String,
    pub cursor: bool,
    pub seq: u64,
    pub path: PathBuf,
    file: std::fs::File,
    /// Клиент ждёт кадр (`frame-next` без накопленных изменений).
    pub waiting: bool,
    /// На выводе были изменения после последнего кадра.
    pub dirty: bool,
    pub target: Option<Box<dyn Any>>,
    size: (i32, i32),
}

impl FrameConn {
    pub fn new(id: u64, output: String, cursor: bool) -> anyhow::Result<Self> {
        let dir = synshell_common::paths::runtime_dir().join("synwm-frames");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}-{id}.rgba", std::process::id()));
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new().create(true).truncate(true).read(true).write(true).mode(0o600).open(&path)?;
        Ok(Self { output, cursor, seq: 0, path, file, waiting: false, dirty: true, target: None, size: (0, 0) })
    }

    /// Записать куски кадра в файл потока.
    fn write(&mut self, r: &Rendered) -> std::io::Result<()> {
        if self.size != (r.width, r.height) {
            self.file.set_len(r.width as u64 * r.height as u64 * 4)?;
            self.size = (r.width, r.height);
        }
        let stride = r.width as usize * 4;
        for ([x, y, w, _h], data) in &r.rects {
            let row = *w as usize * 4;
            for (i, line) in data.chunks_exact(row).enumerate() {
                let off = (*y as usize + i) * stride + *x as usize * 4;
                self.file.write_all_at(line, off as u64)?;
            }
        }
        Ok(())
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
            .ok_or("нет выводов")?;
        if output.name() != fc.output {
            fc.output = output.name();
            fc.target = None;
        }
        let rendered = self
            .backend
            .stream_frame(&mut self.core, &output, &mut fc.target, fc.cursor)
            .map_err(|e| format!("кадр потока: {e:#}"))?;
        fc.write(&rendered).map_err(|e| format!("файл кадра: {e}"))?;
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
        Ok(FrameInfo {
            output: output.name(),
            width: rendered.width as u32,
            height: rendered.height as u32,
            scale,
            seq: fc.seq,
            path: fc.path.to_string_lossy().into_owned(),
            rects: rendered.rects.iter().map(|(r, _)| *r).collect(),
            pointer,
        })
    }
}
