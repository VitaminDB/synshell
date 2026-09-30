//! Бэкенды вывода: сеанс DRM/KMS с GPU (`tty`), сеанс DRM/KMS на CPU
//! (`kms_cpu`: телефоны без GBM/EGL) и вложенное окно (`winit`).

#[cfg(feature = "pixman")]
pub mod kms_cpu;
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
    pub fn screenshot(&mut self, core: &mut Core, output: &Output) -> anyhow::Result<(u32, u32, Vec<u8>)> {
        match self {
            Backend::Winit(w) => w.screenshot(core, output),
            Backend::Tty(t) => t.screenshot(core, output),
            #[cfg(feature = "pixman")]
            Backend::KmsCpu(b) => b.screenshot(core, output),
        }
    }

    /// Кадр потока synlink: дорисовать постоянный буфер по повреждениям.
    pub fn stream_frame(
        &mut self,
        core: &mut Core,
        output: &Output,
        slot: &mut Option<Box<dyn std::any::Any>>,
        cursor: bool,
    ) -> anyhow::Result<crate::stream::Rendered> {
        match self {
            Backend::Winit(w) => w.stream_frame(core, output, slot, cursor),
            Backend::Tty(t) => t.stream_frame(core, output, slot, cursor),
            #[cfg(feature = "pixman")]
            Backend::KmsCpu(b) => b.stream_frame(core, output, slot, cursor),
        }
    }

    /// Доступ к GLES-рендереру основного GPU (снимки, захват экрана).
    pub fn with_gles<T>(&mut self, f: impl FnOnce(&mut smithay::backend::renderer::gles::GlesRenderer) -> T) -> Option<T> {
        match self {
            Backend::Winit(w) => Some(f(w.gles())),
            Backend::Tty(t) => t.with_gles(f),
            #[cfg(feature = "pixman")]
            Backend::KmsCpu(_) => None,
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
) -> anyhow::Result<smithay::reexports::input::Libinput> {
    use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
    use smithay::backend::session::Session;
    use smithay::reexports::input::{DeviceCapability, Libinput};
    let seat = session.seat();
    let mut libinput = Libinput::new_with_udev::<LibinputSessionInterface<_>>(session.clone().into());
    libinput
        .udev_assign_seat(&seat)
        .map_err(|_| anyhow::anyhow!("libinput: не удалось назначить seat"))?;
    let input_backend = LibinputInputBackend::new(libinput.clone());
    event_loop.handle().insert_source(input_backend, |mut event, _, state| {
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
    Ok(libinput)
}

/// Путь к DRM-устройству сеанса: `SYNSHELL_DRM_DEVICE`, иначе основной GPU по udev
/// (seat0 — до открытия сеанса libseat).
pub fn kms_cpu_device_path() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("SYNSHELL_DRM_DEVICE") {
        return Some(std::path::PathBuf::from(p));
    }
    let seat = std::env::var("XDG_SEAT").unwrap_or_else(|_| "seat0".into());
    smithay::backend::udev::primary_gpu(&seat)
        .ok()
        .flatten()
        .or_else(|| smithay::backend::udev::all_gpus(&seat).ok()?.into_iter().next())
}

/// Есть ли на устройстве GPU-рендеринг (EGL поверх GBM). Без него сеанс
/// DRM возможен только на CPU (`kms_cpu`).
pub fn probe_gpu(path: &std::path::Path) -> bool {
    use smithay::backend::egl::{EGLDevice, EGLDisplay};
    use smithay::reexports::gbm::Device as GbmDevice;
    let Ok(file) = std::fs::OpenOptions::new().read(true).write(true).open(path) else { return false };
    let fd = smithay::backend::drm::DrmDeviceFd::new(smithay::utils::DeviceFd::from(std::os::fd::OwnedFd::from(file)));
    let Ok(gbm) = GbmDevice::new(fd) else { return false };
    let Ok(display) = (unsafe { EGLDisplay::new(gbm) }) else { return false };
    EGLDevice::device_for_display(&display).map(|d| !d.is_software()).unwrap_or(false)
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

impl Core {
    /// После кадра: основной вывод поверхностей, frame callbacks,
    /// дробный масштаб.
    pub fn post_repaint(&mut self, output: &Output, states: &RenderElementStates) {
        let time = self.clock.now();
        let throttle = Some(Duration::from_secs(1));

        for w in self.space.elements() {
            w.with_surfaces(|surface, s| {
                update_surface_primary_scanout_output(surface, output, s, states, smithay::backend::renderer::element::default_primary_scanout_output_compare);
            });
            w.send_frame(output, time, throttle, surface_primary_scanout_output);
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
        anim || cursor_anim
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

    pub fn set_monitors_power(&mut self, on: bool) {
        if self.core.monitors_off == !on {
            return;
        }
        self.core.monitors_off = !on;
        self.backend.set_monitors_power(&mut self.core, on);
        if on {
            self.core.queue_redraw_all();
        }
    }
}
