//! Анимация и время в Fusion:
//! - [`eval_spline`] (`BezierSpline` — кубические кривые Безье по `(frame, value)` с LH/RH, Linear/Step/Loop/PingPong);
//! - [`eval_polypath`] (`PolyPath` + `Polyline` — путь по длине дуги с Безье-сегментами и сдвигом от центра `(0.5, 0.5)`);
//! - [`eval_lut_lookup`] (`LUTLookup` / AnimCurves + `LUTBezier` — кривые Easing/Custom/Mirror/Bounce);
//! - [`key_stretch_frame`] (`KeyStretcher` / `KeyStretcherMod` — сохранение скорости интро и аутро);
//! - [`eval_expression`] — лёгкий вычислитель простых Fusion-выражений (`Point(...)`, арифметика, `time`).

use super::{Comp, Lua};

#[derive(Clone, Debug)]
struct Keyframe {
    frame: f64,
    value: f64,
    lh: Option<(f64, f64)>,
    rh: Option<(f64, f64)>,
    linear: bool,
    step_in: bool,
    step_out: bool,
}

fn parse_keyframes(kfs: &Lua) -> Vec<Keyframe> {
    let mut out = Vec::new();
    for (k, v) in kfs.items() {
        let Some(frame) = k.as_deref().and_then(|s| s.parse::<f64>().ok()) else { continue };
        let value = v.items().iter().find(|(ik, _)| ik.is_none()).and_then(|(_, iv)| iv.num()).unwrap_or(0.0);
        let lh = v.get("LH").and_then(Lua::point);
        let rh = v.get("RH").and_then(Lua::point);
        let flags = v.get("Flags");
        let flag = |name: &str| flags.and_then(|f| f.get(name)).and_then(|x| match x { Lua::Bool(b) => Some(*b), Lua::Num(n) => Some(*n != 0.0), _ => None }).unwrap_or(false);
        out.push(Keyframe {
            frame,
            value,
            lh,
            rh,
            linear: flag("Linear"),
            step_in: flag("StepIn"),
            step_out: flag("StepOut"),
        });
    }
    out.sort_by(|a, b| a.frame.total_cmp(&b.frame));
    out
}

/// Оценить кубическую кривую Безье `(x(u), y(u))` при заданном `x = frame`.
fn eval_cubic_segment(k0: &Keyframe, k1: &Keyframe, frame: f64) -> f64 {
    if k0.step_in {
        return k0.value;
    }
    if k1.step_out {
        return k1.value;
    }
    let dx = k1.frame - k0.frame;
    if dx.abs() < 1e-9 {
        return k1.value;
    }
    let t_lin = ((frame - k0.frame) / dx).clamp(0.0, 1.0);
    if k0.linear && k1.linear && k0.rh.is_none() && k1.lh.is_none() {
        return k0.value + (k1.value - k0.value) * t_lin;
    }
    let p0 = (k0.frame, k0.value);
    let p3 = (k1.frame, k1.value);
    let p1 = k0.rh.unwrap_or((k0.frame + dx / 3.0, k0.value + (k1.value - k0.value) / 3.0));
    let p2 = k1.lh.unwrap_or((k1.frame - dx / 3.0, k1.value - (k1.value - k0.value) / 3.0));
    // В Fusion LH/RH иногда хранят абсолютные координаты (frame, val); зажимаем X контрольных точек в [p0.0, p3.0] для монотонности
    let c1x = p1.0.clamp(p0.0, p3.0);
    let c2x = p2.0.clamp(p0.0, p3.0);

    let bez_x = |u: f64| {
        let m = 1.0 - u;
        m * m * m * p0.0 + 3.0 * m * m * u * c1x + 3.0 * m * u * u * c2x + u * u * u * p3.0
    };
    let bez_dx = |u: f64| {
        let m = 1.0 - u;
        3.0 * m * m * (c1x - p0.0) + 6.0 * m * u * (c2x - c1x) + 3.0 * u * u * (p3.0 - c2x)
    };
    let bez_y = |u: f64| {
        let m = 1.0 - u;
        m * m * m * p0.1 + 3.0 * m * m * u * p1.1 + 3.0 * m * u * u * p2.1 + u * u * u * p3.1
    };

    let mut lo = 0.0f64;
    let mut hi = 1.0f64;
    let mut u = t_lin;
    for _ in 0..10 {
        let err = bez_x(u) - frame;
        if err.abs() < 1e-6 {
            break;
        }
        if err < 0.0 {
            lo = u;
        } else {
            hi = u;
        }
        let d = bez_dx(u);
        if d.abs() > 1e-6 {
            let next = u - err / d;
            u = if next > lo && next < hi { next } else { 0.5 * (lo + hi) };
        } else {
            u = 0.5 * (lo + hi);
        }
    }
    bez_y(u)
}

/// Вычислить `BezierSpline` в кадре `frame`.
pub fn eval_spline(node: &Lua, frame: f64) -> f64 {
    let Some(kfs_lua) = node.get("KeyFrames") else { return 0.0 };
    let kfs = parse_keyframes(kfs_lua);
    if kfs.is_empty() {
        return 0.0;
    }
    if kfs.len() == 1 {
        return kfs[0].value;
    }
    let first = kfs.first().unwrap();
    let last = kfs.last().unwrap();
    let f = if frame <= first.frame {
        return first.value;
    } else if frame >= last.frame {
        // Проверяем Loop / PingPong на последнем ключе
        let flags = kfs_lua.items().last().and_then(|(_, v)| v.get("Flags"));
        let is_loop = flags.and_then(|fl| fl.get("Loop")).and_then(|x| match x { Lua::Bool(b) => Some(*b), _ => None }).unwrap_or(false);
        let is_pp = flags.and_then(|fl| fl.get("PingPong")).and_then(|x| match x { Lua::Bool(b) => Some(*b), _ => None }).unwrap_or(false);
        let span = last.frame - first.frame;
        if span > 1e-6 && (is_loop || is_pp) {
            let cyc = ((frame - first.frame) / span).floor() as i64;
            let rem = (frame - first.frame) - cyc as f64 * span;
            if is_pp && cyc % 2 != 0 {
                last.frame - rem
            } else {
                first.frame + rem
            }
        } else {
            return last.value;
        }
    } else {
        frame
    };

    for w in kfs.windows(2) {
        if f >= w[0].frame && f <= w[1].frame {
            return eval_cubic_segment(&w[0], &w[1], f);
        }
    }
    last.value
}

/// Растяжение времени `KeyStretcher` / `KeyStretcherMod`.
pub fn key_stretch_frame(frame: f64, total_frames: f64, src_start: f64, src_end: f64, stretch_start: f64, stretch_end: f64) -> f64 {
    let total = total_frames.max(1.0);
    let intro = (stretch_start - src_start).max(0.0);
    let outro = (src_end - stretch_end).max(0.0);
    if total <= intro + outro + 1e-6 {
        // Клип короче интро+аутро: масштабируем пропорционально
        let u = (frame / total).clamp(0.0, 1.0);
        return src_start + u * (src_end - src_start);
    }
    if frame <= intro {
        src_start + frame
    } else if frame >= total - outro {
        src_end - (total - frame)
    } else {
        let mid_u = (frame - intro) / (total - intro - outro);
        stretch_start + mid_u * (stretch_end - stretch_start)
    }
}

/// Сглаживания для `LUTLookup` (AnimCurves).
fn ease_in(kind: &str, t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    match kind {
        "Sine" => 1.0 - (t * std::f64::consts::FRAC_PI_2).cos(),
        "Quad" => t * t,
        "Cubic" => t * t * t,
        "Quart" => t * t * t * t,
        "Quint" => t * t * t * t * t,
        "Expo" => if t <= 0.0 { 0.0 } else { 2.0f64.powf(10.0 * (t - 1.0)) },
        "Circ" => 1.0 - (1.0 - t * t).max(0.0).sqrt(),
        "Back" => {
            let s = 1.70158;
            t * t * ((s + 1.0) * t - s)
        }
        "Elastic" => {
            if t <= 0.0 || t >= 1.0 {
                t
            } else {
                -2.0f64.powf(10.0 * t - 10.0) * ((t * 10.0 - 10.75) * std::f64::consts::TAU / 3.0).sin()
            }
        }
        "Bounce" => 1.0 - ease_out("Bounce", 1.0 - t),
        _ => t,
    }
}

fn ease_out(kind: &str, t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    match kind {
        "Bounce" => {
            let (n1, d1) = (7.5625, 2.75);
            if t < 1.0 / d1 {
                n1 * t * t
            } else if t < 2.0 / d1 {
                let x = t - 1.5 / d1;
                n1 * x * x + 0.75
            } else if t < 2.5 / d1 {
                let x = t - 2.25 / d1;
                n1 * x * x + 0.9375
            } else {
                let x = t - 2.625 / d1;
                n1 * x * x + 0.984375
            }
        }
        _ => 1.0 - ease_in(kind, 1.0 - t),
    }
}

/// Вычислить `LUTLookup` (AnimCurves) в кадре `frame` при длительности `total_frames`.
pub fn eval_lut_lookup(comp: &Comp, name: &str, frame: f64, total_frames: f64, depth: usize) -> f64 {
    let curve = comp.resolve_input(name, "Curve").and_then(|i| i.get("Value")).and_then(Lua::str).unwrap_or("Linear");
    let ei = comp.resolve_input(name, "EaseIn").and_then(|i| i.get("Value")).and_then(Lua::str).unwrap_or("Linear");
    let eo = comp.resolve_input(name, "EaseOut").and_then(|i| i.get("Value")).and_then(Lua::str).unwrap_or("Linear");
    let scale = eval_num(comp, name, "Scale", 1.0, frame, total_frames, depth + 1);
    let offset = eval_num(comp, name, "Offset", 0.0, frame, total_frames, depth + 1);
    let time_scale = eval_num(comp, name, "TimeScale", 1.0, frame, total_frames, depth + 1);
    let time_offset = eval_num(comp, name, "TimeOffset", 0.0, frame, total_frames, depth + 1);
    let invert = eval_num(comp, name, "Invert", 0.0, frame, total_frames, depth + 1) > 0.5;
    let source = comp.resolve_input(name, "Source").and_then(|i| i.get("Value")).and_then(Lua::str).unwrap_or("Duration");

    let mut u = match source {
        "Input" => eval_num(comp, name, "Input", frame / total_frames.max(1.0), frame, total_frames, depth + 1),
        _ => frame / total_frames.max(1.0),
    };
    u = ((u - time_offset) * time_scale).clamp(0.0, 1.0);

    let mut y = match curve {
        "Easing" => {
            if u < 0.5 {
                0.5 * ease_in(ei, u * 2.0)
            } else {
                0.5 + 0.5 * ease_out(eo, (u - 0.5) * 2.0)
            }
        }
        "Mirror" => {
            let m = if u < 0.5 { u * 2.0 } else { (1.0 - u) * 2.0 };
            ease_out(eo, m)
        }
        "Bounce" => {
            let m = if u < 0.5 { u * 2.0 } else { (1.0 - u) * 2.0 };
            ease_out("Bounce", m)
        }
        "Custom" => {
            if let Some(sop) = comp.resolve_input(name, "Lookup").and_then(|i| i.get("SourceOp")).and_then(Lua::str) {
                if let Some(lut) = comp.tools.get(sop) {
                    eval_lut_bezier(&lut.raw, u)
                } else {
                    u
                }
            } else {
                u
            }
        }
        _ => u,
    };
    if invert {
        y = 1.0 - y;
    }
    offset + y * scale
}

pub fn eval_lut_bezier(node: &Lua, u: f64) -> f64 {
    let Some(splines) = node.get("KeyColorSplines") else { return u };
    let Some(first) = splines.get("0").or_else(|| splines.items().first().map(|(_, v)| v)) else { return u };
    let kfs = parse_keyframes(first);
    if kfs.is_empty() {
        return u;
    }
    if u <= kfs[0].frame {
        return kfs[0].value;
    }
    if u >= kfs.last().unwrap().frame {
        return kfs.last().unwrap().value;
    }
    for w in kfs.windows(2) {
        if u >= w[0].frame && u <= w[1].frame {
            return eval_cubic_segment(&w[0], &w[1], u);
        }
    }
    kfs.last().unwrap().value
}

#[derive(Clone, Copy, Debug)]
pub struct PolyPt {
    pub x: f64,
    pub y: f64,
    pub lx: f64,
    pub ly: f64,
    pub rx: f64,
    pub ry: f64,
    pub linear: bool,
}

pub fn parse_polyline(poly: &Lua) -> (Vec<PolyPt>, bool) {
    let closed = poly.get("Closed").and_then(|v| match v { Lua::Bool(b) => Some(*b), Lua::Num(n) => Some(*n != 0.0), _ => None }).unwrap_or(false);
    let mut pts = Vec::new();
    if let Some(list) = poly.get("Points") {
        for (_, p) in list.items() {
            let x = p.get("X").and_then(Lua::num).unwrap_or(0.0);
            let y = p.get("Y").and_then(Lua::num).unwrap_or(0.0);
            let lx = p.get("LX").and_then(Lua::num).unwrap_or(0.0);
            let ly = p.get("LY").and_then(Lua::num).unwrap_or(0.0);
            let rx = p.get("RX").and_then(Lua::num).unwrap_or(0.0);
            let ry = p.get("RY").and_then(Lua::num).unwrap_or(0.0);
            let linear = p.get("Linear").and_then(|v| match v { Lua::Bool(b) => Some(*b), _ => None }).unwrap_or(false);
            pts.push(PolyPt { x, y, lx, ly, rx, ry, linear });
        }
    }
    (pts, closed)
}

/// Дискретизировать кривую `Polyline` в список точек (координаты относительно центра).
pub fn sample_polyline(pts: &[PolyPt], closed: bool, steps_per_seg: usize) -> Vec<(f64, f64)> {
    if pts.is_empty() {
        return Vec::new();
    }
    if pts.len() == 1 {
        return vec![(pts[0].x, pts[0].y)];
    }
    let segs = if closed { pts.len() } else { pts.len() - 1 };
    let mut out = Vec::with_capacity(segs * steps_per_seg + 1);
    for i in 0..segs {
        let a = pts[i];
        let b = pts[(i + 1) % pts.len()];
        let is_lin = a.linear && b.linear && a.rx.abs() < 1e-9 && a.ry.abs() < 1e-9 && b.lx.abs() < 1e-9 && b.ly.abs() < 1e-9;
        let n = if is_lin { 1 } else { steps_per_seg.max(2) };
        let (p0x, p0y) = (a.x, a.y);
        let (p1x, p1y) = (a.x + a.rx, a.y + a.ry);
        let (p2x, p2y) = (b.x + b.lx, b.y + b.ly);
        let (p3x, p3y) = (b.x, b.y);
        for s in 0..n {
            let t = s as f64 / n as f64;
            let m = 1.0 - t;
            let x = m * m * m * p0x + 3.0 * m * m * t * p1x + 3.0 * m * t * t * p2x + t * t * t * p3x;
            let y = m * m * m * p0y + 3.0 * m * m * t * p1y + 3.0 * m * t * t * p2y + t * t * t * p3y;
            out.push((x, y));
        }
    }
    if !closed {
        let last = pts.last().unwrap();
        out.push((last.x, last.y));
    }
    out
}

/// Вычислить позицию `PolyPath` в кадре `frame`: `(0.5 + x, 0.5 + y)`.
pub fn eval_polypath(comp: &Comp, name: &str, frame: f64, total_frames: f64, depth: usize) -> (f64, f64) {
    let disp = eval_num(comp, name, "Displacement", 0.0, frame, total_frames, depth + 1).clamp(0.0, 1.0);
    let Some(poly_val) = comp.resolve_input(name, "PolyLine").and_then(|i| i.get("Value")) else {
        return (0.5, 0.5);
    };
    let (pts, closed) = parse_polyline(poly_val);
    let samples = sample_polyline(&pts, closed, 16);
    if samples.is_empty() {
        return (0.5, 0.5);
    }
    if samples.len() == 1 {
        return (0.5 + samples[0].0, 0.5 + samples[0].1);
    }
    let mut lengths = Vec::with_capacity(samples.len());
    lengths.push(0.0f64);
    let mut total = 0.0;
    for w in samples.windows(2) {
        total += (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
        lengths.push(total);
    }
    if total < 1e-9 {
        return (0.5 + samples[0].0, 0.5 + samples[0].1);
    }
    let target = disp * total;
    for i in 0..samples.len() - 1 {
        if target >= lengths[i] && target <= lengths[i + 1] {
            let seg = (lengths[i + 1] - lengths[i]).max(1e-9);
            let u = (target - lengths[i]) / seg;
            let x = samples[i].0 + (samples[i + 1].0 - samples[i].0) * u;
            let y = samples[i].1 + (samples[i + 1].1 - samples[i].1) * u;
            return (0.5 + x, 0.5 + y);
        }
    }
    let last = samples.last().unwrap();
    (0.5 + last.0, 0.5 + last.1)
}

/// Вычислить скалярный вход `input_name` инструмента `tool_name`.
pub fn eval_num(comp: &Comp, tool_name: &str, input_name: &str, default: f64, frame: f64, total_frames: f64, depth: usize) -> f64 {
    if depth > 12 {
        return default;
    }
    let Some(inp) = comp.resolve_input(tool_name, input_name) else {
        return default;
    };
    if let Some(sop) = inp.get("SourceOp").and_then(Lua::str) {
        let src = inp.get("Source").and_then(Lua::str).unwrap_or("Value");
        return eval_tool_num(comp, sop, src, default, frame, total_frames, depth + 1);
    }
    if let Some(expr) = inp.get("Expression").and_then(Lua::str) {
        if let Some(v) = eval_expr_num(comp, tool_name, expr, frame, total_frames, depth + 1) {
            return v;
        }
    }
    inp.get("Value").and_then(Lua::num).unwrap_or(default)
}

/// Вычислить 2D-точку (`Center`, `Pivot`, `Start`, `End`, `Offset1`, …).
pub fn eval_point(comp: &Comp, tool_name: &str, input_name: &str, default: (f64, f64), frame: f64, total_frames: f64, depth: usize) -> (f64, f64) {
    if depth > 12 {
        return default;
    }
    let Some(inp) = comp.resolve_input(tool_name, input_name) else {
        return default;
    };
    if let Some(sop) = inp.get("SourceOp").and_then(Lua::str) {
        if let Some(t) = comp.tools.get(sop) {
            match t.ty.as_str() {
                "PolyPath" => return eval_polypath(comp, sop, frame, total_frames, depth + 1),
                "Vector" => {
                    let orig = eval_point(comp, sop, "Origin", (0.5, 0.5), frame, total_frames, depth + 1);
                    let dist = eval_num(comp, sop, "Distance", 0.0, frame, total_frames, depth + 1);
                    let ang = eval_num(comp, sop, "Angle", 0.0, frame, total_frames, depth + 1).to_radians();
                    let aspect = eval_num(comp, sop, "ImageAspect", 16.0 / 9.0, frame, total_frames, depth + 1).max(0.1);
                    return (orig.0 + dist * ang.cos(), orig.1 + dist * ang.sin() * aspect);
                }
                "PublishPoint" => return eval_point(comp, sop, "Value", default, frame, total_frames, depth + 1),
                "Shake" => {
                    let x = eval_tool_num(comp, sop, "Value", 0.5, frame, total_frames, depth + 1);
                    let y = eval_tool_num(comp, sop, "Value", 0.5, frame + 37.0, total_frames, depth + 1);
                    return (x, y);
                }
                _ => {
                    let src = inp.get("Source").and_then(Lua::str).unwrap_or("Center");
                    return eval_point(comp, sop, src, default, frame, total_frames, depth + 1);
                }
            }
        }
    }
    if let Some(expr) = inp.get("Expression").and_then(Lua::str) {
        if let Some(pt) = eval_expr_point(comp, tool_name, expr, frame, total_frames, depth + 1) {
            return pt;
        }
    }
    inp.get("Value").and_then(Lua::point).unwrap_or(default)
}

/// Вычислить строковый вход (`FuID`, `Font`, `Style`, `ApplyMode`, `Type`, …).
pub fn eval_str<'a>(comp: &'a Comp, tool_name: &str, input_name: &str, default: &'a str) -> &'a str {
    comp.resolve_input(tool_name, input_name)
        .and_then(|i| i.get("Value"))
        .and_then(Lua::str)
        .unwrap_or(default)
}

/// Вычислить числовой выход инструмента-модификатора (`BezierSpline`, `LUTLookup`, `KeyStretcherMod`, `Shake`, `PublishNumber`).
pub fn eval_tool_num(comp: &Comp, sop: &str, source: &str, default: f64, frame: f64, total_frames: f64, depth: usize) -> f64 {
    if depth > 12 {
        return default;
    }
    let Some(t) = comp.tools.get(sop) else { return default };
    match t.ty.as_str() {
        "BezierSpline" => eval_spline(&t.raw, frame),
        "LUTLookup" => eval_lut_lookup(comp, sop, frame, total_frames, depth + 1),
        "LUTBezier" => eval_lut_bezier(&t.raw, (frame / total_frames.max(1.0)).clamp(0.0, 1.0)),
        "KeyStretcherMod" => {
            let s0 = eval_num(comp, sop, "SourceStart", 0.0, frame, total_frames, depth + 1);
            let s1 = eval_num(comp, sop, "SourceEnd", comp.source_frames, frame, total_frames, depth + 1);
            let k0 = eval_num(comp, sop, "StretchStart", s0, frame, total_frames, depth + 1);
            let k1 = eval_num(comp, sop, "StretchEnd", s1, frame, total_frames, depth + 1);
            let mapped = key_stretch_frame(frame, total_frames, s0, s1, k0, k1);
            eval_num(comp, sop, "Keyframes", default, mapped, total_frames, depth + 1)
        }
        "PublishNumber" => eval_num(comp, sop, "Value", default, frame, total_frames, depth + 1),
        "Shake" => {
            let min = eval_num(comp, sop, "Minimum", 0.0, frame, total_frames, depth + 1);
            let max = eval_num(comp, sop, "Maximum", 1.0, frame, total_frames, depth + 1);
            let smooth = eval_num(comp, sop, "Smoothness", 1.0, frame, total_frames, depth + 1).max(0.1);
            let seed = eval_num(comp, sop, "RandomSeed", 0.0, frame, total_frames, depth + 1);
            let n = (frame / smooth + seed * 1.31).sin() * 0.5 + 0.5;
            min + (max - min) * n
        }
        "PolyPath" => {
            let p = eval_polypath(comp, sop, frame, total_frames, depth + 1);
            if source == "Y" { p.1 } else { p.0 }
        }
        _ => eval_num(comp, sop, source, default, frame, total_frames, depth + 1),
    }
}

/// Простой вычислитель `Expression` для Point(a, b).
fn eval_expr_point(comp: &Comp, tool_name: &str, expr: &str, frame: f64, total_frames: f64, depth: usize) -> Option<(f64, f64)> {
    let s = expr.trim();
    let inner = s.strip_prefix("Point(")?.strip_suffix(')')?;
    let mut level = 0i32;
    let mut comma = None;
    for (i, ch) in inner.char_indices() {
        match ch {
            '(' => level += 1,
            ')' => level -= 1,
            ',' if level == 0 => {
                comma = Some(i);
                break;
            }
            _ => {}
        }
    }
    let idx = comma?;
    let x = eval_expr_num(comp, tool_name, &inner[..idx], frame, total_frames, depth + 1)?;
    let y = eval_expr_num(comp, tool_name, &inner[idx + 1..], frame, total_frames, depth + 1)?;
    Some((x, y))
}

/// Простой рекурсивный вычислитель арифметических выражений Fusion (числа, `+ - * /`, скобки, `time`, ссылки на входы).
fn eval_expr_num(comp: &Comp, tool_name: &str, expr: &str, frame: f64, total_frames: f64, depth: usize) -> Option<f64> {
    if depth > 12 {
        return None;
    }
    let bytes = expr.as_bytes();
    let mut pos = 0usize;
    let val = parse_add(comp, tool_name, bytes, &mut pos, frame, total_frames, depth)?;
    while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }
    if pos == bytes.len() && val.is_finite() { Some(val) } else { None }
}

fn skip_ws(b: &[u8], i: &mut usize) {
    while *i < b.len() && b[*i].is_ascii_whitespace() {
        *i += 1;
    }
}

fn parse_add(comp: &Comp, tn: &str, b: &[u8], i: &mut usize, f: f64, tf: f64, d: usize) -> Option<f64> {
    let mut lhs = parse_mul(comp, tn, b, i, f, tf, d)?;
    loop {
        skip_ws(b, i);
        let op = *b.get(*i)?;
        if op == b'+' || op == b'-' {
            *i += 1;
            let rhs = parse_mul(comp, tn, b, i, f, tf, d)?;
            lhs = if op == b'+' { lhs + rhs } else { lhs - rhs };
        } else {
            break;
        }
    }
    Some(lhs)
}

fn parse_mul(comp: &Comp, tn: &str, b: &[u8], i: &mut usize, f: f64, tf: f64, d: usize) -> Option<f64> {
    let mut lhs = parse_unary(comp, tn, b, i, f, tf, d)?;
    loop {
        skip_ws(b, i);
        let Some(&op) = b.get(*i) else { break };
        if op == b'*' || op == b'/' {
            *i += 1;
            let rhs = parse_unary(comp, tn, b, i, f, tf, d)?;
            lhs = if op == b'*' { lhs * rhs } else if rhs.abs() > 1e-12 { lhs / rhs } else { 0.0 };
        } else {
            break;
        }
    }
    Some(lhs)
}

fn parse_unary(comp: &Comp, tn: &str, b: &[u8], i: &mut usize, f: f64, tf: f64, d: usize) -> Option<f64> {
    skip_ws(b, i);
    if b.get(*i) == Some(&b'-') {
        *i += 1;
        return Some(-parse_unary(comp, tn, b, i, f, tf, d)?);
    }
    if b.get(*i) == Some(&b'+') {
        *i += 1;
        return parse_unary(comp, tn, b, i, f, tf, d);
    }
    parse_atom(comp, tn, b, i, f, tf, d)
}

fn parse_atom(comp: &Comp, tn: &str, b: &[u8], i: &mut usize, f: f64, tf: f64, d: usize) -> Option<f64> {
    skip_ws(b, i);
    let &ch = b.get(*i)?;
    if ch == b'(' {
        *i += 1;
        let v = parse_add(comp, tn, b, i, f, tf, d)?;
        skip_ws(b, i);
        if b.get(*i) == Some(&b')') {
            *i += 1;
            return Some(v);
        }
        return None;
    }
    if ch.is_ascii_digit() || ch == b'.' {
        let st = *i;
        while *i < b.len() && (b[*i].is_ascii_digit() || b[*i] == b'.' || b[*i] == b'e' || b[*i] == b'E') {
            *i += 1;
        }
        return std::str::from_utf8(&b[st..*i]).ok()?.parse().ok();
    }
    if ch.is_ascii_alphabetic() || ch == b'_' {
        let st = *i;
        while *i < b.len() && (b[*i].is_ascii_alphanumeric() || b[*i] == b'_' || b[*i] == b'.') {
            *i += 1;
        }
        let name = std::str::from_utf8(&b[st..*i]).ok()?;
        match name {
            "time" => return Some(f),
            "comp.RenderEnd" => return Some(tf),
            "comp.RenderStart" => return Some(0.0),
            "pi" | "math.pi" => return Some(std::f64::consts::PI),
            _ => {}
        }
        // Ссылка вида Tool.Input.X / Tool.Input.Y / Tool.Input / Input
        let parts: Vec<&str> = name.split('.').collect();
        match parts.as_slice() {
            [inp] => return Some(eval_num(comp, tn, inp, 0.0, f, tf, d + 1)),
            [tool, inp] => {
                if comp.tools.contains_key(*tool) {
                    return Some(eval_num(comp, tool, inp, 0.0, f, tf, d + 1));
                }
                if matches!(*inp, "X" | "Y") {
                    let pt = eval_point(comp, tn, tool, (0.5, 0.5), f, tf, d + 1);
                    return Some(if *inp == "X" { pt.0 } else { pt.1 });
                }
            }
            [tool, inp, axis] if matches!(*axis, "X" | "Y") => {
                if comp.tools.contains_key(*tool) {
                    let pt = eval_point(comp, tool, inp, (0.5, 0.5), f, tf, d + 1);
                    return Some(if *axis == "X" { pt.0 } else { pt.1 });
                }
            }
            _ => {}
        }
    }
    None
}
