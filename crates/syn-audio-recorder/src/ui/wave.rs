//! Холсты: живая волна записи (бегущие столбики, «сейчас» — вертикальная черта), индикатор уровня,
//! миниатюра и волна плеера по огибающей записи. Цвета — из MSS: `color` (основной), `accent-color` (запись).

use std::time::Instant;

use syngui::core::canvas::CanvasContext;
use syngui::prelude::*;

use crate::engine::{self, BIN_SECS};
use crate::store::loudness;

fn colors(ctx: &CanvasContext) -> (Color, Color) {
    let fg = ctx.mss_color().unwrap_or_else(|| Color::from_hex("#e6e6e6"));
    let accent = ctx.mss_accent().unwrap_or_else(|| Color::from_hex("#ff4757"));
    (fg, accent)
}

/// Живая волна. `phone` — крупнее шаг.
pub fn live(animated: bool) -> Canvas {
    Canvas::new(move |ctx, t| draw_live(ctx, t)).animated(animated)
}

fn draw_live(ctx: &mut CanvasContext, t: f32) {
    let (fg, accent) = colors(ctx);
    let (w, h) = (ctx.width(), ctx.height());
    if w < 4.0 || h < 4.0 {
        return;
    }
    let mid = h * 0.5;
    let step: f32 = if w < 520.0 { 5.0 } else { 6.0 };
    let bar = (step * 0.56).max(2.0);
    let now_x = (w * 0.66).round();
    let amp_h = h * 0.84;

    // данные под замком — копия хвоста
    let need = (now_x / step) as usize + 3;
    let (bars, recording, paused, open, frac, total) = {
        let v = engine::vis();
        let recording = v.recording;
        let src: Vec<f32> = if recording {
            let n = v.hist.len();
            v.hist[n.saturating_sub(need)..].to_vec()
        } else {
            let n = v.live.len();
            v.live.iter().skip(n.saturating_sub(need)).copied().collect()
        };
        let moving = v.open && !(recording && v.paused);
        let frac = if moving { (v.last_bin.elapsed().as_secs_f64() / BIN_SECS).min(1.0) as f32 } else { 0.0 };
        (src, recording, v.paused, v.open, frac, v.hist.len())
    };

    // линия тишины справа — пунктир «будущего»
    ctx.set_color(fg.with_alpha(0.16));
    let mut x = now_x + step;
    while x < w - 2.0 {
        ctx.fill_circle(x, mid, 1.1);
        x += step;
    }

    // столбики: новые у черты, старые уходят влево и гаснут у края
    let n = bars.len();
    for (i, &p) in bars.iter().rev().enumerate() {
        let cx = now_x - (i as f32 + frac) * step - step * 0.5;
        if cx < -bar {
            break;
        }
        let edge = (cx / (w * 0.22)).clamp(0.0, 1.0);
        let a = 0.18 + 0.82 * edge;
        let bh = (loudness(p) * amp_h).max(3.0);
        let col = if recording { accent.with_alpha(a * if paused { 0.55 } else { 1.0 }) } else { fg.with_alpha(0.34 * a) };
        ctx.set_color(col);
        ctx.fill_rounded_rect(cx - bar * 0.5, mid - bh * 0.5, bar, bh, bar * 0.5);
        let _ = n;
    }

    // метки секунд записанной части (каждая секунда, крупнее — каждые 5 с)
    if recording {
        let per_sec = (1.0 / BIN_SECS).round() as usize;
        ctx.set_color(fg.with_alpha(0.22));
        for i in 0..need {
            let k = total as i64 - 1 - i as i64;
            if k < 0 {
                break;
            }
            if (k as usize + 1) % per_sec == 0 {
                let cx = now_x - (i as f32 + frac) * step;
                let big = ((k as usize + 1) / per_sec) % 5 == 0;
                let th = if big { 9.0 } else { 4.0 };
                ctx.fill_rounded_rect(cx - 0.75, 2.0, 1.5, th, 0.75);
                ctx.fill_rounded_rect(cx - 0.75, h - 2.0 - th, 1.5, th, 0.75);
            }
        }
    }

    // черта «сейчас»
    if recording {
        let a = if paused { 0.45 + 0.4 * (t * 3.5).sin().abs() } else { 1.0 };
        ctx.set_color(accent.with_alpha(a));
        ctx.fill_rounded_rect(now_x - 1.0, 6.0, 2.0, h - 12.0, 1.0);
        ctx.fill_circle(now_x, 6.0, 4.0);
        ctx.fill_circle(now_x, h - 6.0, 4.0);
    } else {
        ctx.set_color(fg.with_alpha(if open { 0.28 } else { 0.14 }));
        ctx.fill_rounded_rect(now_x - 0.75, 10.0, 1.5, h - 20.0, 0.75);
    }
}

/// Индикатор уровня: сегменты зелёный → жёлтый → красный, удерживаемый пик.
pub fn meter() -> Canvas {
    Canvas::new(move |ctx, _| {
        let (fg, _) = colors(ctx);
        let (w, h) = (ctx.width(), ctx.height());
        let (level, hold, open) = {
            let v = engine::vis();
            let hold = if v.hold_at.elapsed().as_secs_f32() < 1.5 { v.hold } else { v.level };
            (v.level, hold, v.open)
        };
        let segs = ((w / 7.0) as usize).clamp(12, 64);
        let gap = 2.0;
        let sw = (w - gap * (segs as f32 - 1.0)) / segs as f32;
        let lit = if open { loudness(level) } else { 0.0 };
        let hold_i = if open { (loudness(hold) * segs as f32) as usize } else { 0 };
        let green = Color::from_hex("#22c55e");
        let amber = Color::from_hex("#f5b324");
        let red = Color::from_hex("#ef4444");
        for i in 0..segs {
            let f = (i as f32 + 0.5) / segs as f32;
            let col = if f > 0.92 { red } else if f > 0.75 { amber } else { green };
            let on = f <= lit || (hold_i > 0 && i + 1 == hold_i);
            ctx.set_color(if on { col } else { fg.with_alpha(0.10) });
            ctx.fill_rounded_rect(i as f32 * (sw + gap), 0.0, sw, h, (h * 0.5).min(2.0));
        }
    })
    .animated(true)
}

/// Огибающая записи (0..255): `progress` — проигранная доля (акцентом), `None` — без неё.
pub fn peaks(peaks: Vec<u8>, progress: Option<f32>, step: f32) -> Canvas {
    Canvas::new(move |ctx, _| {
        let (fg, accent) = colors(ctx);
        let (w, h) = (ctx.width(), ctx.height());
        if w < 4.0 {
            return;
        }
        let mid = h * 0.5;
        let n = ((w / step) as usize).max(1);
        let bar = (step * 0.55).max(1.5);
        if peaks.is_empty() {
            ctx.set_color(fg.with_alpha(0.18));
            ctx.fill_rounded_rect(0.0, mid - 1.0, w, 2.0, 1.0);
            return;
        }
        for i in 0..n {
            // отрезок огибающей на столбик (максимум)
            let a = i * peaks.len() / n;
            let b = (((i + 1) * peaks.len()) / n).max(a + 1).min(peaks.len());
            let v = peaks[a..b].iter().copied().max().unwrap_or(0) as f32 / 255.0;
            let bh = (v * h * 0.92).max(2.0);
            let x = i as f32 * step + (step - bar) * 0.5;
            let played = progress.is_some_and(|p| (i as f32 + 0.5) / n as f32 <= p);
            let col = match progress {
                Some(_) if played => accent,
                Some(_) => fg.with_alpha(0.28),
                None => fg.with_alpha(0.42),
            };
            ctx.set_color(col);
            ctx.fill_rounded_rect(x, mid - bh * 0.5, bar, bh, bar * 0.5);
        }
        if let Some(p) = progress {
            let x = (p.clamp(0.0, 1.0) * w).clamp(1.0, w - 1.0);
            ctx.set_color(fg.with_alpha(0.9));
            ctx.fill_rounded_rect(x - 1.0, 0.0, 2.0, h, 1.0);
        }
    })
}

/// Для снимков: волна и уровень как при записи речи.
pub fn demo(seconds: f64, paused: bool) {
    let mut v = engine::vis();
    v.open = true;
    v.recording = true;
    v.paused = paused;
    v.rate = 48_000;
    let n = (seconds / BIN_SECS) as usize;
    v.hist = (0..n).map(speech).collect();
    v.live = v.hist.iter().copied().collect();
    v.samples = (seconds * 48_000.0) as u64;
    v.level = 0.42;
    v.hold = 0.6;
    v.hold_at = Instant::now();
}

/// Похожая на речь огибающая: слоги, паузы между фразами.
pub fn speech(i: usize) -> f32 {
    let x = i as f32;
    let phrase = ((x / 37.0).sin() * 0.5 + 0.5).powf(0.6);
    let syll = ((x * 0.9).sin() * 0.5 + 0.5) * ((x * 0.37 + 1.3).sin() * 0.4 + 0.6);
    let noise = (((i * 7919) % 101) as f32 / 101.0) * 0.25;
    let pause = if (i / 23) % 5 == 4 { 0.03 } else { 1.0 };
    ((phrase * syll + noise * 0.3) * pause * 0.7).clamp(0.002, 1.0)
}
