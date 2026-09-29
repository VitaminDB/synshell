//! Вложенный режим: композитор в окне текущего сеанса (для разработки и
//! проверки без выхода из рабочего стола).

use std::time::Duration;

use smithay::{
    backend::{
        allocator::{dmabuf::Dmabuf, Fourcc},
        egl::EGLDevice,
        renderer::{
            damage::OutputDamageTracker, gles::GlesRenderer, Bind, ExportMem, ImportDma, ImportMemWl, Offscreen,
        },
        winit::{self, WinitEvent, WinitGraphicsBackend},
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::EventLoop,
        wayland_protocols::wp::presentation_time::server::wp_presentation_feedback,
        winit::{dpi::LogicalSize, window::Window as WinitWindow},
    },
    utils::{Rectangle, Size, Transform},
    wayland::{dmabuf::DmabufFeedbackBuilder, presentation::Refresh},
};

use crate::state::{Core, RedrawState, State};

pub struct WinitBackend {
    backend: WinitGraphicsBackend<GlesRenderer>,
    damage_tracker: OutputDamageTracker,
    pub output: Output,
}

impl WinitBackend {
    /// Открыть окно и создать вывод. Возвращает бэкенд; источник событий
    /// winit регистрируется в цикле.
    pub fn new(event_loop: &EventLoop<'static, State>) -> anyhow::Result<Self> {
        let attrs = WinitWindow::default_attributes()
            .with_title("synwm (вложенный)")
            .with_inner_size(LogicalSize::new(1600.0, 960.0))
            .with_visible(true);
        let (backend, winit_loop) = winit::init_from_attributes::<GlesRenderer>(attrs)
            .map_err(|e| anyhow::anyhow!("winit: {e}"))?;
        let size = backend.window_size();
        let mode = Mode { size, refresh: 60_000 };
        let output = Output::new(
            "winit-0".into(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "synwm".into(),
                model: "Вложенное окно".into(),
            },
        );
        output.change_current_state(Some(mode), Some(Transform::Flipped180), None, Some((0, 0).into()));
        output.set_preferred(mode);
        let damage_tracker = OutputDamageTracker::from_output(&output);

        event_loop
            .handle()
            .insert_source(winit_loop, |event, _, state| match event {
                WinitEvent::Resized { size, .. } => {
                    if let crate::backend::Backend::Winit(w) = &mut state.backend {
                        let mode = Mode { size, refresh: 60_000 };
                        w.output.change_current_state(Some(mode), None, None, None);
                        w.output.set_preferred(mode);
                    }
                    state.outputs_changed();
                    state.core.queue_redraw_all();
                }
                WinitEvent::Input(event) => state.process_input_event(event),
                WinitEvent::Redraw => state.core.queue_redraw_all(),
                WinitEvent::CloseRequested => state.quit(),
                WinitEvent::Focus(_) => {}
            })
            .map_err(|e| anyhow::anyhow!("winit source: {e}"))?;

        Ok(Self { backend, damage_tracker, output })
    }

    /// Глобалы, зависящие от рендерера: dmabuf, форматы shm.
    pub fn init_globals(&mut self, core: &mut Core) {
        core.shm_state.update_formats(self.backend.renderer().shm_formats());
        let formats = self.backend.renderer().dmabuf_formats();
        let node = EGLDevice::device_for_display(self.backend.renderer().egl_context().display())
            .and_then(|d| d.try_get_render_node());
        let global = match node {
            Ok(Some(node)) => {
                let feedback = DmabufFeedbackBuilder::new(node.dev_id(), formats.clone()).build().ok();
                match feedback {
                    Some(f) => core.dmabuf_state.create_global_with_default_feedback::<State>(&core.display_handle, &f),
                    None => core.dmabuf_state.create_global::<State>(&core.display_handle, formats),
                }
            }
            _ => core.dmabuf_state.create_global::<State>(&core.display_handle, formats),
        };
        core.dmabuf_global = Some(global);
        let _global = self.output.create_global::<State>(&core.display_handle);
        core.space.map_output(&self.output, (0, 0));
        core.output_data.entry(self.output.clone()).or_default();
    }

    pub fn import_dmabuf(&mut self, dmabuf: &Dmabuf) -> bool {
        self.backend.renderer().import_dmabuf(dmabuf, None).is_ok()
    }

    pub fn gles(&mut self) -> &mut GlesRenderer {
        self.backend.renderer()
    }

    pub fn render(&mut self, core: &mut Core, output: &Output) {
        let age = self.backend.buffer_age().unwrap_or(0);
        let mut damaged = false;
        let result = {
            let (renderer, mut fb) = match self.backend.bind() {
                Ok(x) => x,
                Err(e) => {
                    tracing::warn!(?e, "winit: bind");
                    return;
                }
            };
            let (elements, clear) = crate::render::output_elements(core, renderer, output, true);
            self.damage_tracker.render_output(renderer, &mut fb, age, &elements, clear)
        };
        match result {
            Ok(r) => {
                let has_damage = r.damage.is_some();
                damaged = has_damage;
                if let Some(damage) = r.damage {
                    if let Err(e) = self.backend.submit(Some(damage)) {
                        tracing::warn!(?e, "winit: submit");
                    }
                }
                let states = r.states;
                if has_damage {
                    // Кадр показан (swap ждёт vsync) — сразу frame callbacks.
                    let mut fb = core.take_presentation_feedback(output, &states);
                    fb.presented(
                        core.clock.now(),
                        Refresh::fixed(Duration::from_micros(16_667)),
                        0,
                        wp_presentation_feedback::Kind::Vsync,
                    );
                    core.post_repaint(output, &states);
                } else {
                    // Без повреждений — по расчётному кадру, иначе клиенты,
                    // рисующие одно и то же, крутились бы без паузы.
                    let o = output.clone();
                    let _ = core.loop_handle.insert_source(
                        smithay::reexports::calloop::timer::Timer::from_duration(Duration::from_millis(16)),
                        move |_, _, state| {
                            let states = smithay::backend::renderer::element::RenderElementStates::default();
                            state.core.post_repaint(&o, &states);
                            smithay::reexports::calloop::timer::TimeoutAction::Drop
                        },
                    );
                }
                // Курсор рисуем сами — скрыть системный курсор хоста.
                self.backend.window().set_cursor_visible(false);
            }
            Err(e) => tracing::warn!(?e, "winit: render"),
        }
        let data = core.output_data.entry(output.clone()).or_default();
        data.redraw = RedrawState::Idle;
        data.damaged = damaged;
        data.frames += 1;
        data.last_frame = std::time::Instant::now();
        if core.wants_continuous_redraw() {
            core.queue_redraw(output);
        }
    }

    pub fn screenshot(&mut self, core: &mut Core, output: &Output) -> anyhow::Result<(u32, u32, Vec<u8>)> {
        let size = output.current_mode().map(|m| m.size).unwrap_or_default();
        let transformed = output.current_transform().transform_size(size);
        let buffer_size = Size::from((transformed.w, transformed.h));
        let renderer = self.backend.renderer();
        let mut texture: smithay::backend::renderer::gles::GlesTexture =
            renderer.create_buffer(Fourcc::Abgr8888, buffer_size)?;
        let (elements, clear) = crate::render::output_elements(core, renderer, output, false);
        {
            let mut fb = renderer.bind(&mut texture)?;
            let mut tracker = OutputDamageTracker::new(transformed, output.current_scale().fractional_scale(), Transform::Normal);
            tracker.render_output(renderer, &mut fb, 0, &elements, clear)?;
        }
        let fb = renderer.bind(&mut texture)?;
        let mapping = renderer.copy_framebuffer(&fb, Rectangle::from_size(buffer_size), Fourcc::Abgr8888)?;
        let data = renderer.map_texture(&mapping)?.to_vec();
        Ok((buffer_size.w as u32, buffer_size.h as u32, data))
    }

    pub fn apply_output_config(&mut self, core: &mut Core) {
        // У вложенного окна один вывод: из конфига берём только масштаб.
        let scale = core
            .config
            .outputs
            .iter()
            .find(|o| crate::backend::output_matches(&self.output, &o.name) || o.name == "winit")
            .map(|o| o.scale)
            .filter(|s| *s > 0.0)
            .unwrap_or(1.0);
        self.output.change_current_state(None, None, Some(smithay::output::Scale::Fractional(scale)), None);
    }
}
