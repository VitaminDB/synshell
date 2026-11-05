//! Дыхательный орб (цикл 4-7-8) на стоковом `Canvas`.
//!
//! Орб и текст фазы синхронизированы через настенные часы (`now_ms`).

use syngui::core::Color;
use syngui::widgets::Canvas;

use crate::util::now_ms;

/// Длительности фаз (секунды): вдох 4, задержка 7, выдох 8.
pub const INHALE: f64 = 4.0;
pub const HOLD: f64 = 7.0;
pub const EXHALE: f64 = 8.0;
pub const CYCLE_MS: u64 = 19_000; // (4+7+8) * 1000

fn smoothstep(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Фаза дыхания на момент `now_ms`.
/// Возвращает `(индекс_фазы, масштаб 0..1)`: 0 — вдох, 1 — задержка, 2 — выдох.
pub fn phase_at(now: u64) -> (u8, f32) {
    let t = (now % CYCLE_MS) as f64 / 1000.0;
    if t < INHALE {
        (0, smoothstep(t / INHALE) as f32)
    } else if t < INHALE + HOLD {
        (1, 1.0)
    } else {
        (2, (1.0 - smoothstep((t - INHALE - HOLD) / EXHALE)) as f32)
    }
}

/// Ключ i18n текущей фазы.
pub fn phase_key(now: u64) -> &'static str {
    match phase_at(now).0 {
        0 => "craving.inhale",
        1 => "craving.hold",
        _ => "craving.exhale",
    }
}

/// Виджет-орб заданного размера и акцентного цвета.
pub fn breathing_orb(accent: Color, size: f32) -> Canvas {
    Canvas::new(move |ctx, _t| {
        let (_phase, s) = phase_at(now_ms());
        let w = ctx.width();
        let h = ctx.height();
        let cx = w * 0.5;
        let cy = h * 0.5;
        let rmax = w.min(h) * 0.5;
        let base = rmax * 0.34;
        let peak = rmax * 0.94;
        let r = base + (peak - base) * s;

        // Внешние ореолы (мягкое свечение).
        ctx.set_color(accent.with_alpha(0.10));
        ctx.fill_circle(cx, cy, (r * 1.28).min(rmax));
        ctx.set_color(accent.with_alpha(0.18));
        ctx.fill_circle(cx, cy, (r * 1.12).min(rmax));
        // Тело орба.
        ctx.set_color(accent.with_alpha(0.85));
        ctx.fill_circle(cx, cy, r);
        // Блик.
        ctx.set_color(Color::from_srgb(255, 255, 255, 0.35));
        ctx.fill_circle(cx - r * 0.28, cy - r * 0.30, r * 0.22);
    })
    .animated(true)
    .size(size, size)
}
