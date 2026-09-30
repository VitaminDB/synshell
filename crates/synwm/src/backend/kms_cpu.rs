//! Сеанс DRM/KMS без GPU-рендеринга: pixman + dumb-буферы.
//!
//! Для телефонов с downstream-ядром (Qualcomm msm_drm/sde и т. п.), где нет
//! GBM/EGL для KMS-устройства, а GPU доступен только клиентам через Vulkan
//! (turnip/KGSL). Композитор рисует на CPU в dumb-буфер и показывает его через
//! atomic KMS; клиентские dma-buf с линейным модификатором импортируются через
//! mmap, поэтому Vulkan-клиенты рисуют на GPU, а композит — на CPU.
//!
//! Одно DRM-устройство (`SYNSHELL_DRM_DEVICE` или основное по udev), без
//! горячего подключения GPU. Ввод — libinput через libseat (на телефоне
//! `LIBSEAT_BACKEND=noop`, VT в ядре нет).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use smithay::{
    backend::{
        allocator::{
            dumb::DumbAllocator,
            format::FormatSet,
            Fourcc,
        },
        drm::{
            compositor::FrameFlags,
            output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements},
            DrmDevice, DrmDeviceFd, DrmEvent, DrmEventMetadata, DrmEventTime, DrmNode,
        },
        renderer::{
            damage::OutputDamageTracker,
            pixman::PixmanRenderer,
            Bind, ExportMem, ImportDma, ImportMemWl, Offscreen,
        },
        session::{libseat::LibSeatSession, Event as SessionEvent, Session},
        udev::{all_gpus, primary_gpu},
    },
    desktop::utils::OutputPresentationFeedback,
    output::{Mode as WlMode, Output, PhysicalProperties, Scale},
    reexports::{
        calloop::{
            timer::{TimeoutAction, Timer},
            EventLoop, LoopHandle, RegistrationToken,
        },
        drm::control::{connector, crtc, Device as ControlDevice, ModeTypeFlags},
        gbm::Device as GbmDevice,
        input::Libinput,
        rustix::fs::OFlags,
        wayland_protocols::wp::presentation_time::server::wp_presentation_feedback,
        wayland_server::backend::GlobalId,
    },
    utils::{DeviceFd, Rectangle, Size, Transform},
    wayland::presentation::Refresh,
};

use crate::{
    render::OutputElement,
    state::{Core, RedrawState, State},
};

// DrmOutputManager требует Clone-аллокатор — оборачиваем dumb в Arc<Mutex<_>>.
type Alloc = Arc<Mutex<DumbAllocator>>;
type Manager = DrmOutputManager<Alloc, DrmDeviceFd, Option<OutputPresentationFeedback>, DrmDeviceFd>;
type Out = DrmOutput<Alloc, DrmDeviceFd, Option<OutputPresentationFeedback>, DrmDeviceFd>;

const COLOR_FORMATS: &[Fourcc] = &[Fourcc::Xrgb8888, Fourcc::Argb8888, Fourcc::Xbgr8888, Fourcc::Abgr8888];

struct Surface {
    output: Output,
    drm_output: Out,
    global: Option<GlobalId>,
    connector: connector::Handle,
    estimated_vblank: Option<RegistrationToken>,
    /// Сторож потерянного page-flip: downstream-драйверы (sde) иногда не
    /// присылают событие vblank, и без него кадры остановились бы навсегда.
    flip_watchdog: Option<RegistrationToken>,
}

/// Сколько ждать vblank после отправки кадра, прежде чем считать событие потерянным.
const FLIP_WATCHDOG: Duration = Duration::from_millis(250);

pub struct KmsCpuBackend {
    session: LibSeatSession,
    libinput: Libinput,
    node: DrmNode,
    path: PathBuf,
    renderer: PixmanRenderer,
    manager: Option<Manager>,
    surfaces: HashMap<crtc::Handle, Surface>,
    loop_handle: LoopHandle<'static, State>,
    active: bool,
}

/// Путь к DRM-устройству: `SYNSHELL_DRM_DEVICE`, иначе основной GPU по udev.
pub fn device_path(seat: &str) -> anyhow::Result<PathBuf> {
    if let Ok(p) = std::env::var("SYNSHELL_DRM_DEVICE") {
        return Ok(PathBuf::from(p));
    }
    primary_gpu(seat)?
        .or_else(|| all_gpus(seat).ok()?.into_iter().next())
        .ok_or_else(|| anyhow::anyhow!("нет DRM-устройства"))
}

impl KmsCpuBackend {
    pub fn new(event_loop: &EventLoop<'static, State>) -> anyhow::Result<Self> {
        let (session, notifier) = LibSeatSession::new().map_err(|e| anyhow::anyhow!("libseat: {e}"))?;
        let seat = session.seat();
        let path = device_path(&seat)?;
        let node = DrmNode::from_path(&path)?;
        tracing::info!(path = %path.display(), "DRM-устройство (CPU-рендер)");
        let renderer = PixmanRenderer::new().map_err(|e| anyhow::anyhow!("pixman: {e}"))?;
        let (libinput, _) = super::init_libinput(event_loop, &session)?;

        event_loop.handle().insert_source(notifier, |event, _, state| match event {
            SessionEvent::PauseSession => {
                if let crate::backend::Backend::KmsCpu(b) = &mut state.backend {
                    b.active = false;
                    b.libinput.suspend();
                    if let Some(m) = b.manager.as_mut() {
                        m.pause();
                    }
                }
            }
            SessionEvent::ActivateSession => {
                if let crate::backend::Backend::KmsCpu(b) = &mut state.backend {
                    b.active = true;
                    if let Err(e) = b.libinput.resume() {
                        tracing::error!(?e, "libinput resume");
                    }
                    if let Some(m) = b.manager.as_mut() {
                        if let Err(e) = m.activate(false) {
                            tracing::error!(?e, "DRM activate");
                        }
                    }
                }
                for d in state.core.output_data.values_mut() {
                    d.redraw = RedrawState::Queued;
                }
                state.core.queue_redraw_all();
            }
        }).map_err(|e| anyhow::anyhow!("{}", e.error))?;

        Ok(Self {
            session,
            libinput,
            node,
            path,
            renderer,
            manager: None,
            surfaces: HashMap::new(),
            loop_handle: event_loop.handle(),
            active: true,
        })
    }

    pub fn seat_name(&self) -> String {
        self.session.seat()
    }

    pub fn import_dmabuf(&mut self, dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf) -> bool {
        self.renderer.import_dmabuf(dmabuf, None).is_ok()
    }

    pub fn change_vt(&mut self, vt: i32) {
        if let Err(e) = self.session.change_vt(vt) {
            tracing::warn!(vt, ?e, "не удалось сменить VT");
        }
    }
}

/// Открыть устройство, создать глобалы и включить мониторы — после `State`.
pub fn init(state: &mut State, _event_loop: &EventLoop<'static, State>) -> anyhow::Result<()> {
    let crate::backend::Backend::KmsCpu(b) = &mut state.backend else { unreachable!() };
    let fd = b
        .session
        .open(&b.path, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK)
        .map_err(|e| anyhow::anyhow!("open {}: {e}", b.path.display()))?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));
    let (drm, notifier) = DrmDevice::new(fd.clone(), true)?;
    let node = b.node;
    b.loop_handle.insert_source(notifier, move |event, meta, state| match event {
        DrmEvent::VBlank(crtc) => on_vblank(state, crtc, meta),
        DrmEvent::Error(e) => tracing::error!(?e, "DRM"),
    })?;
    let allocator: Alloc = Arc::new(Mutex::new(DumbAllocator::new(fd.clone())));
    // pixman отдаёт только Linear; DrmOutputManager без поддержки модификаторов у
    // драйвера (или при откате на implicit) оставляет лишь Invalid — dumb-буферы
    // и так линейные, поэтому добавляем те же форматы с Modifier::Invalid.
    let linear_formats: FormatSet = b.renderer.dmabuf_formats();
    let render_formats: FormatSet = linear_formats
        .iter()
        .flat_map(|f| [*f, smithay::backend::allocator::Format { code: f.code, modifier: smithay::backend::allocator::Modifier::Invalid }])
        .collect();
    b.manager = Some(DrmOutputManager::new(
        drm,
        allocator,
        fd,
        None::<GbmDevice<DrmDeviceFd>>,
        COLOR_FORMATS.iter().copied(),
        render_formats,
    ));

    // Глобалы: shm и linux-dmabuf (линейные форматы — клиенты на GPU отдают
    // кадры, которые можно прочитать через mmap). Версия 3, без feedback:
    // с feedback Mesa WSI сравнивает «главное устройство» композитора с DRM-узлом
    // Vulkan-драйвера, а у turnip/KGSL его нет — WSI уходит в prime-blit и падает.
    // Без feedback same_gpu = true и клиент отдаёт свой линейный dma-buf напрямую.
    state.core.shm_state.update_formats(b.renderer.shm_formats());
    let _ = node;
    let global = state
        .core
        .dmabuf_state
        .create_global::<State>(&state.core.display_handle, linear_formats.iter().copied());
    state.core.dmabuf_global = Some(global);

    scan_connectors(state);
    Ok(())
}

fn remove_output(core: &mut Core, output: &Output) {
    core.space.unmap_output(output);
    core.output_data.remove(output);
    core.lock_surfaces.remove(output);
    let layers: Vec<_> = smithay::desktop::layer_map_for_output(output).layers().cloned().collect();
    for l in layers {
        l.layer_surface().send_close();
    }
}

/// Сверить подключённые панели/мониторы с выводами.
pub fn scan_connectors(state: &mut State) {
    let crate::backend::Backend::KmsCpu(b) = &mut state.backend else { return };
    let Some(manager) = b.manager.as_mut() else { return };
    let drm = manager.device();
    let Ok(res) = drm.resource_handles() else { return };
    let mut connected = Vec::new();
    for &conn in res.connectors() {
        if let Ok(info) = drm.get_connector(conn, true) {
            if info.state() == connector::State::Connected {
                connected.push(info);
            }
        }
    }
    let gone: Vec<crtc::Handle> = b
        .surfaces
        .iter()
        .filter(|(_, s)| !connected.iter().any(|c| c.handle() == s.connector))
        .map(|(c, _)| *c)
        .collect();
    let mut removed = Vec::new();
    for crtc in gone {
        if let Some(s) = b.surfaces.remove(&crtc) {
            tracing::info!(output = s.output.name(), "вывод отключён");
            if let Some(g) = s.global {
                state.core.display_handle.remove_global::<State>(g);
            }
            removed.push(s.output);
        }
    }
    for o in &removed {
        remove_output(&mut state.core, o);
    }
    let mut added = false;
    for info in connected {
        let crate::backend::Backend::KmsCpu(b) = &mut state.backend else { return };
        let manager = b.manager.as_mut().unwrap();
        if b.surfaces.values().any(|s| s.connector == info.handle()) {
            continue;
        }
        let name = format!("{}-{}", info.interface().as_str(), info.interface_id());
        if state.core.config.output_ignored(&name) {
            tracing::info!(name, "вывод пропущен (platform.ignore_outputs)");
            continue;
        }
        let cfg = state.core.config.outputs.iter().find(|o| o.name == name).cloned();
        if cfg.as_ref().is_some_and(|c| !c.enabled) {
            tracing::info!(name, "вывод выключен в конфиге");
            continue;
        }
        let drm = manager.device();
        let used: Vec<crtc::Handle> = b.surfaces.keys().copied().collect();
        let crtc = info
            .encoders()
            .iter()
            .filter_map(|e| drm.get_encoder(*e).ok())
            .flat_map(|e| res.filter_crtcs(e.possible_crtcs()))
            .find(|c| !used.contains(c));
        let Some(crtc) = crtc else {
            tracing::warn!(name, "нет свободного CRTC");
            continue;
        };
        let modes = info.modes();
        if modes.is_empty() {
            continue;
        }
        let mode = cfg
            .as_ref()
            .and_then(|c| super::tty::pick_mode(modes, &c.mode))
            .or_else(|| modes.iter().find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED)).copied())
            .unwrap_or(modes[0]);
        let (mm_w, mm_h) = info.size().unwrap_or((0, 0));
        let output = Output::new(
            name.clone(),
            PhysicalProperties {
                size: (mm_w as i32, mm_h as i32).into(),
                subpixel: info.subpixel().into(),
                make: "Unknown".into(),
                model: name.clone(),
            },
        );
        let wl_mode = WlMode::from(mode);
        for m in modes {
            output.add_mode(WlMode::from(*m));
        }
        output.set_preferred(wl_mode);
        let scale = cfg
            .as_ref()
            .map(|c| c.scale)
            .filter(|s| *s > 0.0)
            .unwrap_or_else(|| {
                if state.core.form_factor == synshell_common::config::FormFactor::Phone {
                    crate::backend::phone_scale((wl_mode.size.w, wl_mode.size.h))
                } else {
                    crate::backend::auto_scale((mm_w as i32, mm_h as i32), (wl_mode.size.w, wl_mode.size.h))
                }
            });
        let transform = cfg.as_ref().map(|c| crate::backend::parse_transform(&c.transform)).unwrap_or(Transform::Normal);
        output.change_current_state(Some(wl_mode), Some(transform), Some(Scale::Fractional(scale)), None);
        output.user_data().insert_if_missing(|| crtc);

        // Только primary-план: наложение и курсор на CPU-пути рисуются в кадр.
        let planes = drm.planes(&crtc).ok().map(|mut p| {
            p.overlay.clear();
            p.cursor.clear();
            p
        });
        let drm_output = match manager.initialize_output::<_, OutputElement<PixmanRenderer>>(
            crtc,
            mode,
            &[info.handle()],
            &output,
            planes,
            &mut b.renderer,
            &DrmOutputRenderElements::default(),
        ) {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!(name, ?e, "не удалось включить вывод");
                continue;
            }
        };
        tracing::info!(name, mode = ?wl_mode, scale, "вывод подключён (CPU-рендер)");
        let global = output.create_global::<State>(&state.core.display_handle);
        b.surfaces.insert(
            crtc,
            Surface {
                output: output.clone(),
                drm_output,
                global: Some(global),
                connector: info.handle(),
                estimated_vblank: None,
                flip_watchdog: None,
            },
        );
        let pos = cfg.as_ref().and_then(|c| c.position).map(|[x, y]| (x, y).into()).unwrap_or_else(|| {
            let x = state
                .core
                .space
                .outputs()
                .filter_map(|o| state.core.space.output_geometry(o))
                .map(|g| g.loc.x + g.size.w)
                .max()
                .unwrap_or(0);
            smithay::utils::Point::from((x, 0))
        });
        output.change_current_state(None, None, None, Some(pos));
        state.core.space.map_output(&output, pos);
        state.core.output_data.entry(output.clone()).or_default().redraw = RedrawState::Queued;
        added = true;
    }
    if added || !removed.is_empty() {
        state.outputs_changed();
        state.core.queue_redraw_all();
    }
}

fn surface_of<'a>(b: &'a mut KmsCpuBackend, output: &Output) -> Option<&'a mut Surface> {
    let crtc = *output.user_data().get::<crtc::Handle>()?;
    b.surfaces.get_mut(&crtc)
}

impl KmsCpuBackend {
    pub fn render(&mut self, core: &mut Core, output: &Output) {
        if !self.active {
            return;
        }
        if core.monitors_off {
            if let Some(d) = core.output_data.get_mut(output) {
                d.redraw = RedrawState::Idle;
            }
            return;
        }
        let Some(crtc) = output.user_data().get::<crtc::Handle>().copied() else { return };
        let Some(surface) = self.surfaces.get_mut(&crtc) else { return };
        let (elements, clear) = crate::render::output_elements(core, &mut self.renderer, output, true);
        // Всё в один primary-кадр: прямой вывод клиентских буферов на планы
        // downstream-драйверов ненадёжен.
        let res = surface.drm_output.render_frame(&mut self.renderer, &elements, clear, FrameFlags::empty());
        let (rendered, states) = match res {
            Ok(r) => (!r.is_empty, r.states),
            Err(e) => {
                tracing::warn!(?e, output = output.name(), "кадр не нарисован");
                if let Some(d) = core.output_data.get_mut(output) {
                    d.redraw = RedrawState::Idle;
                }
                return;
            }
        };
        drop(elements);
        core.post_repaint(output, &states);

        let data = core.output_data.entry(output.clone()).or_default();
        data.damaged = rendered;
        if rendered {
            let feedback = core.take_presentation_feedback(output, &states);
            match surface.drm_output.queue_frame(Some(feedback)) {
                Ok(()) => {
                    let data = core.output_data.entry(output.clone()).or_default();
                    data.redraw = RedrawState::WaitingForVBlank { redraw_needed: false };
                    data.frames += 1;
                    data.last_frame = std::time::Instant::now();
                    if let Some(t) = surface.flip_watchdog.take() {
                        self.loop_handle.remove(t);
                    }
                    let o = output.clone();
                    surface.flip_watchdog = self
                        .loop_handle
                        .insert_source(Timer::from_duration(FLIP_WATCHDOG), move |_, _, state| {
                            flip_timeout(state, &o);
                            TimeoutAction::Drop
                        })
                        .ok();
                }
                Err(e) => {
                    tracing::warn!(?e, "queue_frame");
                    let data = core.output_data.entry(output.clone()).or_default();
                    data.redraw = RedrawState::Idle;
                }
            }
        } else {
            data.redraw = RedrawState::WaitingForEstimatedVBlank { redraw_needed: false };
            let refresh = output.current_mode().map(|m| m.refresh).unwrap_or(60_000).max(1000);
            let dur = Duration::from_micros(1_000_000_000 / refresh as u64);
            if surface.estimated_vblank.is_none() {
                let o = output.clone();
                let token = self
                    .loop_handle
                    .insert_source(Timer::from_duration(dur), move |_, _, state| {
                        if let crate::backend::Backend::KmsCpu(b) = &mut state.backend {
                            if let Some(s) = surface_of(b, &o) {
                                s.estimated_vblank = None;
                            }
                        }
                        let states = smithay::backend::renderer::element::RenderElementStates::default();
                        state.core.post_repaint(&o, &states);
                        super::tty::after_frame(&mut state.core, &o);
                        TimeoutAction::Drop
                    })
                    .ok();
                surface.estimated_vblank = token;
            }
        }
    }

    pub fn set_monitors_power(&mut self, core: &mut Core, on: bool) {
        for s in self.surfaces.values_mut() {
            if !on {
                let _ = s.drm_output.with_compositor(|c| c.clear());
            }
            if let Some(data) = core.output_data.get_mut(&s.output) {
                data.redraw = if on { RedrawState::Queued } else { RedrawState::Idle };
                data.powered_off = !on;
            }
        }
    }

    pub fn screenshot(&mut self, core: &mut Core, output: &Output) -> anyhow::Result<(u32, u32, Vec<u8>)> {
        let size = output.current_mode().map(|m| m.size).unwrap_or_default();
        let transformed = output.current_transform().transform_size(size);
        let buffer_size = Size::from((transformed.w, transformed.h));
        let r = &mut self.renderer;
        let mut image = <PixmanRenderer as Offscreen<_>>::create_buffer(r, Fourcc::Abgr8888, buffer_size)?;
        let (elements, clear) = crate::render::output_elements(core, r, output, false);
        {
            let mut fb = r.bind(&mut image)?;
            let mut tracker = OutputDamageTracker::new(transformed, output.current_scale().fractional_scale(), Transform::Normal);
            tracker.render_output(r, &mut fb, 0, &elements, clear)?;
        }
        let fb = r.bind(&mut image)?;
        let mapping = r.copy_framebuffer(&fb, Rectangle::from_size(buffer_size), Fourcc::Abgr8888)?;
        let data = r.map_texture(&mapping)?.to_vec();
        Ok((buffer_size.w as u32, buffer_size.h as u32, data))
    }

    pub fn stream_frame(
        &mut self,
        core: &mut Core,
        output: &Output,
        slot: &mut Option<Box<dyn std::any::Any>>,
        cursor: bool,
        opts: &mut crate::stream::StreamOpts,
    ) -> anyhow::Result<crate::stream::Rendered> {
        crate::stream::render(core, &mut self.renderer, output, slot, cursor, opts, |r, size| {
            Ok(<PixmanRenderer as Offscreen<smithay::reexports::pixman::Image<'static, 'static>>>::create_buffer(r, Fourcc::Abgr8888, size)?)
        }, None)
    }

    /// Применить `[[output]]`: масштаб, поворот, положение, режим.
    pub fn apply_output_config(&mut self, core: &mut Core) {
        let Some(manager) = self.manager.as_mut() else { return };
        for s in self.surfaces.values_mut() {
            let name = s.output.name();
            s.output.change_current_state(None, Some(crate::backend::output_transform(core, &s.output)), None, None);
            let Some(cfg) = core.config.outputs.iter().find(|o| crate::backend::output_matches(&s.output, &o.name)) else {
                continue;
            };
            if cfg.scale > 0.0 {
                s.output.change_current_state(None, None, Some(Scale::Fractional(cfg.scale)), None);
            }
            if let Some([x, y]) = cfg.position {
                s.output.change_current_state(None, None, None, Some((x, y).into()));
                core.space.map_output(&s.output, (x, y));
            }
            if !cfg.mode.is_empty() {
                let drm = manager.device();
                if let Ok(info) = drm.get_connector(s.connector, false) {
                    if let Some(mode) = super::tty::pick_mode(info.modes(), &cfg.mode) {
                        if s.output.current_mode() != Some(WlMode::from(mode)) {
                            match s.drm_output.use_mode::<_, OutputElement<PixmanRenderer>>(mode, &mut self.renderer, &DrmOutputRenderElements::default()) {
                                Ok(()) => {
                                    s.output.change_current_state(Some(WlMode::from(mode)), None, None, None);
                                    tracing::info!(name, ?mode, "режим изменён");
                                }
                                Err(e) => tracing::warn!(name, ?e, "режим не применён"),
                            }
                        }
                    }
                }
            }
        }
    }
}

/// vblank не пришёл вовремя: считаем кадр показанным, чтобы не зависнуть.
fn flip_timeout(state: &mut State, output: &Output) {
    let waiting = matches!(state.core.output_data.get(output).map(|d| d.redraw), Some(RedrawState::WaitingForVBlank { .. }));
    let crate::backend::Backend::KmsCpu(b) = &mut state.backend else { return };
    let Some(surface) = surface_of(b, output) else { return };
    surface.flip_watchdog = None;
    if !waiting {
        return;
    }
    tracing::warn!(output = output.name(), "vblank не пришёл за {FLIP_WATCHDOG:?} — считаем кадр показанным");
    match surface.drm_output.frame_submitted() {
        Ok(Some(Some(mut feedback))) => feedback.discarded(),
        Ok(_) => {}
        Err(e) => tracing::warn!(?e, "frame_submitted"),
    }
    super::tty::after_frame(&mut state.core, output);
}

fn on_vblank(state: &mut State, crtc: crtc::Handle, meta: &mut Option<DrmEventMetadata>) {
    let crate::backend::Backend::KmsCpu(b) = &mut state.backend else { return };
    let Some(surface) = b.surfaces.get_mut(&crtc) else { return };
    let output = surface.output.clone();
    if let Some(t) = surface.flip_watchdog.take() {
        b.loop_handle.remove(t);
    }
    let Some(surface) = b.surfaces.get_mut(&crtc) else { return };
    let refresh = output.current_mode().map(|m| m.refresh).unwrap_or(60_000).max(1000);
    let frame_duration = Duration::from_micros(1_000_000_000 / refresh as u64);
    let (clock, flags) = match meta.as_ref().map(|m| m.time) {
        Some(DrmEventTime::Monotonic(tp)) if !tp.is_zero() => (
            tp.into(),
            wp_presentation_feedback::Kind::Vsync | wp_presentation_feedback::Kind::HwClock | wp_presentation_feedback::Kind::HwCompletion,
        ),
        _ => (state.core.clock.now(), wp_presentation_feedback::Kind::Vsync),
    };
    let seq = meta.as_ref().map(|m| m.sequence).unwrap_or(0);
    match surface.drm_output.frame_submitted() {
        Ok(Some(Some(mut feedback))) => {
            feedback.presented(clock, Refresh::fixed(frame_duration), seq as u64, flags);
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(?e, "frame_submitted"),
    }
    super::tty::after_frame(&mut state.core, &output);
}
