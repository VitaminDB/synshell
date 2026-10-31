//! Прямое перо: штрих рисует композитор прямо из событий пера, не дожидаясь кадра приложения.
//!
//! Так делает сток ONYX (SurfaceFlinger + нативный читатель пера): на E Ink путь «событие → кадр
//! приложения → композиция» стоит 40–150 мс, а отрезок, нарисованный композитором, уходит в драйвер
//! панели за одну перерисовку крошечной области.
//!
//! Приложение само объявляет холст (`Request::InkCanvas`): области окна (логические координаты
//! поверхности), толщину линии при нулевом и полном нажатии и цвет. Вне холста и для других
//! программ композитор поверх окна не рисует. Нарисованное живёт в слое поверх всего вывода
//! ([`Ink::element`]) и стирается, когда приложение после отрыва пера прислало свой кадр (в нём штрих
//! уже есть) или через [`CLEAR_FALLBACK`].

use std::time::{Duration, Instant};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement}, Kind},
            ImportMem, Renderer,
        },
    },
    output::Output,
    utils::{Buffer, Logical, Point, Rectangle, Size, Transform},
};

/// Кадр приложения моложе этого после отрыва пера мог ещё не содержать конец штриха.
const SETTLE: Duration = Duration::from_millis(40);
/// Во время штриха окну холста — кадр не чаще этого.
const APP_FRAME_EVERY: Duration = Duration::from_millis(400);
/// Приложение не прислало кадр — стереть всё равно.
pub const CLEAR_FALLBACK: Duration = Duration::from_millis(1500);

/// Холст пера приложения.
#[derive(Clone, Debug, PartialEq)]
pub struct Canvas {
    pub app_id: String,
    /// Области холста в логических координатах поверхности окна.
    pub rects: Vec<Rectangle<f64, Logical>>,
    /// Толщина (логические px) при нажатии 0 и 1.
    pub width: [f64; 2],
    /// RGBA.
    pub color: [u8; 4],
}

impl Canvas {
    pub fn contains(&self, local: Point<f64, Logical>) -> bool {
        self.rects.iter().any(|r| r.contains(local))
    }

    fn width_at(&self, pressure: f64) -> f64 {
        let p = pressure.clamp(0.0, 1.0);
        (self.width[0] + (self.width[1] - self.width[0]) * p).max(0.5)
    }
}

/// Слой прямого пера одного вывода.
struct Layer {
    output: Output,
    buf: MemoryRenderBuffer,
    /// Размер буфера (физические px) и логический размер вывода.
    px: Size<i32, Buffer>,
    logical: Size<i32, Logical>,
    origin: Point<i32, Logical>,
    scale: f64,
    /// Нарисовано (в пикселях буфера) — что стирать.
    drawn: Option<Rectangle<i32, Buffer>>,
}

#[derive(Default)]
pub struct Ink {
    canvases: Vec<Canvas>,
    layer: Option<Layer>,
    /// Прошлая точка штриха (глобальные логические) и радиус (px буфера).
    last: Option<(Point<f64, Logical>, f64)>,
    /// Приложение, по холсту которого идёт штрих.
    pub app_id: Option<String>,
    /// Кончик прижат.
    pub tip: bool,
    up_at: Option<Instant>,
    /// Когда окну холста последний раз отдан frame callback во время штриха.
    app_frame_at: Option<Instant>,
}

impl Ink {
    /// Объявить холст приложения (`rects` пусто — убрать).
    pub fn set_canvas(&mut self, c: Canvas) {
        self.canvases.retain(|x| x.app_id != c.app_id);
        if !c.rects.is_empty() {
            self.canvases.push(c);
        }
    }

    pub fn canvas(&self, app_id: &str) -> Option<&Canvas> {
        self.canvases.iter().find(|c| c.app_id == app_id)
    }

    /// Кончик прижат.
    pub fn tip_down(&mut self) {
        self.tip = true;
        self.last = None;
        self.up_at = None;
        self.app_frame_at = Some(Instant::now());
    }

    /// Кончик отпущен.
    pub fn tip_up(&mut self) {
        self.tip = false;
        self.last = None;
        if self.drawn().is_some() {
            self.up_at = Some(Instant::now());
        }
    }

    /// Придержать frame callback окну `app_id`: пока перо рисует по его холсту, кадры приложения
    /// не нужны (штрих рисует композитор), а каждый стоит полного кадра (wgpu помечает изменённой всю
    /// поверхность — на E Ink это чтение 17 МБ с GPU, 30–100 мс). Раз в [`APP_FRAME_EVERY`] — отдать,
    /// чтобы приложение разобрало накопленные события. Как сток: приложение рисует штрих после отрыва.
    pub fn hold_frames(&mut self, app_id: &str) -> bool {
        if !self.tip || self.app_id.as_deref() != Some(app_id) || self.drawn().is_none() {
            return false;
        }
        let now = Instant::now();
        match self.app_frame_at {
            Some(t) if now.duration_since(t) < APP_FRAME_EVERY => true,
            _ => {
                self.app_frame_at = Some(now);
                false
            }
        }
    }

    /// Перо вышло за холст: следующий отрезок — с новой точки.
    pub fn break_stroke(&mut self) {
        self.last = None;
    }

    fn drawn(&self) -> Option<Rectangle<i32, Buffer>> {
        self.layer.as_ref().and_then(|l| l.drawn)
    }

    /// Точка штриха `pos` (глобальные логические) по холсту `canvas` на выводе `output`
    /// с геометрией `geo` и масштабом `scale`. true — что-то нарисовано.
    pub fn stroke_to(&mut self, output: &Output, geo: Rectangle<i32, Logical>, scale: f64, canvas: &Canvas, pos: Point<f64, Logical>, pressure: f64) -> bool {
        let px = Size::<i32, Buffer>::from(((geo.size.w as f64 * scale).round() as i32, (geo.size.h as f64 * scale).round() as i32));
        if self.layer.as_ref().is_none_or(|l| &l.output != output || l.px != px || l.origin != geo.loc) {
            self.layer = Some(Layer {
                output: output.clone(),
                buf: MemoryRenderBuffer::new(Fourcc::Argb8888, px, 1, Transform::Normal, None),
                px,
                logical: geo.size,
                origin: geo.loc,
                scale,
                drawn: None,
            });
            self.last = None;
        }
        let layer = self.layer.as_mut().unwrap();
        let r = canvas.width_at(pressure) * scale / 2.0;
        let to_px = |p: Point<f64, Logical>| ((p.x - layer.origin.x as f64) * scale, (p.y - layer.origin.y as f64) * scale);
        let b = to_px(pos);
        let (a, ra) = match self.last {
            Some((lp, lr)) => (to_px(lp), lr),
            None => (b, r),
        };
        self.last = Some((pos, r));
        let [cr, cg, cb, ca] = canvas.color;
        let pixel = u32::from_be_bytes([ca, cr, cg, cb]).to_le_bytes(); // Argb8888: в памяти B, G, R, A
        let (w, h) = (layer.px.w, layer.px.h);
        let rmax = ra.max(r);
        let x0 = ((a.0.min(b.0) - rmax).floor() as i32).clamp(0, w);
        let y0 = ((a.1.min(b.1) - rmax).floor() as i32).clamp(0, h);
        let x1 = ((a.0.max(b.0) + rmax).ceil() as i32 + 1).clamp(0, w);
        let y1 = ((a.1.max(b.1) + rmax).ceil() as i32 + 1).clamp(0, h);
        if x0 >= x1 || y0 >= y1 {
            return false;
        }
        let damage = Rectangle::<i32, Buffer>::new((x0, y0).into(), (x1 - x0, y1 - y0).into());
        let mut ctx = layer.buf.render();
        let _ = ctx.draw(|mem| {
            // Отрезок с круглыми концами: пиксель закрашен, если ближе к отрезку, чем радиус в этой точке.
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len2 = dx * dx + dy * dy;
            for y in y0..y1 {
                for x in x0..x1 {
                    let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                    let t = if len2 > 0.0 { (((px - a.0) * dx + (py - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
                    let (qx, qy) = (a.0 + dx * t - px, a.1 + dy * t - py);
                    let rad = ra + (r - ra) * t;
                    if qx * qx + qy * qy <= rad * rad {
                        let i = (y as usize * w as usize + x as usize) * 4;
                        mem[i..i + 4].copy_from_slice(&pixel);
                    }
                }
            }
            Ok::<_, ()>(vec![damage])
        });
        drop(ctx);
        layer.drawn = Some(match layer.drawn {
            Some(d) => d.merge(damage),
            None => damage,
        });
        true
    }

    /// Кадр приложения `app_id`: если штрих закончен и кадр новее отрыва пера — стереть слой.
    /// true — стёрто (нужна перерисовка).
    pub fn on_app_commit(&mut self, app_id: &str) -> bool {
        if self.tip || self.app_id.as_deref() != Some(app_id) {
            return false;
        }
        match self.up_at {
            Some(t) if t.elapsed() >= SETTLE => self.clear(),
            _ => false,
        }
    }

    /// Стереть нарисованное, если штрих закончен давно (страховка без кадра приложения).
    pub fn clear_stale(&mut self) -> bool {
        if self.tip {
            return false;
        }
        match self.up_at {
            Some(t) if t.elapsed() >= CLEAR_FALLBACK => self.clear(),
            _ => false,
        }
    }

    /// Стереть слой. true — было что стирать.
    pub fn clear(&mut self) -> bool {
        self.up_at = None;
        self.last = None;
        let Some(layer) = self.layer.as_mut() else { return false };
        let Some(d) = layer.drawn.take() else { return false };
        let w = layer.px.w as usize;
        let mut ctx = layer.buf.render();
        let _ = ctx.draw(|mem| {
            for y in d.loc.y..d.loc.y + d.size.h {
                let i = (y as usize * w + d.loc.x as usize) * 4;
                mem[i..i + d.size.w as usize * 4].fill(0);
            }
            Ok::<_, ()>(vec![d])
        });
        true
    }

    /// Элемент слоя для вывода (если слой на нём есть).
    pub fn element<R>(&self, renderer: &mut R, output: &Output) -> Option<MemoryRenderBufferRenderElement<R>>
    where
        R: Renderer + ImportMem,
        R::TextureId: Clone + Send + 'static,
    {
        let layer = self.layer.as_ref().filter(|l| &l.output == output)?;
        let src = Rectangle::<f64, Logical>::from_size((layer.px.w as f64, layer.px.h as f64).into());
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            Point::from((0.0, 0.0)),
            &layer.buf,
            None,
            Some(src),
            Some(layer.logical),
            Kind::Unspecified,
        )
        .ok()
        .filter(|_| layer.scale > 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas() -> Canvas {
        Canvas {
            app_id: "a".into(),
            rects: vec![Rectangle::new((10.0, 10.0).into(), (100.0, 100.0).into())],
            width: [1.0, 3.0],
            color: [0, 0, 0, 255],
        }
    }

    #[test]
    fn canvas_contains_and_width() {
        let c = canvas();
        assert!(c.contains((50.0, 50.0).into()));
        assert!(!c.contains((5.0, 50.0).into()));
        assert_eq!(c.width_at(0.0), 1.0);
        assert_eq!(c.width_at(1.0), 3.0);
        assert_eq!(c.width_at(0.5), 2.0);
    }

    #[test]
    fn set_canvas_replaces_and_removes() {
        let mut ink = Ink::default();
        ink.set_canvas(canvas());
        assert!(ink.canvas("a").is_some());
        let mut c = canvas();
        c.width = [2.0, 2.0];
        ink.set_canvas(c.clone());
        assert_eq!(ink.canvas("a").unwrap().width, [2.0, 2.0]);
        c.rects.clear();
        ink.set_canvas(c);
        assert!(ink.canvas("a").is_none());
    }
}
