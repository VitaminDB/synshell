//! Поворот встроенного экрана по акселерометру (`[rotation]`, как в Android).
//!
//! Ориентацию даёт iio-sensor-proxy (D-Bus `net.hadess.SensorProxy`,
//! свойство `AccelerometerOrientation` — с гистерезисом и отсечкой «лёжа»),
//! поворачивает композитор действием `rotate`. Автоповорот включён — экран
//! идёт за телефоном; выключен — ориентация зафиксирована, а когда телефон
//! повернули, в углу на несколько секунд появляется кнопка «повернуть».

use std::cell::Cell;
use std::time::Duration;

use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use syngui::GestureDetector;
use syngui_layer::{Anchor, KeyboardInteractivity, Layer, SurfaceId, SurfaceSpec};
use synshell_common::action::{Action, Rotation};

use crate::ctx::ShellCtx;
use crate::ui::icon;

/// Сколько висит кнопка «повернуть».
const SUGGEST_FOR: Duration = Duration::from_secs(6);
/// Кнопка «повернуть»: сторона и отступ от углов.
const BUTTON: u32 = 68;
const BUTTON_MARGIN: i32 = 20;

thread_local! {
    /// Ориентация по датчику (`None` — не определена).
    static SENSOR: Cell<Option<Rotation>> = const { Cell::new(None) };
    static SUGGEST: Cell<Option<(SurfaceId, u64)>> = const { Cell::new(None) };
    /// Таймер задержки: ориентация датчика ещё не продержалась `delay_ms`.
    static PENDING: Cell<Option<u64>> = const { Cell::new(None) };
    /// Ориентация, принятая после задержки (её и исполняет политика).
    static STABLE: Cell<Option<Rotation>> = const { Cell::new(None) };
}

/// Прокси, через который занят акселерометр: отпускать и занимать снова надо с того же
/// соединения D-Bus (захват — по отправителю).
static PROXY: std::sync::Mutex<Option<zbus::blocking::Proxy<'static>>> = std::sync::Mutex::new(None);
/// Экран горит; погашен — акселерометр отпущен (SLPI не будит телефон данными).
static SCREEN_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

fn screen_power(on: bool) {
    SCREEN_ON.store(on, std::sync::atomic::Ordering::SeqCst);
    let Some(p) = PROXY.lock().unwrap().clone() else { return };
    std::thread::spawn(move || {
        let m = if on { "ClaimAccelerometer" } else { "ReleaseAccelerometer" };
        if let Err(e) = p.call_method(m, &()) {
            log::warn!("поворот экрана: {m}: {e}");
        } else if on {
            if let Ok(v) = p.get_property::<String>("AccelerometerOrientation") {
                deliver(&v);
            }
        }
    });
}

/// Слушать датчик в фоне; без iio-sensor-proxy или акселерометра — молча
/// ничего не делать (проверка раз в минуту: служба может появиться позже).
pub fn start(ctx: ShellCtx) {
    // Включили автоповорот или поворот сменился не по датчику (действие
    // `rotate`, перезапуск композитора) — встать по датчику. Кнопку
    // «повернуть» показывает только сам поворот телефона.
    crate::on_reload(|ctx, _| {
        send_threshold(ctx.cfg().rotation.threshold_deg);
        if ctx.cfg().rotation.auto {
            apply_policy(ctx);
        }
    });
    create_effect(move || screen_power(ctx.screen_on.get()));
    create_effect(move || {
        let _ = ctx.comp_outputs.get();
        if ctx.cfg().rotation.auto {
            apply_policy(ctx);
        }
    });
    std::thread::Builder::new()
        .name("rotation".into())
        .spawn(|| {
            let mut logged = false;
            loop {
                if let Err(e) = listen() {
                    if !logged {
                        log::info!("поворот экрана: датчика нет ({e})");
                        logged = true;
                    }
                }
                std::thread::sleep(Duration::from_secs(60));
            }
        })
        .ok();
}

fn listen() -> zbus::Result<()> {
    let conn = zbus::blocking::Connection::system()?;
    let proxy = zbus::blocking::Proxy::new(&conn, "net.hadess.SensorProxy", "/net/hadess/SensorProxy", "net.hadess.SensorProxy")?;
    let has: bool = proxy.get_property("HasAccelerometer")?;
    if !has {
        return Err(zbus::Error::Failure(t!("нет акселерометра").into()));
    }
    if SCREEN_ON.load(std::sync::atomic::Ordering::SeqCst) {
        proxy.call_method("ClaimAccelerometer", &())?;
    }
    *PROXY.lock().unwrap() = Some(proxy.clone());
    log::info!("поворот экрана: акселерометр подключён");
    set_threshold(&proxy, synshell_common::Config::load().0.rotation.threshold_deg);
    // Служба перезапустилась — захват был за прежним владельцем имени: занять заново.
    let (c2, p2) = (conn.clone(), proxy.clone());
    std::thread::Builder::new()
        .name("rotation-owner".into())
        .spawn(move || {
            let Ok(dbus) = zbus::blocking::fdo::DBusProxy::new(&c2) else { return };
            let Ok(changes) = dbus.receive_name_owner_changed_with_args(&[(0, "net.hadess.SensorProxy")]) else { return };
            for c in changes {
                if c.args().is_ok_and(|a| a.new_owner().is_some())
                    && SCREEN_ON.load(std::sync::atomic::Ordering::SeqCst)
                    && p2.call_method("ClaimAccelerometer", &()).is_ok()
                {
                    set_threshold(&p2, synshell_common::Config::load().0.rotation.threshold_deg);
                    log::info!("поворот экрана: служба датчиков перезапущена, акселерометр занят снова");
                    if let Ok(v) = p2.get_property::<String>("AccelerometerOrientation") {
                        deliver(&v);
                    }
                }
            }
        })
        .ok();
    let changes = proxy.receive_property_changed::<String>("AccelerometerOrientation");
    let first: String = proxy.get_property("AccelerometerOrientation")?;
    deliver(&first);
    for c in changes {
        if let Ok(v) = c.get() {
            deliver(&v);
        }
    }
    Err(zbus::Error::Failure(t!("поток свойств закрыт").into()))
}

/// Угол срабатывания датчика — службе (метод есть только в нашей сборке
/// iio-sensor-proxy; нет — остаётся её 35°).
fn set_threshold(proxy: &zbus::blocking::Proxy<'_>, degrees: u32) {
    match proxy.call_method("SetOrientationThreshold", &(degrees.clamp(10, 80),)) {
        Ok(_) => log::info!("поворот экрана: угол срабатывания {degrees}°"),
        Err(e) => log::debug!("поворот экрана: угол срабатывания не задать ({e})"),
    }
}

/// То же из главного потока (смена настройки): вызов D-Bus — в фоне.
fn send_threshold(degrees: u32) {
    thread_local! {
        static LAST: Cell<u32> = const { Cell::new(0) };
    }
    if LAST.with(|l| l.replace(degrees)) == degrees {
        return;
    }
    std::thread::spawn(move || {
        let Ok(conn) = zbus::blocking::Connection::system() else { return };
        let Ok(proxy) =
            zbus::blocking::Proxy::new(&conn, "net.hadess.SensorProxy", "/net/hadess/SensorProxy", "net.hadess.SensorProxy")
        else {
            return;
        };
        set_threshold(&proxy, degrees);
    });
}

fn deliver(orientation: &str) {
    // Как в Mutter: левый край вверху — поворот 90°, правый — 270°.
    let r = match orientation {
        "normal" => Some(Rotation::Normal),
        "left-up" => Some(Rotation::R90),
        "bottom-up" => Some(Rotation::R180),
        "right-up" => Some(Rotation::R270),
        _ => None,
    };
    run_on_main_thread(move || {
        if SENSOR.with(|s| s.replace(r)) == r && PENDING.with(|p| p.get()).is_none() {
            return;
        }
        // Новое положение — ждать `delay_ms`: вернули телефон раньше — таймер
        // перезапускается, и мимолётный наклон ничего не поворачивает.
        if let Some(t) = PENDING.with(|p| p.take()) {
            syngui_layer::cancel_timer(t);
        }
        let ctx = ShellCtx::get();
        let delay = ctx.cfg().rotation.delay_ms;
        if delay == 0 || r.is_none() {
            STABLE.with(|s| s.set(r));
            apply_policy(ctx);
            return;
        }
        let t = syngui_layer::add_timer(Duration::from_millis(delay as u64), move || {
            PENDING.with(|p| p.set(None));
            STABLE.with(|s| s.set(SENSOR.with(|s| s.get())));
            apply_policy(ShellCtx::get());
            None
        });
        PENDING.with(|p| p.set(Some(t)));
    });
}

/// Текущий поворот встроенного экрана (по сведениям композитора) поверх
/// `transform` из конфига.
fn current(ctx: &ShellCtx) -> Rotation {
    let q = |t: &str| match t {
        "90" | "flipped-90" => 1u8,
        "180" | "flipped-180" => 2,
        "270" | "flipped-270" => 3,
        _ => 0,
    };
    let outs = ctx.comp_outputs.get_untracked();
    let Some(o) = outs.iter().find(|o| o.internal) else {
        return Rotation::Normal;
    };
    let cfg = ctx.cfg();
    let base = cfg
        .outputs
        .iter()
        .find(|c| c.name.eq_ignore_ascii_case(&o.name))
        .map(|c| q(&c.transform))
        .unwrap_or(0);
    Rotation::from_quarters(q(&o.transform) + 4 - base)
}

/// Ориентация датчика, которую разрешено принять (без «вверх ногами», если
/// оно не разрешено).
fn wanted(ctx: &ShellCtx) -> Option<Rotation> {
    let r = STABLE.with(|s| s.get())?;
    (r != Rotation::R180 || ctx.cfg().rotation.upside_down).then_some(r)
}

fn apply_policy(ctx: ShellCtx) {
    let Some(r) = wanted(&ctx) else {
        hide_suggestion();
        return;
    };
    let cfg = ctx.cfg();
    if r == current(&ctx) {
        hide_suggestion();
    } else if cfg.rotation.auto {
        hide_suggestion();
        crate::actions::run(Action::Rotate(r));
    } else if cfg.rotation.suggest {
        show_suggestion(ctx, r);
    }
}

/// Включить/выключить автоповорот (плитка шторки) и запомнить в конфиге.
pub fn set_auto(on: bool) {
    if let Err(e) = synshell_common::config_edit::set_value(&["rotation", "auto"], toml_edit::Value::from(on)) {
        log::error!("автоповорот: {e:#}");
    }
    crate::reload_after_write();
}

fn show_suggestion(ctx: ShellCtx, r: Rotation) {
    hide_suggestion();
    let id = syngui_layer::create_surface(
        SurfaceSpec {
            namespace: "syndesktop-rotate".into(),
            layer: Layer::Overlay,
            anchor: Anchor::BOTTOM | Anchor::RIGHT,
            size: (BUTTON, BUTTON),
            margin: [0, BUTTON_MARGIN, BUTTON_MARGIN * 3, 0],
            exclusive_zone: -1,
            keyboard: KeyboardInteractivity::None,
            output: crate::manager::focused_output(&ctx),
            auto_size: false,
            clear_color: [0.0; 4],
        },
        move || Box::new(button(r)),
    );
    let timer = syngui_layer::add_timer(SUGGEST_FOR, move || {
        if SUGGEST.with(|s| s.get()).is_some_and(|(sid, _)| sid == id) {
            SUGGEST.with(|s| s.set(None));
            syngui_layer::close_surface(id);
        }
        None
    });
    SUGGEST.with(|s| s.set(Some((id, timer))));
}

/// Отладка вида: показать кнопку «повернуть» (`shell rotate-suggest-preview`);
/// нажатие поворачивает экран в текущую ориентацию — то есть ничего не меняет.
pub fn preview(ctx: ShellCtx) {
    let r = current(&ctx);
    show_suggestion(ctx, r);
}

fn hide_suggestion() {
    if let Some((id, timer)) = SUGGEST.with(|s| s.take()) {
        syngui_layer::cancel_timer(timer);
        syngui_layer::close_surface(id);
    }
}

fn button(r: Rotation) -> impl Widget {
    GestureDetector::new()
        .on_click(move || {
            hide_suggestion();
            crate::actions::run(Action::Rotate(r));
        })
        // Центр — колонкой и рядом во весь круг (выравнивание MSS у бокса
        // без явного размера оставляло значок со смещением).
        .child(
            DecoratedBox::new()
                .child(crate::ui::vcenter(Row::new().main_axis_alignment(MainAxisAlignment::Center).child(icon("\u{E1C1}").class("rotate-suggest-icon"))))
                .class("rotate-suggest"),
        )
}
