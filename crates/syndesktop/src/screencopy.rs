//! Захват экрана по протоколу wlr-screencopy-unstable-v1 (v3): grim,
//! wf-recorder, xdg-desktop-portal-wlr (демонстрация экрана в браузерах и
//! OBS через PipeWire).
//!
//! Кадр рисуется тем же построителем элементов, что и экран, прямо в буфер
//! клиента: dmabuf — рендером в него, shm — рендером в текстуру и чтением.
//! `copy_with_damage` ждёт следующего кадра вывода с изменениями, поэтому
//! запись экрана не крутит GPU впустую, пока ничего не меняется.

use std::sync::atomic::{AtomicBool, Ordering};

use smithay::{
    backend::{
        allocator::{dmabuf::Dmabuf, Buffer as _, Fourcc},
        renderer::{damage::OutputDamageTracker, gles::GlesRenderer, Bind, ExportMem, Offscreen},
    },
    output::Output,
    reexports::{
        wayland_protocols_wlr::screencopy::v1::server::{
            zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
            zwlr_screencopy_manager_v1::{self, ZwlrScreencopyManagerV1},
        },
        wayland_server::{
            protocol::{wl_buffer::WlBuffer, wl_shm},
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
        },
    },
    utils::{Buffer as BufferCoords, Physical, Rectangle, Size},
    wayland::{dmabuf::get_dmabuf, shm::with_buffer_contents_mut},
};

use crate::state::{ClientState, State};

pub struct FrameData {
    output: Output,
    /// Прямоугольник захвата в координатах буфера вывода (физические px).
    region: Rectangle<i32, BufferCoords>,
    /// Полный размер буфера вывода.
    full: Size<i32, BufferCoords>,
    overlay_cursor: bool,
    used: AtomicBool,
}

/// Отложенная копия «с повреждениями» — ждёт кадра вывода.
pub struct PendingCopy {
    pub frame: ZwlrScreencopyFrameV1,
    pub buffer: WlBuffer,
    pub since: std::time::Instant,
}

pub fn init(dh: &DisplayHandle) {
    dh.create_global::<State, ZwlrScreencopyManagerV1, _>(3, ());
}

impl GlobalDispatch<ZwlrScreencopyManagerV1, ()> for State {
    fn bind(
        _state: &mut State,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrScreencopyManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, ());
    }

    fn can_view(client: Client, _global_data: &()) -> bool {
        // Песочницы (flatpak через security-context) — только через портал.
        client.get_data::<ClientState>().is_none_or(|c| c.security_context.is_none())
    }
}

impl Dispatch<ZwlrScreencopyManagerV1, ()> for State {
    fn request(
        state: &mut State,
        _client: &Client,
        _manager: &ZwlrScreencopyManagerV1,
        request: zwlr_screencopy_manager_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        let (frame, overlay_cursor, output, region) = match request {
            zwlr_screencopy_manager_v1::Request::CaptureOutput { frame, overlay_cursor, output } => {
                (frame, overlay_cursor, output, None)
            }
            zwlr_screencopy_manager_v1::Request::CaptureOutputRegion { frame, overlay_cursor, output, x, y, width, height } => {
                (frame, overlay_cursor, output, Some(Rectangle::<i32, smithay::utils::Logical>::new((x, y).into(), (width, height).into())))
            }
            zwlr_screencopy_manager_v1::Request::Destroy => return,
            _ => return,
        };
        let Some(output) = Output::from_resource(&output) else {
            let f = data_init.init(frame, dummy());
            f.failed();
            return;
        };
        let Some(mode) = output.current_mode() else {
            let f = data_init.init(frame, dummy());
            f.failed();
            return;
        };
        let scale = output.current_scale().fractional_scale();
        let transform = output.current_transform();
        // Буфер вывода — в «сыром» положении режима (как при выводе на экран).
        let full = Size::<i32, BufferCoords>::from((mode.size.w, mode.size.h));
        let upright: Size<i32, Physical> = transform.transform_size(mode.size);
        let region = match region {
            None => Rectangle::from_size(full),
            Some(r) => {
                let phys = r.to_f64().to_physical(scale).to_i32_round::<i32>();
                let phys = phys.intersection(Rectangle::from_size(upright)).unwrap_or_default();
                let raw = transform.transform_rect_in(phys, &upright);
                Rectangle::new((raw.loc.x, raw.loc.y).into(), (raw.size.w, raw.size.h).into())
            }
        };
        let data = FrameData { output, region, full, overlay_cursor: overlay_cursor != 0, used: AtomicBool::new(false) };
        let frame = data_init.init(frame, data);
        if region.size.w <= 0 || region.size.h <= 0 {
            frame.failed();
            return;
        }
        let (w, h) = (region.size.w as u32, region.size.h as u32);
        frame.buffer(wl_shm::Format::Xrgb8888, w, h, w * 4);
        if frame.version() >= 3 {
            // dmabuf — только целый вывод: рисуем прямо в буфер клиента.
            if region.size == full {
                frame.linux_dmabuf(Fourcc::Xrgb8888 as u32, w, h);
            }
            frame.buffer_done();
        }
        let _ = state;
    }
}

fn dummy() -> FrameData {
    FrameData {
        output: Output::new("none".into(), smithay::output::PhysicalProperties {
            size: (0, 0).into(),
            subpixel: smithay::output::Subpixel::Unknown,
            make: String::new(),
            model: String::new(),
        }),
        region: Rectangle::default(),
        full: Size::default(),
        overlay_cursor: false,
        used: AtomicBool::new(true),
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, FrameData> for State {
    fn request(
        state: &mut State,
        _client: &Client,
        frame: &ZwlrScreencopyFrameV1,
        request: zwlr_screencopy_frame_v1::Request,
        data: &FrameData,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        let (buffer, with_damage) = match request {
            zwlr_screencopy_frame_v1::Request::Copy { buffer } => (buffer, false),
            zwlr_screencopy_frame_v1::Request::CopyWithDamage { buffer } => (buffer, true),
            zwlr_screencopy_frame_v1::Request::Destroy => return,
            _ => return,
        };
        if data.used.swap(true, Ordering::SeqCst) {
            frame.post_error(zwlr_screencopy_frame_v1::Error::AlreadyUsed, "кадр уже использован");
            return;
        }
        if !buffer_matches(&buffer, data) {
            frame.post_error(zwlr_screencopy_frame_v1::Error::InvalidBuffer, "неподходящий буфер");
            return;
        }
        if with_damage {
            // Ждём ближайшего кадра этого вывода с изменениями.
            state.core.pending_copies.push(PendingCopy { frame: frame.clone(), buffer, since: std::time::Instant::now() });
            let o = data.output.clone();
            state.core.queue_redraw(&o);
        } else {
            state.perform_copy(frame, &buffer, data, false);
        }
    }
}

fn buffer_matches(buffer: &WlBuffer, data: &FrameData) -> bool {
    if let Ok(dmabuf) = get_dmabuf(buffer) {
        let s = dmabuf.size();
        return data.region.size == data.full && s.w == data.full.w && s.h == data.full.h;
    }
    with_buffer_contents_mut(buffer, |_, len, b| {
        b.width == data.region.size.w
            && b.height == data.region.size.h
            && b.stride >= b.width * 4
            && (b.offset + b.stride * b.height) as usize <= len
            && matches!(b.format, wl_shm::Format::Xrgb8888 | wl_shm::Format::Argb8888)
    })
    .unwrap_or(false)
}

impl State {
    /// Выполнить копию в буфер клиента и сообщить результат.
    fn perform_copy(&mut self, frame: &ZwlrScreencopyFrameV1, buffer: &WlBuffer, data: &FrameData, damage: bool) {
        let output = data.output.clone();
        if !self.core.space.outputs().any(|o| o == &output) {
            frame.failed();
            return;
        }
        let core = &mut self.core;
        let res = self.backend.with_gles(|r| render_into(r, core, &output, data, buffer));
        match res {
            Some(Ok(())) => {
                frame.flags(zwlr_screencopy_frame_v1::Flags::empty());
                if damage && frame.version() >= 2 {
                    frame.damage(0, 0, data.region.size.w as u32, data.region.size.h as u32);
                }
                // Время — монотонные часы (как у presentation-time).
                let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
                unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
                let secs = ts.tv_sec as u64;
                frame.ready((secs >> 32) as u32, secs as u32, ts.tv_nsec as u32);
            }
            Some(Err(e)) => {
                tracing::warn!(?e, "screencopy: не удалось скопировать кадр");
                frame.failed();
            }
            None => frame.failed(),
        }
    }

    /// Раз в секунду: копии, ждущие дольше полусекунды (экран не меняется),
    /// отдаются как есть — иначе запись статичного экрана встала бы.
    pub fn flush_stale_copies(&mut self) {
        self.core.pending_copies.retain(|p| p.frame.is_alive());
        let stale: Vec<Output> = self
            .core
            .pending_copies
            .iter()
            .filter(|p| p.since.elapsed() >= std::time::Duration::from_millis(500))
            .filter_map(|p| p.frame.data::<FrameData>().map(|d| d.output.clone()))
            .collect();
        for o in stale {
            self.flush_pending_copies(&o);
        }
    }

    /// После кадра вывода с изменениями — отдать ждущие копии.
    pub fn flush_pending_copies(&mut self, output: &Output) {
        if self.core.pending_copies.is_empty() {
            return;
        }
        let (mine, rest): (Vec<PendingCopy>, Vec<PendingCopy>) =
            std::mem::take(&mut self.core.pending_copies).into_iter().partition(|p| {
                p.frame.data::<FrameData>().is_some_and(|d| &d.output == output)
            });
        self.core.pending_copies = rest;
        for p in mine {
            if !p.frame.is_alive() {
                continue;
            }
            let Some(data) = p.frame.data::<FrameData>() else { continue };
            // FrameData живёт в ресурсе — перенесём нужное в локальную копию.
            let local = FrameData {
                output: data.output.clone(),
                region: data.region,
                full: data.full,
                overlay_cursor: data.overlay_cursor,
                used: AtomicBool::new(true),
            };
            self.perform_copy(&p.frame, &p.buffer, &local, true);
        }
    }
}

fn render_into(
    renderer: &mut GlesRenderer,
    core: &mut crate::state::Core,
    output: &Output,
    data: &FrameData,
    buffer: &WlBuffer,
) -> anyhow::Result<()> {
    let mode = output.current_mode().ok_or_else(|| anyhow::anyhow!("нет режима"))?;
    let scale = output.current_scale().fractional_scale();
    let transform = output.current_transform();
    let (elements, clear) = crate::render::output_elements(core, renderer, output, data.overlay_cursor);
    let mut tracker = OutputDamageTracker::new(mode.size, scale, transform);

    if let Ok(dmabuf) = get_dmabuf(buffer) {
        let mut dmabuf: Dmabuf = dmabuf.clone();
        let mut fb = renderer.bind(&mut dmabuf)?;
        tracker.render_output(renderer, &mut fb, 0, &elements, clear)?;
        return Ok(());
    }

    // shm: рисуем в текстуру, читаем нужный прямоугольник.
    let size = Size::<i32, BufferCoords>::from((mode.size.w, mode.size.h));
    let mut texture: smithay::backend::renderer::gles::GlesTexture = renderer.create_buffer(Fourcc::Abgr8888, size)?;
    {
        let mut fb = renderer.bind(&mut texture)?;
        tracker.render_output(renderer, &mut fb, 0, &elements, clear)?;
    }
    let fb = renderer.bind(&mut texture)?;
    let mapping = renderer.copy_framebuffer(&fb, data.region, Fourcc::Abgr8888)?;
    let pixels = renderer.map_texture(&mapping)?;
    let (w, h) = (data.region.size.w as usize, data.region.size.h as usize);
    with_buffer_contents_mut(buffer, |ptr, len, b| {
        let stride = b.stride as usize;
        let offset = b.offset as usize;
        if offset + stride * h > len || pixels.len() < w * h * 4 {
            return;
        }
        let dst = unsafe { std::slice::from_raw_parts_mut(ptr.add(offset), stride * h) };
        for y in 0..h {
            let src_row = &pixels[y * w * 4..(y + 1) * w * 4];
            let dst_row = &mut dst[y * stride..y * stride + w * 4];
            // RGBA (байты) → XRGB8888 little-endian (байты B,G,R,X).
            for (s, d) in src_row.chunks_exact(4).zip(dst_row.chunks_exact_mut(4)) {
                d[0] = s[2];
                d[1] = s[1];
                d[2] = s[0];
                d[3] = 255;
            }
        }
    })?;
    Ok(())
}
