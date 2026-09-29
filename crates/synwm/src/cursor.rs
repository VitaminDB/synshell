//! Курсоры из темы XCursor: именованные формы (cursor-shape), анимация,
//! кэш буферов по (имя, размер, кадр).

use std::collections::HashMap;
use std::io::Read;
use std::time::Duration;

use smithay::{
    backend::{allocator::Fourcc, renderer::element::memory::MemoryRenderBuffer},
    input::pointer::CursorIcon,
    utils::{Logical, Point, Transform},
};
use xcursor::{
    parser::{parse_xcursor, Image},
    CursorTheme,
};

pub struct CursorManager {
    theme: CursorTheme,
    pub theme_name: String,
    pub size: u32,
    /// Загруженные кадры по имени курсора (None — нет в теме).
    icons: HashMap<CursorIcon, Option<Vec<Image>>>,
    /// Буферы по (иконка, масштаб, индекс кадра).
    buffers: HashMap<(CursorIcon, u32, usize), (MemoryRenderBuffer, Point<i32, Logical>)>,
}

impl CursorManager {
    pub fn new(theme_name: &str, size: u32) -> Self {
        Self {
            theme: CursorTheme::load(theme_name),
            theme_name: theme_name.to_string(),
            size: size.clamp(8, 256),
            icons: HashMap::new(),
            buffers: HashMap::new(),
        }
    }

    /// Сменить тему/размер (из конфига). Кэш сбрасывается только при изменении.
    pub fn reload(&mut self, theme_name: &str, size: u32) {
        if theme_name == self.theme_name && size == self.size {
            return;
        }
        *self = Self::new(theme_name, size);
    }

    fn load(&mut self, icon: CursorIcon) -> Option<&Vec<Image>> {
        if !self.icons.contains_key(&icon) {
            let mut names = vec![icon.name()];
            names.extend(icon.alt_names().iter().copied());
            let images = names.iter().find_map(|n| load_icon(&self.theme, n));
            self.icons.insert(icon, images);
        }
        self.icons.get(&icon).and_then(|v| v.as_ref())
    }

    /// Буфер кадра курсора и его «горячая точка» (логические px).
    pub fn get(&mut self, icon: CursorIcon, scale: u32, time: Duration) -> (MemoryRenderBuffer, Point<i32, Logical>) {
        let scale = scale.max(1);
        let size = self.size;
        let images = match self.load(icon) {
            Some(i) => i.clone(),
            None => match self.load(CursorIcon::Default) {
                Some(i) => i.clone(),
                None => return fallback(),
            },
        };
        let (idx, frame) = frame(time.as_millis() as u32, size * scale, &images);
        let key = (icon, scale, idx);
        if let Some(b) = self.buffers.get(&key) {
            return b.clone();
        }
        let buffer = MemoryRenderBuffer::from_slice(
            &frame.pixels_rgba,
            Fourcc::Abgr8888,
            (frame.width as i32, frame.height as i32),
            scale as i32,
            Transform::Normal,
            None,
        );
        let hotspot = Point::from(((frame.xhot / scale) as i32, (frame.yhot / scale) as i32));
        self.buffers.insert(key, (buffer.clone(), hotspot));
        (buffer, hotspot)
    }

    /// Первый кадр курсора по умолчанию: (RGBA, ширина, высота, xhot, yhot) — для Xwayland.
    pub fn default_image(&mut self) -> Option<(Vec<u8>, u32, u32, u32, u32)> {
        let size = self.size;
        let images = self.load(CursorIcon::Default)?.clone();
        let (_, f) = frame(0, size, &images);
        Some((f.pixels_rgba.clone(), f.width, f.height, f.xhot, f.yhot))
    }

    /// Есть ли у курсора анимация (тогда кадры нужно перерисовывать).
    pub fn is_animated(&mut self, icon: CursorIcon) -> bool {
        self.load(icon).map(|i| i.iter().any(|f| f.delay > 0) && i.len() > 1).unwrap_or(false)
    }
}

fn fallback() -> (MemoryRenderBuffer, Point<i32, Logical>) {
    // Простая стрелка 16×16, если тема не нашлась совсем.
    let mut px = vec![0u8; 16 * 16 * 4];
    for y in 0..16 {
        for x in 0..=y.min(11) {
            let i = (y * 16 + x) * 4;
            let edge = x == 0 || x == y.min(11) || y == 15;
            let v = if edge { 0 } else { 255 };
            px[i..i + 4].copy_from_slice(&[v, v, v, 255]);
        }
    }
    (
        MemoryRenderBuffer::from_slice(&px, Fourcc::Abgr8888, (16, 16), 1, Transform::Normal, None),
        Point::from((0, 0)),
    )
}

fn load_icon(theme: &CursorTheme, name: &str) -> Option<Vec<Image>> {
    let path = theme.load_icon(name)?;
    let mut data = Vec::new();
    std::fs::File::open(path).ok()?.read_to_end(&mut data).ok()?;
    let images = parse_xcursor(&data)?;
    (!images.is_empty()).then_some(images)
}

fn frame(mut millis: u32, size: u32, images: &[Image]) -> (usize, &Image) {
    let nearest = images
        .iter()
        .min_by_key(|i| (size as i32 - i.size as i32).abs())
        .unwrap();
    let frames: Vec<(usize, &Image)> = images
        .iter()
        .enumerate()
        .filter(|(_, i)| i.width == nearest.width && i.height == nearest.height)
        .collect();
    let total: u32 = frames.iter().map(|(_, i)| i.delay).sum();
    if total == 0 {
        return frames[0];
    }
    millis %= total;
    for (idx, img) in &frames {
        if millis < img.delay {
            return (*idx, img);
        }
        millis -= img.delay;
    }
    frames[0]
}
