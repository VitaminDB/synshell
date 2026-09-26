//! Одна поверхность: layer-surface Wayland (или offscreen в headless),
//! дерево syngui, свой рендерер.

use crate::gpu::{self, Gpu, Offscreen};
use crate::{Factory, SurfaceHooks, SurfaceId, SurfaceSpec};
use smithay_client_toolkit::reexports::protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::WpFractionalScaleV1;
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::wp_viewport::WpViewport;
use smithay_client_toolkit::session_lock::SessionLockSurface;
use smithay_client_toolkit::shell::wlr_layer::{Anchor, LayerSurface};
use smithay_client_toolkit::shell::WaylandSurface;
use std::path::Path;
use std::time::Instant;
use syngui::core::{Color, Size};
use syngui::embed::EmbedView;
use syngui::gpu::{Renderer, WindowSurface};
use syngui::mss::StyleEngine;
use syngui::render::DisplayList;
use wayland_client::QueueHandle;

pub enum Backend {
    /// Ждёт появления нужного вывода.
    Pending,
    Wayland {
        // Порядок важен: wgpu-поверхность уничтожается раньше wl_surface.
        wsurf: Option<WindowSurface>,
        viewport: Option<WpViewport>,
        fractional: Option<WpFractionalScaleV1>,
        role: Role,
    },
    Headless {
        offscreen: Option<Offscreen>,
    },
}

/// Роль `wl_surface`: layer-поверхность или поверхность блокировки экрана.
pub enum Role {
    Layer(LayerSurface),
    Lock(SessionLockSurface),
}

impl Role {
    pub fn wl_surface(&self) -> &wayland_client::protocol::wl_surface::WlSurface {
        match self {
            Role::Layer(l) => l.wl_surface(),
            Role::Lock(l) => l.wl_surface(),
        }
    }
}

pub struct Surface {
    pub id: SurfaceId,
    pub spec: SurfaceSpec,
    pub hooks: SurfaceHooks,
    pub factory: Option<Factory>,
    pub view: EmbedView,
    pub backend: Backend,
    pub renderer: Option<Renderer>,
    /// Размер из configure (логический).
    pub logical: (u32, u32),
    /// Масштаб ×120 (как у wp_fractional_scale).
    pub scale120: u32,
    /// Масштаб поверхности пришёл дробным протоколом.
    pub has_fractional: bool,
    pub configured: bool,
    pub frame_pending: bool,
    pub needs_frame: bool,
    last_anim: Instant,
    display_list: DisplayList,
    pub cursor: syngui::input::CursorIcon,
    /// Размер, который мы в последний раз попросили у композитора.
    pub requested: (u32, u32),
    /// Физический размер буфера, под который настроены рендерер/поверхность.
    phys: (u32, u32),
}

impl Surface {
    pub fn new(id: SurfaceId, spec: SurfaceSpec, hooks: SurfaceHooks, factory: Factory) -> Self {
        let requested = spec.size;
        Self {
            id,
            spec,
            hooks,
            factory: Some(factory),
            view: EmbedView::new(),
            backend: Backend::Pending,
            renderer: None,
            logical: (0, 0),
            scale120: 120,
            has_fractional: false,
            configured: false,
            frame_pending: false,
            needs_frame: true,
            last_anim: Instant::now(),
            display_list: DisplayList::new(),
            cursor: syngui::input::CursorIcon::Default,
            requested,
            phys: (0, 0),
        }
    }

    pub fn layer(&self) -> Option<&LayerSurface> {
        match &self.backend {
            Backend::Wayland { role: Role::Layer(l), .. } => Some(l),
            _ => None,
        }
    }

    pub fn wl_surface(&self) -> Option<&wayland_client::protocol::wl_surface::WlSurface> {
        match &self.backend {
            Backend::Wayland { role, .. } => Some(role.wl_surface()),
            _ => None,
        }
    }

    pub fn is_lock(&self) -> bool {
        matches!(self.backend, Backend::Wayland { role: Role::Lock(_), .. })
    }

    pub fn scale(&self) -> f64 {
        self.scale120 as f64 / 120.0
    }

    /// Применить параметры спецификации к layer-surface (без commit).
    pub fn apply_spec(&self) {
        let Some(layer) = self.layer() else { return };
        let s = &self.spec;
        layer.set_anchor(s.anchor);
        // Ноль по оси без обоих якорей — ошибка протокола; до замера
        // содержимого (`auto_size`) просим 1.
        let (sx, sy) = self.stretched();
        let w = if self.requested.0 == 0 && !sx { 1 } else { self.requested.0 };
        let h = if self.requested.1 == 0 && !sy { 1 } else { self.requested.1 };
        layer.set_size(w, h);
        layer.set_margin(s.margin[0], s.margin[1], s.margin[2], s.margin[3]);
        layer.set_exclusive_zone(s.exclusive_zone);
        layer.set_keyboard_interactivity(s.keyboard);
    }

    /// Разобрать поверхность Wayland (дерево остаётся) — перед пересозданием.
    pub fn teardown(&mut self) {
        if let Backend::Wayland { wsurf, viewport, fractional, .. } = &mut self.backend {
            wsurf.take();
            if let Some(v) = viewport.take() {
                v.destroy();
            }
            if let Some(f) = fractional.take() {
                f.destroy();
            }
        }
        self.backend = Backend::Pending;
        self.configured = false;
        self.frame_pending = false;
        self.phys = (0, 0);
        self.view.invalidate();
    }

    fn stretched(&self) -> (bool, bool) {
        let a = self.spec.anchor;
        (a.contains(Anchor::LEFT | Anchor::RIGHT), a.contains(Anchor::TOP | Anchor::BOTTOM))
    }

    /// Нарисовать кадр, если он нужен. Возвращает `true`, если что-то
    /// отправлено композитору.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        gpu: &mut Gpu,
        engine: &StyleEngine,
        qh: Option<&QueueHandle<crate::state::State>>,
        font_family: Option<String>,
        dump_dir: Option<&Path>,
        output_size: (u32, u32),
    ) -> anyhow::Result<bool> {
        if !self.configured || matches!(self.backend, Backend::Pending) {
            return Ok(false);
        }
        let (lw, lh) = self.logical;
        if lw == 0 || lh == 0 {
            return Ok(false);
        }
        let scale = self.scale();
        let phys = ((lw as f64 * scale).round().max(1.0) as u32, (lh as f64 * scale).round().max(1.0) as u32);

        // Поверхность wgpu и рендерер — лениво и при смене размера.
        let format = match &mut self.backend {
            Backend::Wayland { wsurf, role, .. } => {
                if wsurf.is_none() {
                    let s = gpu.create_surface(role.wl_surface())?;
                    let shared = gpu.ensure(Some(&s))?;
                    *wsurf = Some(gpu::configure_surface(shared, s, phys.0, phys.1));
                    self.phys = phys;
                }
                let ws = wsurf.as_mut().unwrap();
                if self.phys != phys {
                    ws.resize(&gpu.shared.as_ref().unwrap().device, phys.0, phys.1);
                }
                ws.surface_config.format
            }
            Backend::Headless { .. } => {
                gpu.ensure(None)?;
                wgpu::TextureFormat::Rgba8UnormSrgb
            }
            Backend::Pending => return Ok(false),
        };
        let shared = gpu.shared.as_ref().unwrap();
        if self.renderer.is_none() {
            let r = Renderer::new(shared, format, phys.0, phys.1, lw, lh, font_family);
            r.font_atlas
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .set_icon_font_data(syngui::text::icon_fonts::material::FONT_DATA.to_vec());
            r.font_atlas.lock().unwrap_or_else(|e| e.into_inner()).set_scale_factor(scale as f32);
            self.view.tree.text_measure =
                Some(r.font_atlas.clone() as std::sync::Arc<dyn syngui::widget::context::TextMeasure>);
            self.view.tree.image_store = Some(r.image_store.clone());
            self.renderer = Some(r);
            self.phys = phys;
            self.view.invalidate();
        } else if self.phys != phys {
            let r = self.renderer.as_mut().unwrap();
            r.resize(&shared.device, phys.0, phys.1, lw, lh);
            r.font_atlas.lock().unwrap_or_else(|e| e.into_inner()).set_scale_factor(scale as f32);
            self.phys = phys;
            self.view.invalidate();
        }

        // Дерево строится, когда у него уже есть измеритель текста и
        // хранилище картинок: иначе `Image` не поставит загрузку, а подгонка
        // размера померит текст оценкой.
        if let Some(factory) = self.factory.take() {
            let w = factory();
            self.view.mount(w, engine);
        }

        // Подгонка размера под содержимое по свободным осям.
        if self.spec.auto_size {
            let (sx, sy) = self.stretched();
            // Заданный размер по оси — жёсткий предел замера (ширина карточек
            // уведомлений), растянутая ось — её размер, иначе — вывод.
            let axis = |stretched: bool, fixed: u32, cur: u32, out: u32| -> f32 {
                if stretched {
                    cur as f32
                } else if fixed != 0 {
                    fixed as f32
                } else {
                    out.max(1) as f32
                }
            };
            let max = Size::new(
                axis(sx, self.spec.size.0, self.logical.0, output_size.0),
                axis(sy, self.spec.size.1, self.logical.1, output_size.1),
            );
            let m = self.view.measure(engine, max);
            let want = (
                if sx || self.spec.size.0 != 0 { self.spec.size.0 } else { m.width.ceil().max(1.0) as u32 },
                if sy || self.spec.size.1 != 0 { self.spec.size.1 } else { m.height.ceil().max(1.0) as u32 },
            );
            if want != self.requested {
                log::trace!("{} #{}: подгонка размера {:?} → {:?}", self.spec.namespace, self.id.0, self.requested, want);
                self.requested = want;
                match &self.backend {
                    Backend::Wayland { role: Role::Layer(layer), .. } => {
                        layer.set_size(want.0, want.1);
                        layer.commit();
                    }
                    Backend::Wayland { .. } => {}
                    Backend::Headless { .. } => {
                        self.logical = (
                            if want.0 == 0 { self.logical.0 } else { want.0 },
                            if want.1 == 0 { self.logical.1 } else { want.1 },
                        );
                        self.needs_frame = true;
                        return Ok(false);
                    }
                    Backend::Pending => {}
                }
            }
        }

        // Анимации.
        let now = Instant::now();
        let dt = now - self.last_anim;
        self.last_anim = now;
        let animating = self.view.animate(dt);

        let renderer = self.renderer.as_mut().unwrap();
        let images_busy = {
            let st = renderer.image_store.lock().unwrap_or_else(|e| e.into_inner());
            st.has_loading() || st.has_pending_uploads() || st.has_pending_frees()
        };
        let changed = self.view.frame(engine, Size::new(lw as f32, lh as f32), scale as f32, &mut self.display_list);
        // Кадр мог запустить переходы (hover сменил класс) — нулевой тик
        // скажет, идёт ли анимация; иначе она замерла бы до следующего события.
        let started = self.view.animate(std::time::Duration::ZERO);
        self.needs_frame = animating || started || images_busy;
        if changed && std::env::var_os("SYNGUI_LAYER_TREE").is_some() {
            if let Some(root) = self.view.root() {
                let mut out = String::new();
                dump_tree(&self.view.tree, root, 0, &mut out);
                log::info!("дерево {} #{}:\n{out}", self.spec.namespace, self.id.0);
            }
        }
        log::trace!(
            "кадр {} #{}: {lw}x{lh}@{scale}, изменён={changed}, анимация={}, картинки={images_busy}",
            self.spec.namespace,
            self.id.0,
            animating || started
        );
        if !changed && !images_busy {
            return Ok(false);
        }

        let c = self.spec.clear_color;
        let clear = Color::from_srgb(
            (c[0] * 255.0) as u8,
            (c[1] * 255.0) as u8,
            (c[2] * 255.0) as u8,
            c[3],
        );

        match &mut self.backend {
            Backend::Wayland { wsurf, viewport, role, .. } => {
                let wl = role.wl_surface();
                if let Some(vp) = viewport {
                    wl.set_buffer_scale(1);
                    vp.set_destination(lw as i32, lh as i32);
                } else {
                    wl.set_buffer_scale((self.scale120 / 120).max(1) as i32);
                }
                if let Some(qh) = qh {
                    wl.frame(qh, wl.clone());
                    self.frame_pending = true;
                }
                let stats = renderer.render(shared, wsurf.as_ref().unwrap(), &self.display_list, clear);
                if stats.draw_calls == 0 {
                    // Кадр не дошёл (поверхность потеряна) — следующий не пропускать.
                    self.view.invalidate();
                }
            }
            Backend::Headless { .. } => {}
            Backend::Pending => {}
        }

        if let Some(dir) = dump_dir {
            let off = match &mut self.backend {
                Backend::Headless { offscreen } => {
                    if offscreen.as_ref().is_none_or(|o| o.size != phys) {
                        *offscreen = Some(Offscreen::new(shared, format, phys.0, phys.1));
                    }
                    None
                }
                _ => Some(Offscreen::new(shared, format, phys.0, phys.1)),
            };
            let target = match (&self.backend, &off) {
                (_, Some(o)) => o,
                (Backend::Headless { offscreen: Some(o) }, None) => o,
                _ => unreachable!(),
            };
            renderer.render_to_view(shared, &target.view, phys, &self.display_list, clear);
            let path = dir.join(format!("{}-{}.png", self.spec.namespace, self.id.0));
            if let Err(e) = target.save_png(shared, &path) {
                log::warn!("syngui-layer: не удалось сохранить {}: {e}", path.display());
            }
        }
        Ok(true)
    }
}

/// Отладка: дерево элементов с границами и классами (`SYNGUI_LAYER_TREE=1`).
fn dump_tree(tree: &syngui::widget::ElementTree, id: syngui::widget::ElementId, depth: usize, out: &mut String) {
    let Some(el) = tree.get(id) else { return };
    let b = el.bounds();
    out.push_str(&format!(
        "{:indent$}{} [{:.0},{:.0} {:.0}x{:.0}] {}\n",
        "",
        el.element_type_name(),
        b.origin.x,
        b.origin.y,
        b.size.width,
        b.size.height,
        el.get_classes().join(" "),
        indent = depth * 2
    ));
    for c in tree.children_of(id).to_vec() {
        dump_tree(tree, c, depth + 1, out);
    }
}
