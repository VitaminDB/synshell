//! Настоящий сеанс: DRM/KMS + GBM + libinput через libseat.
//!
//! Несколько GPU поддерживаются через `GpuManager`: рисует основной GPU,
//! выводы на других GPU получают копию кадра. Перерисовка — по запросу:
//! кадр ставится в очередь, следующий рисуется после vblank, если есть что
//! рисовать; без повреждений — frame callbacks по расчётному vblank.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use smithay::{
    backend::{
        allocator::{
            format::FormatSet,
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
            Fourcc, Modifier,
        },
        drm::{
            compositor::FrameFlags,
            exporter::gbm::GbmFramebufferExporter,
            output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements},
            DrmDevice, DrmDeviceFd, DrmEvent, DrmEventMetadata, DrmEventTime, DrmNode, NodeType,
        },
        egl::{EGLDevice, EGLDisplay},
        renderer::{
            damage::OutputDamageTracker,
            gles::{GlesRenderer, GlesTexture},
            multigpu::{gbm::GbmGlesBackend, GpuManager, MultiRenderer},
            Bind, ExportMem, ImportDma, ImportMemWl, Offscreen,
        },
        session::{libseat::LibSeatSession, Event as SessionEvent, Session},
        udev::{all_gpus, primary_gpu, UdevBackend, UdevEvent},
    },
    desktop::utils::OutputPresentationFeedback,
    output::{Mode as WlMode, Output, PhysicalProperties, Scale},
    reexports::{
        calloop::{
            timer::{TimeoutAction, Timer},
            EventLoop, LoopHandle, RegistrationToken,
        },
        drm::control::{connector, crtc, property, Device as ControlDevice, ModeTypeFlags},
        input::Libinput,
        rustix::fs::OFlags,
        wayland_protocols::wp::presentation_time::server::wp_presentation_feedback,
        wayland_server::{backend::GlobalId, protocol::wl_surface::WlSurface},
    },
    utils::{DeviceFd, Rectangle, Size, Transform},
    wayland::{dmabuf::DmabufFeedbackBuilder, presentation::Refresh},
};

use smithay::reexports::drm::Device as _;

use crate::{
    render::OutputElement,
    state::{Core, RedrawState, State},
};

pub type TtyRenderer<'a> = MultiRenderer<'a, 'a, GbmGlesBackend<GlesRenderer, DrmDeviceFd>, GbmGlesBackend<GlesRenderer, DrmDeviceFd>>;
type Manager = DrmOutputManager<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, Option<OutputPresentationFeedback>, DrmDeviceFd>;
type Out = DrmOutput<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, Option<OutputPresentationFeedback>, DrmDeviceFd>;

const COLOR_FORMATS: &[Fourcc] = &[Fourcc::Argb8888, Fourcc::Abgr8888, Fourcc::Xrgb8888, Fourcc::Xbgr8888];
/// Дисплей Qualcomm (msm_drm/sde) показывает сжатые (UBWC) кадры только в ABGR/XBGR8888 — они первыми: кадр
/// композитора (интерфейс, Firefox, игры) идёт на дисплей сжатым.
const COLOR_FORMATS_QCOM: &[Fourcc] = &[Fourcc::Xbgr8888, Fourcc::Abgr8888, Fourcc::Argb8888, Fourcc::Xrgb8888];

/// Привязка вывода к устройству и CRTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OutputId {
    node: DrmNode,
    crtc: crtc::Handle,
}

struct Surface {
    output: Output,
    drm_output: Out,
    global: Option<GlobalId>,
    connector: connector::Handle,
    render_node: DrmNode,
    /// Таймер «расчётного vblank» для кадров без повреждений.
    estimated_vblank: Option<RegistrationToken>,
    /// Сторож потерянного page-flip: downstream-драйверы (sde) иногда не
    /// присылают событие vblank, и без него кадры остановились бы навсегда.
    flip_watchdog: Option<RegistrationToken>,
    /// Последний кадр — буфер клиента прямо на primary-плане (без композитинга).
    direct: bool,
    /// Обратная связь dmabuf для поверхностей этого вывода.
    feedback: Option<crate::backend::OutputFeedback>,
}

/// Сколько ждать vblank после отправки кадра, прежде чем считать событие потерянным.
const FLIP_WATCHDOG: Duration = Duration::from_millis(250);

struct Device {
    manager: Manager,
    render_node: Option<DrmNode>,
    surfaces: HashMap<crtc::Handle, Surface>,
    token: RegistrationToken,
}

pub struct TtyBackend {
    session: LibSeatSession,
    libinput: Libinput,
    primary_gpu: DrmNode,
    gpus: GpuManager<GbmGlesBackend<GlesRenderer, DrmDeviceFd>>,
    devices: HashMap<DrmNode, Device>,
    loop_handle: LoopHandle<'static, State>,
    active: bool,
    /// Источники событий бэкенда (libinput, сеанс, udev) — снимаются в `teardown`.
    tokens: Vec<RegistrationToken>,
    /// Сколько мониторов не удалось включить: все не включились — повод откатиться на CPU.
    pub failed_outputs: usize,
}

/// Узел рендеринга DRM-устройства, если он есть: основной GPU, узел EGL и
/// узлы устройств должны совпадать, чтобы `GpuManager` находил рендерер.
/// Устройство без своего render-узла (simpledrm за kmsro) — узел GPU, который за ним рисует (через EGL).
fn render_node_of(node: DrmNode) -> DrmNode {
    if let Some(Ok(rn)) = node.node_with_type(NodeType::Render) {
        return rn;
    }
    node.dev_path()
        .and_then(|p| super::egl_render_node(&p))
        .inspect(|rn| tracing::info!(%node, %rn, "рендер для устройства без render-узла — через EGL"))
        .unwrap_or(node)
}

impl TtyBackend {
    pub fn new(event_loop: &EventLoop<'static, State>) -> anyhow::Result<Self> {
        let (session, notifier) = LibSeatSession::new().map_err(|e| anyhow::anyhow!("libseat: {e}"))?;
        let seat = session.seat();
        let primary_gpu = if let Ok(p) = std::env::var("SYNSHELL_DRM_DEVICE") {
            render_node_of(DrmNode::from_path(p)?)
        } else {
            primary_gpu(&seat)?
                .and_then(|p| DrmNode::from_path(p).ok())
                .map(render_node_of)
                .filter(|n| n.ty() == NodeType::Render)
                .or_else(|| all_gpus(&seat).ok()?.into_iter().find_map(|p| DrmNode::from_path(p).ok()))
                .ok_or_else(|| anyhow::anyhow!("нет GPU"))?
        };
        tracing::info!(%primary_gpu, "основной GPU");
        let gpus = GpuManager::new(GbmGlesBackend::with_context_priority(smithay::backend::egl::context::ContextPriority::High))
            .map_err(|e| anyhow::anyhow!("GpuManager: {e}"))?;

        let (libinput, input_token) = super::init_libinput(event_loop, &session)?;

        let session_token = event_loop.handle().insert_source(notifier, |event, _, state| match event {
            SessionEvent::PauseSession => {
                tracing::info!("сеанс приостановлен (переключение VT)");
                if let crate::backend::Backend::Tty(t) = &mut state.backend {
                    t.active = false;
                    t.libinput.suspend();
                    for d in t.devices.values_mut() {
                        d.manager.pause();
                    }
                }
            }
            SessionEvent::ActivateSession => {
                tracing::info!("сеанс возобновлён");
                if let crate::backend::Backend::Tty(t) = &mut state.backend {
                    t.active = true;
                    if let Err(e) = t.libinput.resume() {
                        tracing::error!(?e, "libinput resume");
                    }
                    for d in t.devices.values_mut() {
                        if let Err(e) = d.manager.activate(false) {
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
            primary_gpu,
            gpus,
            devices: HashMap::new(),
            loop_handle: event_loop.handle(),
            active: true,
            tokens: vec![input_token, session_token],
            failed_outputs: 0,
        })
    }

    pub fn seat_name(&self) -> String {
        self.session.seat()
    }

    /// Снять все источники событий и отпустить устройства — перед откатом на
    /// CPU-бэкенд в том же процессе (иначе fd DRM-мастера остался бы открыт в
    /// нотификаторе, а libinput дублировал бы ввод).
    pub fn teardown(&mut self) {
        self.libinput.suspend();
        for (_, d) in self.devices.drain() {
            for s in d.surfaces.values() {
                for t in [s.estimated_vblank, s.flip_watchdog].into_iter().flatten() {
                    self.loop_handle.remove(t);
                }
            }
            self.loop_handle.remove(d.token);
        }
        for t in self.tokens.drain(..) {
            self.loop_handle.remove(t);
        }
    }

    pub fn early_import(&mut self, surface: &WlSurface) {
        if let Err(e) = self.gpus.early_import(self.primary_gpu, surface) {
            tracing::trace!(?e, "early import");
        }
    }

    pub fn import_dmabuf(&mut self, dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf) -> bool {
        let ok = self
            .gpus
            .single_renderer(&self.primary_gpu)
            .and_then(|mut r| r.import_dmabuf(dmabuf, None))
            .is_ok();
        if ok {
            dmabuf.set_node(self.primary_gpu);
        }
        ok
    }

    pub fn change_vt(&mut self, vt: i32) {
        if let Err(e) = self.session.change_vt(vt) {
            tracing::warn!(vt, ?e, "не удалось сменить VT");
        }
    }
}

/// Инициализация устройств — после создания `State` (нужны глобалы и Space).
pub fn init(state: &mut State, event_loop: &EventLoop<'static, State>) -> anyhow::Result<()> {
    let seat = state.backend.seat_name();
    let udev = UdevBackend::new(&seat)?;
    let primary = match &state.backend {
        crate::backend::Backend::Tty(t) => t.primary_gpu,
        _ => unreachable!(),
    };
    // Сначала основной GPU — остальные могут рисовать через него.
    let mut list: Vec<(u64, std::path::PathBuf)> = udev.device_list().map(|(id, p)| (id, p.to_path_buf())).collect();
    let primary_node = primary.node_with_type(NodeType::Primary).and_then(|n| n.ok());
    list.sort_by_key(|(id, _)| if Some(*id) == primary_node.map(|n| n.dev_id()) || *id == primary.dev_id() { 0 } else { 1 });
    for (id, path) in list {
        match DrmNode::from_dev_id(id) {
            Ok(node) => {
                if let Err(e) = device_added(state, node, &path) {
                    tracing::warn!(?e, %node, "устройство пропущено");
                }
            }
            Err(e) => tracing::warn!(?e, "DrmNode"),
        }
    }

    // Глобалы, зависящие от рендерера.
    {
        let crate::backend::Backend::Tty(t) = &mut state.backend else { unreachable!() };
        let renderer = t.gpus.single_renderer(&t.primary_gpu)?;
        state.core.shm_state.update_formats(renderer.shm_formats());
        let formats = renderer.dmabuf_formats();
        let feedback = DmabufFeedbackBuilder::new(t.primary_gpu.dev_id(), formats.clone()).build()?;
        let global = state
            .core
            .dmabuf_state
            .create_global_with_default_feedback::<State>(&state.core.display_handle, &feedback);
        state.core.dmabuf_global = Some(global);
    }

    let udev_token = event_loop.handle().insert_source(udev, |event, _, state| match event {
        UdevEvent::Added { device_id, path } => {
            if let Ok(node) = DrmNode::from_dev_id(device_id) {
                if let Err(e) = device_added(state, node, &path) {
                    tracing::warn!(?e, %node, "горячее подключение GPU");
                }
            }
        }
        UdevEvent::Changed { device_id } => {
            if let Ok(node) = DrmNode::from_dev_id(device_id) {
                scan_connectors(state, node);
            }
        }
        UdevEvent::Removed { device_id } => {
            if let Ok(node) = DrmNode::from_dev_id(device_id) {
                device_removed(state, node);
            }
        }
    }).map_err(|e| anyhow::anyhow!("{}", e.error))?;
    if let crate::backend::Backend::Tty(t) = &mut state.backend {
        t.tokens.push(udev_token);
    }
    Ok(())
}

fn device_added(state: &mut State, node: DrmNode, path: &Path) -> anyhow::Result<()> {
    let crate::backend::Backend::Tty(t) = &mut state.backend else { unreachable!() };
    let fd = t
        .session
        .open(path, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK)
        .map_err(|e| anyhow::anyhow!("open {}: {e}", path.display()))?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));
    let (drm, notifier) = DrmDevice::new(fd.clone(), true)?;
    let gbm = GbmDevice::new(fd)?;

    let render_node = (|| -> anyhow::Result<DrmNode> {
        let display = unsafe { EGLDisplay::new(gbm.clone())? };
        let egl_device = EGLDevice::device_for_display(&display)?;
        if egl_device.is_software() {
            anyhow::bail!("программный рендер");
        }
        let rn = render_node_of(egl_device.try_get_render_node().ok().flatten().unwrap_or(node));
        t.gpus.as_mut().add_node(rn, gbm.clone())?;
        Ok(rn)
    })()
    .inspect_err(|e| tracing::warn!(?e, %node, "GPU без рендеринга"))
    .ok();

    let allocator = if render_node.is_some() {
        GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT)
    } else {
        t.devices
            .values()
            .find(|d| d.render_node == Some(t.primary_gpu))
            .map(|d| d.manager.allocator().clone())
            .ok_or_else(|| anyhow::anyhow!("нет основного GPU для устройства без рендеринга"))?
    };
    let exporter = GbmFramebufferExporter::new(gbm.clone(), render_node);
    let mut renderer = t.gpus.single_renderer(&render_node.unwrap_or(t.primary_gpu))?;
    let render_formats = renderer
        .as_mut()
        .egl_context()
        .dmabuf_render_formats()
        .iter()
        .filter(|f| render_node.is_some() || f.modifier == Modifier::Linear)
        // `SYNSHELL_NO_UBWC=1` — кадр композитора без сжатия (сравнение, отладка дисплея).
        .filter(|f| std::env::var_os("SYNSHELL_NO_UBWC").is_none() || matches!(f.modifier, Modifier::Linear | Modifier::Invalid))
        .copied()
        .collect::<FormatSet>();
    let qcom = drm.get_driver().is_ok_and(|d| d.name().to_string_lossy().eq_ignore_ascii_case("msm_drm"));
    let color_formats = if qcom { COLOR_FORMATS_QCOM } else { COLOR_FORMATS };
    let manager = DrmOutputManager::new(drm, allocator, exporter, Some(gbm), color_formats.iter().copied(), render_formats);
    // Нотификатор держит fd устройства (и DRM-мастер) — регистрируем, только когда
    // устройство точно принято: иначе после ошибки fd остался бы жить в цикле событий.
    let token = t.loop_handle.insert_source(notifier, move |event, meta, state| match event {
        DrmEvent::VBlank(crtc) => on_vblank(state, node, crtc, meta),
        DrmEvent::Error(e) => tracing::error!(?e, "DRM"),
    })?;
    t.devices.insert(node, Device { manager, render_node, surfaces: HashMap::new(), token });
    scan_connectors(state, node);
    Ok(())
}

fn device_removed(state: &mut State, node: DrmNode) {
    let crate::backend::Backend::Tty(t) = &mut state.backend else { return };
    let Some(device) = t.devices.remove(&node) else { return };
    let outputs: Vec<Output> = device.surfaces.values().map(|s| s.output.clone()).collect();
    if let Some(rn) = device.render_node {
        t.gpus.as_mut().remove_node(&rn);
    }
    t.loop_handle.remove(device.token);
    for o in outputs {
        remove_output(&mut state.core, &o);
    }
    state.outputs_changed();
}

fn remove_output(core: &mut Core, output: &Output) {
    core.space.unmap_output(output);
    core.output_data.remove(output);
    core.lock_surfaces.remove(output);
    // Слои этого вывода закрываются — оболочка создаст их заново.
    let layers: Vec<_> = smithay::desktop::layer_map_for_output(output).layers().cloned().collect();
    for l in layers {
        l.layer_surface().send_close();
    }
}

/// Сверить подключённые мониторы с выводами.
pub fn scan_connectors(state: &mut State, node: DrmNode) {
    let crate::backend::Backend::Tty(t) = &mut state.backend else { return };
    let Some(device) = t.devices.get_mut(&node) else { return };
    let drm = device.manager.device();
    let Ok(res) = drm.resource_handles() else { return };
    let mut connected = Vec::new();
    for &conn in res.connectors() {
        if let Ok(info) = drm.get_connector(conn, true) {
            if info.state() == connector::State::Connected {
                connected.push(info);
            }
        }
    }
    // Отключённые.
    let gone: Vec<crtc::Handle> = device
        .surfaces
        .iter()
        .filter(|(_, s)| !connected.iter().any(|c| c.handle() == s.connector))
        .map(|(c, _)| *c)
        .collect();
    let mut removed = Vec::new();
    for crtc in gone {
        if let Some(s) = device.surfaces.remove(&crtc) {
            tracing::info!(output = s.output.name(), "монитор отключён");
            if let Some(g) = s.global {
                state.core.display_handle.remove_global::<State>(g);
            }
            removed.push(s.output);
        }
    }
    for o in &removed {
        remove_output(&mut state.core, o);
    }
    // Новые.
    let mut added = false;
    for info in connected {
        let crate::backend::Backend::Tty(t) = &mut state.backend else { return };
        let device = t.devices.get_mut(&node).unwrap();
        if device.surfaces.values().any(|s| s.connector == info.handle()) {
            continue;
        }
        let name = format!("{}-{}", info.interface().as_str(), info.interface_id());
        if state.core.config.output_ignored(&name) {
            tracing::info!(name, "вывод пропущен (platform.ignore_outputs)");
            continue;
        }
        let cfg = state.core.config.outputs.iter().find(|o| o.name == name).cloned();
        if cfg.as_ref().is_some_and(|c| !c.enabled) {
            tracing::info!(name, "монитор выключен в конфиге");
            continue;
        }
        // Свободный CRTC.
        let drm = device.manager.device();
        let used: Vec<crtc::Handle> = device.surfaces.keys().copied().collect();
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
            .and_then(|c| pick_mode(modes, &c.mode))
            .or_else(|| modes.iter().find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED)).copied())
            .unwrap_or(modes[0]);
        let (make, model, _serial) = edid_info(drm, info.handle());
        let (mm_w, mm_h) = info.size().unwrap_or((0, 0));
        let output = Output::new(
            name.clone(),
            PhysicalProperties {
                size: (mm_w as i32, mm_h as i32).into(),
                subpixel: info.subpixel().into(),
                make: make.clone(),
                model: model.clone(),
            },
        );
        let wl_mode = WlMode::from(mode);
        for m in modes {
            output.add_mode(WlMode::from(*m));
        }
        output.set_preferred(
            modes
                .iter()
                .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
                .map(|m| WlMode::from(*m))
                .unwrap_or(wl_mode),
        );
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
        output.user_data().insert_if_missing(|| OutputId { node, crtc });

        let render_node = device.render_node.unwrap_or(t.primary_gpu);
        let primary = t.primary_gpu;
        let mut renderer = if render_node == primary {
            t.gpus.single_renderer(&render_node)
        } else {
            t.gpus.renderer(&primary, &render_node, Fourcc::Argb8888)
        };
        let Ok(renderer) = renderer.as_mut() else {
            tracing::warn!(name, "нет рендерера");
            continue;
        };
        let mut planes = drm.planes(&crtc).ok();
        let driver = drm.get_driver().map(|d| d.name().to_string_lossy().to_lowercase()).unwrap_or_default();
        // Android-драйвер дисплея Qualcomm (msm_drm/sde): прямой вывод клиентских
        // буферов на overlay- и курсорные планы ненадёжен — всё в primary-кадр.
        let downstream = driver == "msm_drm";
        if driver.contains("nvidia") || downstream {
            if let Some(p) = planes.as_mut() {
                p.overlay.clear();
                if downstream {
                    p.cursor.clear();
                }
            }
        }
        let drm_output = match device.manager.initialize_output::<_, OutputElement<TtyRenderer<'_>>>(
            crtc,
            mode,
            &[info.handle()],
            &output,
            planes,
            renderer,
            &DrmOutputRenderElements::default(),
        ) {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!(name, ?e, "не удалось включить монитор");
                t.failed_outputs += 1;
                continue;
            }
        };
        tracing::info!(name, make, model, mode = ?wl_mode, scale, "монитор подключён");
        let feedback = output_feedback(t.primary_gpu, &mut t.gpus, &drm_output, downstream);
        let global = output.create_global::<State>(&state.core.display_handle);
        device.surfaces.insert(
            crtc,
            Surface {
                output: output.clone(),
                drm_output,
                global: Some(global),
                connector: info.handle(),
                render_node,
                estimated_vblank: None,
                flip_watchdog: None,
                direct: false,
                feedback,
            },
        );
        // Положение: из конфига или справа от остальных.
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

pub(crate) fn pick_mode(modes: &[smithay::reexports::drm::control::Mode], spec: &str) -> Option<smithay::reexports::drm::control::Mode> {
    let spec = spec.trim();
    if spec.is_empty() {
        return None;
    }
    let (res, rate) = match spec.split_once('@') {
        Some((r, f)) => (r, f.trim().trim_end_matches("Hz").parse::<f64>().ok()),
        None => (spec, None),
    };
    let (w, h) = res.split_once('x')?;
    let (w, h): (u16, u16) = (w.trim().parse().ok()?, h.trim().parse().ok()?);
    modes
        .iter()
        .filter(|m| m.size() == (w, h))
        .min_by_key(|m| match rate {
            Some(r) => ((m.vrefresh() as f64 - r).abs() * 1000.0) as i64,
            None => -(m.vrefresh() as i64),
        })
        .copied()
}

/// Производитель и модель из EDID.
fn edid_info(drm: &DrmDevice, conn: connector::Handle) -> (String, String, String) {
    let unknown = || ("Unknown".to_string(), "Unknown".to_string(), String::new());
    let Ok(props) = drm.get_properties(conn) else { return unknown() };
    let (handles, values) = props.as_props_and_values();
    for (h, v) in handles.iter().zip(values) {
        let Ok(info) = drm.get_property(*h) else { continue };
        if info.name().to_str() != Ok("EDID") {
            continue;
        }
        let property::Value::Blob(blob) = info.value_type().convert_value(*v) else { continue };
        let Ok(data) = drm.get_property_blob(blob) else { continue };
        return parse_edid(&data).unwrap_or_else(unknown);
    }
    unknown()
}

fn parse_edid(d: &[u8]) -> Option<(String, String, String)> {
    if d.len() < 128 {
        return None;
    }
    let id = u16::from_be_bytes([d[8], d[9]]);
    let letter = |v: u16| (((v & 0x1f) as u8) + b'@') as char;
    let pnp: String = [letter(id >> 10), letter(id >> 5), letter(id)].iter().collect();
    let make = match pnp.as_str() {
        "SAM" => "Samsung",
        "DEL" => "Dell",
        "LGD" | "GSM" => "LG",
        "AUO" => "AU Optronics",
        "BOE" => "BOE",
        "CMN" => "Chimei Innolux",
        "SHP" => "Sharp",
        "ACR" => "Acer",
        "AUS" => "ASUS",
        "BNQ" => "BenQ",
        "HWP" => "HP",
        "LEN" => "Lenovo",
        "PHL" => "Philips",
        "VSC" => "ViewSonic",
        "AOC" => "AOC",
        "MSI" => "MSI",
        "GBT" => "Gigabyte",
        "SDC" => "Samsung Display",
        "CSO" => "CSOT",
        "APP" => "Apple",
        other => other,
    }
    .to_string();
    let mut model = String::new();
    let mut serial = String::new();
    for i in 0..4 {
        let o = 54 + i * 18;
        let b = &d[o..o + 18];
        if b[0] == 0 && b[1] == 0 {
            let text = || {
                String::from_utf8_lossy(&b[5..18])
                    .trim_end_matches(['\n', ' ', '\0'])
                    .trim()
                    .to_string()
            };
            match b[3] {
                0xFC => model = text(),
                0xFF => serial = text(),
                _ => {}
            }
        }
    }
    if model.is_empty() {
        model = format!("{:04X}", u16::from_le_bytes([d[10], d[11]]));
    }
    Some((make, model, serial))
}

/// Обратная связь dmabuf вывода: для отрисовки — форматы основного GPU; для
/// вывода на план — сначала форматы primary-плана, которые мы и нарисовать
/// сумеем (запасной путь, если план всё же не примет буфер). Так клиент не
/// выберет то, чего дисплей не умеет (у Qualcomm sde: UBWC для XRGB8888).
/// Сжатие UBWC Qualcomm (`DRM_FORMAT_MOD_QCOM_COMPRESSED`).
const MOD_QCOM_COMPRESSED: u64 = 0x0500_0000_0000_0001;

fn output_feedback(primary: DrmNode, gpus: &mut GpuManager<GbmGlesBackend<GlesRenderer, DrmDeviceFd>>, drm_output: &Out, qcom: bool) -> Option<crate::backend::OutputFeedback> {
    use smithay::reexports::wayland_protocols::wp::linux_dmabuf::zv1::server::zwp_linux_dmabuf_feedback_v1::TrancheFlags;
    let render_formats = gpus.single_renderer(&primary).ok()?.dmabuf_formats();
    let (plane_formats, scanout_dev) = drm_output.with_compositor(|c| {
        let s = c.surface();
        (s.plane_info().formats.clone(), s.device_fd().dev_id().ok())
    });
    // План без IN_FORMATS (msm_drm/sde) сообщает только «неявный» модификатор — Mesa такой
    // группой не пользуется и берёт общий список (с UBWC). Явно: те же форматы в LINEAR,
    // его дисплей принимает всегда.
    let scanout: FormatSet = plane_formats
        .iter()
        .flat_map(|f| {
            let linear = (f.modifier == Modifier::Invalid).then_some(smithay::backend::allocator::Format { code: f.code, modifier: Modifier::Linear });
            [Some(*f), linear]
        })
        .flatten()
        // Дисплей Qualcomm (sde) берёт UBWC для ABGR/XBGR8888 (не для ARGB/XRGB): кадр со сжатием — меньше
        // трафика памяти и у GPU, рисующего его, и у дисплея, читающего его 120 раз в секунду. Первым в группе.
        .flat_map(|f| {
            let ubwc = (qcom && f.modifier == Modifier::Invalid && matches!(f.code, Fourcc::Abgr8888 | Fourcc::Xbgr8888))
                .then_some(smithay::backend::allocator::Format { code: f.code, modifier: Modifier::from(MOD_QCOM_COMPRESSED) });
            [ubwc, Some(f)]
        })
        .flatten()
        .filter(|f| render_formats.contains(f))
        // BGRA-клиентам (XRGB/ARGB: Firefox, игры через Xwayland) на Qualcomm группу плана не даём: дисплей
        // берёт их только несжатыми, и прямой вывод заставил бы клиента рисовать без UBWC. Сжатый буфер
        // клиента + проход композитора в сжатый кадр дешевле: 4K-видео — GPU 75 % на 222 МГц против 77 % на
        // 297 МГц (docs/08, 8.8). `SYNSHELL_SCANOUT_BGRA=1` — прямой вывод и для них.
        .filter(|f| !(qcom && !std::env::var("SYNSHELL_SCANOUT_BGRA").is_ok_and(|v| v == "1") && matches!(f.code, Fourcc::Argb8888 | Fourcc::Xrgb8888)))
        .collect();
    tracing::info!(
        форматов_плана = plane_formats.iter().count(),
        для_плана = scanout.iter().count(),
        "обратная связь dmabuf: {}",
        scanout.iter().map(|f| format!("{:?}/{:#x}", f.code, u64::from(f.modifier))).collect::<Vec<_>>().join(" ")
    );
    let builder = DmabufFeedbackBuilder::new(primary.dev_id(), render_formats);
    let render = builder.clone().build().ok()?;
    let scanout = builder.add_preference_tranche(scanout_dev?, Some(TrancheFlags::Scanout), scanout).build().ok()?;
    Some(crate::backend::OutputFeedback { render, scanout })
}

fn find_surface<'a>(t: &'a mut TtyBackend, output: &Output) -> Option<(&'a mut Surface, DrmNode)> {
    let id = *output.user_data().get::<OutputId>()?;
    let d = t.devices.get_mut(&id.node)?;
    d.surfaces.get_mut(&id.crtc).map(|s| (s, id.node))
}

impl TtyBackend {
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
        let primary = self.primary_gpu;
        let Some(id) = output.user_data().get::<OutputId>().copied() else { return };
        let Some(device) = self.devices.get_mut(&id.node) else { return };
        let Some(surface) = device.surfaces.get_mut(&id.crtc) else { return };
        let render_node = surface.render_node;
        let format = surface.drm_output.format();
        let renderer = if render_node == primary {
            self.gpus.single_renderer(&render_node)
        } else {
            self.gpus.renderer(&primary, &render_node, format)
        };
        let mut renderer = match renderer {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(?e, "рендерер");
                return;
            }
        };
        let (elements, clear) = crate::render::output_elements(core, &mut renderer, output, true);
        // Полноэкранный клиент — прямо на primary-план (без композитинга), формат его буфера может отличаться от
        // формата swapchain (Vulkan-клиенты рисуют в ABGR8888): годность плана проверяет тестовый коммит.
        let flags = FrameFlags::DEFAULT | FrameFlags::ALLOW_PRIMARY_PLANE_SCANOUT_ANY;
        let res = surface.drm_output.render_frame(&mut renderer, &elements, clear, flags);
        let (rendered, states) = match res {
            Ok(r) => {
                // Нет явной синхронизации с KMS (у msm_drm/sde нет syncobj и
                // IN_FENCE_FD), а неявной у turnip/KGSL на dma-buf нет: без
                // ожидания панель показывает недорисованный кадр — артефакты,
                // заметные под нагрузкой GPU (трансляция экрана, игры).
                if !r.is_empty && r.needs_sync() {
                    if let smithay::backend::drm::compositor::PrimaryPlaneElement::Swapchain(ref el) = r.primary_element {
                        let _ = el.sync.wait();
                    }
                }
                let direct = matches!(r.primary_element, smithay::backend::drm::compositor::PrimaryPlaneElement::Element(_));
                if direct != surface.direct {
                    surface.direct = direct;
                    tracing::info!(output = output.name(), "{}", if direct { "прямой вывод клиента на план" } else { "композитинг" });
                }
                (!r.is_empty, r.states)
            }
            Err(e) => {
                tracing::warn!(?e, output = output.name(), "кадр не нарисован");
                if let Some(d) = core.output_data.get_mut(output) {
                    d.redraw = RedrawState::Idle;
                }
                return;
            }
        };
        drop(elements);
        drop(renderer);
        core.post_repaint(output, &states);
        if let Some(fb) = surface.feedback.as_ref() {
            core.send_dmabuf_feedback(output, fb, &states);
        }

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
            // Нет повреждений: frame callbacks по расчётному vblank.
            data.redraw = RedrawState::WaitingForEstimatedVBlank { redraw_needed: false };
            let refresh = output.current_mode().map(|m| m.refresh).unwrap_or(60_000).max(1000);
            let dur = Duration::from_micros(1_000_000_000 / refresh as u64);
            if surface.estimated_vblank.is_none() {
                let o = output.clone();
                let token = self
                    .loop_handle
                    .insert_source(Timer::from_duration(dur), move |_, _, state| {
                        if let crate::backend::Backend::Tty(t) = &mut state.backend {
                            if let Some((s, _)) = find_surface(t, &o) {
                                s.estimated_vblank = None;
                            }
                        }
                        let states = smithay::backend::renderer::element::RenderElementStates::default();
                        state.core.post_repaint(&o, &states);
                        after_frame(&mut state.core, &o);
                        TimeoutAction::Drop
                    })
                    .ok();
                surface.estimated_vblank = token;
            }
        }
    }

    pub fn set_monitors_power(&mut self, core: &mut Core, on: bool) {
        for d in self.devices.values_mut() {
            for s in d.surfaces.values_mut() {
                if !on {
                    let _ = s.drm_output.with_compositor(|c| c.clear());
                }
                if let Some(data) = core.output_data.get_mut(&s.output) {
                    data.redraw = if on { RedrawState::Queued } else { RedrawState::Idle };
                    data.powered_off = !on;
                }
            }
        }
    }

    pub fn with_gles<T>(&mut self, f: impl FnOnce(&mut GlesRenderer) -> T) -> Option<T> {
        let mut r = self.gpus.single_renderer(&self.primary_gpu).ok()?;
        Some(f(r.as_mut()))
    }

    pub fn screenshot(&mut self, core: &mut Core, output: &Output) -> anyhow::Result<(u32, u32, Vec<u8>)> {
        let size = output.current_mode().map(|m| m.size).unwrap_or_default();
        let transformed = output.current_transform().transform_size(size);
        let buffer_size = Size::from((transformed.w, transformed.h));
        let mut renderer = self.gpus.single_renderer(&self.primary_gpu)?;
        let gles: &mut GlesRenderer = renderer.as_mut();
        let mut texture: GlesTexture = gles.create_buffer(Fourcc::Abgr8888, buffer_size)?;
        let (elements, clear) = crate::render::output_elements(core, gles, output, false);
        {
            let mut fb = gles.bind(&mut texture)?;
            let mut tracker = OutputDamageTracker::new(transformed, output.current_scale().fractional_scale(), Transform::Normal);
            tracker.render_output(gles, &mut fb, 0, &elements, clear)?;
        }
        let fb = gles.bind(&mut texture)?;
        let mapping = gles.copy_framebuffer(&fb, Rectangle::from_size(buffer_size), Fourcc::Abgr8888)?;
        let data = gles.map_texture(&mapping)?.to_vec();
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
        let mut renderer = self.gpus.single_renderer(&self.primary_gpu)?;
        let gles: &mut GlesRenderer = renderer.as_mut();
        crate::encode::stream_render(core, gles, output, slot, cursor, opts)
    }

    /// Применить `[[output]]`: масштаб, поворот, положение, режим.
    pub fn apply_output_config(&mut self, core: &mut Core) {
        let primary = self.primary_gpu;
        for (node, d) in self.devices.iter_mut() {
            let render_node = d.render_node.unwrap_or(primary);
            for s in d.surfaces.values_mut() {
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
                // Режим.
                if !cfg.mode.is_empty() {
                    let drm = d.manager.device();
                    if let Ok(info) = drm.get_connector(s.connector, false) {
                        if let Some(mode) = pick_mode(info.modes(), &cfg.mode) {
                            let current = s.output.current_mode();
                            if current != Some(WlMode::from(mode)) {
                                let renderer = if render_node == primary {
                                    self.gpus.single_renderer(&render_node)
                                } else {
                                    self.gpus.renderer(&primary, &render_node, s.drm_output.format())
                                };
                                if let Ok(mut r) = renderer {
                                    match s.drm_output.use_mode::<_, OutputElement<TtyRenderer<'_>>>(mode, &mut r, &DrmOutputRenderElements::default()) {
                                        Ok(()) => {
                                            s.output.change_current_state(Some(WlMode::from(mode)), None, None, None);
                                            tracing::info!(name, ?mode, "режим монитора изменён");
                                        }
                                        Err(e) => tracing::warn!(name, ?e, "режим не применён"),
                                    }
                                }
                            }
                        }
                    }
                }
                let _ = node;
            }
        }
    }
}

fn on_vblank(state: &mut State, node: DrmNode, crtc: crtc::Handle, meta: &mut Option<DrmEventMetadata>) {
    let crate::backend::Backend::Tty(t) = &mut state.backend else { return };
    let loop_handle = t.loop_handle.clone();
    let Some(device) = t.devices.get_mut(&node) else { return };
    let Some(surface) = device.surfaces.get_mut(&crtc) else { return };
    let output = surface.output.clone();
    if let Some(token) = surface.flip_watchdog.take() {
        loop_handle.remove(token);
    }
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
    after_frame(&mut state.core, &output);
}

/// vblank не пришёл вовремя: считаем кадр показанным, чтобы не зависнуть.
fn flip_timeout(state: &mut State, output: &Output) {
    let waiting = matches!(state.core.output_data.get(output).map(|d| d.redraw), Some(RedrawState::WaitingForVBlank { .. }));
    let crate::backend::Backend::Tty(t) = &mut state.backend else { return };
    let Some((surface, _)) = find_surface(t, output) else { return };
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
    after_frame(&mut state.core, output);
}

/// Кадр показан (или прошёл расчётный vblank): решить, рисовать ли дальше.
pub(crate) fn after_frame(core: &mut Core, output: &Output) {
    let redraw_needed = match core.output_data.get(output).map(|d| d.redraw) {
        Some(RedrawState::WaitingForVBlank { redraw_needed }) | Some(RedrawState::WaitingForEstimatedVBlank { redraw_needed }) => {
            redraw_needed
        }
        _ => false,
    };
    let continuous = core.wants_continuous_redraw();
    if let Some(d) = core.output_data.get_mut(output) {
        d.redraw = if redraw_needed || continuous { RedrawState::Queued } else { RedrawState::Idle };
    }
}
