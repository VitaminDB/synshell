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
    /// Видимость карточки: `false` — растворяется, по концу поверхность
    /// закрывается.
    static VISIBLE: Cell<Option<RwSignal<bool>>> = const { Cell::new(None) };
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
            match (SURFACE.with(|s| s.get()), VISIBLE.with(|v| v.get())) {
                (Some(_), Some(visible)) if crate::anim::on(&ctx) => visible.set(false),
                (Some(id), _) => {
                    SURFACE.with(|s| s.set(None));
                    VISIBLE.with(|v| v.set(None));
                    syngui_layer::close_surface(id);
                }
                _ => {}
            }
            return;
        }
        if let Some(visible) = VISIBLE.with(|v| v.get()) {
            // Новое значение во время ухода — вернуть карточку.
            visible.set(true);
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
            let visible = use_signal(true);
            let sid: std::sync::Arc<std::sync::Mutex<Option<SurfaceId>>> = Default::default();
            let sid2 = sid.clone();
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
                move || {
                    let dur = crate::anim::ms(&ctx, 220);
                    Box::new(
                        Presence::signal(visible, move || Box::new(view(ctx)))
                            .enter(Motion::fade().slide(0.0, 14.0).scale(0.96))
                            .exit(Motion::fade().scale(0.97))
                            .duration_ms(dur)
                            .initial(dur > 0)
                            .on_exit_complete(move || {
                                if let Some(id) = *sid2.lock().unwrap() {
                                    // Закрыть только если за это время не показали новое.
                                    if SURFACE.with(|s| s.get()) == Some(id) && ShellCtx::get().osd.get_untracked().is_none() {
                                        SURFACE.with(|s| s.set(None));
                                        VISIBLE.with(|v| v.set(None));
                                        syngui_layer::close_surface(id);
                                    }
                                }
                            }),
                    )
                },
            );
            *sid.lock().unwrap() = Some(id);
            SURFACE.with(|s| s.set(Some(id)));
            VISIBLE.with(|v| v.set(Some(visible)));
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
