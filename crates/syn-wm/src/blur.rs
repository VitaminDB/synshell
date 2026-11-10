//! Размытие фона под поверхностями, попросившими его (`ext-background-effect-v1`, см.
//! `crate::background_effect`): меню, шторка, всплывающие окна оболочки.
//!
//! Кадр вывода собирается спереди назад, поэтому всё, что лежит под поверхностью, — элементы
//! после неё в списке. Их рисуем во внеэкранную текстуру только в области размытия (с полями под
//! радиус, чтобы край не темнел), размываем двойным Kawase (вниз 1/2 → 1/4 → 1/8, вверх до 1/2) и
//! кладём элементом сразу под поверхность — со скруглением `[appearance] corner_radius`.
//! На CPU-композиторе (pixman) размытия нет.

use std::cell::RefCell;
use std::collections::HashMap;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::{
    element::{Element, Id, Kind, RenderElement, UnderlyingStorage},
    gles::{GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture, Uniform, UniformName, UniformType},
    utils::{CommitCounter, OpaqueRegions},
    Bind, Color32F, Frame, Offscreen, Renderer, Texture,
};
use smithay::utils::{Buffer as BufferCoords, Physical, Rectangle, Scale, Size, Transform};

/// Запрос размытия под поверхностью (координаты вывода, физические).
pub struct BlurReq {
    /// Устойчивый ключ (вывод + поверхность): кэш текстур и id элемента.
    pub key: String,
    /// Область размытия (уже в пределах вывода).
    pub area: Rectangle<i32, Physical>,
    /// Весь вывод — поля области не выходят за него.
    pub output: Rectangle<i32, Physical>,
    /// Радиус скругления, физические px.
    pub radius: f32,
    /// Цвет очистки кадра (фон под всем).
    pub clear: Color32F,
}

/// Размытый фон — текстура половинного разрешения, растянутая на область.
pub struct BlurElement {
    id: Id,
    commit: CommitCounter,
    dst: Rectangle<i32, Physical>,
    texture: GlesTexture,
    src: Rectangle<f64, BufferCoords>,
    radius: f32,
    program: GlesTexProgram,
}

impl Element for BlurElement {
    fn id(&self) -> &Id {
        &self.id
    }
    fn current_commit(&self) -> CommitCounter {
        self.commit
    }
    fn src(&self) -> Rectangle<f64, BufferCoords> {
        self.src
    }
    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.dst
    }
    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }
    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

/// Рендереры, умеющие размыть фон (GLES; у pixman — ничего).
pub trait BlurDraw: Renderer + Sized {
    fn draw_blur(frame: &mut Self::Frame<'_, '_>, e: &BlurElement, dst: Rectangle<i32, Physical>, damage: &[Rectangle<i32, Physical>]) -> Result<(), Self::Error>;
    /// Нарисовать `behind` (элементы под поверхностью, спереди назад) в области `req` и размыть.
    fn make_blur<E: RenderElement<Self>>(&mut self, behind: &[E], req: &BlurReq, scale: Scale<f64>) -> Option<BlurElement>;
}

impl<R: BlurDraw> RenderElement<R> for BlurElement {
    fn draw(
        &self,
        frame: &mut R::Frame<'_, '_>,
        _src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), R::Error> {
        R::draw_blur(frame, self, dst, damage)
    }
    fn underlying_storage(&self, _renderer: &mut R) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

impl BlurDraw for GlesRenderer {
    fn draw_blur(frame: &mut GlesFrame<'_, '_>, e: &BlurElement, dst: Rectangle<i32, Physical>, damage: &[Rectangle<i32, Physical>]) -> Result<(), GlesError> {
        draw_gles(frame, e, dst, damage)
    }
    fn make_blur<E: RenderElement<Self>>(&mut self, behind: &[E], req: &BlurReq, scale: Scale<f64>) -> Option<BlurElement> {
        make(self, |r| r, behind, req, scale)
    }
}

impl<'a> BlurDraw for crate::backend::tty::TtyRenderer<'a> {
    fn draw_blur(frame: &mut Self::Frame<'_, '_>, e: &BlurElement, dst: Rectangle<i32, Physical>, damage: &[Rectangle<i32, Physical>]) -> Result<(), Self::Error> {
        let gles: &mut GlesFrame<'_, '_> = frame.as_mut();
        if let Err(err) = draw_gles(gles, e, dst, damage) {
            tracing::warn!(?err, "размытие фона");
        }
        Ok(())
    }
    fn make_blur<E: RenderElement<Self>>(&mut self, behind: &[E], req: &BlurReq, scale: Scale<f64>) -> Option<BlurElement> {
        make(self, |r| r.as_mut(), behind, req, scale)
    }
}

#[cfg(feature = "pixman")]
impl BlurDraw for smithay::backend::renderer::pixman::PixmanRenderer {
    fn draw_blur(_frame: &mut Self::Frame<'_, '_>, _e: &BlurElement, _dst: Rectangle<i32, Physical>, _damage: &[Rectangle<i32, Physical>]) -> Result<(), Self::Error> {
        Ok(())
    }
    fn make_blur<E: RenderElement<Self>>(&mut self, _behind: &[E], _req: &BlurReq, _scale: Scale<f64>) -> Option<BlurElement> {
        None
    }
}

fn draw_gles(frame: &mut GlesFrame<'_, '_>, e: &BlurElement, dst: Rectangle<i32, Physical>, damage: &[Rectangle<i32, Physical>]) -> Result<(), GlesError> {
    let ts = e.texture.size();
    let uv = (
        (e.src.loc.x / ts.w.max(1) as f64) as f32,
        (e.src.loc.y / ts.h.max(1) as f64) as f32,
        (e.src.size.w / ts.w.max(1) as f64) as f32,
        (e.src.size.h / ts.h.max(1) as f64) as f32,
    );
    frame.render_texture_from_to(
        &e.texture,
        e.src,
        dst,
        damage,
        &[],
        Transform::Normal,
        1.0,
        Some(&e.program),
        &[
            Uniform::new("size", (dst.size.w as f32, dst.size.h as f32)),
            Uniform::new("radius", e.radius),
            Uniform::new("uv_rect", uv),
        ],
    )
}

/// Шапка своего шейдера текстуры smithay (как у `hdr::PQ_SHADER`).
const HEAD: &str = r#"#version 100
//_DEFINES_
#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif
precision mediump float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif
uniform float alpha;
varying vec2 v_coords;
"#;

const DOWN: &str = r#"
uniform vec2 halfpixel;
uniform float offset;
void main() {
    vec2 uv = v_coords;
    vec4 sum = texture2D(tex, uv) * 4.0;
    sum += texture2D(tex, uv - halfpixel * offset);
    sum += texture2D(tex, uv + halfpixel * offset);
    sum += texture2D(tex, uv + vec2(halfpixel.x, -halfpixel.y) * offset);
    sum += texture2D(tex, uv - vec2(halfpixel.x, -halfpixel.y) * offset);
    gl_FragColor = sum / 8.0;
}
"#;

const UP: &str = r#"
uniform vec2 halfpixel;
uniform float offset;
void main() {
    vec2 uv = v_coords;
    vec4 sum = texture2D(tex, uv + vec2(-halfpixel.x * 2.0, 0.0) * offset);
    sum += texture2D(tex, uv + vec2(-halfpixel.x, halfpixel.y) * offset) * 2.0;
    sum += texture2D(tex, uv + vec2(0.0, halfpixel.y * 2.0) * offset);
    sum += texture2D(tex, uv + vec2(halfpixel.x, halfpixel.y) * offset) * 2.0;
    sum += texture2D(tex, uv + vec2(halfpixel.x * 2.0, 0.0) * offset);
    sum += texture2D(tex, uv + vec2(halfpixel.x, -halfpixel.y) * offset) * 2.0;
    sum += texture2D(tex, uv + vec2(0.0, -halfpixel.y * 2.0) * offset);
    sum += texture2D(tex, uv + vec2(-halfpixel.x, -halfpixel.y) * offset) * 2.0;
    gl_FragColor = sum / 12.0;
}
"#;

/// Вывод: размытая текстура, обрезанная по скруглённому прямоугольнику области.
const OUT: &str = r#"
uniform vec2 size;
uniform float radius;
uniform vec4 uv_rect;
void main() {
    vec4 c = texture2D(tex, v_coords);
    vec2 p = (v_coords - uv_rect.xy) / max(uv_rect.zw, vec2(1e-6)) * size;
    vec2 h = size * 0.5;
    float r = min(radius, min(h.x, h.y));
    vec2 q = abs(p - h) - h + r;
    float d = min(max(q.x, q.y), 0.0) + length(max(q, vec2(0.0))) - r;
    float m = clamp(0.5 - d, 0.0, 1.0) * alpha;
    gl_FragColor = vec4(c.rgb, 1.0) * m;
}
"#;

#[derive(Clone)]
struct Programs {
    down: GlesTexProgram,
    up: GlesTexProgram,
    out: GlesTexProgram,
}

fn programs(gles: &mut GlesRenderer) -> Option<Programs> {
    if let Some(p) = gles.egl_context().user_data().get::<Programs>() {
        return Some(p.clone());
    }
    let pass = [UniformName::new("halfpixel", UniformType::_2f), UniformName::new("offset", UniformType::_1f)];
    let compile = |gles: &mut GlesRenderer, body: &str, u: &[UniformName<'_>]| {
        gles.compile_custom_texture_shader(format!("{HEAD}{body}"), u).map_err(|e| tracing::warn!(?e, "размытие фона: шейдер")).ok()
    };
    let p = Programs {
        down: compile(gles, DOWN, &pass)?,
        up: compile(gles, UP, &pass)?,
        out: compile(
            gles,
            OUT,
            &[UniformName::new("size", UniformType::_2f), UniformName::new("radius", UniformType::_1f), UniformName::new("uv_rect", UniformType::_4f)],
        )?,
    };
    gles.egl_context().user_data().insert_if_missing(|| p.clone());
    Some(p)
}

/// Текстуры и id элемента одного запроса (между кадрами).
struct Cache {
    id: Id,
    commit: CommitCounter,
    /// Уровни: 0 — полный размер области с полями, затем 1/2, 1/4, 1/8.
    levels: Vec<GlesTexture>,
}

thread_local! {
    static CACHE: RefCell<HashMap<String, Cache>> = RefCell::new(HashMap::new());
}

/// Уровней уменьшения и сила размытия.
const LEVELS: usize = 3;
const OFFSET: f32 = 2.5;

fn make<R, E>(r: &mut R, gles_of: fn(&mut R) -> &mut GlesRenderer, behind: &[E], req: &BlurReq, scale: Scale<f64>) -> Option<BlurElement>
where
    R: Renderer + Bind<GlesTexture> + Offscreen<GlesTexture>,
    R::Error: std::fmt::Debug,
    E: RenderElement<R>,
{
    let progs = programs(gles_of(r))?;
    // Поля под радиус размытия (на уровне 1/8 шаг выборки — 8 px): край не берёт чёрное.
    let pad = (8.0 * (1 << LEVELS) as f64 * OFFSET as f64 / 2.0).ceil() as i32;
    let big = req.area.intersection(req.output)?;
    let big = Rectangle::new((big.loc.x - pad, big.loc.y - pad).into(), (big.size.w + 2 * pad, big.size.h + 2 * pad).into()).intersection(req.output)?;
    if big.size.w < 2 || big.size.h < 2 {
        return None;
    }
    let sizes: Vec<Size<i32, BufferCoords>> = (0..=LEVELS).map(|l| ((big.size.w >> l).max(1), (big.size.h >> l).max(1)).into()).collect();

    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let entry = cache.entry(req.key.clone()).or_insert_with(|| Cache { id: Id::new(), commit: CommitCounter::default(), levels: Vec::new() });
        if entry.levels.len() != sizes.len() || entry.levels.iter().zip(&sizes).any(|(t, s)| t.size() != *s) {
            let mut levels = Vec::new();
            for s in &sizes {
                match Offscreen::<GlesTexture>::create_buffer(r, Fourcc::Abgr8888, *s) {
                    Ok(t) => levels.push(t),
                    Err(e) => {
                        tracing::warn!(?e, "размытие фона: текстура");
                        return None;
                    }
                }
            }
            entry.levels = levels;
        }

        // 1. То, что под поверхностью, — в уровень 0 (только эта область).
        {
            let mut t0 = entry.levels[0].clone();
            let size0 = Size::<i32, Physical>::from((big.size.w, big.size.h));
            let mut fb = r.bind(&mut t0).map_err(|e| tracing::warn!(?e, "размытие фона: привязка")).ok()?;
            let mut frame = r.render(&mut fb, size0, Transform::Normal).map_err(|e| tracing::warn!(?e, "размытие фона: кадр")).ok()?;
            let full = Rectangle::from_size(size0);
            let _ = frame.clear(req.clear, &[full]);
            for e in behind.iter().rev() {
                let geo = e.geometry(scale);
                let Some(inter) = geo.intersection(big) else { continue };
                let mut dst = geo;
                dst.loc -= big.loc;
                let mut dmg = inter;
                dmg.loc -= geo.loc;
                if let Err(err) = e.draw(&mut frame, e.src(), dst, &[dmg], &[]) {
                    tracing::debug!(?err, "размытие фона: элемент");
                }
            }
            let _ = frame.finish();
        }

        // 2. Kawase: вниз до 1/8, вверх до 1/2.
        let gles = gles_of(r);
        let pass = |gles: &mut GlesRenderer, src: &GlesTexture, dst: &mut GlesTexture, prog: &GlesTexProgram| -> Option<()> {
            let ss = src.size();
            let ds = dst.size();
            let dsize = Size::<i32, Physical>::from((ds.w, ds.h));
            let mut fb = gles.bind(dst).ok()?;
            let mut frame = gles.render(&mut fb, dsize, Transform::Normal).ok()?;
            let full = Rectangle::from_size(dsize);
            frame
                .render_texture_from_to(
                    src,
                    Rectangle::from_size(ss.to_f64()),
                    full,
                    &[full],
                    &[],
                    Transform::Normal,
                    1.0,
                    Some(prog),
                    &[Uniform::new("halfpixel", (0.5 / ss.w.max(1) as f32, 0.5 / ss.h.max(1) as f32)), Uniform::new("offset", OFFSET)],
                )
                .ok()?;
            frame.finish().ok()?;
            Some(())
        };
        for l in 1..=LEVELS {
            let src = entry.levels[l - 1].clone();
            let mut dst = entry.levels[l].clone();
            pass(gles, &src, &mut dst, &progs.down)?;
        }
        for l in (1..LEVELS).rev() {
            let src = entry.levels[l + 1].clone();
            let mut dst = entry.levels[l].clone();
            pass(gles, &src, &mut dst, &progs.up)?;
        }

        // 3. Элемент: уровень 1/2, вырезка области без полей (`SYNWM_BLUR_DEBUG=raw` — уровень 0 без размытия).
        entry.commit.increment();
        let raw = std::env::var("SYNWM_BLUR_DEBUG").is_ok_and(|v| v == "raw");
        let (lvl, div) = if raw { (0, 1.0) } else { (1, 2.0) };
        let half = entry.levels[lvl].clone();
        let src = Rectangle::new(
            (((req.area.loc.x - big.loc.x) as f64) / div, ((req.area.loc.y - big.loc.y) as f64) / div).into(),
            ((req.area.size.w as f64) / div, (req.area.size.h as f64) / div).into(),
        );
        Some(BlurElement { id: entry.id.clone(), commit: entry.commit, dst: req.area, texture: half, src, radius: req.radius, program: progs.out.clone() })
    })
}

/// Забыть кэш запросов вывода `output`, которых в его кадре не было (поверхность закрылась).
pub fn retain(output: &str, keys: &[String]) {
    let prefix = format!("{output}:");
    CACHE.with(|c| c.borrow_mut().retain(|k, _| !k.starts_with(&prefix) || keys.iter().any(|x| x == k)));
}
