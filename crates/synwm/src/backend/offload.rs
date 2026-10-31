//! Композиция на отдельном GPU для вывода без своего рендеринга (`kms_cpu`).
//!
//! Случай: устройство вывода умеет только dumb-буферы (E Ink `onyx-epd`, simpledrm без kmsro в Mesa),
//! а в системе есть отдельный GPU с render-узлом (Adreno как `msm.separate_gpu_kms=1`: card1 без
//! коннекторов + renderD128). Кадр собирает GLES на этом узле во внеэкранной текстуре (клиенты рисуют на
//! GPU — linux-dmabuf с feedback на этот узел, их буферы импортируются в GLES, в том числе с тайлингом/UBWC),
//! изменённые прямоугольники читаются (`glReadPixels`) в теневой образ в памяти, а дальше прежний путь
//! `kms_cpu`: pixman копирует повреждения тени в dumb-буфер, `DrmCompositor` считает точные повреждения
//! (E Ink), режим пера и сторожа vblank — без изменений.
//!
//! Почему не рисовать GPU прямо в dumb-буфер вывода (как Mesa kmsro): GPU пишет через некогерентный IOMMU,
//! а драйвер вывода читает кадр процессором через кэш — без инвалидации в драйвере видны куски прошлых
//! кадров (SM-T295, synmobile docs/27 §27.10). Чтение кадра средствами GL синхронизировано самой Mesa.

use std::path::{Path, PathBuf};

use smithay::{
    backend::{
        allocator::{format::FormatSet, Fourcc},
        drm::{DrmDeviceFd, DrmNode},
        egl::{context::ContextPriority, EGLContext, EGLDevice, EGLDisplay},
        renderer::{
            damage::OutputDamageTracker,
            element::{Element, Id, Kind, RenderElement},
            gles::{GlesRenderer, GlesTexture},
            pixman::{PixmanRenderer, PixmanTexture},
            utils::{CommitCounter, DamageBag, DamageSet, OpaqueRegions},
            Bind, ExportMem, Frame, ImportDma, Offscreen, RendererSuper,
        },
    },
    output::Output,
    reexports::{gbm::Device as GbmDevice, pixman},
    utils::{Buffer as BufferCoords, DeviceFd, Physical, Rectangle, Scale, Size, Transform},
};

/// Переменная окружения: путь к render-узлу для композиции (`/dev/dri/renderD128`) или `none` — без GPU.
pub const ENV: &str = "SYNSHELL_RENDER_NODE";

/// Больше прямоугольников повреждений — читаем их общую рамку одним `glReadPixels`.
const MAX_READ_RECTS: usize = 8;

/// Render-узел отдельного GPU для вывода `display` без своего рендеринга: `SYNSHELL_RENDER_NODE`, иначе первый
/// `/dev/dri/renderD*`, на котором проба (`synwm --probe-gpu`, в дочернем процессе) нашла аппаратный EGL.
pub fn find_render_node(display: &Path) -> Option<PathBuf> {
    if let Ok(v) = std::env::var(ENV) {
        let v = v.trim();
        if v.is_empty() || matches!(v, "none" | "0" | "off" | "cpu") {
            tracing::info!("{ENV}={v}: композиция на CPU");
            return None;
        }
        return Some(PathBuf::from(v));
    }
    let own = DrmNode::from_path(display).ok().and_then(|n| n.node_with_type(smithay::backend::drm::NodeType::Render)?.ok());
    let mut nodes: Vec<PathBuf> = std::fs::read_dir("/dev/dri")
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("renderD")))
        // свой render-узел вывода уже проверен пробой GPU самого вывода
        .filter(|p| own.is_none_or(|o| DrmNode::from_path(p).ok() != Some(o)))
        .collect();
    nodes.sort();
    nodes.into_iter().find(|p| {
        let ok = super::probe_gpu(p);
        tracing::info!(node = %p.display(), ok, "проба отдельного GPU для композиции");
        ok
    })
}

/// GLES на отдельном render-узле.
pub struct GpuCompose {
    pub renderer: GlesRenderer,
    pub node: DrmNode,
    /// Формат чтения кадра: Argb8888 (BGRA, как XRGB8888 вывода) или Abgr8888 с перестановкой R и B.
    read_bgra: bool,
}

impl GpuCompose {
    pub fn new(path: &Path) -> anyhow::Result<Self> {
        let file = std::fs::OpenOptions::new().read(true).write(true).open(path)?;
        let fd = DrmDeviceFd::new(DeviceFd::from(std::os::fd::OwnedFd::from(file)));
        let node = DrmNode::from_path(path)?;
        let gbm = GbmDevice::new(fd)?;
        let display = unsafe { EGLDisplay::new(gbm)? };
        let device = EGLDevice::device_for_display(&display)?;
        if device.is_software() {
            anyhow::bail!("программный рендер EGL");
        }
        let ctx = EGLContext::new_with_priority(&display, ContextPriority::High)?;
        let renderer = unsafe { GlesRenderer::new(ctx)? };
        Ok(Self { renderer, node, read_bgra: true })
    }

    /// Форматы dma-buf, которые GPU импортирует как текстуры (для linux-dmabuf клиентов).
    pub fn dmabuf_formats(&self) -> FormatSet {
        self.renderer.dmabuf_formats()
    }

    /// Нарисовать кадр вывода в `frame` и перенести изменённое в тень.
    /// `None` — нечего показывать (повреждений нет); иначе состояния элементов кадра.
    pub fn render<E: RenderElement<GlesRenderer>>(
        &mut self,
        frame: &mut GpuFrame,
        elements: &[E],
        clear: smithay::backend::renderer::Color32F,
    ) -> anyhow::Result<(bool, smithay::backend::renderer::element::RenderElementStates)> {
        let (damage, states) = {
            let mut fb = self.renderer.bind(&mut frame.texture)?;
            let res = frame
                .tracker
                .render_output(&mut self.renderer, &mut fb, 1, elements, clear)
                .map_err(|e| anyhow::anyhow!("GLES: {e:?}"))?;
            (res.damage.cloned(), res.states)
        };
        let Some(damage) = damage.filter(|d| !d.is_empty()) else {
            return Ok((false, states));
        };
        self.read_back(frame, &damage)?;
        frame.shadow.damage.add(damage);
        Ok((true, states))
    }

    /// Прочитать повреждения (пространство вывода) из текстуры в тень.
    fn read_back(&mut self, frame: &mut GpuFrame, damage: &[Rectangle<i32, Physical>]) -> anyhow::Result<()> {
        let started = std::time::Instant::now();
        let transform = frame.transform;
        let space = transform.transform_size(frame.size);
        let bounds = Rectangle::<i32, Physical>::from_size(frame.size);
        // пространство вывода → буфер: обратный поворот (как у OutputDamageTracker/DrmCompositor)
        let mut rects: Vec<Rectangle<i32, Physical>> =
            damage.iter().filter_map(|d| transform.invert().transform_rect_in(*d, &space).intersection(bounds)).collect();
        if rects.len() > MAX_READ_RECTS {
            let all = rects.iter().skip(1).fold(rects[0], |a, r| a.merge(*r));
            rects = vec![all];
        }
        let fb = self.renderer.bind(&mut frame.texture)?;
        let mut maps = Vec::with_capacity(rects.len());
        for r in &rects {
            let region = Rectangle::<i32, BufferCoords>::new((r.loc.x, r.loc.y).into(), (r.size.w, r.size.h).into());
            let m = if self.read_bgra {
                match self.renderer.copy_framebuffer(&fb, region, Fourcc::Argb8888) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::info!(?e, "чтение кадра в BGRA не поддерживается — RGBA с перестановкой");
                        self.read_bgra = false;
                        self.renderer.copy_framebuffer(&fb, region, Fourcc::Abgr8888)?
                    }
                }
            } else {
                self.renderer.copy_framebuffer(&fb, region, Fourcc::Abgr8888)?
            };
            maps.push((*r, m));
        }
        drop(fb);
        let mut bytes = 0usize;
        for (r, m) in &maps {
            let data = self.renderer.map_texture(m)?;
            frame.shadow.write(*r, data, !self.read_bgra);
            bytes += data.len();
        }
        tracing::debug!(target: "synwm::frame", rects = rects.len(), bytes, us = started.elapsed().as_micros() as u64, "чтение кадра GPU");
        Ok(())
    }
}

/// Внеэкранный кадр одного вывода: текстура в ориентации буфера вывода (размер режима, поворот вывода уже
/// применён GLES) и тень в памяти.
pub struct GpuFrame {
    texture: GlesTexture,
    tracker: OutputDamageTracker,
    /// Размер режима (буфера), поворот вывода.
    size: Size<i32, Physical>,
    transform: Transform,
    pub shadow: Shadow,
}

impl GpuFrame {
    pub fn new(gpu: &mut GpuCompose, output: &Output) -> anyhow::Result<Self> {
        let mode = output.current_mode().ok_or_else(|| anyhow::anyhow!("у вывода нет режима"))?;
        let size: Size<i32, Physical> = mode.size;
        let buf = Size::<i32, BufferCoords>::from((size.w, size.h));
        let texture = <GlesRenderer as Offscreen<GlesTexture>>::create_buffer(&mut gpu.renderer, Fourcc::Argb8888, buf)
            .or_else(|_| <GlesRenderer as Offscreen<GlesTexture>>::create_buffer(&mut gpu.renderer, Fourcc::Abgr8888, buf))?;
        let transform = output.current_transform();
        Ok(Self {
            texture,
            tracker: OutputDamageTracker::from_output(output),
            size,
            transform,
            shadow: Shadow::new(size, transform)?,
        })
    }

    /// Кадр ещё подходит выводу (тот же режим и поворот).
    pub fn matches(&self, output: &Output) -> bool {
        output.current_mode().map(|m| m.size) == Some(self.size) && output.current_transform() == self.transform
    }
}

/// Копия кадра в памяти (x8r8g8b8, ориентация буфера вывода) — текстура pixman с историей повреждений.
pub struct Shadow {
    texture: PixmanTexture,
    data: *mut u8,
    stride: usize,
    size: Size<i32, Physical>,
    transform: Transform,
    id: Id,
    damage: DamageBag<i32, Physical>,
}

impl Shadow {
    fn new(size: Size<i32, Physical>, transform: Transform) -> anyhow::Result<Self> {
        let image = pixman::Image::new(pixman::FormatCode::X8R8G8B8, size.w as usize, size.h as usize, true)
            .map_err(|_| anyhow::anyhow!("pixman: тень {size:?}"))?;
        let data = unsafe { image.data() } as *mut u8;
        let stride = image.stride();
        Ok(Self {
            texture: PixmanTexture::from(image),
            data,
            stride,
            size,
            transform,
            id: Id::new(),
            damage: DamageBag::new(8),
        })
    }

    /// Записать прочитанный прямоугольник `r` (буфер; строки сверху вниз, 4 байта на пиксель, плотно).
    fn write(&mut self, r: Rectangle<i32, Physical>, src: &[u8], swap_rb: bool) {
        let (w, h) = (r.size.w as usize, r.size.h as usize);
        let row = w * 4;
        if src.len() < row * h || r.loc.x < 0 || r.loc.y < 0 || r.loc.x + r.size.w > self.size.w || r.loc.y + r.size.h > self.size.h {
            tracing::warn!(?r, len = src.len(), "чтение кадра: размер не сходится");
            return;
        }
        for y in 0..h {
            let s = &src[y * row..(y + 1) * row];
            // SAFETY: прямоугольник внутри образа (проверено выше), образ живёт в `texture`
            let d = unsafe {
                std::slice::from_raw_parts_mut(self.data.add((r.loc.y as usize + y) * self.stride + r.loc.x as usize * 4), row)
            };
            if swap_rb {
                for (d, s) in d.chunks_exact_mut(4).zip(s.chunks_exact(4)) {
                    d[0] = s[2];
                    d[1] = s[1];
                    d[2] = s[0];
                    d[3] = 0xff;
                }
            } else {
                d.copy_from_slice(s);
            }
        }
    }

    /// Элемент кадра для `DrmCompositor` (pixman): вся тень на весь вывод.
    pub fn element(&self) -> ShadowElement<'_> {
        ShadowElement { shadow: self }
    }
}

pub struct ShadowElement<'a> {
    shadow: &'a Shadow,
}

impl ShadowElement<'_> {
    fn space(&self) -> Size<i32, Physical> {
        self.shadow.transform.transform_size(self.shadow.size)
    }
}

impl Element for ShadowElement<'_> {
    fn id(&self) -> &Id {
        &self.shadow.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.shadow.damage.current_commit()
    }

    fn src(&self) -> Rectangle<f64, BufferCoords> {
        Rectangle::from_size((self.shadow.size.w as f64, self.shadow.size.h as f64).into())
    }

    /// Буфер тени уже повёрнут, как буфер вывода: тот же поворот у элемента взаимно гасит поворот кадра
    /// (pixman копирует без преобразования).
    fn transform(&self) -> Transform {
        self.shadow.transform
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        Rectangle::from_size(self.space())
    }

    fn damage_since(&self, _scale: Scale<f64>, commit: Option<CommitCounter>) -> DamageSet<i32, Physical> {
        self.shadow
            .damage
            .damage_since(commit)
            .unwrap_or_else(|| DamageSet::from_slice(&[Rectangle::from_size(self.space())]))
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::from_slice(&[Rectangle::from_size(self.space())])
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

impl RenderElement<PixmanRenderer> for ShadowElement<'_> {
    fn draw(
        &self,
        frame: &mut <PixmanRenderer as RendererSuper>::Frame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), <PixmanRenderer as RendererSuper>::Error> {
        frame.render_texture_from_to(&self.shadow.texture, src, dst, damage, opaque_regions, self.shadow.transform, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::backend::renderer::{
        element::solid::{SolidColorBuffer, SolidColorRenderElement},
        Color32F,
    };
    use smithay::output::{Mode, PhysicalProperties, Subpixel};

    const ALL: [Transform; 8] = [
        Transform::Normal,
        Transform::_90,
        Transform::_180,
        Transform::_270,
        Transform::Flipped,
        Transform::Flipped90,
        Transform::Flipped180,
        Transform::Flipped270,
    ];

    fn read_xrgb(pixman: &mut PixmanRenderer, target: &mut pixman::Image<'static, 'static>, buf: Size<i32, BufferCoords>) -> Vec<u8> {
        let fb = pixman.bind(target).unwrap();
        let m = pixman.copy_framebuffer(&fb, Rectangle::from_size(buf), Fourcc::Xrgb8888).unwrap();
        pixman.map_texture(&m).unwrap().to_vec()
    }

    /// Тень, повёрнутая как буфер вывода, копируется в буфер вывода без поворота (взаимно гасится).
    #[test]
    fn shadow_cancels_output_transform() {
        let mut pixman = PixmanRenderer::new().unwrap();
        let size: Size<i32, Physical> = (8, 4).into();
        let buf = Size::<i32, BufferCoords>::from((size.w, size.h));
        for transform in ALL {
            let mut shadow = Shadow::new(size, transform).unwrap();
            // пиксели тени: значение = номер пикселя
            let mut src = Vec::new();
            for i in 0..(size.w * size.h) as u32 {
                src.extend_from_slice(&(i | 0xff00_0000).to_le_bytes());
            }
            shadow.write(Rectangle::from_size(size), &src, false);
            let space = transform.transform_size(size);
            shadow.damage.add([Rectangle::from_size(space)]);
            let mut target = <PixmanRenderer as Offscreen<pixman::Image<'static, 'static>>>::create_buffer(&mut pixman, Fourcc::Xrgb8888, buf).unwrap();
            {
                let mut fb = pixman.bind(&mut target).unwrap();
                let mut tracker = OutputDamageTracker::new(size, 1.0, transform);
                tracker.render_output(&mut pixman, &mut fb, 0, &[shadow.element()], Color32F::BLACK).unwrap();
            }
            let out = read_xrgb(&mut pixman, &mut target, buf);
            for (i, px) in out.chunks_exact(4).enumerate() {
                let v = u32::from_le_bytes([px[0], px[1], px[2], 0]);
                assert_eq!(v, i as u32, "{transform:?}: пиксель {i}");
            }
        }
    }

    /// Кадр GLES на render-узле хоста → тень → буфер вывода pixman совпадает с тем же кадром, нарисованным
    /// pixman напрямую (ориентация чтения и поворот). Без GPU на хосте тест пропускается.
    #[test]
    fn gpu_frame_matches_pixman() {
        let node = std::env::var("SYNWM_TEST_RENDER_NODE").unwrap_or_else(|_| "/dev/dri/renderD128".into());
        let mut gpu = match GpuCompose::new(Path::new(&node)) {
            Ok(g) => g,
            Err(e) => {
                eprintln!("пропуск: нет GPU {node}: {e}");
                return;
            }
        };
        let mut pixman = PixmanRenderer::new().unwrap();
        let mode = Mode { size: (40, 24).into(), refresh: 60_000 };
        let buf = Size::<i32, BufferCoords>::from((mode.size.w, mode.size.h));
        for transform in ALL {
            let output = Output::new(
                "test".into(),
                PhysicalProperties { size: (0, 0).into(), subpixel: Subpixel::Unknown, make: "t".into(), model: "t".into() },
            );
            output.change_current_state(Some(mode), Some(transform), Some(smithay::output::Scale::Integer(1)), None);
            let red = SolidColorBuffer::new((7, 5), [1.0, 0.0, 0.0, 1.0]);
            let blue = SolidColorBuffer::new((3, 9), [0.0, 0.0, 1.0, 1.0]);
            let mut green = SolidColorBuffer::new((2, 2), [0.0, 1.0, 0.0, 1.0]);
            let els = |green: &SolidColorBuffer, gx: i32| {
                vec![
                    SolidColorRenderElement::from_buffer(&red, (2, 3), 1.0, 1.0, Kind::Unspecified),
                    SolidColorRenderElement::from_buffer(&blue, (15, 1), 1.0, 1.0, Kind::Unspecified),
                    SolidColorRenderElement::from_buffer(green, (gx, 10), 1.0, 1.0, Kind::Unspecified),
                ]
            };
            let mut frame = GpuFrame::new(&mut gpu, &output).unwrap();
            let mut target = <PixmanRenderer as Offscreen<pixman::Image<'static, 'static>>>::create_buffer(&mut pixman, Fourcc::Xrgb8888, buf).unwrap();
            let mut tracker = OutputDamageTracker::new(mode.size, 1.0, transform);
            // два кадра: второй — только повреждения (зелёный квадрат переехал)
            for (step, gx) in [(0, 4), (1, 9)] {
                if step == 1 {
                    green.update((2, 2), [0.0, 1.0, 0.0, 1.0]);
                }
                let (shown, _) = gpu.render(&mut frame, &els(&green, gx), Color32F::new(0.5, 0.5, 0.5, 1.0)).unwrap();
                assert!(shown);
                let mut fb = pixman.bind(&mut target).unwrap();
                tracker.render_output(&mut pixman, &mut fb, 1, &[frame.shadow.element()], Color32F::BLACK).unwrap();
            }
            let got = read_xrgb(&mut pixman, &mut target, buf);
            let mut reference = <PixmanRenderer as Offscreen<pixman::Image<'static, 'static>>>::create_buffer(&mut pixman, Fourcc::Xrgb8888, buf).unwrap();
            {
                let mut fb = pixman.bind(&mut reference).unwrap();
                let mut t = OutputDamageTracker::new(mode.size, 1.0, transform);
                t.render_output(&mut pixman, &mut fb, 0, &els(&green, 9), Color32F::new(0.5, 0.5, 0.5, 1.0)).unwrap();
            }
            let want = read_xrgb(&mut pixman, &mut reference, buf);
            let diff = got
                .chunks_exact(4)
                .zip(want.chunks_exact(4))
                .filter(|(a, b)| a[..3].iter().zip(&b[..3]).any(|(x, y)| x.abs_diff(*y) > 2))
                .count();
            assert_eq!(diff, 0, "{transform:?}: {diff} пикселей отличаются");
        }
    }
}
