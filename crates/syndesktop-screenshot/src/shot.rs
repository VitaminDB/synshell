//! Застывший экран: кадры выводов, окна, указатель — и вырезание области.

use std::sync::Arc;

use syndesktop_common::ipc::{CaptureInfo, Client, Request, Response};

/// Прямоугольник в глобальных логических координатах.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct R {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl R {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }

    pub fn from_i32(r: [i32; 4]) -> Self {
        Self::new(r[0] as f64, r[1] as f64, r[2] as f64, r[3] as f64)
    }

    /// По двум углам в любом порядке.
    pub fn from_points(a: (f64, f64), b: (f64, f64)) -> Self {
        Self::new(a.0.min(b.0), a.1.min(b.1), (a.0 - b.0).abs(), (a.1 - b.1).abs())
    }

    pub fn right(&self) -> f64 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }

    pub fn contains(&self, p: (f64, f64)) -> bool {
        p.0 >= self.x && p.0 < self.right() && p.1 >= self.y && p.1 < self.bottom()
    }

    pub fn intersect(&self, o: &R) -> Option<R> {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = self.right().min(o.right());
        let y1 = self.bottom().min(o.bottom());
        (x1 > x0 && y1 > y0).then(|| R::new(x0, y0, x1 - x0, y1 - y0))
    }

    pub fn union(&self, o: &R) -> R {
        let x0 = self.x.min(o.x);
        let y0 = self.y.min(o.y);
        R::new(x0, y0, self.right().max(o.right()) - x0, self.bottom().max(o.bottom()) - y0)
    }

    pub fn is_empty(&self) -> bool {
        self.w < 1.0 || self.h < 1.0
    }

    /// Сдвинуть внутрь `bounds`, не меняя размера (если помещается).
    pub fn clamp_into(&self, bounds: &R) -> R {
        let w = self.w.min(bounds.w);
        let h = self.h.min(bounds.h);
        let x = self.x.clamp(bounds.x, bounds.right() - w);
        let y = self.y.clamp(bounds.y, bounds.bottom() - h);
        R::new(x, y, w, h)
    }

    pub fn round(&self) -> R {
        let x0 = self.x.round();
        let y0 = self.y.round();
        R::new(x0, y0, self.right().round() - x0, self.bottom().round() - y0)
    }
}

/// Кадр одного вывода.
pub struct Frame {
    pub name: String,
    /// Логическая геометрия вывода.
    pub geo: R,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
}

impl Frame {
    /// Физический пиксель под глобальной логической точкой.
    pub fn pixel_pos(&self, p: (f64, f64)) -> (i64, i64) {
        let sx = self.width as f64 / self.geo.w.max(1.0);
        let sy = self.height as f64 / self.geo.h.max(1.0);
        (((p.0 - self.geo.x) * sx).floor() as i64, ((p.1 - self.geo.y) * sy).floor() as i64)
    }

    pub fn pixel(&self, px: i64, py: i64) -> Option<[u8; 4]> {
        if px < 0 || py < 0 || px >= self.width as i64 || py >= self.height as i64 {
            return None;
        }
        let i = ((py as usize) * self.width as usize + px as usize) * 4;
        self.rgba.get(i..i + 4).map(|s| [s[0], s[1], s[2], s[3]])
    }

    /// Пикселей на логическую единицу (по ширине кадра — у повёрнутых
    /// выводов кадр уже в логической ориентации).
    pub fn density(&self) -> f64 {
        self.width as f64 / self.geo.w.max(1.0)
    }
}

pub struct Window {
    pub title: String,
    pub rect: R,
}

pub struct Shot {
    pub frames: Vec<Frame>,
    /// Окна сверху вниз по стопке.
    pub windows: Vec<Window>,
    pub pointer: (f64, f64),
    /// Охват всех выводов.
    pub bounds: R,
}

impl Shot {
    /// Кадры из описания захвата; файлы кадров удаляются по прочтении.
    pub fn load(info: CaptureInfo) -> anyhow::Result<Shot> {
        let mut frames = Vec::new();
        for o in &info.outputs {
            let data = std::fs::read(&o.path);
            let _ = std::fs::remove_file(&o.path);
            let data = data.map_err(|e| anyhow::anyhow!("{}: {e}", o.path))?;
            let need = o.width as usize * o.height as usize * 4;
            if data.len() < need {
                anyhow::bail!("{}: кадр короче {}×{}", o.path, o.width, o.height);
            }
            frames.push(Frame {
                name: o.name.clone(),
                geo: R::from_i32(o.geometry),
                width: o.width,
                height: o.height,
                rgba: Arc::new(data),
            });
        }
        if frames.is_empty() {
            anyhow::bail!("композитор не вернул ни одного вывода");
        }
        let bounds = frames.iter().skip(1).fold(frames[0].geo, |acc, f| acc.union(&f.geo));
        let windows = info
            .windows
            .into_iter()
            .filter_map(|w| {
                let rect = R::from_i32(w.rect).intersect(&bounds)?;
                Some(Window { title: w.title, rect })
            })
            .collect();
        Ok(Shot { frames, windows, pointer: (info.pointer[0], info.pointer[1]), bounds })
    }

    /// Застыть сейчас: попросить композитор снять все выводы.
    pub fn capture_now() -> anyhow::Result<CaptureInfo> {
        let mut c = Client::connect().map_err(|e| anyhow::anyhow!("нет связи с композитором syndesktop: {e}"))?;
        match c.request(&Request::Capture)? {
            Response::Capture { capture } => Ok(capture),
            Response::Error { message } => anyhow::bail!("{message}"),
            other => anyhow::bail!("неожиданный ответ: {other:?}"),
        }
    }

    /// Описание захвата, записанное композитором по Print.
    pub fn read_capture_file(path: &str) -> anyhow::Result<CaptureInfo> {
        let text = std::fs::read(path);
        let _ = std::fs::remove_file(path);
        Ok(serde_json::from_slice(&text?)?)
    }

    pub fn frame_index_at(&self, p: (f64, f64)) -> Option<usize> {
        self.frames.iter().position(|f| f.geo.contains(p))
    }

    /// Вывод под точкой или ближайший к ней.
    pub fn nearest_frame(&self, p: (f64, f64)) -> usize {
        self.frame_index_at(p).unwrap_or_else(|| {
            let d = |f: &Frame| {
                let cx = p.0.clamp(f.geo.x, f.geo.right());
                let cy = p.1.clamp(f.geo.y, f.geo.bottom());
                (cx - p.0).powi(2) + (cy - p.1).powi(2)
            };
            (0..self.frames.len()).min_by(|a, b| d(&self.frames[*a]).total_cmp(&d(&self.frames[*b]))).unwrap_or(0)
        })
    }

    /// Что выберет щелчок в точке: окно под ней (верхнее), иначе весь вывод.
    pub fn candidate_at(&self, p: (f64, f64)) -> R {
        self.window_at(p).map(|w| w.rect).unwrap_or_else(|| self.frames[self.nearest_frame(p)].geo)
    }

    pub fn window_at(&self, p: (f64, f64)) -> Option<&Window> {
        self.windows.iter().find(|w| w.rect.contains(p))
    }

    /// Плотность пикселей для области: наибольшая среди задетых выводов.
    pub fn density_for(&self, r: &R) -> f64 {
        self.frames
            .iter()
            .filter(|f| f.geo.intersect(r).is_some())
            .map(Frame::density)
            .fold(0.0, f64::max)
            .max(1.0)
    }

    /// Размер области в пикселях итоговой картинки.
    pub fn pixel_size(&self, r: &R) -> (u32, u32) {
        let d = self.density_for(r);
        ((r.w * d).round().max(1.0) as u32, (r.h * d).round().max(1.0) as u32)
    }

    /// Вырезать область: внутри одного вывода — его пиксели как есть, через
    /// несколько — сборка в плотности самого чёткого; места вне выводов
    /// прозрачные.
    pub fn crop(&self, r: &R) -> Option<(u32, u32, Vec<u8>)> {
        let r = r.intersect(&self.bounds)?;
        if let Some(f) = self.frames.iter().find(|f| f.geo.intersect(&r) == Some(r)) {
            let d = f.density();
            let x0 = (((r.x - f.geo.x) * d).round() as i64).clamp(0, f.width as i64);
            let y0 = (((r.y - f.geo.y) * d).round() as i64).clamp(0, f.height as i64);
            let x1 = (((r.right() - f.geo.x) * d).round() as i64).clamp(0, f.width as i64);
            let y1 = (((r.bottom() - f.geo.y) * d).round() as i64).clamp(0, f.height as i64);
            let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
            if w == 0 || h == 0 {
                return None;
            }
            let mut out = Vec::with_capacity(w * h * 4);
            for row in y0 as usize..y1 as usize {
                let start = (row * f.width as usize + x0 as usize) * 4;
                out.extend_from_slice(&f.rgba[start..start + w * 4]);
            }
            return Some((w as u32, h as u32, out));
        }
        let (w, h) = self.pixel_size(&r);
        let d = self.density_for(&r);
        let mut out = vec![0u8; w as usize * h as usize * 4];
        for f in &self.frames {
            let Some(part) = f.geo.intersect(&r) else { continue };
            let dx0 = ((part.x - r.x) * d).round() as u32;
            let dy0 = ((part.y - r.y) * d).round() as u32;
            let dx1 = (((part.right() - r.x) * d).round() as u32).min(w);
            let dy1 = (((part.bottom() - r.y) * d).round() as u32).min(h);
            for dy in dy0..dy1 {
                let gy = r.y + (dy as f64 + 0.5) / d;
                for dx in dx0..dx1 {
                    let gx = r.x + (dx as f64 + 0.5) / d;
                    let (px, py) = f.pixel_pos((gx, gy));
                    if let Some(p) = f.pixel(px, py) {
                        let i = (dy as usize * w as usize + dx as usize) * 4;
                        out[i..i + 4].copy_from_slice(&p);
                    }
                }
            }
        }
        Some((w, h, out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(x: f64, w: u32, h: u32, scale: f64, fill: u8) -> Frame {
        Frame {
            name: format!("O{x}"),
            geo: R::new(x, 0.0, w as f64 / scale, h as f64 / scale),
            width: w,
            height: h,
            rgba: Arc::new((0..w * h).flat_map(|i| [fill, (i % 251) as u8, 0, 255]).collect()),
        }
    }

    fn shot(frames: Vec<Frame>) -> Shot {
        let bounds = frames.iter().skip(1).fold(frames[0].geo, |a, f| a.union(&f.geo));
        Shot { frames, windows: Vec::new(), pointer: (0.0, 0.0), bounds }
    }

    #[test]
    fn crop_inside_one_output_is_exact() {
        let s = shot(vec![frame(0.0, 20, 10, 1.0, 1)]);
        let (w, h, px) = s.crop(&R::new(2.0, 3.0, 5.0, 4.0)).unwrap();
        assert_eq!((w, h), (5, 4));
        let src = &s.frames[0].rgba;
        let i = (3 * 20 + 2) * 4;
        assert_eq!(&px[0..4], &src[i..i + 4]);
    }

    #[test]
    fn crop_hidpi_uses_physical_pixels() {
        let s = shot(vec![frame(0.0, 40, 20, 2.0, 1)]);
        let (w, h, _) = s.crop(&R::new(1.0, 1.0, 5.0, 4.0)).unwrap();
        assert_eq!((w, h), (10, 8));
    }

    #[test]
    fn crop_across_outputs_composes() {
        let s = shot(vec![frame(0.0, 10, 10, 1.0, 1), frame(10.0, 10, 10, 1.0, 2)]);
        let (w, h, px) = s.crop(&R::new(8.0, 0.0, 4.0, 2.0)).unwrap();
        assert_eq!((w, h), (4, 2));
        assert_eq!(px[0], 1);
        assert_eq!(px[3 * 4], 2);
    }

    #[test]
    fn from_points_normalizes() {
        assert_eq!(R::from_points((5.0, 7.0), (1.0, 2.0)), R::new(1.0, 2.0, 4.0, 5.0));
    }
}
