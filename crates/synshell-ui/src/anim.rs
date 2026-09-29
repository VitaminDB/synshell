//! Анимации оболочки: длительности с учётом `[animations]` и готовые
//! движения в стиле Material 3 Expressive — «перетекания», когда
//! всплывающее окно вырастает из панели, а содержимое переливается между
//! состояниями, вместо мгновенных подмен.

use synshell_common::config::Edge;
use syngui::animation::Easing;
use syngui::prelude::*;
use syngui::widgets::{AnimationAxis, Motion, Presence, TransformOrigin};

use crate::ctx::ShellCtx;

/// Длительность анимации оболочки в мс; 0 — анимации выключены.
pub fn ms(ctx: &ShellCtx, base: u32) -> u32 {
    ctx.cfg().animations.shell_ms(base)
}

/// Включены ли анимации оболочки.
/// Длительность анимации группы (`[animations] home|menu|shade|dock|pages`).
pub fn group_ms(ctx: &ShellCtx, group: &str, base: u32) -> u32 {
    ctx.cfg().animations.group_ms(group, base)
}

pub fn on(ctx: &ShellCtx) -> bool {
    ms(ctx, 100) > 0
}

/// Всплывающее окно у края панели: вырастает из края (высота или ширина
/// раскрывается от панели), слегка выезжая и проявляясь; уход — обратно.
/// `attached` — карточка примыкает к панели (перетекает в неё), иначе
/// появляется с лёгким увеличением, как обычная карточка.
pub fn popup_presence(
    ctx: &ShellCtx,
    open: RwSignal<bool>,
    edge: Option<Edge>,
    attached: bool,
    builder: impl Fn() -> Box<dyn Widget> + Send + Sync + 'static,
) -> Presence {
    let dur = ms(ctx, 260);
    let (enter, exit, origin, collapse) = match edge {
        Some(Edge::Top) => (Motion::fade().slide(0.0, -10.0), Motion::fade().slide(0.0, -6.0), TransformOrigin::Custom(0.5, 0.0), AnimationAxis::Height),
        Some(Edge::Bottom) => (Motion::fade().slide(0.0, 10.0), Motion::fade().slide(0.0, 6.0), TransformOrigin::Custom(0.5, 1.0), AnimationAxis::Height),
        Some(Edge::Left) => (Motion::fade().slide(-10.0, 0.0), Motion::fade().slide(-6.0, 0.0), TransformOrigin::Custom(0.0, 0.5), AnimationAxis::Width),
        Some(Edge::Right) => (Motion::fade().slide(10.0, 0.0), Motion::fade().slide(6.0, 0.0), TransformOrigin::Custom(1.0, 0.5), AnimationAxis::Width),
        None => (Motion::fade().scale(0.94), Motion::fade().scale(0.97), TransformOrigin::Center, AnimationAxis::Both),
    };
    let mut p = Presence::signal(open, builder)
        .enter(if attached { Motion::fade() } else { enter })
        .exit(if attached { Motion::fade() } else { exit })
        .duration_ms(dur)
        .exit_duration_ms(dur * 3 / 5)
        .easing(Easing::EMPHASIZED_DECELERATE)
        .exit_easing(Easing::EMPHASIZED_ACCELERATE)
        .origin(origin)
        .initial(dur > 0);
    if attached && edge.is_some() {
        p = p.collapse(collapse);
    }
    p
}
