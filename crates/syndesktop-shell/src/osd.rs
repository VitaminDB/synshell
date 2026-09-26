//! Экранная подсказка громкости/яркости: карточка внизу по центру,
//! гаснет через полторы секунды после последнего изменения.

use std::cell::Cell;
use std::time::Duration;
use syngui::prelude::*;
use syngui_layer::{Anchor, KeyboardInteractivity, Layer, SurfaceId, SurfaceSpec};

use crate::ctx::{Osd, ShellCtx};
use crate::ui::{boxed, icon};

thread_local! {
    static SURFACE: Cell<Option<SurfaceId>> = const { Cell::new(None) };
    static TIMER: Cell<Option<u64>> = const { Cell::new(None) };
}

static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Показать (из любого потока).
pub fn show(ctx: ShellCtx, icon: &'static str, value: Option<u32>, label: String) {
    let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    ctx.osd.set(Some(Osd { icon, value, label, serial }));
}

/// Эффект: поверхность живёт, пока есть что показывать.
pub fn install(ctx: ShellCtx) {
    create_effect(move || {
        let osd = ctx.osd.get();
        if osd.is_none() {
            if let Some(id) = SURFACE.with(|s| s.take()) {
                syngui_layer::close_surface(id);
            }
            return;
        }
        if let Some(t) = TIMER.with(|t| t.take()) {
            syngui_layer::cancel_timer(t);
        }
        let t = syngui_layer::add_timer(Duration::from_millis(1500), move || {
            ctx.osd.set(None);
            None
        });
        TIMER.with(|c| c.set(Some(t)));
        if SURFACE.with(|s| s.get()).is_none() {
            let id = syngui_layer::create_surface(
                SurfaceSpec {
                    namespace: "syndesktop-osd".into(),
                    layer: Layer::Overlay,
                    anchor: Anchor::BOTTOM,
                    size: (300, 64),
                    margin: [0, 0, 120, 0],
                    exclusive_zone: -1,
                    keyboard: KeyboardInteractivity::None,
                    ..Default::default()
                },
                move || Box::new(view(ctx)),
            );
            SURFACE.with(|s| s.set(Some(id)));
        }
    });
}

fn view(ctx: ShellCtx) -> impl Widget {
    boxed("osd", crate::ui::rx(move || {
        let Some(o) = ctx.osd.get() else { return Box::new(Row::new()) as Box<dyn Widget> };
        let mut row = Row::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(o.icon).class("osd-icon"));
        if let Some(v) = o.value {
            row = row
                .child(crate::ui::meter(v).class("grow"))
                .child(Text::new(format!("{v}")).class("osd-value"));
        }
        if !o.label.is_empty() {
            row = row.child(Text::new(o.label).class("osd-label"));
        }
        Box::new(row)
    }))
}
