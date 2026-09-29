//! Строка состояния: часы слева, батарея справа. Layer top с exclusive zone —
//! окна не заходят под неё.

use std::time::Duration;
use syngui::prelude::*;
use syngui_layer::{Anchor, KeyboardInteractivity, Layer, SurfaceSpec};

use crate::Ctx;

pub const HEIGHT: u32 = 36;

pub fn install(ctx: Ctx) {
    tick(ctx);
    syngui_layer::add_timer(Duration::from_secs(10), move || {
        tick(ctx);
        Some(Duration::from_secs(10))
    });
    syngui_layer::create_surface(
        SurfaceSpec {
            namespace: "synmobile-statusbar".into(),
            layer: Layer::Top,
            anchor: Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
            size: (0, HEIGHT),
            exclusive_zone: HEIGHT as i32,
            keyboard: KeyboardInteractivity::None,
            ..Default::default()
        },
        move || Box::new(view(ctx)),
    );
}

fn tick(ctx: Ctx) {
    ctx.clock.set(now_hm());
    ctx.battery.set(read_battery());
}

fn view(ctx: Ctx) -> impl Widget {
    use syngui::widget::WidgetExt;
    DecoratedBox::new()
        .child(
            Row::new()
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Reactive::new(move || vec![Box::new(Text::new(ctx.clock.get()).class("statusbar-text")) as Box<dyn Widget>]))
                .child(DecoratedBox::new().class("grow"))
                .child(Reactive::new(move || {
                    let s = match ctx.battery.get() {
                        Some((p, true)) => format!("⚡ {p}%"),
                        Some((p, false)) => format!("{p}%"),
                        None => String::new(),
                    };
                    vec![Box::new(Text::new(s).class("statusbar-text")) as Box<dyn Widget>]
                })),
        )
        .class("statusbar")
}

fn now_hm() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let local = secs as i64 + local_offset();
    let day = local.rem_euclid(86_400);
    format!("{:02}:{:02}", day / 3600, (day % 3600) / 60)
}

/// Смещение локального времени в секундах (через libc localtime_r).
fn local_offset() -> i64 {
    unsafe {
        let t: libc::time_t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm.tm_gmtoff as i64
    }
}

/// Первая батарея в /sys/class/power_supply.
fn read_battery() -> Option<(u32, bool)> {
    let dir = std::fs::read_dir("/sys/class/power_supply").ok()?;
    for e in dir.flatten() {
        let p = e.path();
        let ty = std::fs::read_to_string(p.join("type")).unwrap_or_default();
        if ty.trim() != "Battery" {
            continue;
        }
        let cap: u32 = std::fs::read_to_string(p.join("capacity")).ok()?.trim().parse().ok()?;
        let status = std::fs::read_to_string(p.join("status")).unwrap_or_default();
        return Some((cap, status.trim() == "Charging"));
    }
    None
}
