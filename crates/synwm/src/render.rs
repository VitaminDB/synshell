//! Сборка элементов кадра вывода (спереди назад) — общая для всех бэкендов.

use smithay::{
    backend::renderer::{
        element::{
            memory::MemoryRenderBufferRenderElement,
            solid::SolidColorRenderElement,
            surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement},
            utils::{Relocate, RelocateRenderElement, RescaleRenderElement},
            AsRenderElements, Id, Kind,
        },
        utils::CommitCounter,
        Color32F, ImportAll, ImportMem, Renderer,
    },
    desktop::{layer_map_for_output, LayerSurface},
    input::pointer::{CursorImageStatus, CursorImageSurfaceData},
    output::Output,
    utils::{Logical, Physical, Point, Rectangle, Scale, Size},
    wayland::{compositor::with_states, shell::wlr_layer::Layer},
};
use synshell_common::config::Rgba;

use crate::rotate_anim::RotateDraw;

use crate::{
    deco::TitleKey,
    state::Core,
    wm::{Managed, WindowId},
};

smithay::backend::renderer::element::render_elements! {
    /// Элементы одного окна (содержимое, рамка, тень, обводка).
    pub WinElement<R> where R: ImportAll + ImportMem;
    Surface = WaylandSurfaceRenderElement<R>,
    Memory = MemoryRenderBufferRenderElement<R>,
    Solid = SolidColorRenderElement,
}

smithay::backend::renderer::element::render_elements! {
    /// Элементы кадра вывода.
    pub OutputElement<R> where R: ImportAll + ImportMem + RotateDraw;
    Surface = WaylandSurfaceRenderElement<R>,
    Memory = MemoryRenderBufferRenderElement<R>,
    Solid = SolidColorRenderElement,
    Window = WinElement<R>,
    Transformed = RelocateRenderElement<RescaleRenderElement<WinElement<R>>>,
    Rotate = crate::rotate_anim::RotateElement,
}

/// Стабильные id сплошных элементов окна — для трекера повреждений.
pub struct WindowIds {
    pub border: [Id; 4],
    pub dim: Id,
    pub color_commit: std::cell::Cell<(u32, usize)>,
}

impl Default for WindowIds {
    fn default() -> Self {
        Self {
            border: [Id::new(), Id::new(), Id::new(), Id::new()],
            dim: Id::new(),
            color_commit: std::cell::Cell::new((0, 0)),
        }
    }
}

impl WindowIds {
    /// Счётчик коммитов, растущий при смене цвета — иначе трекер не увидит
    /// перекраску сплошного элемента того же размера.
    fn commit_for(&self, color: Rgba) -> CommitCounter {
        let key = u32::from_be_bytes([color.r, color.g, color.b, color.a]);
        let (last, mut n) = self.color_commit.get();
        if last != key {
            n += 1;
            self.color_commit.set((key, n));
        }
        CommitCounter::from(n)
    }
}

/// Постоянные id служебных элементов кадра.
pub struct FrameIds {
    pub lock_bg: Id,
    pub snap: [Id; 5],
    pub overview_dim: Id,
    pub overview_sel: [Id; 4],
}

impl Default for FrameIds {
    fn default() -> Self {
        Self {
            lock_bg: Id::new(),
            snap: [Id::new(), Id::new(), Id::new(), Id::new(), Id::new()],
            overview_dim: Id::new(),
            overview_sel: [Id::new(), Id::new(), Id::new(), Id::new()],
        }
    }
}

fn color32(c: Rgba, alpha: f32) -> Color32F {
    let [r, g, b, a] = c.to_f32();
    let a = a * alpha;
    Color32F::new(r * a, g * a, b * a, a)
}

fn to_phys(p: Point<i32, Logical>, scale: f64) -> Point<i32, Physical> {
    p.to_f64().to_physical(scale).to_i32_round()
}

fn rect_phys(r: Rectangle<i32, Logical>, scale: f64) -> Rectangle<i32, Physical> {
    r.to_f64().to_physical(scale).to_i32_round()
}

/// Все элементы вывода спереди назад и цвет очистки.
pub fn output_elements<R>(core: &mut Core, renderer: &mut R, output: &Output, include_cursor: bool) -> (Vec<OutputElement<R>>, Color32F)
where
    R: Renderer + ImportAll + ImportMem + RotateDraw,
    R::TextureId: Clone + Send + 'static,
{
    let scale_f = output.current_scale().fractional_scale();
    let scale = Scale::from(scale_f);
    let output_geo = core.space.output_geometry(output).unwrap_or_default();
    let bg = core.config.appearance.wallpaper_color(&core.config.wallpaper);
    let clear = color32(bg, 1.0);
    let mut out: Vec<OutputElement<R>> = Vec::new();

    // Снимок поворота экрана — поверх всего, и курсора тоже.
    if let Some(a) = core.rotate_anim.as_mut().filter(|a| &a.output == output) {
        let mode = output.current_mode().map(|m| m.size).unwrap_or_default();
        let size = output.current_transform().transform_size(mode);
        out.push(OutputElement::Rotate(a.element(size)));
    }
    if include_cursor {
        cursor_elements(core, renderer, output, &mut out);
    }

    // ── блокировка ──
    if core.is_locked() {
        if let Some(ls) = core.lock_surfaces.get(output) {
            let loc = Point::<i32, Physical>::from((0, 0));
            out.extend(
                render_elements_from_surface_tree::<R, WaylandSurfaceRenderElement<R>>(
                    renderer,
                    ls.wl_surface(),
                    loc,
                    scale,
                    1.0,
                    Kind::Unspecified,
                )
                .into_iter()
                .map(OutputElement::Surface),
            );
        }
        let size = output_geo.size.to_f64().to_physical(scale_f).to_i32_round();
        out.push(OutputElement::Solid(SolidColorRenderElement::new(
            core.frame_ids.lock_bg.clone(),
            Rectangle::from_size(size),
            CommitCounter::default(),
            Color32F::new(0.02, 0.02, 0.03, 1.0),
            Kind::Unspecified,
        )));
        return (out, Color32F::new(0.0, 0.0, 0.0, 1.0));
    }

    let fullscreen = fullscreen_top(core, output);
    let overview = core.wm.overview.as_ref().map(|o| (o.anim.value(), o.slots.clone(), o.hovered, o.selected));

    // ── слои overlay ──
    layer_elements(renderer, output, Layer::Overlay, scale, &mut out);

    // ── обзор окон поверх всего остального ──
    if let Some((progress, slots, hovered, selected)) = &overview {
        overview_elements(core, renderer, output, *progress, slots, *hovered, *selected, &mut out);
    }

    // ── подсветка прилипания ──
    if let Some((rect, anim)) = &core.wm.snap_preview {
        let t = anim.value() as f32;
        if let Some(r) = rect.intersection(output_geo) {
            let r = Rectangle::new(r.loc - output_geo.loc, r.size);
            let accent = core.config.appearance.palette().accent;
            let pr = rect_phys(r, scale_f);
            let bw = (2.0 * scale_f).round() as i32;
            let ids = &core.frame_ids.snap;
            for (i, edge) in border_rects(pr, bw).into_iter().enumerate() {
                out.push(OutputElement::Solid(SolidColorRenderElement::new(
                    ids[i].clone(),
                    edge,
                    CommitCounter::default(),
                    color32(accent, 0.9 * t),
                    Kind::Unspecified,
                )));
            }
            out.push(OutputElement::Solid(SolidColorRenderElement::new(
                ids[4].clone(),
                pr,
                CommitCounter::default(),
                color32(accent, 0.22 * t),
                Kind::Unspecified,
            )));
        }
    }

    // ── окно во весь экран — поверх панелей ──
    if overview.is_none() {
        if let Some(fs) = fullscreen {
            window_elements(core, renderer, output, fs, Point::from((0, 0)), 1.0, &mut out);
        }
    }

    // ── слои top ──
    layer_elements(renderer, output, Layer::Top, scale, &mut out);

    // ── окна ──
    // Телефон, режим страниц: переход между страницами или палец тащит.
    let page_tr = core.wm.mobile.transition.as_ref().map(|t| (t.from, t.to, t.dir, t.anim.value()));
    let page_drag = core.wm.mobile.drag.as_ref().map(|d| (d.from, d.dx));
    let paging = overview.is_none() && core.wm.mobile.pages_mode() && (page_tr.is_some() || page_drag.is_some());
    if paging {
        let span = output_geo.size.w as f64;
        let mut draw = |core: &mut Core, page: Option<WindowId>, dx: f64, out: &mut Vec<OutputElement<R>>| {
            for id in core.wm.ids_on_page(page).into_iter().rev() {
                window_elements(core, renderer, output, id, Point::from((dx.round() as i32, 0)), 1.0, out);
            }
        };
        if let Some((from, dx)) = page_drag {
            draw(core, from, dx, &mut out);
            if let Some(nb) = core.wm.page_neighbor_of(from, dx) {
                let off = if dx < 0.0 { dx + span } else { dx - span };
                draw(core, nb, off, &mut out);
            }
        } else if let Some((from, to, dir, t)) = page_tr {
            draw(core, to, dir as f64 * span * (1.0 - t), &mut out);
            draw(core, from, -(dir as f64) * span * t, &mut out);
        }
    }
    if overview.is_none() && !paging {
        let switch = core.wm.switch.as_ref().map(|s| (s.from, s.to, s.dir, s.anim.value()));
        match switch {
            Some((from, to, dir, t)) if core.config.animations.workspace_switch != "fade" => {
                let vertical = core.config.animations.workspace_switch == "slide-vertical";
                let span = if vertical { output_geo.size.h } else { output_geo.size.w } as f64;
                let off_new = (dir as f64 * span * (1.0 - t)).round() as i32;
                let off_old = (-(dir as f64) * span * t).round() as i32;
                let (on, oo) = if vertical {
                    (Point::from((0, off_new)), Point::from((0, off_old)))
                } else {
                    (Point::from((off_new, 0)), Point::from((off_old, 0)))
                };
                // Закреплённые окна не едут.
                let sticky: Vec<WindowId> = core
                    .wm
                    .visible_ids()
                    .into_iter()
                    .filter(|id| core.wm.get(*id).is_some_and(|m| m.sticky))
                    .collect();
                for id in sticky.iter().rev() {
                    if Some(*id) != fullscreen {
                        window_elements(core, renderer, output, *id, Point::from((0, 0)), 1.0, &mut out);
                    }
                }
                for id in core.wm.ids_on_workspace(to).into_iter().rev() {
                    if Some(id) != fullscreen {
                        window_elements(core, renderer, output, id, on, 1.0, &mut out);
                    }
                }
                for id in core.wm.ids_on_workspace(from).into_iter().rev() {
                    window_elements(core, renderer, output, id, oo, 1.0, &mut out);
                }
            }
            Some((from, _to, _dir, t)) => {
                for id in core.wm.visible_ids().into_iter().rev() {
                    if Some(id) != fullscreen {
                        window_elements(core, renderer, output, id, Point::from((0, 0)), t as f32, &mut out);
                    }
                }
                for id in core.wm.ids_on_workspace(from).into_iter().rev() {
                    window_elements(core, renderer, output, id, Point::from((0, 0)), 1.0 - t as f32, &mut out);
                }
            }
            None => {
                for id in core.wm.visible_ids().into_iter().rev() {
                    if Some(id) != fullscreen {
                        window_elements(core, renderer, output, id, Point::from((0, 0)), 1.0, &mut out);
                    }
                }
            }
        }
        // Сворачивающиеся окна (уже не видимы, но анимация идёт).
        let minimizing: Vec<WindowId> = core
            .wm
            .windows
            .iter()
            .filter(|m| m.minimized && m.minimize_anim.is_some() && m.on_workspace(core.wm.active))
            .map(|m| m.id)
            .collect();
        for id in minimizing {
            window_elements(core, renderer, output, id, Point::from((0, 0)), 1.0, &mut out);
        }
    }

    // ── слои bottom и background ──
    layer_elements(renderer, output, Layer::Bottom, scale, &mut out);
    layer_elements(renderer, output, Layer::Background, scale, &mut out);

    (out, clear)
}

fn fullscreen_top(core: &Core, output: &Output) -> Option<WindowId> {
    let name = output.name();
    let top = core
        .wm
        .visible_ids()
        .into_iter()
        .rev()
        .find(|id| core.wm.get(*id).is_some_and(|m| m.output.as_deref() == Some(&name)))?;
    core.wm.get(top).filter(|m| m.fullscreen).map(|m| m.id)
}

fn layer_elements<R>(renderer: &mut R, output: &Output, layer: Layer, scale: Scale<f64>, out: &mut Vec<OutputElement<R>>)
where
    R: Renderer + ImportAll + ImportMem + RotateDraw,
    R::TextureId: Clone + Send + 'static,
{
    let map = layer_map_for_output(output);
    let layers: Vec<(LayerSurface, Rectangle<i32, Logical>)> = map
        .layers_on(layer)
        .rev()
        .filter_map(|l| map.layer_geometry(l).map(|g| (l.clone(), g)))
        .collect();
    drop(map);
    for (l, geo) in layers {
        let loc = geo.loc.to_physical_precise_round(scale);
        out.extend(
            AsRenderElements::<R>::render_elements::<WaylandSurfaceRenderElement<R>>(&l, renderer, loc, scale, 1.0)
                .into_iter()
                .map(OutputElement::Surface),
        );
    }
}

fn border_rects(r: Rectangle<i32, Physical>, bw: i32) -> [Rectangle<i32, Physical>; 4] {
    [
        Rectangle::new(r.loc, (r.size.w, bw).into()),
        Rectangle::new((r.loc.x, r.loc.y + r.size.h - bw).into(), (r.size.w, bw).into()),
        Rectangle::new((r.loc.x, r.loc.y + bw).into(), (bw, (r.size.h - 2 * bw).max(0)).into()),
        Rectangle::new((r.loc.x + r.size.w - bw, r.loc.y + bw).into(), (bw, (r.size.h - 2 * bw).max(0)).into()),
    ]
}

/// Элементы окна (без трансформаций) относительно вывода + смещение.
fn window_parts<R>(core: &mut Core, renderer: &mut R, output: &Output, id: WindowId, offset: Point<i32, Logical>, alpha: f32) -> Vec<WinElement<R>>
where
    R: Renderer + ImportAll + ImportMem + RotateDraw,
    R::TextureId: Clone + Send + 'static,
{
    let scale_f = output.current_scale().fractional_scale();
    let scale = Scale::from(scale_f);
    let int_scale = scale_f.ceil().max(1.0) as i32;
    let output_geo = core.space.output_geometry(output).unwrap_or_default();
    let focused = core.wm.focused == Some(id);
    let active_ws = core.wm.active;
    let layout = core.wm.workspace(active_ws).layout;
    let dim_inactive = core.config.windows.dim_inactive.clamp(0.0, 0.9);
    let border_w = core.config.windows.border_width.max(0);
    let palette = core.config.appearance.palette();
    // Уменьшенный стол показывает и окна за краем экрана.
    let desk_zoomed = core.wm.mobile.view.active() && core.wm.mobile.mode == synshell_common::action::MobileMode::Free;
    let Core { wm, deco_theme, title_font, .. } = core;
    let Some(m) = wm.get_mut(id) else { return Vec::new() };
    let alpha = alpha * m.opacity;
    let geo = m.geometry();
    let top = if m.has_titlebar() { deco_theme.height } else { 0 };
    let frame = Rectangle::new((geo.loc.x, geo.loc.y - top).into(), (geo.size.w, geo.size.h + top).into());
    // Не на этом выводе — пропустить (с запасом на тень).
    let margin = deco_theme.shadow_size;
    let reach = Rectangle::new(
        (frame.loc.x - margin + offset.x, frame.loc.y - margin + offset.y).into(),
        (frame.size.w + 2 * margin, frame.size.h + 2 * margin).into(),
    );
    if !reach.overlaps(output_geo) && !desk_zoomed {
        return Vec::new();
    }
    let rel = |p: Point<i32, Logical>| p + offset - output_geo.loc;
    let mut parts: Vec<WinElement<R>> = Vec::new();

    // Содержимое (со всплывающими меню сверху).
    let origin = rel(m.loc - m.window.geometry().loc);
    parts.extend(
        AsRenderElements::<R>::render_elements::<WaylandSurfaceRenderElement<R>>(
            &m.window,
            renderer,
            to_phys(origin, scale_f),
            scale,
            alpha,
        )
        .into_iter()
        .map(WinElement::Surface),
    );

    // Затемнение неактивного.
    if dim_inactive > 0.0 && !focused {
        let r = rect_phys(Rectangle::new(rel(geo.loc), geo.size), scale_f);
        parts.push(WinElement::Solid(SolidColorRenderElement::new(
            m_ids(m).dim.clone(),
            r,
            CommitCounter::default(),
            Color32F::new(0.0, 0.0, 0.0, dim_inactive * alpha),
            Kind::Unspecified,
        )));
    }

    // Заголовок.
    if top > 0 {
        let key = TitleKey {
            width: frame.size.w,
            scale: int_scale,
            title: m.title(),
            app_id: m.app_id(),
            focused,
            maximized: m.maximized || m.snap.is_some() || m.is_tiled(layout),
            sticky: m.sticky,
            above: m.above,
            hover: m.hover,
            pressed: m.pressed,
            generation: deco_theme.generation,
        };
        let buffer = m.title_bar.buffer(key, deco_theme, title_font).clone();
        if let Ok(e) = MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            rel(frame.loc).to_f64().to_physical(scale_f),
            &buffer,
            Some(alpha),
            None,
            None,
            Kind::Unspecified,
        ) {
            parts.push(WinElement::Memory(e));
        }
    }

    // Обводка: у плиточных окон и окон без заголовка (кроме развёрнутых —
    // у них заголовок убран ради панели, рамка там лишняя).
    let tiled = m.is_tiled(layout);
    if border_w > 0 && !m.fullscreen && !m.maximized && (tiled || top == 0) && m.ssd {
        let color = if focused { palette.accent } else { palette.border };
        let bw_phys = (border_w as f64 * scale_f).round() as i32;
        let outer = rect_phys(
            Rectangle::new(
                (rel(frame.loc).x - border_w, rel(frame.loc).y - border_w).into(),
                (frame.size.w + 2 * border_w, frame.size.h + 2 * border_w).into(),
            ),
            scale_f,
        );
        let ids = m_ids(m);
        let commit = ids.commit_for(color);
        for (i, r) in border_rects(outer, bw_phys).into_iter().enumerate() {
            parts.push(WinElement::Solid(SolidColorRenderElement::new(
                ids.border[i].clone(),
                r,
                commit,
                color32(color, alpha),
                Kind::Unspecified,
            )));
        }
    }

    // Тень (позади всего). Окну со своей рамкой (CSD) — тоже, если оно не
    // рисует тень само: буфер совпадает с геометрией окна (у GTK с тенью
    // геометрия меньше буфера на поля под неё). Радиус тот же, что у рамок:
    // свои окна (synfiles) скругляют углы им же, под прямыми углами чужих
    // скругление тени незаметно.
    let csd_plain = !m.ssd && m.window.bbox() == m.window.geometry();
    let shadow_on = deco_theme.shadow && (m.ssd || csd_plain) && !m.fullscreen && !m.maximized && !tiled && deco_theme.shadow_size > 0;
    if shadow_on {
        let radius = deco_theme.radius.round() as i32;
        m.shadow.update(deco_theme, radius, int_scale);
        if let Some(buf) = m.shadow.buffer.clone() {
            let s = m.shadow.size;
            let c = m.shadow.corner;
            let a = alpha * if focused { 1.0 } else { 0.6 };
            let fx = rel(frame.loc).x - s;
            let fy = rel(frame.loc).y - s + (s / 6);
            let fw = frame.size.w + 2 * s;
            let fh = frame.size.h + 2 * s;
            let mid_w = (fw - 2 * c).max(0);
            let mid_h = (fh - 2 * c).max(0);
            let tex = (2 * c + 1) as f64;
            // 9 частей: (src x, src w, dst x, dst w) по каждой оси.
            let cols = [(0.0, c as f64, fx, c), (c as f64, 1.0, fx + c, mid_w), (c as f64 + 1.0, c as f64, fx + c + mid_w, c)];
            let rows = [(0.0, c as f64, fy, c), (c as f64, 1.0, fy + c, mid_h), (c as f64 + 1.0, c as f64, fy + c + mid_h, c)];
            let _ = tex;
            for (ri, (sy, sh, dy, dh)) in rows.iter().enumerate() {
                for (ci, (sx, sw, dx, dw)) in cols.iter().enumerate() {
                    if ri == 1 && ci == 1 {
                        continue; // середина закрыта окном
                    }
                    if *dw <= 0 || *dh <= 0 {
                        continue;
                    }
                    let src = Rectangle::<f64, Logical>::new((*sx, *sy).into(), (*sw, *sh).into());
                    if let Ok(e) = MemoryRenderBufferRenderElement::from_buffer(
                        renderer,
                        Point::<i32, Logical>::from((*dx, *dy)).to_f64().to_physical(scale_f),
                        &buf,
                        Some(a),
                        Some(src),
                        Some(Size::from((*dw, *dh))),
                        Kind::Unspecified,
                    ) {
                        parts.push(WinElement::Memory(e));
                    }
                }
            }
        }
    }
    parts
}

fn m_ids(m: &Managed) -> &WindowIds {
    m.window.user_data().insert_if_missing(WindowIds::default);
    m.window.user_data().get::<WindowIds>().unwrap()
}

/// Окно с учётом анимаций (появление, сворачивание, перемещение).
fn window_elements<R>(core: &mut Core, renderer: &mut R, output: &Output, id: WindowId, offset: Point<i32, Logical>, alpha: f32, out: &mut Vec<OutputElement<R>>)
where
    R: Renderer + ImportAll + ImportMem + RotateDraw,
    R::TextureId: Clone + Send + 'static,
{
    let scale_f = output.current_scale().fractional_scale();
    let output_geo = core.space.output_geometry(output).unwrap_or_default();
    let title_h = core.deco_theme.height;
    let open_style = core.config.animations.window_open.clone();
    let Some(m) = core.wm.get(id) else { return };
    let geo = m.geometry();
    let top = if m.has_titlebar() { title_h } else { 0 };
    let frame: Rectangle<i32, Logical> = Rectangle::new((geo.loc.x, geo.loc.y - top).into(), (geo.size.w, geo.size.h + top).into());

    // Перемещение при смене раскладки: рисуем со сдвигом от старого места.
    let mut offset = offset;
    if let Some((a, from)) = &m.move_anim {
        let t = a.value();
        let dx = ((from.x - m.loc.x) as f64 * (1.0 - t)).round() as i32;
        let dy = ((from.y - m.loc.y) as f64 * (1.0 - t)).round() as i32;
        offset += Point::from((dx, dy));
    }

    let mut scale = 1.0f64;
    let mut alpha = alpha;
    let mut extra = Point::<i32, Logical>::from((0, 0));
    let mut center = Point::<i32, Logical>::from((frame.loc.x + frame.size.w / 2, frame.loc.y + frame.size.h / 2));
    if let Some(a) = &m.open_anim {
        let t = a.value();
        alpha *= t as f32;
        match open_style.as_str() {
            "zoom" => scale = 0.86 + 0.14 * t,
            "slide" => extra.y = ((1.0 - t) * 40.0) as i32,
            _ => {}
        }
    }
    if let Some((a, target)) = &m.minimize_anim {
        let t = a.value();
        // t=0 — на месте, t=1 — в значке панели.
        let tc = Point::<i32, Logical>::from((target.loc.x + target.size.w / 2, target.loc.y + target.size.h / 2));
        let sw = (target.size.w.max(24) as f64 / frame.size.w.max(1) as f64).min(1.0);
        scale = 1.0 + (sw - 1.0) * t;
        extra.x = ((tc.x - center.x) as f64 * t) as i32;
        extra.y = ((tc.y - center.y) as f64 * t) as i32;
        alpha *= (1.0 - t * 0.9) as f32;
        center = Point::from((center.x, center.y));
    }

    // Свободный стол телефона, уменьшенный щипком: точка `w` (от вывода)
    // видна в `w × zoom + shift`.
    let desk = desk_view(core, id);
    let parts = window_parts(core, renderer, output, id, offset, alpha);
    if scale == 1.0 && extra == Point::from((0, 0)) && desk.is_none() {
        out.extend(parts.into_iter().map(OutputElement::Window));
    } else {
        // Масштаб `scale` вокруг `c` со сдвигом `extra`, затем вид стола:
        // (p − c)·scale·z + c + (c·(z − 1) + extra·z + shift).
        let (z, dshift) = desk.unwrap_or((1.0, Point::from((0.0, 0.0))));
        let c = center + offset - output_geo.loc;
        let origin = to_phys(c, scale_f);
        let sx = c.x as f64 * (z - 1.0) + extra.x as f64 * z + dshift.x;
        let sy = c.y as f64 * (z - 1.0) + extra.y as f64 * z + dshift.y;
        let shift = Point::<f64, Logical>::from((sx, sy)).to_physical(scale_f).to_i32_round();
        out.extend(parts.into_iter().map(|p| {
            OutputElement::Transformed(RelocateRenderElement::from_element(
                RescaleRenderElement::from_element(p, origin, scale * z),
                shift,
                Relocate::Relative,
            ))
        }));
    }
}

/// Вид свободного стола для окна: `(масштаб, сдвиг)`, если стол сейчас не
/// 1:1 и окно лежит на нём (не закреплённое и не во весь экран).
fn desk_view(core: &Core, id: WindowId) -> Option<(f64, Point<f64, Logical>)> {
    let mobile = &core.wm.mobile;
    if !mobile.enabled || mobile.mode != synshell_common::action::MobileMode::Free || !mobile.view.active() {
        return None;
    }
    let m = core.wm.get(id)?;
    (!m.sticky && !m.fullscreen).then(|| mobile.view.current())
}

#[allow(clippy::too_many_arguments)]
fn overview_elements<R>(
    core: &mut Core,
    renderer: &mut R,
    output: &Output,
    progress: f64,
    slots: &[(WindowId, Rectangle<i32, Logical>)],
    hovered: Option<WindowId>,
    selected: usize,
    out: &mut Vec<OutputElement<R>>,
) where
    R: Renderer + ImportAll + ImportMem + RotateDraw,
    R::TextureId: Clone + Send + 'static,
{
    let scale_f = output.current_scale().fractional_scale();
    let output_geo = core.space.output_geometry(output).unwrap_or_default();
    let title_h = core.deco_theme.height;
    let accent = core.config.appearance.palette().accent;
    let selected_id = hovered.or_else(|| slots.get(selected).map(|s| s.0));

    for (id, slot) in slots {
        if !slot.overlaps(output_geo) {
            continue;
        }
        let Some(m) = core.wm.get(*id) else { continue };
        let geo = m.geometry();
        let top = if m.has_titlebar() { title_h } else { 0 };
        let frame = Rectangle::new((geo.loc.x, geo.loc.y - top).into(), (geo.size.w, geo.size.h + top).into());
        let s_full = slot.size.w as f64 / frame.size.w.max(1) as f64;
        let s = 1.0 + (s_full - 1.0) * progress;
        // Левый верхний угол рамки: от места окна к слоту.
        let tx = frame.loc.x as f64 + (slot.loc.x - frame.loc.x) as f64 * progress;
        let ty = frame.loc.y as f64 + (slot.loc.y - frame.loc.y) as f64 * progress;
        let origin = to_phys(frame.loc - output_geo.loc, scale_f);
        let shift = Point::<f64, Logical>::from((tx - frame.loc.x as f64, ty - frame.loc.y as f64))
            .to_physical(scale_f)
            .to_i32_round();

        if Some(*id) == selected_id && progress > 0.5 {
            let r = rect_phys(
                Rectangle::new((slot.loc.x - 4, slot.loc.y - 4).into(), (slot.size.w + 8, slot.size.h + 8).into()),
                scale_f,
            );
            let r = Rectangle::new(r.loc - output_geo.loc.to_physical_precise_round(scale_f), r.size);
            let bw = (3.0 * scale_f).round() as i32;
            for (i, e) in border_rects(r, bw).into_iter().enumerate() {
                out.push(OutputElement::Solid(SolidColorRenderElement::new(
                    core.frame_ids.overview_sel[i].clone(),
                    e,
                    CommitCounter::default(),
                    color32(accent, progress as f32),
                    Kind::Unspecified,
                )));
            }
        }
        let parts = window_parts(core, renderer, output, *id, Point::from((0, 0)), 1.0);
        out.extend(parts.into_iter().map(|p| {
            OutputElement::Transformed(RelocateRenderElement::from_element(
                RescaleRenderElement::from_element(p, origin, s),
                shift,
                Relocate::Relative,
            ))
        }));
    }
    // Затемнение фона.
    let size = output_geo.size.to_f64().to_physical(scale_f).to_i32_round();
    out.push(OutputElement::Solid(SolidColorRenderElement::new(
        core.frame_ids.overview_dim.clone(),
        Rectangle::from_size(size),
        CommitCounter::from((progress * 100.0) as usize),
        Color32F::new(0.0, 0.0, 0.0, 0.55 * progress as f32),
        Kind::Unspecified,
    )));
}

fn cursor_elements<R>(core: &mut Core, renderer: &mut R, output: &Output, out: &mut Vec<OutputElement<R>>)
where
    R: Renderer + ImportAll + ImportMem + RotateDraw,
    R::TextureId: Clone + Send + 'static,
{
    let scale_f = output.current_scale().fractional_scale();
    let scale = Scale::from(scale_f);
    let output_geo = core.space.output_geometry(output).unwrap_or_default();
    let pos = core.pointer.current_location();
    if !output_geo.to_f64().contains(pos) || core.cursor_hidden || core.config.appearance.cursor_hide == "always" {
        return;
    }
    let rel = pos - output_geo.loc.to_f64();

    // DnD-значок.
    if let Some(icon) = &core.dnd_icon {
        if icon.alive() {
            let loc = rel.to_physical(scale_f).to_i32_round();
            out.extend(
                render_elements_from_surface_tree::<R, WaylandSurfaceRenderElement<R>>(renderer, icon, loc, scale, 1.0, Kind::Unspecified)
                    .into_iter()
                    .map(OutputElement::Surface),
            );
        }
    }

    if let CursorImageStatus::Surface(s) = &core.cursor_status {
        if !s.alive() {
            core.cursor_status = CursorImageStatus::default_named();
        }
    }
    match &core.cursor_status {
        CursorImageStatus::Hidden => {}
        CursorImageStatus::Surface(surface) => {
            let hotspot = with_states(surface, |states| {
                states
                    .data_map
                    .get::<CursorImageSurfaceData>()
                    .map(|a| a.lock().unwrap().hotspot)
                    .unwrap_or_default()
            });
            let loc = (rel - hotspot.to_f64()).to_physical(scale_f).to_i32_round();
            out.extend(
                render_elements_from_surface_tree::<R, WaylandSurfaceRenderElement<R>>(renderer, surface, loc, scale, 1.0, Kind::Cursor)
                    .into_iter()
                    .map(OutputElement::Surface),
            );
        }
        CursorImageStatus::Named(icon) => {
            let (buffer, hotspot) = core.cursor.get(*icon, scale_f.ceil() as u32, core.start_time.elapsed());
            let loc = (rel - hotspot.to_f64()).to_physical(scale_f);
            if let Ok(e) = MemoryRenderBufferRenderElement::from_buffer(renderer, loc, &buffer, None, None, None, Kind::Cursor) {
                out.push(OutputElement::Memory(e));
            }
        }
    }
}

use smithay::utils::IsAlive;

/// Миниатюра окна («Недавние» телефона): последний кадр клиента со
/// всплывающими меню, без рамки и заголовка, вписанный в `max` физических
/// px. Окно может быть скрыто (монокль) — буферы клиента остаются у
/// поверхности. `None` — окна нет или у него ещё нет размера.
pub fn thumbnail<R, T>(core: &Core, renderer: &mut R, id: WindowId, max: (i32, i32)) -> anyhow::Result<Option<(u32, u32, Vec<u8>)>>
where
    R: Renderer + ImportAll + ImportMem + smithay::backend::renderer::Offscreen<T> + smithay::backend::renderer::Bind<T> + smithay::backend::renderer::ExportMem,
    R::TextureId: Clone + Send + 'static,
    R::Error: Send + Sync + 'static,
{
    use smithay::backend::{allocator::Fourcc, renderer::damage::OutputDamageTracker};
    let Some(m) = core.wm.get(id) else { return Ok(None) };
    let geo = m.window.geometry();
    if geo.size.w <= 0 || geo.size.h <= 0 || max.0 <= 0 || max.1 <= 0 {
        return Ok(None);
    }
    let s = (max.0 as f64 / geo.size.w as f64).min(max.1 as f64 / geo.size.h as f64).min(4.0);
    let size: Size<i32, Physical> = geo.size.to_f64().to_physical(s).to_i32_round();
    let size = Size::from((size.w.max(1), size.h.max(1)));
    let elements: Vec<WaylandSurfaceRenderElement<R>> =
        AsRenderElements::<R>::render_elements(&m.window, renderer, to_phys(Point::from((0, 0)) - geo.loc, s), Scale::from(s), 1.0);
    let mut buf: T = renderer.create_buffer(Fourcc::Abgr8888, Size::from((size.w, size.h)))?;
    {
        let mut fb = renderer.bind(&mut buf)?;
        let mut tracker = OutputDamageTracker::new(size, 1.0, smithay::utils::Transform::Normal);
        tracker
            .render_output(renderer, &mut fb, 0, &elements, Color32F::new(0.0, 0.0, 0.0, 0.0))
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    }
    let fb = renderer.bind(&mut buf)?;
    let mapping = renderer.copy_framebuffer(&fb, Rectangle::from_size(Size::from((size.w, size.h))), Fourcc::Abgr8888)?;
    let data = renderer.map_texture(&mapping)?.to_vec();
    Ok(Some((size.w as u32, size.h as u32, data)))
}
