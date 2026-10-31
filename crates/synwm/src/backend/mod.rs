//! Бэкенды вывода: сеанс DRM/KMS с GPU (`tty`), сеанс DRM/KMS на CPU
//! (`kms_cpu`: телефоны без GBM/EGL) и вложенное окно (`winit`).

#[cfg(feature = "pixman")]
pub mod kms_cpu;
#[cfg(feature = "pixman")]
pub mod offload;
pub mod tty;
pub mod winit;

use std::time::Duration;

use smithay::{
    backend::{allocator::dmabuf::Dmabuf, renderer::element::RenderElementStates},
    desktop::{
        layer_map_for_output,
        utils::{
            surface_presentation_feedback_flags_from_states, surface_primary_scanout_output,
            update_surface_primary_scanout_output, OutputPresentationFeedback,
        },
    },
    input::pointer::CursorImageStatus,
    output::Output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    wayland::compositor::with_surface_tree_downward,
};

use crate::state::{Core, State};
use synshell_common::config::OutputConfig;

pub enum Backend {
    Winit(winit::WinitBackend),
    Tty(tty::TtyBackend),
    #[cfg(feature = "pixman")]
    KmsCpu(kms_cpu::KmsCpuBackend),
}

impl Backend {
    pub fn seat_name(&self) -> String {
        match self {
            Backend::Winit(_) => "seat0".into(),
            Backend::Tty(t) => t.seat_name(),
            #[cfg(feature = "pixman")]
            Backend::KmsCpu(b) => b.seat_name(),
        }
    }

    pub fn early_import(&mut self, surface: &WlSurface) {
        if let Backend::Tty(t) = self {
            t.early_import(surface);
        }
    }

    pub fn import_dmabuf(&mut self, dmabuf: &Dmabuf) -> bool {
        match self {
            Backend::Winit(w) => w.import_dmabuf(dmabuf),
            Backend::Tty(t) => t.import_dmabuf(dmabuf),
            #[cfg(feature = "pixman")]
            Backend::KmsCpu(b) => b.import_dmabuf(dmabuf),
        }
    }

    pub fn change_vt(&mut self, vt: i32) {
        #[cfg(feature = "pixman")]
        if let Backend::KmsCpu(b) = self {
            b.change_vt(vt);
            return;
        }
        if let Backend::Tty(t) = self {
            t.change_vt(vt);
        }
    }

    /// Нарисовать вывод, если он ждёт перерисовки.
    pub fn render(&mut self, core: &mut Core, output: &Output) {
        match self {
            Backend::Winit(w) => w.render(core, output),
            Backend::Tty(t) => t.render(core, output),
            #[cfg(feature = "pixman")]
            Backend::KmsCpu(b) => b.render(core, output),
        }
    }

    pub fn set_monitors_power(&mut self, core: &mut Core, on: bool) {
        #[cfg(feature = "pixman")]
        if let Backend::KmsCpu(b) = self {
            b.set_monitors_power(core, on);
            return;
        }
        if let Backend::Tty(t) = self {
            t.set_monitors_power(core, on);
        }
    }

    /// Снимок вывода: (ширина, высота, RGBA).
    /// Снимок вывода; `cursor` — с указателем мыши.
    pub fn screenshot(&mut self, core: &mut Core, output: &Output, cursor: bool) -> anyhow::Result<(u32, u32, Vec<u8>)> {
        match self {
            Backend::Winit(w) => w.screenshot(core, output, cursor),
            Backend::Tty(t) => t.screenshot(core, output, cursor),
            #[cfg(feature = "pixman")]
            Backend::KmsCpu(b) => b.screenshot(core, output, cursor),
        }
    }

    /// Миниатюра окна: (ширина, высота, RGBA), см. [`crate::render::thumbnail`].
    pub fn thumbnail(&mut self, core: &mut Core, id: crate::wm::WindowId, max: (i32, i32)) -> anyhow::Result<Option<(u32, u32, Vec<u8>)>> {
        use smithay::backend::renderer::gles::GlesTexture;
        match self {
            Backend::Winit(w) => crate::render::thumbnail::<_, GlesTexture>(core, w.gles(), id, max),
            Backend::Tty(t) => t.with_gles(|r| crate::render::thumbnail::<_, GlesTexture>(core, r, id, max)).unwrap_or(Ok(None)),
            #[cfg(feature = "pixman")]
            Backend::KmsCpu(b) => b.thumbnail(core, id, max),
        }
    }

    /// Картинка вывода в текстуре GLES (снимок для анимации поворота);
    /// `None` — рендерер не GLES.
    pub fn snapshot(
        &mut self,
        core: &mut Core,
        output: &Output,
    ) -> Option<anyhow::Result<(smithay::backend::renderer::gles::GlesTexture, smithay::utils::Size<i32, smithay::utils::Physical>)>> {
        let gles = |core: &mut Core, r: &mut smithay::backend::renderer::gles::GlesRenderer| {
            use smithay::backend::renderer::{damage::OutputDamageTracker, Bind, Offscreen};
            let size = output.current_mode().map(|m| m.size).unwrap_or_default();
            let transformed = output.current_transform().transform_size(size);
            let buffer = smithay::utils::Size::from((transformed.w, transformed.h));
            let mut tex: smithay::backend::renderer::gles::GlesTexture =
                r.create_buffer(smithay::backend::allocator::Fourcc::Abgr8888, buffer)?;
            let (elements, clear) = crate::render::output_elements(core, r, output, false);
            {
                let mut fb = r.bind(&mut tex)?;
                let mut tracker = OutputDamageTracker::new(
                    transformed,
                    output.current_scale().fractional_scale(),
                    smithay::utils::Transform::Normal,
                );
                tracker.render_output(r, &mut fb, 0, &elements, clear).map_err(|e| anyhow::anyhow!("{e:?}"))?;
            }
            Ok((tex, transformed))
        };
        match self {
            Backend::Winit(w) => Some(gles(core, w.gles())),
            Backend::Tty(t) => t.with_gles(|r| gles(core, r)),
            #[cfg(feature = "pixman")]
            Backend::KmsCpu(b) => b.with_gles(|r| gles(core, r)),
        }
    }

    /// Кадр потока synlink: дорисовать постоянный буфер по повреждениям.
    pub fn stream_frame(
        &mut self,
        core: &mut Core,
        output: &Output,
        slot: &mut Option<Box<dyn std::any::Any>>,
        cursor: bool,
        opts: &mut crate::stream::StreamOpts,
    ) -> anyhow::Result<crate::stream::Rendered> {
        match self {
            Backend::Winit(w) => w.stream_frame(core, output, slot, cursor, opts),
            Backend::Tty(t) => t.stream_frame(core, output, slot, cursor, opts),
            #[cfg(feature = "pixman")]
            Backend::KmsCpu(b) => b.stream_frame(core, output, slot, cursor, opts),
        }
    }

    /// Доступ к GLES-рендереру основного GPU (снимки, захват экрана).
    pub fn with_gles<T>(&mut self, f: impl FnOnce(&mut smithay::backend::renderer::gles::GlesRenderer) -> T) -> Option<T> {
        match self {
            Backend::Winit(w) => Some(f(w.gles())),
            Backend::Tty(t) => t.with_gles(f),
            #[cfg(feature = "pixman")]
            Backend::KmsCpu(b) => b.with_gles(f),
        }
    }

    /// Применить `[[output]]` из конфига (режимы, масштаб, положение).
    pub fn apply_output_config(&mut self, core: &mut Core) {
        match self {
            Backend::Winit(w) => w.apply_output_config(core),
            Backend::Tty(t) => t.apply_output_config(core),
            #[cfg(feature = "pixman")]
            Backend::KmsCpu(b) => b.apply_output_config(core),
        }
    }
}

/// Подходит ли вывод под имя из конфига: имя коннектора или описание.
/// libinput через libseat: общий для DRM-бэкендов. События идут в
/// `State::process_input_event`; настройки устройств — из `[input]`.
pub(crate) fn init_libinput(
    event_loop: &smithay::reexports::calloop::EventLoop<'static, State>,
    session: &smithay::backend::session::libseat::LibSeatSession,
) -> anyhow::Result<(smithay::reexports::input::Libinput, smithay::reexports::calloop::RegistrationToken)> {
    use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
    use smithay::backend::session::Session;
    use smithay::reexports::input::{DeviceCapability, Libinput};
    let seat = session.seat();
    let mut libinput = Libinput::new_with_udev::<LibinputSessionInterface<_>>(session.clone().into());
    libinput
        .udev_assign_seat(&seat)
        .map_err(|_| anyhow::anyhow!("libinput: не удалось назначить seat"))?;
    let input_backend = LibinputInputBackend::new(libinput.clone());
    let token = event_loop.handle().insert_source(input_backend, |mut event, _, state| {
        use smithay::backend::input::InputEvent;
        match &mut event {
            InputEvent::DeviceAdded { device } => {
                tracing::info!(
                    name = device.name(),
                    touch = device.has_capability(DeviceCapability::Touch),
                    keyboard = device.has_capability(DeviceCapability::Keyboard),
                    pointer = device.has_capability(DeviceCapability::Pointer),
                    "устройство ввода"
                );
                crate::libinput_config::apply(device, &state.core.config.input);
                if device.has_capability(DeviceCapability::Keyboard) {
                    if let Some(leds) = state.core.seat.get_keyboard().map(|k| k.led_state()) {
                        device.led_update(leds.into());
                    }
                }
                state.core.input_devices.push(device.clone());
            }
            InputEvent::DeviceRemoved { device } => {
                state.core.input_devices.retain(|d| d != device);
            }
            _ => {}
        }
        state.process_input_event(event);
    }).map_err(|e| anyhow::anyhow!("{}", e.error))?;
    Ok((libinput, token))
}

/// Путь к DRM-устройству сеанса: `SYNSHELL_DRM_DEVICE`, иначе основной GPU по udev
/// (seat0 — до открытия сеанса libseat).
///
/// Предпочтение — устройству с коннекторами: отдельный GPU без вывода (Adreno как
/// `msm.separate_gpu_kms=1`: card1 без коннекторов рядом с дисплейным card0) не годится
/// как устройство сеанса, даже если udev назвал основным его.
pub fn kms_cpu_device_path() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("SYNSHELL_DRM_DEVICE") {
        return Some(std::path::PathBuf::from(p));
    }
    let seat = std::env::var("XDG_SEAT").unwrap_or_else(|_| "seat0".into());
    let primary = smithay::backend::udev::primary_gpu(&seat).ok().flatten();
    let all = smithay::backend::udev::all_gpus(&seat).unwrap_or_default();
    primary
        .iter()
        .chain(all.iter())
        .find(|p| has_connectors(p))
        .cloned()
        .or(primary)
        .or_else(|| all.into_iter().next())
}

/// Имя DRM-драйвера устройства (`drmGetVersion`), например `onyx-epd`, `simpledrm`, `msm`.
pub fn drm_driver_name(path: &std::path::Path) -> Option<String> {
    use smithay::reexports::drm::Device as _;
    let file = std::fs::OpenOptions::new().read(true).write(true).open(path).ok()?;
    let fd = smithay::backend::drm::DrmDeviceFd::new(smithay::utils::DeviceFd::from(std::os::fd::OwnedFd::from(file)));
    fd.get_driver().ok().map(|d| d.name().to_string_lossy().into_owned())
}

/// Вывод — экран E Ink (драйвер из [`synshell_common::display::EINK_DRM_DRIVERS`]).
///
/// Такой вывод ведёт только `kms_cpu` (с композицией на отдельном GPU, если он есть): там точная
/// область изменений, режим пера и E Ink-настройки. GPU-путь `tty` их не умеет, а проба GPU на
/// дисплейном устройстве без render-узла может пройти — Mesa kmsro/renderonly подставит отдельный GPU.
pub fn is_eink_output(path: &std::path::Path) -> bool {
    drm_driver_name(path).is_some_and(|d| synshell_common::display::EINK_DRM_DRIVERS.contains(&d.as_str()))
}

/// Есть ли у DRM-устройства коннекторы (устройство вывода, а не только GPU).
fn has_connectors(path: &std::path::Path) -> bool {
    use smithay::reexports::drm::control::Device as _;
    let Ok(file) = std::fs::OpenOptions::new().read(true).write(true).open(path) else { return false };
    let fd = smithay::backend::drm::DrmDeviceFd::new(smithay::utils::DeviceFd::from(std::os::fd::OwnedFd::from(file)));
    fd.resource_handles().is_ok_and(|r| !r.connectors().is_empty())
}

/// Есть ли на устройстве GPU-рендеринг (EGL поверх GBM). Без него сеанс
/// DRM возможен только на CPU (`kms_cpu`).
///
/// Проба идёт в дочернем процессе (`synwm --probe-gpu ПУТЬ`): Mesa при
/// отсутствии аппаратного драйвера откатывается на llvmpipe, а тот на
/// Android-ядрах без SVE падает с SIGILL (synmobile docs/08); так же
/// может упасть и недоделанный драйвер. Упал или завис дольше 20 с — GPU нет.
pub fn probe_gpu(path: &std::path::Path) -> bool {
    let Ok(exe) = std::env::current_exe() else { return probe_gpu_here(path) };
    let child = std::process::Command::new(exe)
        .arg("--probe-gpu")
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .spawn();
    let Ok(mut child) = child else { return probe_gpu_here(path) };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        match child.try_wait() {
            Ok(Some(st)) => {
                if !st.success() {
                    tracing::info!(?st, path = %path.display(), "проба GPU: рендеринга на GPU нет");
                }
                return st.success();
            }
            Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(20)),
            _ => {
                tracing::warn!(path = %path.display(), "проба GPU зависла — считаем, что GPU нет");
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// Проба GPU в текущем процессе (для `synwm --probe-gpu`).
pub fn probe_gpu_here(path: &std::path::Path) -> bool {
    use smithay::backend::egl::{EGLDevice, EGLDisplay};
    use smithay::reexports::gbm::Device as GbmDevice;
    let Ok(file) = std::fs::OpenOptions::new().read(true).write(true).open(path) else { return false };
    let fd = smithay::backend::drm::DrmDeviceFd::new(smithay::utils::DeviceFd::from(std::os::fd::OwnedFd::from(file)));
    let Ok(gbm) = GbmDevice::new(fd) else { return false };
    let Ok(display) = (unsafe { EGLDisplay::new(gbm) }) else { return false };
    EGLDevice::device_for_display(&display).map(|d| !d.is_software()).unwrap_or(false)
}

/// Render-узел GPU, который реально рисует для устройства вывода `path`, — через EGL.
/// Нужен для устройств без своего render-узла (simpledrm/дисплейный KMS), за которыми
/// Mesa (kmsro) подставляет отдельный GPU: SM-T295 — simpledrm card0 + Adreno renderD128.
pub fn egl_render_node(path: &std::path::Path) -> Option<smithay::backend::drm::DrmNode> {
    use smithay::backend::egl::{EGLDevice, EGLDisplay};
    use smithay::reexports::gbm::Device as GbmDevice;
    let file = std::fs::OpenOptions::new().read(true).write(true).open(path).ok()?;
    let fd = smithay::backend::drm::DrmDeviceFd::new(smithay::utils::DeviceFd::from(std::os::fd::OwnedFd::from(file)));
    let gbm = GbmDevice::new(fd).ok()?;
    let display = unsafe { EGLDisplay::new(gbm) }.ok()?;
    let dev = EGLDevice::device_for_display(&display).ok()?;
    if dev.is_software() {
        return None;
    }
    dev.try_get_render_node().ok().flatten()
}

/// Форм-фактор по подключённым коннекторам: телефон/планшет, если единственный
/// подключённый — встроенная DSI-панель.
pub fn probe_form_factor(path: &std::path::Path) -> Option<synshell_common::config::FormFactor> {
    use smithay::reexports::drm::control::{connector, Device as ControlDevice};
    use synshell_common::config::FormFactor;
    let file = std::fs::OpenOptions::new().read(true).write(true).open(path).ok()?;
    let fd = smithay::backend::drm::DrmDeviceFd::new(smithay::utils::DeviceFd::from(std::os::fd::OwnedFd::from(file)));
    let res = fd.resource_handles().ok()?;
    let mut dsi = 0;
    let mut other = 0;
    for &c in res.connectors() {
        let Ok(info) = fd.get_connector(c, false) else { continue };
        if info.state() != connector::State::Connected {
            continue;
        }
        match info.interface() {
            connector::Interface::DSI => dsi += 1,
            connector::Interface::Virtual | connector::Interface::Writeback => {}
            _ => other += 1,
        }
    }
    Some(if dsi > 0 && other == 0 { FormFactor::Phone } else { FormFactor::Desktop })
}

pub fn output_matches(output: &Output, name: &str) -> bool {
    let name = name.trim();
    if name.is_empty() {
        return false;
    }
    if output.name().eq_ignore_ascii_case(name) {
        return true;
    }
    let p = output.physical_properties();
    let desc = format!("{} {}", p.make, p.model);
    desc.eq_ignore_ascii_case(name) || p.model.eq_ignore_ascii_case(name)
}

/// `[[output]]` для монитора — см. [`synshell_common::config::output_config_index`].
pub fn output_config<'a>(outputs: &'a [OutputConfig], name: &str, make: &str, model: &str) -> Option<&'a OutputConfig> {
    synshell_common::config::output_config_index(outputs, name, make, model).map(|i| &outputs[i])
}

/// [`output_config`] для уже созданного вывода.
pub fn output_config_for<'a>(outputs: &'a [OutputConfig], output: &Output) -> Option<&'a OutputConfig> {
    let p = output.physical_properties();
    output_config(outputs, &output.name(), &p.make, &p.model)
}

/// Действующий поворот вывода: `transform` из `[[output]]` и поверх —
/// поворот на лету (`rotate`).
pub fn output_transform(core: &Core, output: &Output) -> smithay::utils::Transform {
    let base = output_config_for(&core.config.outputs, output)
        .map(|c| parse_transform(&c.transform))
        .unwrap_or(smithay::utils::Transform::Normal);
    rotate_transform(base, current_rotation(core, output).quarters())
}

/// Линейная часть поворота: куда он переводит вектор `v`.
fn transform_vec(t: smithay::utils::Transform, v: (f64, f64)) -> (f64, f64) {
    let p = t.transform_point_in(smithay::utils::Point::<f64, smithay::utils::Logical>::from(v), &smithay::utils::Size::from((0.0, 0.0)));
    (p.x, p.y)
}

/// Поворот «сначала `b`, потом `a`».
pub fn compose_transform(a: smithay::utils::Transform, b: smithay::utils::Transform) -> smithay::utils::Transform {
    use smithay::utils::Transform as T;
    let want = |v| transform_vec(a, transform_vec(b, v));
    [T::Normal, T::_90, T::_180, T::_270, T::Flipped, T::Flipped90, T::Flipped180, T::Flipped270]
        .into_iter()
        .find(|c| transform_vec(*c, (1.0, 0.0)) == want((1.0, 0.0)) && transform_vec(*c, (0.0, 1.0)) == want((0.0, 1.0)))
        .unwrap_or(T::Normal)
}

/// Вывод, который поворачивает `rotate`: встроенная панель, нет её — основной.
pub fn rotation_target(core: &Core) -> Option<Output> {
    core.space.outputs().find(|o| is_internal(o)).cloned().or_else(|| core.primary_output())
}

/// Поворот вывода поверх `transform` из конфига: заданный на лету (`rotate`),
/// а пока его не было (старт) — только для [`rotation_target`]: при выключенном
/// автоповороте — выбранный вручную (`[rotation] orientation`), иначе — последний
/// применённый в прошлый раз (`rotation_state`, по датчику или вручную), нет его — ручной.
pub fn current_rotation(core: &Core, output: &Output) -> synshell_common::action::Rotation {
    let name = output.name();
    if let Some(r) = core.rotation.get(&name) {
        return *r;
    }
    if !rotation_target(core).is_some_and(|t| &t == output) {
        return Default::default();
    }
    startup_rotation(&core.config.rotation, core.saved_rotation.get(&name).copied())
}

/// `transform` нового вывода с запомненной ориентацией ([`startup_rotation`]), если его поворачивает
/// `rotate`: встроенная панель или первый вывод. Ставится при создании вывода, до первого кадра:
/// поворот уже включённого вывода (`apply_output_config`) сбрасывает буферы и даёт лишний коммит.
pub fn restored_transform_on_connect(core: &mut Core, output: &Output, base: smithay::utils::Transform) -> smithay::utils::Transform {
    let name = output.name();
    if core.rotation.contains_key(&name) || !(is_internal(output) || core.space.outputs().next().is_none()) {
        return base;
    }
    let r = startup_rotation(&core.config.rotation, core.saved_rotation.get(&name).copied());
    if r == synshell_common::action::Rotation::Normal {
        return base;
    }
    tracing::info!(output = name, rotation = r.as_str(), "поворот экрана при старте");
    core.rotation.insert(name, r);
    rotate_transform(base, r.quarters())
}

/// Ориентация при старте: см. [`current_rotation`].
pub fn startup_rotation(
    cfg: &synshell_common::config::ScreenRotation,
    saved: Option<synshell_common::action::Rotation>,
) -> synshell_common::action::Rotation {
    match (cfg.auto, cfg.orientation) {
        (false, Some(r)) => r,
        (_, manual) => saved.or(manual).unwrap_or_default(),
    }
}

/// Довернуть `t` на `q` четвертей по часовой стрелке (отражение сохраняется).
pub fn rotate_transform(t: smithay::utils::Transform, q: u8) -> smithay::utils::Transform {
    use smithay::utils::Transform as T;
    let (flipped, base) = match t {
        T::Normal => (false, 0),
        T::_90 => (false, 1),
        T::_180 => (false, 2),
        T::_270 => (false, 3),
        T::Flipped => (true, 0),
        T::Flipped90 => (true, 1),
        T::Flipped180 => (true, 2),
        T::Flipped270 => (true, 3),
    };
    match ((base + q) % 4, flipped) {
        (0, false) => T::Normal,
        (1, false) => T::_90,
        (2, false) => T::_180,
        (3, false) => T::_270,
        (0, true) => T::Flipped,
        (1, true) => T::Flipped90,
        (2, true) => T::Flipped180,
        _ => T::Flipped270,
    }
}

/// Встроенная панель (телефон, ноутбук) — её поворачивает `rotate`.
pub fn is_internal(output: &Output) -> bool {
    let n = output.name();
    // Unknown — коннектор simpledrm/efidrm: экран, который включил загрузчик (планшет без дисплейного драйвера, SM-T295)
    ["DSI", "eDP", "LVDS", "DPI", "Unknown"].iter().any(|p| n.starts_with(p))
}

/// Разбор `transform` из конфига.
pub fn parse_transform(s: &str) -> smithay::utils::Transform {
    use smithay::utils::Transform as T;
    match s.trim() {
        "90" => T::_90,
        "180" => T::_180,
        "270" => T::_270,
        "flipped" => T::Flipped,
        "flipped-90" => T::Flipped90,
        "flipped-180" => T::Flipped180,
        "flipped-270" => T::Flipped270,
        _ => T::Normal,
    }
}

pub fn transform_name(t: smithay::utils::Transform) -> &'static str {
    use smithay::utils::Transform as T;
    match t {
        T::Normal => "normal",
        T::_90 => "90",
        T::_180 => "180",
        T::_270 => "270",
        T::Flipped => "flipped",
        T::Flipped90 => "flipped-90",
        T::Flipped180 => "flipped-180",
        T::Flipped270 => "flipped-270",
    }
}

/// Масштаб «по DPI», если в конфиге 0: как у Plasma — ступенями по 0.25.
/// Масштаб для телефона: DRM-панели часто сообщают размер в неверных единицах,
/// поэтому берём логическую ширину ≈ 400 (как 360–412 dp в Android).
pub fn phone_scale(pixels: (i32, i32)) -> f64 {
    let w = pixels.0.min(pixels.1).max(1) as f64;
    (((w / 400.0) * 2.0).round() / 2.0).clamp(1.0, 4.0)
}

pub fn auto_scale(physical_mm: (i32, i32), pixels: (i32, i32)) -> f64 {
    let (mm_w, _) = physical_mm;
    if mm_w <= 0 || pixels.0 <= 0 {
        return 1.0;
    }
    let dpi = pixels.0 as f64 / (mm_w as f64 / 25.4);
    // 96 dpi — 1.0; ноутбуки ~ 140–170 dpi — 1.25–1.5.
    let raw = dpi / 110.0;
    ((raw * 4.0).round() / 4.0).clamp(1.0, 3.0)
}

/// Обратная связь linux-dmabuf вывода: для отрисовки композитором и для
/// вывода буфера прямо на план (первым — форматы плана с флагом scanout).
pub struct OutputFeedback {
    pub render: smithay::wayland::dmabuf::DmabufFeedback,
    pub scanout: smithay::wayland::dmabuf::DmabufFeedback,
}

impl Core {
    /// Поверхностям вывода — подходящая обратная связь dmabuf: попавшей (или
    /// почти попавшей) на план — с форматами плана, остальным — для отрисовки.
    /// Клиент (Mesa WSI) перевыбирает формат и модификатор буферов; smithay
    /// шлёт обратную связь, только когда она у поверхности сменилась.
    pub fn send_dmabuf_feedback(&self, output: &Output, fb: &OutputFeedback, states: &RenderElementStates) {
        use smithay::backend::renderer::element::utils::select_dmabuf_feedback;
        for w in self.space.elements() {
            w.send_dmabuf_feedback(output, surface_primary_scanout_output, |surface, _| {
                select_dmabuf_feedback(surface, states, &fb.render, &fb.scanout)
            });
        }
        let map = layer_map_for_output(output);
        for layer in map.layers() {
            layer.send_dmabuf_feedback(output, surface_primary_scanout_output, |surface, _| {
                select_dmabuf_feedback(surface, states, &fb.render, &fb.scanout)
            });
        }
    }

    /// После кадра: основной вывод поверхностей, frame callbacks,
    /// дробный масштаб.
    pub fn post_repaint(&mut self, output: &Output, states: &RenderElementStates) {
        let time = self.clock.now();
        let throttle = Some(Duration::from_secs(1));
        // Режим пера E Ink: окнам на выводе — frame callback каждым кадром, без секундного
        // throttle (как send_frames_headless), даже если основной вывод поверхности сброшен.
        let pen_geo = self.pen.fast().then(|| self.space.output_geometry(output).unwrap_or_default());

        for w in self.space.elements() {
            w.with_surfaces(|surface, s| {
                update_surface_primary_scanout_output(surface, output, s, states, smithay::backend::renderer::element::default_primary_scanout_output_compare);
            });
            if pen_geo.is_some_and(|geo| self.space.element_geometry(w).is_some_and(|g| g.overlaps(geo))) {
                w.send_frame(output, time, throttle, |_, _| Some(output.clone()));
            } else {
                w.send_frame(output, time, throttle, surface_primary_scanout_output);
            }
        }
        // Окна в анимации переключения столов (не в Space) тоже получают кадры.
        if self.wm.switch.is_some() {
            for m in &self.wm.windows {
                if m.mapped && !self.space.elements().any(|e| e == &m.window) {
                    m.window.send_frame(output, time, throttle, |_, _| Some(output.clone()));
                }
            }
        }
        let map = layer_map_for_output(output);
        for layer in map.layers() {
            layer.with_surfaces(|surface, s| {
                update_surface_primary_scanout_output(surface, output, s, states, smithay::backend::renderer::element::default_primary_scanout_output_compare);
            });
            layer.send_frame(output, time, throttle, surface_primary_scanout_output);
        }
        drop(map);
        if let Some(ls) = self.lock_surfaces.get(output) {
            send_frames_surface_tree(ls.wl_surface(), output, time);
        }
        if let CursorImageStatus::Surface(s) = &self.cursor_status {
            send_frames_surface_tree(s, output, time);
        }
        if let Some(icon) = &self.dnd_icon {
            send_frames_surface_tree(icon, output, time);
        }
    }

    /// Кадр без экрана (монитор погашен, а вывод смотрят через поток кадров
    /// synlink): frame callbacks всем поверхностям вывода — клиенты рисуют
    /// дальше, анимации идут.
    pub fn send_frames_headless(&mut self, output: &Output) {
        let time = self.clock.now();
        let throttle = Some(Duration::from_secs(1));
        let geo = self.space.output_geometry(output).unwrap_or_default();
        for w in self.space.elements() {
            let on = self.space.element_geometry(w).is_some_and(|g| g.overlaps(geo));
            if on {
                w.send_frame(output, time, throttle, |_, _| Some(output.clone()));
            }
        }
        let map = layer_map_for_output(output);
        for layer in map.layers() {
            layer.send_frame(output, time, throttle, |_, _| Some(output.clone()));
        }
        drop(map);
        if let Some(ls) = self.lock_surfaces.get(output) {
            send_frames_surface_tree(ls.wl_surface(), output, time);
        }
    }

    /// Обратная связь о показе кадра (presentation-time).
    pub fn take_presentation_feedback(&self, output: &Output, states: &RenderElementStates) -> OutputPresentationFeedback {
        let mut feedback = OutputPresentationFeedback::new(output);
        for w in self.space.elements() {
            w.take_presentation_feedback(&mut feedback, surface_primary_scanout_output, |surface, _| {
                surface_presentation_feedback_flags_from_states(surface, states)
            });
        }
        let map = layer_map_for_output(output);
        for layer in map.layers() {
            layer.take_presentation_feedback(&mut feedback, surface_primary_scanout_output, |surface, _| {
                surface_presentation_feedback_flags_from_states(surface, states)
            });
        }
        feedback
    }

    /// Нужны ли следующие кадры сами по себе (анимации, анимированный курсор).
    pub fn wants_continuous_redraw(&mut self) -> bool {
        let anim = self.wm.tick_animations();
        let cursor_anim = match &self.cursor_status {
            CursorImageStatus::Named(icon) => self.cursor.is_animated(*icon),
            _ => false,
        };
        // Анимация поворота кончилась — убрать снимок (ещё один кадр без него).
        let rotating = match &self.rotate_anim {
            Some(a) if a.done() => {
                self.rotate_anim = None;
                self.queue_redraw_all();
                false
            }
            Some(_) => true,
            None => false,
        };
        anim || cursor_anim || rotating
    }
}

fn send_frames_surface_tree(surface: &WlSurface, output: &Output, time: smithay::utils::Time<smithay::utils::Monotonic>) {
    use smithay::wayland::compositor::{SurfaceAttributes, TraversalAction};
    with_surface_tree_downward(
        surface,
        (),
        |_, _, _| TraversalAction::DoChildren(()),
        |_, states, _| {
            for callback in states.cached_state.get::<SurfaceAttributes>().current().frame_callbacks.drain(..) {
                callback.done(time.as_millis());
            }
        },
        |_, _, _| true,
    );
    let _ = output;
}

impl State {
    /// Нарисовать все выводы, ожидающие перерисовки.
    pub fn redraw_queued(&mut self) {
        let outputs: Vec<Output> = self.core.space.outputs().cloned().collect();
        for o in outputs {
            let queued = self
                .core
                .output_data
                .get(&o)
                .is_some_and(|d| d.redraw == crate::state::RedrawState::Queued);
            if queued && self.core.monitors_off && self.has_frame_streams(&o) {
                self.headless_frame(&o);
            } else if queued {
                self.backend.render(&mut self.core, &o);
                let damaged = self.core.output_data.get(&o).is_some_and(|d| d.damaged);
                if damaged {
                    self.flush_pending_copies(&o);
                    self.frame_streams_damaged(&o);
                }
            }
        }
    }

    /// Кадр погашенного вывода для потока кадров: не чаще 60 раз в секунду
    /// (раньше — таймер), клиентам — frame callbacks, потокам — изменения.
    fn headless_frame(&mut self, o: &Output) {
        const PERIOD: std::time::Duration = std::time::Duration::from_millis(16);
        let last = self.core.output_data.get(o).map(|d| d.last_frame).unwrap_or_else(std::time::Instant::now);
        let el = last.elapsed();
        if el < PERIOD {
            if !self.core.headless_timer {
                self.core.headless_timer = true;
                let _ = self.core.loop_handle.insert_source(
                    smithay::reexports::calloop::timer::Timer::from_duration(PERIOD - el),
                    |_, _, st: &mut State| {
                        st.core.headless_timer = false;
                        smithay::reexports::calloop::timer::TimeoutAction::Drop
                    },
                );
            }
            return;
        }
        if let Some(d) = self.core.output_data.get_mut(o) {
            d.redraw = crate::state::RedrawState::Idle;
            d.last_frame = std::time::Instant::now();
        }
        self.core.send_frames_headless(o);
        self.frame_streams_damaged(o);
    }

    /// Действие `rotate`: повернуть встроенную панель (нет её — вывод в
    /// фокусе) поверх `transform` из конфига.
    pub fn rotate(&mut self, r: synshell_common::action::Rotation) {
        let Some(o) = rotation_target(&self.core) else { return };
        let name = o.name();
        let old = current_rotation(&self.core, &o);
        if old == r {
            return;
        }
        tracing::info!(output = name, rotation = r.as_str(), "поворот экрана");
        let ms = self.core.config.rotation.animation_ms;
        self.core.rotate_anim = None;
        // На E Ink каждый кадр анимации — обновление панели: сразу новый кадр.
        if ms > 0 && !self.core.eink {
            // Снимок до поворота: только на GPU (на CPU поворот сразу).
            let quarters = (r.quarters() + 4 - old.quarters()) % 4;
            if let Some(Ok(anim)) = self.backend.snapshot(&mut self.core, &o).map(|s| {
                s.map(|(tex, size)| crate::rotate_anim::RotateAnim::new(o.clone(), tex, size, quarters, std::time::Duration::from_millis(ms as u64)))
            }) {
                self.core.rotate_anim = Some(anim);
            }
        }
        // Запомнить для следующего старта (и экрана входа) — состояние, не конфиг.
        if self.core.saved_rotation.get(&name) != Some(&r) {
            match synshell_common::rotation_state::save(&name, r) {
                Ok(path) => tracing::debug!(output = name, rotation = r.as_str(), path = %path.display(), "ориентация запомнена"),
                Err(e) => tracing::warn!(output = name, ?e, "ориентация не запомнена"),
            }
            self.core.saved_rotation.insert(name.clone(), r);
        }
        self.core.rotation.insert(name, r);
        self.backend.apply_output_config(&mut self.core);
        self.outputs_changed();
        self.core.queue_redraw_all();
    }

    /// Только что подключённый поворачиваемый вывод ещё в `transform` из конфига: довернуть его в
    /// запомненную ориентацию ([`current_rotation`]: прошлый запуск или `[rotation] orientation`).
    pub fn apply_restored_rotation(&mut self) {
        let Some(o) = rotation_target(&self.core) else { return };
        let want = output_transform(&self.core, &o);
        if o.current_transform() == want {
            return;
        }
        let r = current_rotation(&self.core, &o);
        tracing::info!(output = o.name(), rotation = r.as_str(), "поворот экрана при старте (после подключения)");
        self.core.rotation.insert(o.name(), r);
        self.backend.apply_output_config(&mut self.core);
    }

    pub fn set_monitors_power(&mut self, on: bool) {
        if self.core.monitors_off == !on {
            return;
        }
        self.core.monitors_off = !on;
        self.backend.set_monitors_power(&mut self.core, on);
        if on {
            self.core.queue_redraw_all();
            self.core.proximity_blank = false;
        }
        // Погасил датчик приближения — идёт разговор, системе спать нельзя
        if on || !self.core.proximity_blank {
            crate::sleep::screen_power(on, self.core.config.idle.sleep_delay);
        }
        self.core.ipc.broadcast(&synshell_common::ipc::Event::ScreenPower { on });
    }
}

#[cfg(test)]
mod rotation_tests {
    use super::*;
    use smithay::utils::Transform as T;

    #[test]
    fn compose() {
        assert_eq!(compose_transform(T::Normal, T::Flipped180), T::Flipped180);
        assert_eq!(compose_transform(T::_90, T::_90), T::_180);
        assert_eq!(compose_transform(T::_270, T::_90), T::Normal);
        assert_eq!(compose_transform(T::Flipped180, T::Flipped180), T::Normal);
        // Вложенное окно: поворот поверх переворота GL и обратно.
        for q in 0..4 {
            let r = rotate_transform(T::Normal, q);
            let w = compose_transform(r, T::Flipped180);
            assert_eq!(compose_transform(w, T::Flipped180), r);
        }
    }

    #[test]
    fn startup_rotation_policy() {
        use synshell_common::action::Rotation as R;
        let mut cfg = synshell_common::config::ScreenRotation::default();
        // автоповорот: последняя применённая, нет её — ручная, нет и её — без поворота
        assert_eq!(startup_rotation(&cfg, None), R::Normal);
        assert_eq!(startup_rotation(&cfg, Some(R::R90)), R::R90);
        cfg.orientation = Some(R::R270);
        assert_eq!(startup_rotation(&cfg, Some(R::R90)), R::R90);
        assert_eq!(startup_rotation(&cfg, None), R::R270);
        // зафиксировано вручную — ручная
        cfg.auto = false;
        assert_eq!(startup_rotation(&cfg, Some(R::R90)), R::R270);
        cfg.orientation = None;
        assert_eq!(startup_rotation(&cfg, Some(R::R90)), R::R90);
    }
}
