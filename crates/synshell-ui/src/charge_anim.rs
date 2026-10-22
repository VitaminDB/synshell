//! Подключение зарядки на телефоне: экран просыпается, снизу (от разъёма)
//! к кольцу заряда течёт поток искр, кольцо дорастает до текущего процента,
//! в центре — молния и процент; через три с половиной секунды всё гаснет.
//! Вне блокировки — отдельная поверхность поверх всего (касания проходят
//! насквозь), на экране блокировки — слой внутри него ([`layer`]).

use std::cell::Cell;
use std::f32::consts::{FRAC_PI_2, PI, TAU};
use std::time::Duration;

use syngui::core::Color;
use syngui::prelude::*;
use syngui::widgets::visual::canvas::Canvas;
use syngui::StyleValue;
use syngui_layer::{Anchor, KeyboardInteractivity, Layer, SurfaceId, SurfaceSpec};

use crate::ctx::ShellCtx;

/// Сколько длится показ, с.
const DURATION: f32 = 3.5;
/// Когда начинает гаснуть надпись, с.
const TEXT_OUT: f32 = 2.9;

/// Что показать: номер показа (новый — анимация сначала) и заряд.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Show {
    pub serial: u64,
    pub percent: u32,
}

thread_local! {
    static SURFACE: Cell<Option<SurfaceId>> = const { Cell::new(None) };
    static TIMERS: Cell<(Option<u64>, Option<u64>, Option<u64>)> = const { Cell::new((None, None, None)) };
}

fn state() -> (RwSignal<Option<Show>>, RwSignal<bool>) {
    thread_local! {
        static S: Cell<Option<(RwSignal<Option<Show>>, RwSignal<bool>)>> = const { Cell::new(None) };
    }
    S.with(|s| {
        if s.get().is_none() {
            s.set(Some((use_signal(None), use_signal(false))));
        }
        s.get().unwrap()
    })
}

/// Подходит ли устройство: телефонный вид или игровой режим на телефоне.
pub fn wanted(ctx: &ShellCtx) -> bool {
    ctx.is_phone() || crate::lock::device_is_phone(ctx)
}

/// Показать анимацию (главный поток).
pub fn show(ctx: ShellCtx, percent: u32) {
    let (cur, text) = state();
    let serial = cur.get_untracked().map(|s| s.serial + 1).unwrap_or(1);
    cur.set(Some(Show { serial, percent }));
    text.set(false);
    let (t0, t1, t2) = TIMERS.with(|t| t.take());
    for t in [t0, t1, t2].into_iter().flatten() {
        syngui_layer::cancel_timer(t);
    }
    // Надпись — когда экран уже затемнён.
    let t0 = syngui_layer::add_timer(Duration::from_millis(250), move || {
        state().1.set(true);
        None
    });
    let t1 = syngui_layer::add_timer(Duration::from_secs_f32(TEXT_OUT), move || {
        state().1.set(false);
        None
    });
    let t2 = syngui_layer::add_timer(Duration::from_secs_f32(DURATION + 0.1), move || {
        hide();
        None
    });
    TIMERS.with(|t| t.set((Some(t0), Some(t1), Some(t2))));
    // На экране блокировки анимация — его слой; иначе своя поверхность.
    if syngui_layer::session_locked().get_untracked() || SURFACE.with(|s| s.get()).is_some() {
        return;
    }
    let id = syngui_layer::create_surface(
        SurfaceSpec {
            namespace: "syndesktop-charging".into(),
            layer: Layer::Overlay,
            anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            size: (0, 0),
            margin: [0; 4],
            exclusive_zone: -1,
            keyboard: KeyboardInteractivity::None,
            output: crate::manager::focused_output(&ctx),
            auto_size: false,
            clear_color: [0.0; 4],
        },
        || Box::new(layer()),
    );
    // Касания проходят к окнам под анимацией.
    syngui_layer::set_input_region(id, Some(Vec::new()));
    SURFACE.with(|s| s.set(Some(id)));
}

/// Убрать (по таймеру, при отключении кабеля).
pub fn hide() {
    let (t0, t1, t2) = TIMERS.with(|t| t.take());
    for t in [t0, t1, t2].into_iter().flatten() {
        syngui_layer::cancel_timer(t);
    }
    let (cur, text) = state();
    cur.set(None);
    text.set(false);
    if let Some(id) = SURFACE.with(|s| s.take()) {
        syngui_layer::close_surface(id);
    }
}

/// Слой анимации на весь экран; пустой, пока показывать нечего.
pub fn layer() -> impl Widget {
    let (cur, text) = state();
    crate::ui::rx(move || {
        let Some(s) = cur.get() else { return Box::new(Column::new()) as Box<dyn Widget> };
        let percent = s.percent.min(100);
        let label = Presence::signal(text, move || {
            Box::new(
                Column::new()
                    .gap(2.0)
                    .main_axis_alignment(MainAxisAlignment::Center)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new(format!("{percent}%")).class("charge-anim-pct"))
                    .child(Text::new(t!("Заряжается")).class("charge-anim-label")),
            )
        })
        .enter(Motion::fade().scale(0.85).slide(0.0, 12.0))
        .exit(Motion::fade().scale(1.04))
        .duration_ms(450)
        .initial(true);
        Box::new(
            Stack::new()
                .fit(StackFit::Expand)
                .child(
                    Canvas::new(move |c, t| draw(c, t, percent as f32 / 100.0))
                        .animated_for(DURATION)
                        .replay(s.serial)
                        .style("height", StyleValue::percent(100.0)),
                )
                .child(
                    Column::new()
                        .main_axis_alignment(MainAxisAlignment::Center)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(label)
                        .style("padding-top", StyleValue::px(28.0)),
                ),
        )
    })
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn ease_out(x: f32) -> f32 {
    1.0 - (1.0 - x.clamp(0.0, 1.0)).powi(3)
}

/// Пружинка: 0 → чуть больше 1 → 1.
fn pop(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    1.0 + 2.7 * (x - 1.0).powi(3) + 1.7 * (x - 1.0).powi(2)
}

/// Цвет заряда по уровню: красный → янтарный → зелёный.
fn level_color(p: f32) -> Color {
    let red = Color::from_hex("#ff5a5f");
    let amber = Color::from_hex("#ffc145");
    let green = Color::from_hex("#3ee089");
    if p < 0.2 {
        red.lerp(&amber, p / 0.2)
    } else {
        amber.lerp(&green, ((p - 0.2) / 0.3).min(1.0))
    }
}

fn draw(c: &mut syngui::core::canvas::CanvasContext, t: f32, pct: f32) {
    let (w, h) = (c.width(), c.height());
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let fade = smooth(0.0, 0.35, t) * (1.0 - smooth(DURATION - 0.6, DURATION, t));
    if fade <= 0.0 {
        return;
    }
    let (cx, cy) = (w * 0.5, h * 0.5);
    let r = (w.min(h) * 0.26).clamp(80.0, 190.0);
    let main = level_color(pct);
    let cyan = Color::from_hex("#38d6ff");
    let white = Color::rgb(1.0, 1.0, 1.0);

    // Затемнение.
    c.set_color(Color::rgba(0.0, 0.01, 0.02, 0.88 * fade));
    c.fill_rect(0.0, 0.0, w, h);

    // Вспышка у разъёма: полукольца расходятся от низа экрана.
    for k in 0..3 {
        let p = ((t - k as f32 * 0.14) / 0.9).clamp(0.0, 1.0);
        if p <= 0.0 || p >= 1.0 {
            continue;
        }
        let rad = ease_out(p) * w * 0.75;
        c.set_stroke_width(3.0 + 6.0 * (1.0 - p));
        c.set_color(cyan.lerp(&main, p).with_alpha(0.55 * (1.0 - p) * fade));
        c.draw_arc(cx, h + 4.0, rad.max(1.0), PI, TAU);
    }
    // Свечение у разъёма.
    let glow = (0.35 + 0.15 * (t * 6.0).sin()) * fade * (1.0 - smooth(2.4, 3.0, t));
    for k in 0..6 {
        let rr = 14.0 + k as f32 * 12.0;
        c.set_color(cyan.with_alpha(glow * 0.12));
        c.fill_circle(cx, h, rr);
    }

    // Луч от разъёма к кольцу.
    let beam_top = cy + r + 14.0;
    let beam = smooth(0.1, 0.5, t) * (1.0 - smooth(2.3, 2.9, t)) * fade;
    if beam > 0.0 {
        for (wd, a) in [(14.0, 0.05), (6.0, 0.12), (2.0, 0.5)] {
            c.set_stroke_width(wd);
            c.set_color(cyan.lerp(&main, 0.5).with_alpha(a * beam));
            c.draw_line(cx, h, cx, beam_top);
        }
    }

    // Искры: поднимаются от разъёма, виляя, и втягиваются в кольцо.
    const N: usize = 34;
    let spawn_end = 2.4;
    for i in 0..N {
        let seed = i as f32 * 12.9898;
        let rnd = |k: f32| ((seed + k * 78.233).sin() * 43758.547).fract().abs();
        let speed = 0.75 + rnd(1.0) * 0.6;
        let start = rnd(2.0) * 1.1 + 0.15;
        let life = (t - start) * speed;
        if life < 0.0 {
            continue;
        }
        let p = life.fract();
        let cycle = life.floor();
        // Новые искры не появляются после spawn_end.
        if start + cycle / speed > spawn_end {
            continue;
        }
        let sway = (p * PI * (2.0 + rnd(3.0) * 2.0) + seed).sin() * (20.0 + rnd(4.0) * 40.0) * (1.0 - p);
        let y = h - (h - beam_top) * ease_out(p);
        let x = cx + sway * (1.0 - p * 0.6);
        let size = (2.0 + rnd(5.0) * 3.5) * (1.0 - p * 0.5);
        let a = smooth(0.0, 0.1, p) * (1.0 - smooth(0.85, 1.0, p)) * fade;
        let col = cyan.lerp(&main, p);
        c.set_color(col.with_alpha(0.18 * a));
        c.fill_circle(x, y, size * 3.0);
        c.set_color(col.lerp(&white, 0.4).with_alpha(0.95 * a));
        c.fill_circle(x, y, size);
    }

    // Мягкий ореол за кольцом, «дышит».
    let breath = 0.5 + 0.5 * (t * 3.2).sin();
    for k in 0..8 {
        let rr = r + 6.0 + k as f32 * 9.0;
        c.set_color(main.with_alpha((0.035 + 0.02 * breath) * (1.0 - k as f32 / 8.0) * fade * smooth(0.3, 1.0, t)));
        c.fill_circle(cx, cy, rr);
    }
    c.set_color(Color::rgba(0.02, 0.03, 0.045, fade));
    c.fill_circle(cx, cy, r - 2.0);

    // Дорожка кольца.
    let ring_w = (r * 0.085).clamp(8.0, 16.0);
    c.set_stroke_width(ring_w);
    c.set_color(white.with_alpha(0.05 * fade));
    c.stroke_circle(cx, cy, r);

    // Заполнение: дорастает до заряда, голова кольца светится.
    let grow = ease_out((t - 0.35) / 1.3) * pct;
    if grow > 0.002 {
        let a0 = -FRAC_PI_2;
        let a1 = a0 + grow * TAU;
        for (extra, a) in [(14.0, 0.07), (7.0, 0.14)] {
            c.set_stroke_width(ring_w + extra);
            c.set_color(main.with_alpha(a * fade));
            c.draw_arc(cx, cy, r, a0, a1);
        }
        // Плавный переход цвета вдоль дуги.
        const SEG: usize = 48;
        let n = ((SEG as f32 * grow).ceil() as usize).max(1);
        c.set_stroke_width(ring_w);
        for s in 0..n {
            let f0 = s as f32 / n as f32;
            let f1 = (s + 1) as f32 / n as f32;
            c.set_color(cyan.lerp(&main, f1).with_alpha(fade));
            c.draw_arc(cx, cy, r, a0 + (a1 - a0) * f0, a0 + (a1 - a0) * f1 + 0.01);
        }
        let (hx, hy) = (cx + a1.cos() * r, cy + a1.sin() * r);
        c.set_color(main.with_alpha(0.25 * fade));
        c.fill_circle(hx, hy, ring_w * 1.4);
        c.set_color(main.lerp(&white, 0.6).with_alpha(fade));
        c.fill_circle(hx, hy, ring_w * 0.5);
        c.set_color(cyan.lerp(&main, 0.0).with_alpha(fade));
        c.fill_circle(cx + a0.cos() * r, cy + a0.sin() * r, ring_w * 0.5);
    }

    // Огоньки по орбите.
    for k in 0..3 {
        let ang = t * 1.6 + k as f32 * TAU / 3.0;
        let orb = r + ring_w + 14.0;
        let a = smooth(0.6, 1.2, t) * fade * 0.8;
        c.set_color(main.lerp(&white, 0.3).with_alpha(a));
        c.fill_circle(cx + ang.cos() * orb, cy + ang.sin() * orb, 2.5);
        c.set_color(main.with_alpha(a * 0.2));
        c.fill_circle(cx + ang.cos() * orb, cy + ang.sin() * orb, 7.0);
    }

    // Молния над процентом: выскакивает с пружинкой.
    let s = pop((t - 0.25) / 0.5) * r * 0.55;
    if s > 0.0 {
        let (bx, by) = (cx, cy - r * 0.48);
        let pts: [(f32, f32); 6] = [(0.10, -0.50), (-0.30, 0.06), (-0.02, 0.06), (-0.10, 0.50), (0.30, -0.06), (0.02, -0.06)];
        let poly: Vec<(f32, f32)> = pts.iter().map(|(x, y)| (bx + x * s * 0.7, by + y * s * 0.7)).collect();
        for k in 0..4 {
            c.set_color(main.with_alpha(0.07 * fade));
            c.fill_circle(bx, by, s * (0.22 + k as f32 * 0.08));
        }
        c.set_color(main.lerp(&white, 0.55).with_alpha(fade));
        c.fill_polygon_concave(&poly);
    }
}
