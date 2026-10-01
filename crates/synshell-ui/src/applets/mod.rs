//! Апплеты панели. Каждый — функция `(настройки, панель) → виджет`.
//! Внешний вид — классы `.applet`, `.applet-<тип>` в MSS.

pub mod taskbar;
pub mod window;
mod workspaces;

use std::time::Duration;
use synshell_common::config::Applet;
use synshell_common::ipc::WindowOp;
use syngui::input::MouseButton;
use syngui::prelude::*;

use crate::ctx::{PopupKind, ShellCtx};
use crate::panel::PanelCtx;
use crate::ui::{icon, mi, InputArea};

/// Виджет апплета `a` — номер `index` на панели `pc`.
pub fn build(a: &Applet, pc: &PanelCtx, index: usize) -> Box<dyn Widget> {
    match a.kind.as_str() {
        "app" => crate::launchers::app_applet(a, pc, index),
        "group" | "folder" => crate::launchers::stack_applet(a, pc, index),
        "launcher" => launcher(a, pc),
        "taskbar" => taskbar::build(a, pc),
        "workspaces" | "pager" => workspaces::build(a, pc),
        "spacer" => Box::new(DecoratedBox::new().class("applet-spacer")),
        "separator" => Box::new(DecoratedBox::new().class(if pc.vertical { "applet-separator-h" } else { "applet-separator" })),
        "clock" => clock(a, pc),
        "keyboard" => keyboard(a, pc),
        "volume" => volume(a, pc),
        "battery" => battery(a, pc),
        "network" => network(a, pc),
        "link" => crate::link::applet(pc),
        "notifications" => notifications(a, pc),
        "power" => simple_popup(pc, a.str_or("icon", mi::POWER), "power", PopupKind::Power),
        "button" => button(a),
        "command" => command(a),
        "cpu" => cpu(pc),
        "memory" => memory(pc),
        "layout" => layout(pc),
        "show-desktop" => show_desktop(),
        "tray" => crate::tray::applet(a, pc),
        "window-title" => window::title(a, pc),
        "window-buttons" => window::buttons(a, pc),
        "appmenu" => window::appmenu(a, pc),
        other => {
            log::warn!("неизвестный апплет «{other}»");
            Box::new(Text::new(format!("?{other}")).class("applet-label"))
        }
    }
}

/// Кнопка апплета: область ввода + коробка `.applet .applet-<kind>`.
pub fn applet_button<M>(kind: &str, content: impl syngui::widgets::IntoWidget<M>) -> InputArea {
    InputArea::new(DecoratedBox::new().child(crate::ui::vcenter(content)).class(format!("applet applet-{kind}")))
        .pointer()
        .buttons(&[MouseButton::Left, MouseButton::Middle])
}

fn simple_popup(pc: &PanelCtx, glyph: &str, kind: &str, popup: PopupKind) -> Box<dyn Widget> {
    let pc = pc.clone();
    Box::new(applet_button(kind, icon(glyph)).on_click(move |b, _, r| {
        if b == MouseButton::Left {
            ShellCtx::get().open_popup(popup.clone(), pc.anchor(r));
        }
    }))
}

fn launcher(a: &Applet, pc: &PanelCtx) -> Box<dyn Widget> {
    let pc = pc.clone();
    let label = a.str("label").map(String::from);
    let glyph = a.str_or("icon", mi::APPS).to_string();
    let content: Box<dyn Widget> = match label {
        Some(l) => Box::new(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(&glyph)).child(Text::new(l).class("applet-label"))),
        None => Box::new(icon(&glyph)),
    };
    Box::new(applet_button("launcher", content).on_click(move |b, _, r| {
        if b == MouseButton::Left {
            ShellCtx::get().open_popup(PopupKind::Launcher, pc.anchor(r));
        }
    }))
}

fn clock(a: &Applet, pc: &PanelCtx) -> Box<dyn Widget> {
    let ctx = ShellCtx::get();
    let seconds = a.bool_or("seconds", false);
    let fmt = a.str("format").map(String::from).unwrap_or_else(|| if seconds { "%H:%M:%S" } else { "%H:%M" }.into());
    let date_fmt = a.str("date_format").map(String::from).unwrap_or_else(|| if pc.vertical { "%d.%m".into() } else { "%a, %d %b".into() });
    // На тонкой панели (строка состояния телефона) дата под часами не
    // помещается — по умолчанию только время.
    let show_date = a.bool_or("date", pc.vertical || pc.size >= 40);
    let pc2 = pc.clone();
    let vertical = pc.vertical;
    Box::new(
        applet_button(
            "clock",
            Row::new().child(crate::ui::rx(move || {
                let now = ctx.now.get();
                // Без секунд — перерисовка раз в минуту.
                let t = if fmt.contains("%S") || fmt.contains("%T") { now } else { now - now % 60 };
                let mut col = Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(crate::clock::format(t, &fmt)).class("clock-time"));
                if show_date {
                    col = col.child(Text::new(crate::clock::format(t, &date_fmt)).class(if vertical { "clock-date clock-date-v" } else { "clock-date" }));
                }
                Box::new(col)
            })),
        )
        .on_click(move |b, _, r| {
            if b == MouseButton::Left {
                ShellCtx::get().open_popup(PopupKind::Calendar, pc2.anchor(r));
            }
        }),
    )
}

fn keyboard(a: &Applet, _pc: &PanelCtx) -> Box<dyn Widget> {
    let ctx = ShellCtx::get();
    let show_icon = a.bool_or("icon", false);
    Box::new(
        applet_button(
            "keyboard",
            Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).child(move || {
                let kb = ctx.keyboard.get();
                let short = kb
                    .short
                    .get(kb.current as usize)
                    .cloned()
                    .or_else(|| kb.names.get(kb.current as usize).map(|n| n.chars().take(2).collect()))
                    .unwrap_or_else(|| {
                        // Без композитора — первая раскладка из конфига.
                        ctx.config.get().input.keyboard.layouts.split(',').next().unwrap_or("").trim().to_string()
                    });
                let mut row = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center);
                if show_icon {
                    row = row.child(icon(mi::KEYBOARD));
                }
                row.child(Text::new(short.to_uppercase()).class("applet-label keyboard-label"))
            }),
        )
        .on_click(|b, _, _| {
            if b == MouseButton::Left {
                crate::actions::run(synshell_common::Action::KeyboardLayoutNext);
            }
        })
        .on_wheel(|_| crate::actions::run(synshell_common::Action::KeyboardLayoutNext)),
    )
}

pub fn volume_glyph(v: &crate::system::Volume) -> &'static str {
    if v.muted || v.percent == 0 {
        mi::VOLUME_OFF
    } else if v.percent < 50 {
        mi::VOLUME_DOWN
    } else {
        mi::VOLUME_UP
    }
}

fn volume(a: &Applet, pc: &PanelCtx) -> Box<dyn Widget> {
    let ctx = ShellCtx::get();
    let show_pct = a.bool_or("percent", false);
    let step = a.int_or("step", 5) as i32;
    let pc = pc.clone();
    Box::new(
        applet_button(
            "volume",
            Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).child(move || {
                let v = ctx.volume.get();
                let mut row = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center);
                match v {
                    Some(v) => {
                        row = row.child(icon(volume_glyph(&v)).class(if v.muted { "muted" } else { "" }));
                        if show_pct {
                            row = row.child(Text::new(format!("{}%", v.percent)).class("applet-label"));
                        }
                    }
                    None => row = row.child(icon(mi::VOLUME_OFF).class("muted")),
                }
                row
            }),
        )
        .on_click(move |b, _, r| match b {
            MouseButton::Left => ShellCtx::get().open_popup(PopupKind::Volume, pc.anchor(r)),
            MouseButton::Middle => crate::system::toggle_mute(ShellCtx::get(), false),
            _ => {}
        })
        .on_wheel(move |dy| crate::system::change_volume(ShellCtx::get(), if dy > 0.0 { step } else { -step }, None)),
    )
}

pub fn battery_glyph(b: &crate::system::Battery) -> &'static str {
    if b.charging {
        mi::BATTERY_CHARGING
    } else if b.percent <= 15 {
        mi::BATTERY_ALERT
    } else if b.full || b.percent >= 95 {
        mi::BATTERY_FULL
    } else {
        mi::BATTERY_STD
    }
}

fn battery(a: &Applet, pc: &PanelCtx) -> Box<dyn Widget> {
    let ctx = ShellCtx::get();
    let show_pct = a.bool_or("percent", true);
    let pc = pc.clone();
    Box::new(crate::ui::rx(move || {
        let Some(b) = ctx.battery.get() else {
            // Батареи нет (настольный ПК) — апплет не занимает места.
            return Box::new(DecoratedBox::new());
        };
        let mut row = Row::new().gap(3.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(battery_glyph(&b)));
        if show_pct {
            row = row.child(Text::new(format!("{}%", b.percent)).class("applet-label"));
        }
        let low = b.percent <= 15 && !b.charging;
        let pc = pc.clone();
        Box::new(
            InputArea::new(DecoratedBox::new().child(row).class(if low { "applet applet-battery battery-low" } else { "applet applet-battery" }))
                .pointer()
                .buttons(&[MouseButton::Left])
                .on_click(move |btn, _, r| {
                    if btn == MouseButton::Left {
                        ShellCtx::get().open_popup(PopupKind::Battery, pc.anchor(r));
                    }
                }),
        )
    }))
}

pub fn network_glyph(n: &crate::system::Network) -> &'static str {
    match n.kind.as_str() {
        "wifi" => mi::WIFI,
        "ethernet" => mi::ETHERNET,
        _ => mi::NET_OFF,
    }
}

fn network(_a: &Applet, pc: &PanelCtx) -> Box<dyn Widget> {
    let ctx = ShellCtx::get();
    let pc = pc.clone();
    Box::new(
        applet_button("network", Row::new().child(move || {
            let n = ctx.network.get();
            let mut row = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center);
            // Приёмник GNSS работает — кто-то читает местоположение
            if ctx.modem.get().is_some_and(|m| m.gnss) {
                row = row.child(icon(crate::location::GLYPH));
            }
            row.child(icon(network_glyph(&n)).class(if n.online { "" } else { "muted" }))
        }))
        .on_click(move |b, _, r| {
            if b == MouseButton::Left {
                ShellCtx::get().open_popup(PopupKind::Network, pc.anchor(r));
            }
        }),
    )
}

fn notifications(_a: &Applet, pc: &PanelCtx) -> Box<dyn Widget> {
    let ctx = ShellCtx::get();
    let pc = pc.clone();
    Box::new(
        applet_button("notifications", Row::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Center).child(move || {
            let n = ctx.history.get().len();
            let dnd = ctx.dnd.get();
            let mut row = Row::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(if dnd { mi::BELL_OFF } else { mi::BELL }));
            if n > 0 {
                row = row.child(DecoratedBox::new().child(Text::new(n.min(99).to_string()).class("badge-text")).class("badge"));
            }
            row
        }))
        .on_click(move |b, _, r| match b {
            MouseButton::Left => ShellCtx::get().open_popup(PopupKind::Notifications, pc.anchor(r)),
            MouseButton::Middle => {
                let c = ShellCtx::get();
                c.dnd.set(!c.dnd.get_untracked());
            }
            _ => {}
        }),
    )
}

fn button(a: &Applet) -> Box<dyn Widget> {
    let glyph = a.str("icon").map(String::from);
    let label = a.str("label").map(String::from);
    let action = a.str("action").map(String::from).unwrap_or_default();
    let middle = a.str("middle_action").map(String::from);
    let mut row = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
    if let Some(g) = &glyph {
        // Имя значка темы (`firefox`) или глиф Material.
        if g.chars().count() == 1 {
            row = row.child(icon(g));
        } else if let Some(p) = crate::xdg::lookup_icon(g) {
            row = row.child(Image::new(p.to_string_lossy()).fit(ImageFit::Contain).placeholder(false).class("applet-image"));
        }
    }
    if let Some(l) = label {
        row = row.child(Text::new(l).class("applet-label"));
    }
    Box::new(applet_button("button", row).on_click(move |b, _, _| match b {
        MouseButton::Left => crate::actions::run_str(&action),
        MouseButton::Middle => {
            if let Some(m) = &middle {
                crate::actions::run_str(m);
            }
        }
        _ => {}
    }))
}

fn command(a: &Applet) -> Box<dyn Widget> {
    let cmd = a.str("command").unwrap_or("").to_string();
    let interval = a.int_or("interval", 5).max(1) as u64;
    let action = a.str("action").map(String::from);
    let out = use_signal(String::new());
    // Опрос в фоне, чтобы долгая команда не держала интерфейс.
    let c = cmd.clone();
    syngui_layer::add_timer(Duration::from_millis(10), move || {
        let c = c.clone();
        std::thread::spawn(move || {
            let s = crate::actions::output("sh", &["-c", &c]).unwrap_or_default();
            out.set(s.lines().next().unwrap_or("").trim().to_string());
        });
        Some(Duration::from_secs(interval))
    });
    Box::new(
        applet_button("command", Row::new().child(move || Text::new(out.get()).class("applet-label"))).on_click(move |b, _, _| {
            if b == MouseButton::Left {
                if let Some(a) = &action {
                    crate::actions::run_str(a);
                }
            }
        }),
    )
}

fn cpu(pc: &PanelCtx) -> Box<dyn Widget> {
    let ctx = ShellCtx::get();
    let v = pc.vertical;
    Box::new(applet_button(
        "cpu",
        Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(mi::CPU)).child(move || {
            let c = ctx.cpu.get();
            Text::new(if v { format!("{c:.0}") } else { format!("{c:.0}%") }).class("applet-label applet-mono")
        }),
    ))
}

fn memory(pc: &PanelCtx) -> Box<dyn Widget> {
    let ctx = ShellCtx::get();
    let v = pc.vertical;
    Box::new(applet_button(
        "memory",
        Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(mi::MEMORY)).child(move || {
            let m = ctx.memory.get();
            Text::new(if v { format!("{m:.0}") } else { format!("{m:.0}%") }).class("applet-label applet-mono")
        }),
    ))
}

fn layout(_pc: &PanelCtx) -> Box<dyn Widget> {
    let ctx = ShellCtx::get();
    Box::new(
        applet_button(
            "layout",
            Row::new().child(move || {
                let l = ctx.workspaces.get().iter().find(|w| w.active).map(|w| w.layout).unwrap_or_default();
                let g = match l {
                    synshell_common::action::LayoutKind::Floating => mi::FLOAT,
                    synshell_common::action::LayoutKind::Monocle => mi::FULLSCREEN,
                    _ => mi::TILE,
                };
                icon(g)
            }),
        )
        .buttons(&[MouseButton::Left, MouseButton::Right])
        .on_click(|b, _, _| match b {
            MouseButton::Left => crate::actions::run(synshell_common::Action::CycleLayout),
            MouseButton::Right => crate::actions::run(synshell_common::Action::Layout(Default::default())),
            _ => {}
        }),
    )
}

/// «Показать рабочий стол»: свернуть окна текущего стола, повторно — вернуть.
fn show_desktop() -> Box<dyn Widget> {
    Box::new(applet_button("show-desktop", icon(mi::DESKTOP)).on_click(|b, _, _| {
        if b != MouseButton::Left {
            return;
        }
        toggle_show_desktop(&ShellCtx::get());
    }))
}

/// Свернуть окна текущего стола (показать рабочий стол); повторно —
/// вернуть их. `true` — окна свёрнуты.
pub fn toggle_show_desktop(ctx: &ShellCtx) -> bool {
    let shown = ctx.shown_desktop.get_untracked();
    if !shown.is_empty() {
        for id in &shown {
            crate::actions::window_op(*id, WindowOp::Activate);
        }
        ctx.shown_desktop.set(Vec::new());
        return false;
    }
    show_desktop_now(ctx);
    true
}

/// Свернуть все окна текущего стола (жест «домой»).
pub fn show_desktop_now(ctx: &ShellCtx) {
    let ws = ctx.workspaces.get_untracked().iter().find(|w| w.active).map(|w| w.index);
    let ids: Vec<u64> = ctx
        .windows
        .get_untracked()
        .iter()
        .filter(|w| !w.minimized && (Some(w.workspace) == ws || w.sticky))
        .map(|w| w.id)
        .collect();
    for id in &ids {
        crate::actions::window_op(*id, WindowOp::Minimize);
    }
    if !ids.is_empty() {
        ctx.shown_desktop.set(ids);
    }
}
