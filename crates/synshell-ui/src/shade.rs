//! Шторка (телефон): выезжает сверху по жесту от верхнего края или свайпу
//! вниз по рабочему столу (`shell shade`). Сверху — время, дата, параметры и
//! питание; плитки быстрых настроек; яркость (проценты и ниты, если в
//! `[[output]] max_nits` задан максимум панели) и громкость; ниже — центр
//! уведомлений. Закрывается свайпом вверх, тапом по затемнению, жестом
//! «назад».

use std::cell::Cell;
use std::sync::{Arc, Mutex};
use synshell_common::action::MobileMode;
use synshell_common::Action;
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::widgets::{Motion, PanAxis, Presence, SwipeDirection, TransformOrigin};
use syngui::GestureDetector;
use syngui_layer::{Anchor, KeyboardInteractivity, Layer, SurfaceId, SurfaceSpec};

use crate::ctx::ShellCtx;
use crate::ui::{icon, mi, rx};

thread_local! {
    static SURFACE: Cell<Option<SurfaceId>> = const { Cell::new(None) };
    static OPEN: Cell<Option<RwSignal<bool>>> = const { Cell::new(None) };
}

pub fn is_open() -> bool {
    OPEN.with(|o| o.get()).is_some_and(|s| s.get_untracked())
}

pub fn toggle() {
    if is_open() {
        close();
    } else {
        open();
    }
}

pub fn close() {
    let ctx = ShellCtx::get();
    match OPEN.with(|o| o.get()) {
        // Уход с анимацией; поверхность закроется по её концу.
        Some(s) if crate::anim::group_ms(&ctx, "shade", 100) > 0 => s.set(false),
        _ => close_now(),
    }
}

fn close_now() {
    if let Some(id) = SURFACE.with(|s| s.take()) {
        syngui_layer::close_surface(id);
    }
    OPEN.with(|o| o.set(None));
}

pub fn open() {
    if SURFACE.with(|s| s.get()).is_some() {
        return;
    }
    let ctx = ShellCtx::get();
    ctx.close_popup();
    // Экранная клавиатура поверх ленты/шторки не нужна.
    crate::actions::spawn("synkeyboard hide");
    let open = use_signal(true);
    OPEN.with(|o| o.set(Some(open)));
    let spec = SurfaceSpec {
        namespace: "syndesktop-shade".into(),
        layer: Layer::Overlay,
        anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
        size: (0, 0),
        margin: [0; 4],
        exclusive_zone: -1,
        keyboard: KeyboardInteractivity::OnDemand,
        output: crate::manager::focused_output(&ctx),
        auto_size: false,
        clear_color: [0.0; 4],
    };
    let id = syngui_layer::create_surface(spec, move || Box::new(view(ShellCtx::get(), open)));
    SURFACE.with(|s| s.set(Some(id)));
}

fn view(ctx: ShellCtx, open: RwSignal<bool>) -> impl Widget {
    let dur = crate::anim::group_ms(&ctx, "shade", 320);
    let scrim_dur = dur;
    let scrim = Presence::signal(open, || {
        Box::new(
            GestureDetector::new()
                .on_click(close)
                .child(DecoratedBox::new().class("shade-scrim")),
        )
    })
    .enter(Motion::fade())
    .exit(Motion::fade())
    .duration_ms(scrim_dur)
    .initial(scrim_dur > 0);
    let panel = Presence::signal(open, move || {
        Box::new(
            GestureDetector::new()
                .pan_axis(PanAxis::Vertical)
                .on_swipe(|dir, _| {
                    if dir == SwipeDirection::Up {
                        close();
                    }
                })
                .child(content(ShellCtx::get())),
        )
    })
    .enter(Motion::fade().slide(0.0, -80.0))
    .exit(Motion::fade().slide(0.0, -60.0))
    .origin(TransformOrigin::Custom(0.5, 0.0))
    .easing(syngui::animation::Easing::EMPHASIZED_DECELERATE)
    .exit_easing(syngui::animation::Easing::EMPHASIZED_ACCELERATE)
    .duration_ms(dur)
    .exit_duration_ms(dur * 2 / 3)
    .initial(dur > 0)
    .on_exit_complete(close_now);
    let _ = ctx;
    Stack::new()
        .fit(StackFit::Expand)
        .child(scrim)
        .child(Column::new().main_axis_alignment(MainAxisAlignment::Start).child(panel))
}

fn content(ctx: ShellCtx) -> impl Widget {
    Column::new()
        .gap(14.0)
        .child(header(ctx))
        .child(tiles(ctx))
        .child(sliders(ctx))
        .child(DecoratedBox::new().child(crate::notifications::center(ctx)).class("shade-notifications"))
        .child(Row::new().main_axis_alignment(MainAxisAlignment::Center).child(DecoratedBox::new().class("shade-handle")))
        .class("shade")
}

fn header(ctx: ShellCtx) -> impl Widget {
    let time = rx(move || {
        let now = ctx.now.get();
        Box::new(
            Column::new()
                .gap(0.0)
                .child(Text::new(crate::clock::format(now, "%H:%M")).class("shade-time"))
                .child(Text::new(crate::clock::format(now, "%A, %d %B")).class("shade-date")),
        )
    });
    let btn = |glyph: &'static str, f: fn()| {
        GestureDetector::new().on_click(f).child(DecoratedBox::new().child(icon(glyph).class("shade-head-icon")).class("shade-head-btn"))
    };
    Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().child(time).class("grow"))
        .child(btn(mi::SETTINGS, || {
            close();
            crate::actions::spawn("synsettings");
        }))
        .child(btn(mi::POWER, || {
            close();
            crate::commands::handle("power-menu");
        }))
}

/// Плитка быстрой настройки: значок, подпись, состояние; `on` — включена.
fn tile(glyph: &str, label: String, state: String, on: bool, f: impl Fn() + Send + Sync + 'static) -> impl Widget {
    let class = if on { "shade-tile shade-tile-on" } else { "shade-tile" };
    GestureDetector::new().on_click(f).child(
        DecoratedBox::new()
            .child(
                Row::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(DecoratedBox::new().child(icon(glyph).class("shade-tile-icon")).class("shade-tile-badge"))
                    .child(
                        Column::new()
                            .gap(0.0)
                            .child(Text::new(label).max_lines(1).class("shade-tile-label"))
                            .child(Text::new(state).max_lines(1).class("shade-tile-state"))
                            .class("grow"),
                    ),
            )
            .class(class),
    )
}

fn mode_label(m: MobileMode) -> &'static str {
    match m {
        MobileMode::Pages => "Страницы",
        MobileMode::Free => "Свободный стол",
    }
}

fn tiles(ctx: ShellCtx) -> impl Widget {
    // Состояния вне сигналов (фонарик) — пересобрать плитки по счётчику.
    let rev = use_signal(0u32);
    rx(move || {
        let _ = rev.get();
        let net = ctx.network.get();
        let dnd = ctx.dnd.get();
        let cfg = ctx.config.get();
        let _ = &cfg;
        let mode = ctx.mobile_mode();
        let wifi_state = if net.online { net.connection.clone() } else { "Нет подключения".into() };
        let torch = torch_state();
        let mut grid = Grid::new(2).gap(8.0);
        grid = grid
            .child(tile(crate::applets::network_glyph(&net), "Сеть".into(), wifi_state, net.online, || {
                // Окно сети: Wi-Fi, подключение, «Параметры сети…».
                close();
                ShellCtx::get().open_popup(crate::ctx::PopupKind::Network, crate::commands::centered());
            }))
            .child(tile("\u{E1A7}", "Bluetooth".into(), "Параметры".into(), false, || {
                close();
                crate::actions::spawn("synsettings bluetooth");
            }))
            .child(tile(if dnd { mi::BELL_OFF } else { mi::BELL }, "Не беспокоить".into(), if dnd { "Включено" } else { "Выключено" }.into(), dnd, || {
                let c = ShellCtx::get();
                c.dnd.set(!c.dnd.get_untracked());
            }))
            .child(tile(mi::WINDOW, "Режим окон".into(), mode_label(mode).into(), true, move || {
                crate::actions::run(Action::MobileModeCycle);
            }));
        if let Some(on) = torch {
            grid = grid.child(tile("\u{E3E7}", "Фонарик".into(), if on { "Включён" } else { "Выключен" }.into(), on, move || {
                set_torch(!torch_state().unwrap_or(false));
                rev.set(rev.get_untracked() + 1);
            }));
        }
        grid = grid
            .child(tile(mi::KEYBOARD, "Клавиатура".into(), "Показать".into(), false, || {
                close();
                crate::actions::spawn("synkeyboard toggle");
            }))
            .child(tile("\u{E3B0}", "Снимок".into(), "Экрана".into(), false, || {
                close();
                // Шторка должна успеть уйти с экрана.
                syngui_layer::add_timer(std::time::Duration::from_millis(450), || {
                    crate::actions::run(Action::Screenshot);
                    None
                });
            }))
            .child(tile(mi::LOCK, "Блокировка".into(), "Экрана".into(), false, || {
                close();
                crate::commands::handle("lock");
            }));
        Box::new(grid)
    })
}

// ─── Яркость и громкость ─────────────────────────────────────────────────────

/// Максимальная яркость панели, нит (`[[output]] max_nits` вывода с
/// фокусом или первого, где задано).
fn max_nits(ctx: &ShellCtx) -> Option<f32> {
    let cfg = ctx.cfg();
    let out = crate::manager::focused_output(ctx);
    cfg.outputs
        .iter()
        .find(|o| out.as_deref() == Some(o.name.as_str()) && o.max_nits.is_some())
        .or_else(|| cfg.outputs.iter().find(|o| o.max_nits.is_some()))
        .and_then(|o| o.max_nits)
}

fn sliders(ctx: ShellCtx) -> impl Widget {
    let sys = synsystem::Sys::host();
    let bl = synsystem::backlight::primary(&sys);
    let nits = max_nits(&ctx);
    let bright = use_signal(bl.as_ref().map(|b| b.percent()).unwrap_or(50.0));
    // Запись в sysfs — в фоне, последнее значение побеждает.
    let pending: Arc<Mutex<Option<f32>>> = Arc::new(Mutex::new(None));
    let mut col = Column::new().gap(12.0);
    if bl.is_some() {
        let p2 = pending.clone();
        let label = rx(move || {
            let pct = bright.get();
            let text = match nits {
                Some(max) => format!("{:.0} нит · {pct:.0}%", pct / 100.0 * max),
                None => format!("{pct:.0}%"),
            };
            Box::new(Text::new(text).class("shade-slider-value"))
        });
        col = col.child(
            DecoratedBox::new()
                .child(
                    Row::new()
                        .gap(10.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(icon(mi::BRIGHTNESS).class("shade-slider-icon"))
                        .child(
                            Slider::new()
                                .range(1.0, 100.0)
                                .step(1.0)
                                .value(bright.get_untracked())
                                .on_change(move |v| {
                                    bright.set(v);
                                    let first = p2.lock().unwrap().replace(v).is_none();
                                    if first {
                                        let p3 = p2.clone();
                                        std::thread::spawn(move || {
                                            let sys = synsystem::Sys::host();
                                            while let Some(v) = p3.lock().unwrap().take() {
                                                if let Err(e) = synsystem::backlight::set_percent(&sys, v) {
                                                    log::warn!("яркость: {e}");
                                                }
                                            }
                                        });
                                    }
                                })
                                .class("grow"),
                        )
                        .child(label),
                )
                .class("shade-slider"),
        );
    }
    let vol = rx(move || {
        let v = ctx.volume.get();
        let Some(v) = v else { return Box::new(DecoratedBox::new()) as Box<dyn Widget> };
        Box::new(
            DecoratedBox::new()
                .child(
                    Row::new()
                        .gap(10.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(
                            GestureDetector::new()
                                .on_click(|| crate::system::toggle_mute(ShellCtx::get(), false))
                                .child(icon(if v.muted { mi::VOLUME_OFF } else { mi::VOLUME_UP }).class("shade-slider-icon")),
                        )
                        .child(
                            Slider::new()
                                .range(0.0, 150.0)
                                .step(1.0)
                                .value(v.percent as f32)
                                .on_change(|x| crate::system::change_volume(ShellCtx::get(), 0, Some(x.round() as u32)))
                                .class("grow"),
                        )
                        .child(Text::new(format!("{}%", v.percent)).class("shade-slider-value")),
                )
                .class("shade-slider"),
        )
    });
    col.child(vol).style("padding", StyleValue::px(0.0))
}

// ─── Фонарик ─────────────────────────────────────────────────────────────────

/// Светодиод вспышки (`/sys/class/leds/*flash*` или `*torch*`).
fn torch_led() -> Option<std::path::PathBuf> {
    std::fs::read_dir("/sys/class/leds")
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.contains("torch") || n.contains("flash")))
}

fn torch_state() -> Option<bool> {
    let led = torch_led()?;
    let v: u32 = std::fs::read_to_string(led.join("brightness")).ok()?.trim().parse().ok()?;
    Some(v > 0)
}

fn set_torch(on: bool) {
    let Some(led) = torch_led() else { return };
    let max = std::fs::read_to_string(led.join("max_brightness")).ok().and_then(|s| s.trim().parse::<u32>().ok()).unwrap_or(1);
    let v = if on { max.min(100).max(1) } else { 0 };
    if let Err(e) = std::fs::write(led.join("brightness"), v.to_string()) {
        log::warn!("фонарик: {e}");
    }
}
