//! Исполнитель графа Fusion (`Comp`): мемоизированный обход DAG от выходного
//! узла с поддержкой `MediaIn`, `KeyStretcher`, `Transform`, `Merge`, `Background`,
//! `Gradient`, масок, `TextPlus` (+ `StyledTextFollower`), `Dissolve`, `Blur`,
//! `DirectionalBlur`, `Glow`, `Displace`, `Fuse.Duplicate`, `BrightnessContrast`,
//! `ColorCurves`, `DropShadow`, `FastNoise`, `CornerPositioner`.

use std::collections::HashMap;
use std::sync::Arc;

use syngui::video::RgbaFrame;

use super::anim::{self, eval_num, eval_point, eval_str};
use super::img::{self, Color4, GradStop, Image, MaskCommon};
use super::{Comp, Lua};

/// Переопределения шаблона (пользовательские строки текста, шрифт, цвет, значения входов).
#[derive(Clone, Debug, Default)]
pub struct Overrides {
    /// Замены строк по порядку TextPlus (из `Source::Title.text` разделённого `\n`).
    pub texts: Vec<String>,
    /// Явное переопределение шрифта (если пользователь выбрал шрифт в инспекторе).
    pub font: Option<String>,
    /// Явные переопределения параметров `(ToolName, InputName) -> f64`.
    pub nums: HashMap<(String, String), f64>,
    /// Явные переопределения строк `(ToolName, InputName) -> String`.
    pub strs: HashMap<(String, String), String>,
    /// Явные переопределения 2D точек `(ToolName, InputName) -> (x, y)`.
    pub points: HashMap<(String, String), (f64, f64)>,
    /// Явные переопределения цветов точек градиента `(BackgroundName, stop_idx) -> Color4`.
    pub grad_colors: HashMap<(String, usize), Color4>,
}

/// Контекст одного кадра исполнения.
pub struct RenderCtx<'a> {
    pub comp: &'a Comp,
    pub width: u32,
    pub height: u32,
    pub total_frames: f64,
    pub media_in: &'a [Arc<Image>],
    pub overrides: &'a Overrides,
    text_order: Vec<String>,
    /// Кэш узлов `(tool_name, frame_key)` внутри одного кадра (DAG-мемоизация).
    cache: HashMap<(String, i64), Arc<Image>>,
}

/// Отрендерить один кадр шаблона Fusion в `RgbaFrame` (straight RGBA8).
pub fn render(
    comp: &Comp,
    width: u32,
    height: u32,
    local_time: f64,
    duration: f64,
    fps: f64,
    media_in: &[Arc<Image>],
    overrides: &Overrides,
) -> RgbaFrame {
    let w = width.max(2);
    let h = height.max(2);
    let fps = fps.max(1.0);
    let total_frames = (duration * fps).max(1.0);
    let frame = (local_time * fps).clamp(0.0, total_frames);
    let mut ctx = RenderCtx {
        comp,
        width: w,
        height: h,
        total_frames,
        media_in,
        overrides,
        text_order: comp.text_tool_names(),
        cache: HashMap::new(),
    };
    let img = match &comp.output {
        Some(out) => ctx.eval_node(out, frame, 0),
        None => Arc::new(Image::empty(w, h)),
    };
    RgbaFrame {
        width: w,
        height: h,
        pts: local_time,
        rgba: Arc::new(img.to_rgba8()),
    }
}

impl<'a> RenderCtx<'a> {
    fn num(&self, tool: &str, inp: &str, default: f64, frame: f64) -> f64 {
        if let Some(&v) = self.overrides.nums.get(&(tool.to_string(), inp.to_string())) {
            return v;
        }
        eval_num(self.comp, tool, inp, default, frame, self.total_frames, 0)
    }

    fn pt(&self, tool: &str, inp: &str, default: (f64, f64), frame: f64) -> (f32, f32) {
        if let Some(&(x, y)) = self.overrides.points.get(&(tool.to_string(), inp.to_string())) {
            return (x as f32, y as f32);
        }
        let (x, y) = eval_point(self.comp, tool, inp, default, frame, self.total_frames, 0);
        (x as f32, y as f32)
    }

    fn str_val(&self, tool: &str, inp: &str, default: &'a str) -> &'a str {
        if let Some(s) = self.overrides.strs.get(&(tool.to_string(), inp.to_string())) {
            return s.as_str();
        }
        eval_str(self.comp, tool, inp, default)
    }

    fn connected_image(&mut self, tool: &str, inp: &str, frame: f64, depth: usize) -> Option<Arc<Image>> {
        if let Some(src_op) = self.comp.resolve_input(tool, inp).and_then(|i| i.get("SourceOp")).and_then(Lua::str) {
            let sop = src_op.to_string();
            return Some(self.eval_node(&sop, frame, depth + 1));
        }
        // Входы MacroOperator (`MainInput1` → clip A, `MainInput2` → clip B)
        if let Some(idx) = self.comp.main_inputs.iter().position(|(op, src)| op == tool && src == inp) {
            return self.media_in.get(idx).cloned();
        }
        None
    }

    fn effect_mask(&mut self, tool: &str, frame: f64, depth: usize) -> Option<Arc<Image>> {
        self.connected_image(tool, "EffectMask", frame, depth)
    }

    fn channels(&self, tool: &str, frame: f64) -> [bool; 4] {
        [
            self.num(tool, "ProcessRed", 1.0, frame) > 0.5,
            self.num(tool, "ProcessGreen", 1.0, frame) > 0.5,
            self.num(tool, "ProcessBlue", 1.0, frame) > 0.5,
            self.num(tool, "ProcessAlpha", 1.0, frame) > 0.5,
        ]
    }

    fn mask_common(&self, tool: &str, frame: f64) -> MaskCommon {
        let pm = match self.str_val(tool, "PaintMode", "Merge") {
            "Subtract" => "Subtract",
            "Multiply" => "Multiply",
            "Minimum" => "Minimum",
            "Add" => "Add",
            "Invert" => "Invert",
            "Replace" => "Replace",
            _ => "Merge",
        };
        MaskCommon {
            level: self.num(tool, "Level", 1.0, frame) as f32,
            soft_edge: self.num(tool, "SoftEdge", 0.0, frame) as f32,
            border_width: self.num(tool, "BorderWidth", 0.0, frame) as f32,
            solid: self.num(tool, "Solid", 1.0, frame) > 0.5,
            invert: self.num(tool, "Invert", 0.0, frame) > 0.5,
            paint_mode: pm,
        }
    }

    fn eval_node(&mut self, name: &str, frame: f64, depth: usize) -> Arc<Image> {
        if depth > 48 {
            return Arc::new(Image::empty(self.width, self.height));
        }
        let fkey = (frame * 100.0).round() as i64;
        if let Some(hit) = self.cache.get(&(name.to_string(), fkey)) {
            return hit.clone();
        }
        let Some(ty) = self.comp.tools.get(name).map(|t| t.ty.clone()) else {
            return Arc::new(Image::empty(self.width, self.height));
        };

        let result = match ty.as_str() {
            "MediaOut" | "PipeRouter" | "BetterResize" | "Resize" | "Crop" | "SetDomain" | "AlphaMultiply" => self
                .connected_image(name, "Input", frame, depth)
                .unwrap_or_else(|| Arc::new(Image::empty(self.width, self.height))),

            "MediaIn" => {
                let idx = self.num(name, "MediaID", 0.0, frame).round() as usize;
                let fallback = if name.ends_with('2') { 1 } else { 0 };
                self.media_in
                    .get(idx)
                    .or_else(|| self.media_in.get(fallback))
                    .or_else(|| self.media_in.first())
                    .cloned()
                    .unwrap_or_else(|| Arc::new(Image::empty(self.width, self.height)))
            }

            "KeyStretcher" => {
                let s0 = self.num(name, "SourceStart", 0.0, frame);
                let s1 = self.num(name, "SourceEnd", self.comp.source_frames, frame);
                let k0 = self.num(name, "StretchStart", s0, frame);
                let k1 = self.num(name, "StretchEnd", s1, frame);
                let mapped = anim::key_stretch_frame(frame, self.total_frames, s0, s1, k0, k1);
                self.connected_image(name, "Keyframes", mapped, depth)
                    .or_else(|| self.connected_image(name, "Input", mapped, depth))
                    .unwrap_or_else(|| Arc::new(Image::empty(self.width, self.height)))
            }

            "TimeStretcher" => {
                let st = self.num(name, "SourceTime", frame, frame);
                self.connected_image(name, "Input", st, depth)
                    .unwrap_or_else(|| Arc::new(Image::empty(self.width, self.height)))
            }

            "TimeSpeed" => {
                let speed = self.num(name, "Speed", 1.0, frame);
                let delay = self.num(name, "Delay", 0.0, frame);
                let mapped = ((frame - delay) * speed).max(0.0);
                self.connected_image(name, "Input", mapped, depth)
                    .unwrap_or_else(|| Arc::new(Image::empty(self.width, self.height)))
            }

            "Background" => self.eval_background(name, frame, depth),
            "RectangleMask" => self.eval_rect_or_ellipse(name, false, frame, depth),
            "EllipseMask" => self.eval_rect_or_ellipse(name, true, frame, depth),
            "TriangleMask" => self.eval_triangle(name, frame, depth),
            "PolylineMask" | "BSplineMask" => self.eval_polyline_mask(name, frame, depth),
            "BitmapMask" => self.eval_bitmap_mask(name, frame, depth),
            "Transform" => self.eval_transform(name, frame, depth),
            "Merge" => self.eval_merge(name, frame, depth),
            "Dissolve" => self.eval_dissolve(name, frame, depth),
            "TextPlus" => self.eval_text_plus(name, frame, depth),
            "Blur" | "ofx.com.blackmagicdesign.resolvefx.GaussianBlur" => self.eval_blur(name, frame, depth),
            "DirectionalBlur"
            | "ofx.com.blackmagicdesign.resolvefx.DirectionalBlur"
            | "ofx.com.blackmagicdesign.resolvefx.RadialBlur"
            | "ofx.com.blackmagicdesign.resolvefx.ZoomBlur" => self.eval_dir_blur(name, &ty, frame, depth),
            "Glow" | "SoftGlow" => self.eval_glow(name, frame, depth),
            "Sharpen" => {
                let Some(src) = self.connected_image(name, "Input", frame, depth) else {
                    return Arc::new(Image::empty(self.width, self.height));
                };
                let amt = self.num(name, "XAmount", 1.0, frame) as f32;
                let blend = self.num(name, "Blend", 1.0, frame) as f32;
                let mut out = img::sharpen_image(&src, amt);
                let mask = self.effect_mask(name, frame, depth);
                img::blend_with_mask(&src, &mut out, mask.as_deref(), blend);
                Arc::new(out)
            }
            "LensDistort" => {
                let Some(src) = self.connected_image(name, "Input", frame, depth) else {
                    return Arc::new(Image::empty(self.width, self.height));
                };
                let k = self.num(name, "DEClassicLDModel.Distortion", self.num(name, "DEClassicLDModel.CurvatureX", self.num(name, "Distortion", 0.0, frame), frame), frame) as f32;
                let sq = self.num(name, "DEClassicLDModel.AnamorphicSqueeze", 1.0, frame) as f32;
                let blend = self.num(name, "Blend", 1.0, frame) as f32;
                let mut out = img::lens_distort(&src, k, sq);
                let mask = self.effect_mask(name, frame, depth);
                img::blend_with_mask(&src, &mut out, mask.as_deref(), blend);
                Arc::new(out)
            }
            "ofx.com.blackmagicdesign.resolvefx.MosaicBlur" | "ofx.com.blackmagicdesign.resolvefx.mosaicblur" => {
                let Some(src) = self.connected_image(name, "Source", frame, depth).or_else(|| self.connected_image(name, "Input", frame, depth)) else {
                    return Arc::new(Image::empty(self.width, self.height));
                };
                let freq = self.num(name, "PixelFrequency", 24.0, frame) as f32;
                let blend = self.num(name, "Blend", 1.0, frame) as f32;
                let mut out = img::mosaic_blur(&src, freq);
                let mask = self.effect_mask(name, frame, depth);
                img::blend_with_mask(&src, &mut out, mask.as_deref(), blend);
                Arc::new(out)
            }
            "ofx.com.blackmagicdesign.resolvefx.Waviness" | "ofx.com.blackmagicdesign.resolvefx.waviness" => {
                let Some(src) = self.connected_image(name, "Source", frame, depth).or_else(|| self.connected_image(name, "Input", frame, depth)) else {
                    return Arc::new(Image::empty(self.width, self.height));
                };
                let scale = self.num(name, "scale", 4.0, frame) as f32;
                let strength = self.num(name, "strength", 0.5, frame) as f32;
                let phase = self.num(name, "phase", 0.0, frame) as f32;
                let vert = self.num(name, "waveType", 0.0, frame).round() as i32 == 1;
                let blend = self.num(name, "Blend", 1.0, frame) as f32;
                let mut out = img::waviness(&src, scale, strength, phase, vert);
                let mask = self.effect_mask(name, frame, depth);
                img::blend_with_mask(&src, &mut out, mask.as_deref(), blend);
                Arc::new(out)
            }
            "LumaKeyer" => {
                let Some(src) = self.connected_image(name, "Input", frame, depth) else {
                    return Arc::new(Image::empty(self.width, self.height));
                };
                let low = self.num(name, "Low", 0.0, frame) as f32;
                let high = self.num(name, "High", 1.0, frame) as f32;
                let inv = self.num(name, "Invert", 0.0, frame) > 0.5;
                Arc::new(img::luma_keyer(&src, low, high, inv))
            }
            "Displace" => self.eval_displace(name, frame, depth),
            "Fuse.Duplicate" => self.eval_duplicate(name, frame, depth),
            "BrightnessContrast" | "ColorCorrector" => self.eval_bc(name, frame, depth),
            "ColorCurves" => self.eval_color_curves(name, frame, depth),
            "Shadow" | "ofx.com.blackmagicdesign.resolvefx.dropshadow" | "ofx.com.blackmagicdesign.resolvefx.DropShadow" => self.eval_drop_shadow(name, frame, depth),
            "FastNoise" => self.eval_fast_noise(name, frame, depth),
            "CornerPositioner" => self.eval_corner_pos(name, frame, depth),
            // Неизвестный фильтр — мягкий проход входа Input / Background / Source
            _ => self
                .connected_image(name, "Input", frame, depth)
                .or_else(|| self.connected_image(name, "Background", frame, depth))
                .or_else(|| self.connected_image(name, "Source", frame, depth))
                .unwrap_or_else(|| Arc::new(Image::empty(self.width, self.height))),
        };

        self.cache.insert((name.to_string(), fkey), result.clone());
        result
    }

    fn eval_background(&mut self, name: &str, frame: f64, depth: usize) -> Arc<Image> {
        let bg_type = self.str_val(name, "Type", "Solid");
        let corner = |p: &str, dr: f64, dg: f64, db: f64, da: f64| {
            Color4::from_straight(
                self.num(name, &format!("{p}Red"), dr, frame) as f32,
                self.num(name, &format!("{p}Green"), dg, frame) as f32,
                self.num(name, &format!("{p}Blue"), db, frame) as f32,
                self.num(name, &format!("{p}Alpha"), da, frame) as f32,
            )
        };
        let tl = corner("TopLeft", 0.0, 0.0, 0.0, 1.0);
        let tr = corner("TopRight", 0.0, 0.0, 0.0, 1.0);
        let bl = corner("BottomLeft", 0.0, 0.0, 0.0, 1.0);
        let br = corner("BottomRight", 0.0, 0.0, 0.0, 1.0);

        let grad_type = self.str_val(name, "GradientType", "Linear");
        let start = self.pt(name, "Start", (0.0, 0.5), frame);
        let end = self.pt(name, "End", (1.0, 0.5), frame);
        let mut stops = Vec::new();
        if let Some(colors) = self.comp.resolve_input(name, "Gradient").and_then(|i| i.get("Value")).and_then(|g| g.get("Colors")) {
            for (k, v) in colors.items() {
                let Some(pos) = k.as_deref().and_then(|s| s.parse::<f32>().ok()) else { continue };
                let ns: Vec<f32> = v.items().iter().filter_map(|(_, x)| x.num().map(|n| n as f32)).collect();
                let r = ns.first().copied().unwrap_or(0.0);
                let g = ns.get(1).copied().unwrap_or(0.0);
                let b = ns.get(2).copied().unwrap_or(0.0);
                let a = ns.get(3).copied().unwrap_or(1.0);
                stops.push(GradStop { pos, color: Color4::from_straight(r, g, b, a) });
            }
            stops.sort_by(|a, b| a.pos.total_cmp(&b.pos));
            for (idx, st) in stops.iter_mut().enumerate() {
                if let Some(&col) = self.overrides.grad_colors.get(&(name.to_string(), idx)) {
                    st.color = col;
                }
            }
        }

        let mut img = img::render_background(self.width, self.height, bg_type, tl, tr, bl, br, grad_type, start, end, &stops);
        if let Some(mask) = self.effect_mask(name, frame, depth) {
            img::apply_mask_mul(&mut img, &mask);
        }
        Arc::new(img)
    }

    fn eval_rect_or_ellipse(&mut self, name: &str, ellipse: bool, frame: f64, depth: usize) -> Arc<Image> {
        let center = self.pt(name, "Center", (0.5, 0.5), frame);
        let mw = self.num(name, "Width", 0.5, frame) as f32;
        let mh = self.num(name, "Height", 0.5, frame) as f32;
        let corner = self.num(name, "CornerRadius", 0.0, frame) as f32;
        let angle = self.num(name, "Angle", 0.0, frame) as f32;
        let common = self.mask_common(name, frame);
        let parent = self.effect_mask(name, frame, depth);
        Arc::new(img::render_shape_mask(self.width, self.height, ellipse, center, mw, mh, corner, angle, common, parent.as_deref()))
    }

    fn eval_triangle(&mut self, name: &str, frame: f64, depth: usize) -> Arc<Image> {
        let p1 = self.pt(name, "Point1", (0.5, 0.75), frame);
        let p2 = self.pt(name, "Point2", (0.25, 0.25), frame);
        let p3 = self.pt(name, "Point3", (0.75, 0.25), frame);
        let common = self.mask_common(name, frame);
        let parent = self.effect_mask(name, frame, depth);
        Arc::new(img::render_triangle_mask(self.width, self.height, p1, p2, p3, common, parent.as_deref()))
    }

    fn eval_polyline_mask(&mut self, name: &str, frame: f64, depth: usize) -> Arc<Image> {
        let center = self.pt(name, "Center", (0.5, 0.5), frame);
        let size = self.num(name, "Size", 1.0, frame) as f32;
        let angle = self.num(name, "Angle", 0.0, frame) as f32;
        let write_len = self.num(name, "WriteLength", 1.0, frame) as f32;
        let write_pos = self.num(name, "WritePosition", 0.0, frame) as f32;
        let common = self.mask_common(name, frame);
        let parent = self.effect_mask(name, frame, depth);

        let poly_val = self.comp.resolve_input(name, "Polyline").and_then(|i| {
            if let Some(sop) = i.get("SourceOp").and_then(Lua::str) {
                // Анимированная кривая BezierSpline с KeyFrames -> берём первый ключевой Polyline
                self.comp
                    .tools
                    .get(sop)
                    .and_then(|t| t.raw.get("KeyFrames"))
                    .and_then(|kfs| kfs.items().first())
                    .and_then(|(_, kv)| kv.get("Value"))
            } else {
                i.get("Value")
            }
        });
        let (pts, closed) = match poly_val {
            Some(pv) => {
                let (raw_pts, cl) = anim::parse_polyline(pv);
                (anim::sample_polyline(&raw_pts, cl, 10), cl)
            }
            None => (Vec::new(), true),
        };
        Arc::new(img::render_poly_mask(self.width, self.height, &pts, closed, center, size, angle, write_len, write_pos, common, parent.as_deref()))
    }

    fn eval_bitmap_mask(&mut self, name: &str, frame: f64, depth: usize) -> Arc<Image> {
        let src = self
            .connected_image(name, "Image", frame, depth)
            .or_else(|| self.connected_image(name, "Input", frame, depth))
            .unwrap_or_else(|| Arc::new(Image::empty(self.width, self.height)));
        let ch = self.str_val(name, "Channel", "Alpha");
        let inv = self.num(name, "Invert", 0.0, frame) > 0.5;
        let level = self.num(name, "Level", 1.0, frame) as f32;
        let mut out = Image::empty(self.width, self.height);
        for (dst, s) in out.data.chunks_exact_mut(4).zip(src.data.chunks_exact(4)) {
            let mut m = match ch {
                "Luminance" => 0.299 * s[0] + 0.587 * s[1] + 0.114 * s[2],
                "Red" => s[0],
                "Green" => s[1],
                "Blue" => s[2],
                _ => s[3],
            }
            .clamp(0.0, 1.0);
            if inv {
                m = 1.0 - m;
            }
            m *= level;
            dst[0] = m;
            dst[1] = m;
            dst[2] = m;
            dst[3] = m;
        }
        Arc::new(out)
    }

    fn eval_transform(&mut self, name: &str, frame: f64, depth: usize) -> Arc<Image> {
        let Some(src) = self.connected_image(name, "Input", frame, depth) else {
            return Arc::new(Image::empty(self.width, self.height));
        };
        let center = self.pt(name, "Center", (0.5, 0.5), frame);
        let pivot = self.pt(name, "Pivot", (0.5, 0.5), frame);
        let use_sa = self.num(name, "UseSizeAndAspect", 1.0, frame) > 0.5;
        let (sx, sy) = if use_sa {
            let size = self.num(name, "Size", 1.0, frame) as f32;
            let aspect = self.num(name, "Aspect", 1.0, frame) as f32;
            (size, size / aspect.max(1e-4))
        } else {
            (self.num(name, "XSize", 1.0, frame) as f32, self.num(name, "YSize", 1.0, frame) as f32)
        };
        let angle = self.num(name, "Angle", 0.0, frame) as f32;
        let flip_h = self.num(name, "FlipHoriz", 0.0, frame) > 0.5;
        let flip_v = self.num(name, "FlipVert", 0.0, frame) > 0.5;
        let invert = self.num(name, "InvertTransform", 0.0, frame) > 0.5;
        let edges = self.num(name, "Edges", 0.0, frame).round() as i32;
        let blend = self.num(name, "Blend", 1.0, frame) as f32;

        let (center, sx, sy, angle) = if invert {
            let isx = if sx.abs() > 1e-4 { 1.0 / sx } else { 1.0 };
            let isy = if sy.abs() > 1e-4 { 1.0 / sy } else { 1.0 };
            let iang = -angle;
            let (s, c) = iang.to_radians().sin_cos();
            let ox = center.0 - 0.5;
            let oy = center.1 - 0.5;
            let iox = -(ox * c - oy * s) * isx;
            let ioy = -(ox * s + oy * c) * isy;
            ((0.5 + iox, 0.5 + ioy), isx, isy, iang)
        } else {
            (center, sx, sy, angle)
        };

        let mut out = img::transform_image(&src, center, pivot, sx, sy, angle, flip_h, flip_v, edges);
        img::apply_channel_mask(&src, &mut out, self.channels(name, frame));
        let mask = self.effect_mask(name, frame, depth);
        img::blend_with_mask(&src, &mut out, mask.as_deref(), blend);
        Arc::new(out)
    }

    fn eval_merge(&mut self, name: &str, frame: f64, depth: usize) -> Arc<Image> {
        let bg = self
            .connected_image(name, "Background", frame, depth)
            .unwrap_or_else(|| Arc::new(Image::empty(self.width, self.height)));
        let Some(fg) = self.connected_image(name, "Foreground", frame, depth) else {
            return bg;
        };
        let center = self.pt(name, "Center", (0.5, 0.5), frame);
        let size = self.num(name, "Size", 1.0, frame) as f32;
        let angle = self.num(name, "Angle", 0.0, frame) as f32;
        let blend = self.num(name, "Blend", 1.0, frame) as f32;
        let alpha_gain = self.num(name, "AlphaGain", 1.0, frame) as f32;
        let apply_mode = self.str_val(name, "ApplyMode", "Normal");
        let operator = self.str_val(name, "Operator", "Over");
        let mask = self.effect_mask(name, frame, depth);
        Arc::new(img::merge_images(&bg, &fg, center, size, angle, blend, alpha_gain, apply_mode, operator, mask.as_deref()))
    }

    fn eval_dissolve(&mut self, name: &str, frame: f64, depth: usize) -> Arc<Image> {
        let bg = self
            .connected_image(name, "Background", frame, depth)
            .unwrap_or_else(|| Arc::new(Image::empty(self.width, self.height)));
        let fg = self
            .connected_image(name, "Foreground", frame, depth)
            .unwrap_or_else(|| Arc::new(Image::empty(self.width, self.height)));
        let mix = self.num(name, "Mix", 0.0, frame) as f32;
        Arc::new(img::dissolve_images(&bg, &fg, mix))
    }

    fn eval_text_plus(&mut self, name: &str, frame: f64, depth: usize) -> Arc<Image> {
        // Определяем текст: 1) явный override, 2) из overrides.texts по индексу TextPlus, 3) StyledText / Follower
        let mut text = if let Some(s) = self.overrides.strs.get(&(name.to_string(), "StyledText".into())) {
            s.clone()
        } else if let Some(idx) = self.text_order.iter().position(|n| n == name) {
            if let Some(s) = self.overrides.texts.get(idx) {
                s.clone()
            } else {
                self.read_text_input(name)
            }
        } else {
            self.read_text_input(name)
        };

        // Если подключён StyledTextFollower — поддерживаем его задержку/сдвиг и непрозрачность
        let mut follower_alpha = 1.0f32;
        let mut follower_offset = (0.0f32, 0.0f32);
        if let Some(sop) = self.comp.resolve_input(name, "StyledText").and_then(|i| i.get("SourceOp")).and_then(Lua::str) {
            if self.comp.tools.get(sop).is_some_and(|t| t.ty == "StyledTextFollower") {
                let op = self.num(sop, "Opacity1", 1.0, frame) as f32;
                follower_alpha = op.clamp(0.0, 1.0);
                let off = self.pt(sop, "Offset1", (0.5, 0.5), frame);
                if (off.0 - 0.5).abs() > 1e-4 || (off.1 - 0.5).abs() > 1e-4 {
                    follower_offset = (off.0 - 0.5, off.1 - 0.5);
                }
            }
        }
        // WriteOn (0..1) обрезает длину строки
        let w_end = self.num(name, "End", 1.0, frame).clamp(0.0, 1.0);
        let w_start = self.num(name, "Start", 0.0, frame).clamp(0.0, 1.0);
        if w_start > 0.001 || w_end < 0.999 {
            let chars: Vec<char> = text.chars().collect();
            let a = (chars.len() as f64 * w_start).round() as usize;
            let b = (chars.len() as f64 * w_end).round() as usize;
            text = chars.get(a.min(chars.len())..b.min(chars.len())).map(|s| s.iter().collect()).unwrap_or_default();
        }

        let font = self
            .overrides
            .strs
            .get(&(name.to_string(), "Font".into()))
            .map(String::as_str)
            .filter(|f| !f.is_empty())
            .or_else(|| self.overrides.font.as_deref().filter(|f| !f.is_empty()))
            .unwrap_or_else(|| self.str_val(name, "Font", ""));
        let style = self.str_val(name, "Style", "");
        let size = self.num(name, "Size", 0.08, frame) as f32;
        let r = self.num(name, "Red1", 1.0, frame) as f32;
        let g = self.num(name, "Green1", 1.0, frame) as f32;
        let b = self.num(name, "Blue1", 1.0, frame) as f32;
        let a = (self.num(name, "Alpha1", 1.0, frame) * self.num(name, "Opacity1", 1.0, frame)) as f32 * follower_alpha;
        let color = Color4::from_straight(r, g, b, a);
        let mut center = self.pt(name, "Center", (0.5, 0.5), frame);
        center.0 += follower_offset.0;
        center.1 += follower_offset.1;
        let tracking = self.num(name, "CharacterSpacing", 1.0, frame) as f32;
        let line_sp = self.num(name, "LineSpacing", 1.0, frame) as f32;
        // В Fusion HorizontalLeftCenterRight: -1=влево, 0=центр, 1=вправо;
        // VerticalTopCenterBottom (в Y-up): -1=верхняя кромка в якоре (текст вниз), 0=центр, 1=нижняя кромка в якоре (текст вверх).
        let h_align = self.num(name, "HorizontalLeftCenterRight", 0.0, frame) as f32;
        let v_align = -self.num(name, "VerticalTopCenterBottom", 0.0, frame) as f32;
        let angle = self.num(name, "AngleZ", 0.0, frame) as f32;

        let mut img = img::render_text_plus(self.width, self.height, &text, font, style, size, color, center, tracking, line_sp, h_align, v_align, angle);
        if let Some(mask) = self.effect_mask(name, frame, depth) {
            img::apply_mask_mul(&mut img, &mask);
        }
        Arc::new(img)
    }

    fn read_text_input(&self, name: &str) -> String {
        if let Some(inp) = self.comp.resolve_input(name, "StyledText") {
            if let Some(s) = inp.get("Value").and_then(Lua::str) {
                return s.to_string();
            }
            if let Some(sop) = inp.get("SourceOp").and_then(Lua::str) {
                if let Some(t) = self.comp.tools.get(sop) {
                    if let Some(s) = t.raw.input("Text").and_then(Lua::str) {
                        return s.to_string();
                    }
                    if let Some(s) = t.raw.input("Value").and_then(Lua::str) {
                        return s.to_string();
                    }
                }
            }
        }
        String::new()
    }

    fn eval_blur(&mut self, name: &str, frame: f64, depth: usize) -> Arc<Image> {
        let Some(src) = self.connected_image(name, "Input", frame, depth).or_else(|| self.connected_image(name, "Source", frame, depth)) else {
            return Arc::new(Image::empty(self.width, self.height));
        };
        let scale = self.width as f32 / 1920.0;
        let sx = self.num(name, "XBlurSize", self.num(name, "HorizontalStrength", 0.0, frame) * 40.0, frame) as f32 * scale;
        let lock = self.num(name, "LockXY", 1.0, frame) > 0.5;
        let sy = if lock { sx } else { self.num(name, "YBlurSize", sx as f64, frame) as f32 * scale };
        let blend = self.num(name, "Blend", 1.0, frame) as f32;
        let mut out = img::box_blur(&src, sx, sy);
        img::apply_channel_mask(&src, &mut out, self.channels(name, frame));
        let mask = self.effect_mask(name, frame, depth);
        img::blend_with_mask(&src, &mut out, mask.as_deref(), blend);
        Arc::new(out)
    }

    fn eval_dir_blur(&mut self, name: &str, ty: &str, frame: f64, depth: usize) -> Arc<Image> {
        let Some(src) = self.connected_image(name, "Input", frame, depth).or_else(|| self.connected_image(name, "Source", frame, depth)) else {
            return Arc::new(Image::empty(self.width, self.height));
        };
        let default_kind = if ty.contains("ZoomBlur") {
            3
        } else if ty.contains("RadialBlur") {
            1
        } else {
            0
        };
        let kind = self.num(name, "Type", default_kind as f64, frame).round() as i32;
        let length = self.num(name, "Length", self.num(name, "SmoothStrength", self.num(name, "BlurStrength", 0.0, frame), frame), frame) as f32;
        let angle = self.num(name, "Angle", self.num(name, "BlurAngle", 0.0, frame), frame) as f32;
        let center = self.pt(name, "Center", (0.5, 0.5), frame);
        let blend = self.num(name, "Blend", 1.0, frame) as f32;
        let mut out = img::directional_blur(&src, kind, length, angle, center);
        img::apply_channel_mask(&src, &mut out, self.channels(name, frame));
        let mask = self.effect_mask(name, frame, depth);
        img::blend_with_mask(&src, &mut out, mask.as_deref(), blend);
        Arc::new(out)
    }

    fn eval_glow(&mut self, name: &str, frame: f64, depth: usize) -> Arc<Image> {
        let Some(src) = self.connected_image(name, "Input", frame, depth) else {
            return Arc::new(Image::empty(self.width, self.height));
        };
        let size = self.num(name, "GlowSize", 10.0, frame) as f32;
        let glow = self.num(name, "Glow", 0.8, frame) as f32;
        let gain = self.num(name, "Gain", 1.0, frame) as f32;
        let blend = self.num(name, "Blend", 1.0, frame) as f32;
        let mut out = img::glow_image(&src, size, glow, gain);
        let mask = self.effect_mask(name, frame, depth);
        img::blend_with_mask(&src, &mut out, mask.as_deref(), blend);
        Arc::new(out)
    }

    fn eval_displace(&mut self, name: &str, frame: f64, depth: usize) -> Arc<Image> {
        let Some(src) = self.connected_image(name, "Input", frame, depth) else {
            return Arc::new(Image::empty(self.width, self.height));
        };
        let Some(map) = self.connected_image(name, "Foreground", frame, depth) else {
            return src;
        };
        let kind = self.num(name, "Type", 0.0, frame).round() as i32;
        let x_off = self.num(name, "XOffset", 0.0, frame) as f32;
        let x_ref = self.num(name, "XRefraction", 0.0, frame) as f32;
        let y_off = self.num(name, "YOffset", 0.0, frame) as f32;
        let y_ref = self.num(name, "YRefraction", 0.0, frame) as f32;
        let off = self.num(name, "Offset", 0.0, frame) as f32;
        let refr = self.num(name, "RefractionStrength", 0.0, frame) as f32;
        let blend = self.num(name, "Blend", 1.0, frame) as f32;
        let mut out = img::displace_image(&src, &map, kind, x_off, x_ref, y_off, y_ref, off, refr);
        let mask = self.effect_mask(name, frame, depth);
        img::blend_with_mask(&src, &mut out, mask.as_deref(), blend);
        Arc::new(out)
    }

    fn eval_duplicate(&mut self, name: &str, frame: f64, depth: usize) -> Arc<Image> {
        let copies = (self.num(name, "Copies", 2.0, frame).round() as usize).clamp(1, 32);
        let time_off = self.num(name, "TimeOffset", 0.0, frame);
        let center = self.pt(name, "Center", (0.5, 0.5), frame);
        let pivot = self.pt(name, "Axis", (0.5, 0.5), frame);
        let size = self.num(name, "Size", 1.0, frame) as f32;
        let angle = self.num(name, "Angle", 0.0, frame) as f32;
        let gain = self.num(name, "Gain", 1.0, frame) as f32;
        let blend = self.num(name, "Blend", 1.0, frame) as f32;
        let apply_mode = self.str_val(name, "ApplyMode", "Normal");

        let mut acc = Image::empty(self.width, self.height);
        for i in 0..copies {
            let f = (frame + i as f64 * time_off).max(0.0);
            let Some(src) = self.connected_image(name, "Background", f, depth) else { break };
            let fi = i as f32;
            let cx = 0.5 + (center.0 - 0.5) * fi;
            let cy = 0.5 + (center.1 - 0.5) * fi;
            let s = size.powf(fi);
            let a = angle * fi;
            let g = gain.powf(fi) * blend;
            let layer = if i == 0 {
                (*src).clone()
            } else {
                img::transform_image(&src, (cx, cy), pivot, s, s, a, false, false, 0)
            };
            acc = img::merge_images(&acc, &layer, (0.5, 0.5), 1.0, 0.0, g.clamp(0.0, 1.0), 1.0, apply_mode, "Over", None);
        }
        Arc::new(acc)
    }

    fn eval_bc(&mut self, name: &str, frame: f64, depth: usize) -> Arc<Image> {
        let Some(src) = self.connected_image(name, "Input", frame, depth) else {
            return Arc::new(Image::empty(self.width, self.height));
        };
        let gain = self.num(name, "Gain", 1.0, frame) as f32;
        let lift = self.num(name, "Lift", 0.0, frame) as f32;
        let gamma = self.num(name, "Gamma", 1.0, frame) as f32;
        let contrast = self.num(name, "Contrast", 0.0, frame) as f32;
        let bright = self.num(name, "Brightness", 0.0, frame) as f32;
        let sat = self.num(name, "Saturation", 1.0, frame) as f32;
        let blend = self.num(name, "Blend", 1.0, frame) as f32;
        let mut out = img::brightness_contrast(&src, gain, lift, gamma, contrast, bright, sat);
        let mask = self.effect_mask(name, frame, depth);
        img::blend_with_mask(&src, &mut out, mask.as_deref(), blend);
        Arc::new(out)
    }

    fn eval_color_curves(&mut self, name: &str, frame: f64, depth: usize) -> Arc<Image> {
        let Some(src) = self.connected_image(name, "Input", frame, depth) else {
            return Arc::new(Image::empty(self.width, self.height));
        };
        let mut luts = [[0.0f32; 256]; 4];
        for c in 0..4 {
            for i in 0..256 {
                luts[c][i] = i as f32 / 255.0;
            }
        }
        for (c, ch_name) in ["Red", "Green", "Blue", "Alpha"].iter().enumerate() {
            if let Some(sop) = self.comp.resolve_input(name, ch_name).and_then(|i| i.get("SourceOp")).and_then(Lua::str) {
                if let Some(lut_tool) = self.comp.tools.get(sop) {
                    for i in 0..256 {
                        luts[c][i] = anim::eval_lut_bezier(&lut_tool.raw, i as f64 / 255.0).clamp(0.0, 1.0) as f32;
                    }
                }
            }
        }
        let blend = self.num(name, "Blend", 1.0, frame) as f32;
        let mut out = img::apply_color_curves(&src, &luts);
        let mask = self.effect_mask(name, frame, depth);
        img::blend_with_mask(&src, &mut out, mask.as_deref(), blend);
        Arc::new(out)
    }

    fn eval_drop_shadow(&mut self, name: &str, frame: f64, depth: usize) -> Arc<Image> {
        let Some(src) = self.connected_image(name, "Input", frame, depth).or_else(|| self.connected_image(name, "Source", frame, depth)) else {
            return Arc::new(Image::empty(self.width, self.height));
        };
        let strength = self.num(name, "shadowStrength", self.num(name, "Softness", 0.5, frame), frame) as f32;
        let angle = self.num(name, "dropAngle", 37.5, frame) as f32;
        let dist = self.num(name, "dropDistance", 0.02, frame) as f32;
        let blur = self.num(name, "Blur", 0.2, frame) as f32 * self.width as f32 * 0.02;
        let dx = angle.to_radians().cos() * dist;
        let dy = -angle.to_radians().sin() * dist;
        let col = Color4::from_straight(0.0, 0.0, 0.0, strength.clamp(0.0, 1.0));
        Arc::new(img::drop_shadow(&src, dx, dy, blur, col))
    }

    fn eval_fast_noise(&mut self, name: &str, frame: f64, depth: usize) -> Arc<Image> {
        let scale = self.num(name, "XScale", 2.0, frame) as f32;
        let seethe = (self.num(name, "Seethe", 0.0, frame) + self.num(name, "SeetheRate", 0.0, frame) * frame * 0.05) as f32;
        let detail = self.num(name, "Detail", 2.0, frame).round() as i32;
        let c1 = Color4::from_straight(
            self.num(name, "Color1Red", 0.0, frame) as f32,
            self.num(name, "Color1Green", 0.0, frame) as f32,
            self.num(name, "Color1Blue", 0.0, frame) as f32,
            self.num(name, "Color1Alpha", 0.0, frame) as f32,
        );
        let c2 = Color4::from_straight(
            self.num(name, "Color2Red", 1.0, frame) as f32,
            self.num(name, "Color2Green", 1.0, frame) as f32,
            self.num(name, "Color2Blue", 1.0, frame) as f32,
            self.num(name, "Color2Alpha", 1.0, frame) as f32,
        );
        let mut out = img::fast_noise(self.width, self.height, scale, seethe, detail, c1, c2);
        if let Some(mask) = self.effect_mask(name, frame, depth) {
            img::apply_mask_mul(&mut out, &mask);
        }
        Arc::new(out)
    }

    fn eval_corner_pos(&mut self, name: &str, frame: f64, depth: usize) -> Arc<Image> {
        let Some(src) = self.connected_image(name, "Input", frame, depth) else {
            return Arc::new(Image::empty(self.width, self.height));
        };
        let bl = self.pt(name, "BottomLeft", (0.0, 0.0), frame);
        let br = self.pt(name, "BottomRight", (1.0, 0.0), frame);
        let tl = self.pt(name, "TopLeft", (0.0, 1.0), frame);
        let tr = self.pt(name, "TopRight", (1.0, 1.0), frame);
        Arc::new(img::corner_positioner(&src, bl, br, tl, tr))
    }
}
