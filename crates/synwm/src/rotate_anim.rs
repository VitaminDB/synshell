//! Анимация поворота экрана (как в Android): перед поворотом снимается
//! картинка вывода, после — снимок поверх нового кадра поворачивается от
//! «как было на экране» к новой ориентации, ужимается под новые стороны и
//! растворяется, открывая уже переложенный интерфейс.
//!
//! Поворот на произвольный угол есть только у GLES (своя матрица в
//! `GlesFrame::render_texture`); на CPU (pixman) поворот — сразу, без анимации.

use std::time::{Duration, Instant};

use smithay::backend::renderer::{
    element::{Element, Id, Kind, RenderElement, UnderlyingStorage},
    gles::{GlesError, GlesFrame, GlesRenderer, GlesTexture},
    utils::CommitCounter,
    Renderer, Texture,
};
use smithay::output::Output;
use smithay::utils::{Buffer as BufferCoords, Physical, Rectangle, Scale, Size};

/// Снимок и ход анимации одного вывода.
pub struct RotateAnim {
    pub output: Output,
    texture: GlesTexture,
    /// Размер снимка (пиксели кадра до поворота).
    size: Size<i32, Physical>,
    /// Начальный угол снимка в новых координатах, радианы: при нём снимок
    /// выглядит на панели так же, как до поворота.
    angle0: f32,
    start: Instant,
    duration: Duration,
    id: Id,
    commit: CommitCounter,
}

impl RotateAnim {
    /// `quarters` — на сколько четвертей (по часовой, как `rotate`) повернулся вывод.
    pub fn new(output: Output, texture: GlesTexture, size: Size<i32, Physical>, quarters: u8, duration: Duration) -> Self {
        let q = quarters % 4;
        // Поворот на 270° — это −90°: снимок идёт короткой дорогой.
        let signed = if q == 3 { -1.0 } else { q as f32 };
        Self {
            output,
            texture,
            size,
            angle0: -signed * std::f32::consts::FRAC_PI_2,
            start: Instant::now(),
            duration,
            id: Id::new(),
            commit: CommitCounter::default(),
        }
    }

    pub fn done(&self) -> bool {
        self.start.elapsed() >= self.duration
    }

    /// Элемент кадра (каждый кадр — новый коммит: снимок движется).
    pub fn element(&mut self, out_size: Size<i32, Physical>) -> RotateElement {
        self.commit.increment();
        let t = (self.start.elapsed().as_secs_f32() / self.duration.as_secs_f32().max(0.001)).min(1.0);
        RotateElement {
            id: self.id.clone(),
            commit: self.commit,
            out_size,
            texture: self.texture.clone(),
            snap: self.size,
            angle0: self.angle0,
            t,
        }
    }
}

/// Снимок в полёте — элемент поверх всего кадра.
pub struct RotateElement {
    id: Id,
    commit: CommitCounter,
    out_size: Size<i32, Physical>,
    texture: GlesTexture,
    snap: Size<i32, Physical>,
    angle0: f32,
    t: f32,
}

impl Element for RotateElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, BufferCoords> {
        Rectangle::from_size((self.out_size.w as f64, self.out_size.h as f64).into())
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        Rectangle::from_size(self.out_size)
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

/// Рендереры, умеющие нарисовать снимок под произвольным углом.
pub trait RotateDraw: Renderer {
    fn draw_rotate(frame: &mut Self::Frame<'_, '_>, e: &RotateElement);
}

impl RotateDraw for GlesRenderer {
    fn draw_rotate(frame: &mut GlesFrame<'_, '_>, e: &RotateElement) {
        if let Err(err) = draw_gles(frame, e) {
            tracing::warn!(?err, "анимация поворота");
        }
    }
}

impl<'a> RotateDraw for crate::backend::tty::TtyRenderer<'a> {
    fn draw_rotate(frame: &mut Self::Frame<'_, '_>, e: &RotateElement) {
        let gles: &mut GlesFrame<'_, '_> = frame.as_mut();
        if let Err(err) = draw_gles(gles, e) {
            tracing::warn!(?err, "анимация поворота");
        }
    }
}

#[cfg(feature = "pixman")]
impl RotateDraw for smithay::backend::renderer::pixman::PixmanRenderer {
    fn draw_rotate(_frame: &mut Self::Frame<'_, '_>, _e: &RotateElement) {}
}

impl<R: RotateDraw> RenderElement<R> for RotateElement {
    fn draw(
        &self,
        frame: &mut R::Frame<'_, '_>,
        _src: Rectangle<f64, BufferCoords>,
        _dst: Rectangle<i32, Physical>,
        _damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), R::Error> {
        R::draw_rotate(frame, self);
        Ok(())
    }

    fn underlying_storage(&self, _renderer: &mut R) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

/// Плавный конец, как у Android (decelerate).
fn ease(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

fn draw_gles(frame: &mut GlesFrame<'_, '_>, e: &RotateElement) -> Result<(), GlesError> {
    use cgmath::{Matrix3, Rad, Vector2};
    let p = ease(e.t);
    let (w0, h0) = (e.snap.w as f32, e.snap.h as f32);
    let (w, h) = (e.out_size.w as f32, e.out_size.h as f32);
    let angle = e.angle0 * (1.0 - p);
    // В начале снимок, повёрнутый обратно, ровно закрывает экран; в конце —
    // стоит прямо и вписан в новые стороны.
    let fit = (w / w0).min(h / h0);
    let s = 1.0 + (fit - 1.0) * p;
    let alpha = 1.0 - p;
    let m = Matrix3::from_translation(Vector2::new(w / 2.0, h / 2.0))
        * Matrix3::from_angle_z(Rad(angle))
        * Matrix3::from_nonuniform_scale(s, s)
        * Matrix3::from_translation(Vector2::new(-w0 / 2.0, -h0 / 2.0));
    let ts = e.texture.size();
    let mut tex_m = Matrix3::from_nonuniform_scale(1.0 / ts.w.max(1) as f32, 1.0 / ts.h.max(1) as f32);
    if e.texture.is_y_inverted() {
        tex_m = Matrix3::new(1.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 1.0, 1.0) * tex_m;
    }
    frame.render_texture(&e.texture, tex_m, m, Some([0.0, 0.0, w0, h0]), alpha, None, &[])
}
