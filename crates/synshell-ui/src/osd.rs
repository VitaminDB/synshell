//! Экранная подсказка громкости/яркости: карточка внизу по центру,
//! гаснет через полторы секунды после последнего изменения. Долгое действие
//! ([`busy`], например запуск Android) — карточка с вращающимся кольцом, пока
//! его не закончат ([`done`]).

use std::cell::Cell;
use std::time::Duration;
use syngui::prelude::*;
use syngui::StyleValue;
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

/// Поля поверхности вокруг карточки под её тень (`box-shadow` тем — до
/// `0 20px 50px`); = padding `.osd-surface`.
const PAD: u32 = 48;
const PAD_BOTTOM: u32 = 72;

static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Показать (из любого потока).
pub fn show(ctx: ShellCtx, icon: &'static str, value: Option<u32>, label: String) {
    let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    ctx.osd.set(Some(Osd { icon, value, label, serial, busy: false }));
}

/// Долгое действие: кольцо-индикатор вокруг значка, без тайм-аута (из любого потока).
pub fn busy(ctx: ShellCtx, icon: &'static str, label: String) {
    let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    ctx.osd.set(Some(Osd { icon, value: None, label, serial, busy: true }));
}

/// Долгое действие закончилось — убрать его карточку (обычную подсказку не трогает).
pub fn done(ctx: ShellCtx) {
    if ctx.osd.get_untracked().is_some_and(|o| o.busy) {
        ctx.osd.set(None);
    }
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
        let busy = osd.as_ref().is_some_and(|o| o.busy);
        if !busy {
            let t = syngui_layer::add_timer(Duration::from_millis(1500), move || {
                ctx.osd.set(None);
                None
            });
            TIMER.with(|c| c.set(Some(t)));
        }
        if SURFACE.with(|s| s.get()).is_none() {
            let visible = use_signal(true);
            let sid: std::sync::Arc<std::sync::Mutex<Option<SurfaceId>>> = Default::default();
            let sid2 = sid.clone();
            let id = syngui_layer::create_surface(
                SurfaceSpec {
                    namespace: "syndesktop-osd".into(),
                    layer: Layer::Overlay,
                    anchor: Anchor::BOTTOM,
                    // Поля внутри поверхности — место под тень карточки (иначе
                    // край поверхности обрезает её в тёмный прямоугольник).
                    size: (if busy { 380 } else { 300 } + 2 * PAD, 64 + PAD + PAD_BOTTOM),
                    margin: [0, 0, 120 - PAD_BOTTOM as i32, 0],
                    exclusive_zone: -1,
                    keyboard: KeyboardInteractivity::None,
                    ..Default::default()
                },
                move || {
                    let dur = crate::anim::ms(&ctx, 220);
                    Box::new(boxed(
                        "osd-surface",
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
                    ))
                },
            );
            *sid.lock().unwrap() = Some(id);
            // Поля под тень пропускают указатель к окнам под ними.
            syngui_layer::set_input_region(id, Some(Vec::new()));
            SURFACE.with(|s| s.set(Some(id)));
            VISIBLE.with(|v| v.set(Some(visible)));
        }
    });
}

fn view(ctx: ShellCtx) -> impl Widget {
    boxed("osd", crate::ui::rx(move || {
        let Some(o) = ctx.osd.get() else { return Box::new(Row::new()) as Box<dyn Widget> };
        if o.busy {
            let ring = Stack::new()
                .child(CircularProgress::new().indeterminate().size(36.0).stroke_width(3.0))
                .child(
                    Column::new()
                        .main_axis_alignment(MainAxisAlignment::Center)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(icon(o.icon).class("osd-busy-icon"))
                        .style("width", StyleValue::px(36.0))
                        .style("height", StyleValue::px(36.0)),
                );
            return Box::new(
                Row::new()
                    .gap(14.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(ring)
                    .child(Text::new(o.label).max_lines(2).class("osd-label grow")),
            );
        }
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
