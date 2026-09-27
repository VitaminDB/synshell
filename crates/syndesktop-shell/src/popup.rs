//! Всплывающие окна апплетов. Каждое — прозрачная overlay-поверхность на
//! весь вывод: карточка прижата к кнопке апплета, клик мимо карточки или
//! Esc закрывают окно (layer-shell не сообщает клиенту о кликах мимо его
//! поверхностей, поэтому «мимо» — это та же поверхность).

use std::cell::{Cell, RefCell};
use syndesktop_common::action::WorkspaceTarget;
use syndesktop_common::config::Edge;
use syndesktop_common::ipc::WindowOp;
use syndesktop_common::Action;
use syngui::input::{Key, MouseButton};
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui_layer::{Anchor, KeyInfo, KeyboardInteractivity, Layer, SurfaceHooks, SurfaceId, SurfaceSpec};

use crate::ctx::{Popup, PopupKind, ShellCtx};
use crate::ui::{boxed, icon, mi, InputArea};

const GAP: f32 = 8.0;

thread_local! {
    static SURFACE: Cell<Option<SurfaceId>> = const { Cell::new(None) };
    static CURRENT: RefCell<Option<Popup>> = const { RefCell::new(None) };
    /// Перехватчик клавиш содержимого (меню запуска: стрелки, Enter).
    static KEY_HANDLER: RefCell<Option<Box<dyn FnMut(&KeyInfo) -> bool>>> = RefCell::new(None);
}

/// Задать перехватчик клавиш текущего окна.
pub fn set_key_handler(f: impl FnMut(&KeyInfo) -> bool + 'static) {
    KEY_HANDLER.with(|k| *k.borrow_mut() = Some(Box::new(f)));
}

/// Ширина карточки окна.
fn width_of(kind: &PopupKind, ctx: &ShellCtx) -> f32 {
    match kind {
        PopupKind::Launcher => ctx.cfg().launcher.width as f32,
        PopupKind::Run => 560.0,
        PopupKind::Calendar => 340.0,
        PopupKind::Volume => 320.0,
        PopupKind::Network | PopupKind::Battery => 300.0,
        PopupKind::Power => 360.0,
        PopupKind::Notifications => ctx.cfg().notifications.width as f32 + 20.0,
        PopupKind::WindowMenu(_) => 240.0,
        PopupKind::TrayMenu(_) => 280.0,
        PopupKind::TrayOverflow => 260.0,
        PopupKind::Stack { panel, index, .. } => crate::launchers::stack_width(ctx, *panel, *index),
        PopupKind::ItemMenu { .. } | PopupKind::AppMenu { .. } | PopupKind::PanelMenu(_) => 280.0,
        PopupKind::AddItem(_) => 480.0,
        PopupKind::EditItem { .. } => 480.0,
    }
}

/// Стек-веер: без карточки, значки «висят» над доком.
fn is_fan(kind: &PopupKind, ctx: &ShellCtx) -> bool {
    let PopupKind::Stack { panel, index, .. } = kind else { return false };
    let cfg = ctx.cfg();
    let Some(p) = cfg.panels.get(*panel) else { return false };
    p.applets.get(*index).is_some_and(|a| crate::launchers::stack_view_kind(a, p.edge) == "fan")
}

pub fn install(ctx: ShellCtx) {
    create_effect(move || {
        let p = ctx.popup.get();
        let same = CURRENT.with(|c| *c.borrow() == p);
        if same {
            return;
        }
        CURRENT.with(|c| *c.borrow_mut() = p.clone());
        KEY_HANDLER.with(|k| k.borrow_mut().take());
        if let Some(id) = SURFACE.with(|s| s.take()) {
            syngui_layer::close_surface(id);
        }
        let Some(p) = p else { return };
        let output = p.anchor.output.clone().or_else(|| crate::manager::focused_output(&ctx));
        let fullscreen_launcher = matches!(p.kind, PopupKind::Launcher) && ctx.cfg().launcher.style == "fullscreen";
        let hooks = SurfaceHooks {
            on_key: Some(Box::new(move |k: &KeyInfo| {
                if k.pressed && k.key == Key::Escape {
                    ShellCtx::get().close_popup();
                    return true;
                }
                KEY_HANDLER.with(|h| h.borrow_mut().as_mut().map(|f| f(k)).unwrap_or(false))
            })),
            on_closed: Some(Box::new(move || {
                SURFACE.with(|s| s.set(None));
                ShellCtx::get().close_popup();
            })),
            ..Default::default()
        };
        let spec = SurfaceSpec {
            namespace: if matches!(p.kind, PopupKind::Launcher | PopupKind::Run) {
                "syndesktop-launcher".into()
            } else {
                "syndesktop-popup".into()
            },
            layer: Layer::Overlay,
            anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            size: (0, 0),
            margin: [0; 4],
            exclusive_zone: -1,
            keyboard: KeyboardInteractivity::Exclusive,
            output: output.clone(),
            auto_size: false,
            clear_color: [0.0; 4],
        };
        let id = syngui_layer::create_surface_with(spec, hooks, move || {
            let ctx = ShellCtx::get();
            if fullscreen_launcher {
                return Box::new(crate::launcher::fullscreen(ctx));
            }
            let out_size = crate::manager::output_size(output.as_deref());
            Box::new(frame(&p, content(&p.kind, ctx), width_of(&p.kind, &ctx), out_size, is_fan(&p.kind, &ctx)))
        });
        SURFACE.with(|s| s.set(Some(id)));
    });
}

/// Подложка (клик — закрыть) + карточка у якоря.
fn frame(p: &Popup, card: Box<dyn Widget>, width: f32, out: (f32, f32), fan: bool) -> impl Widget {
    let backdrop = InputArea::new(DecoratedBox::new().class("popup-backdrop")).on_press(|_, _, _| ShellCtx::get().close_popup());
    let hover_close = matches!(p.kind, PopupKind::Stack { hover: true, .. });
    let leave_timer: std::sync::Arc<std::sync::Mutex<Option<u64>>> = Default::default();
    if hover_close {
        // Открыто наведением: закрыть, если указатель так и не дошёл до окна.
        arm_leave(&leave_timer, 1400);
    }
    let card_class = if fan { "popup-card popup-card-fan" } else { "popup-card" };
    let card = InputArea::new(
        DecoratedBox::new().child(card).class(card_class).style("width", StyleValue::px(width)),
    )
    .absorb()
    .on_hover(move |inside| {
        if !hover_close {
            return;
        }
        if inside {
            if let Some(t) = leave_timer.lock().unwrap().take() {
                syngui_layer::cancel_timer(t);
            }
        } else {
            arm_leave(&leave_timer, 450);
        }
    });
    let (ow, oh) = out;
    let placed: Box<dyn Widget> = match p.anchor.rect {
        None => Box::new(
            Column::new()
                .main_axis_alignment(MainAxisAlignment::Center)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(card)
                .class("popup-center"),
        ),
        Some([x, y, w, h]) => {
            let clamp_x = |cx: f32| cx.clamp(GAP, (ow - width - GAP).max(GAP));
            match p.anchor.edge {
                Edge::Bottom | Edge::Top => {
                    // Точка (меню окна) — от неё вправо, кнопка — по центру над ней.
                    let left = if w <= 0.0 { clamp_x(x) } else { clamp_x(x + w / 2.0 - width / 2.0) };
                    let row = Row::new().child(card).style("padding-left", StyleValue::px(left));
                    let col = if p.anchor.edge == Edge::Bottom {
                        Column::new()
                            .main_axis_alignment(MainAxisAlignment::End)
                            .child(row)
                            .style("padding-bottom", StyleValue::px((oh - y + GAP).max(GAP)))
                    } else {
                        Column::new()
                            .main_axis_alignment(MainAxisAlignment::Start)
                            .child(row)
                            .style("padding-top", StyleValue::px(y + h + GAP))
                    };
                    Box::new(col.class("popup-place"))
                }
                Edge::Left | Edge::Right => {
                    let top = y.clamp(GAP, (oh - 200.0).max(GAP));
                    let col = Column::new().child(card).style("padding-top", StyleValue::px(top));
                    let row = if p.anchor.edge == Edge::Left {
                        Row::new().main_axis_alignment(MainAxisAlignment::Start).child(col).style("padding-left", StyleValue::px(x + w + GAP))
                    } else {
                        Row::new()
                            .main_axis_alignment(MainAxisAlignment::End)
                            .child(col)
                            .style("padding-right", StyleValue::px((ow - x + GAP).max(GAP)))
                    };
                    Box::new(row.class("popup-place"))
                }
            }
        }
    };
    Stack::new().fit(StackFit::Expand).child(backdrop).child(placed)
}

/// Закрыть окно через `ms`, если указатель не вернётся.
fn arm_leave(timer: &std::sync::Arc<std::sync::Mutex<Option<u64>>>, ms: u64) {
    if let Some(t) = timer.lock().unwrap().take() {
        syngui_layer::cancel_timer(t);
    }
    let t = syngui_layer::add_timer(std::time::Duration::from_millis(ms), || {
        let ctx = ShellCtx::get();
        if ctx.popup.get_untracked().is_some_and(|p| matches!(p.kind, PopupKind::Stack { hover: true, .. })) {
            ctx.close_popup();
        }
        None
    });
    *timer.lock().unwrap() = Some(t);
}

fn content(kind: &PopupKind, ctx: ShellCtx) -> Box<dyn Widget> {
    match kind {
        PopupKind::Launcher => Box::new(crate::launcher::menu(ctx)),
        PopupKind::Run => Box::new(crate::launcher::run_prompt(ctx)),
        PopupKind::Calendar => Box::new(calendar(ctx)),
        PopupKind::Volume => Box::new(volume(ctx)),
        PopupKind::Network => Box::new(network(ctx)),
        PopupKind::Battery => Box::new(battery(ctx)),
        PopupKind::Power => Box::new(power()),
        PopupKind::Notifications => Box::new(crate::notifications::center(ctx)),
        PopupKind::WindowMenu(id) => Box::new(window_menu(ctx, *id)),
        PopupKind::TrayMenu(key) => Box::new(crate::tray::menu_view(ctx, key.clone())),
        PopupKind::TrayOverflow => Box::new(crate::tray::overflow_view(ctx)),
        PopupKind::Stack { panel, index, .. } => crate::launchers::stack_view(ctx, *panel, *index),
        PopupKind::ItemMenu { panel, index } => Box::new(crate::edit::item_menu(ctx, *panel, *index)),
        PopupKind::AppMenu { panel, app } => Box::new(crate::edit::app_menu(ctx, *panel, app)),
        PopupKind::PanelMenu(panel) => Box::new(crate::edit::panel_menu(ctx, *panel)),
        PopupKind::AddItem(panel) => Box::new(crate::edit::add_view(ctx, *panel)),
        PopupKind::EditItem { panel, index } => crate::edit::edit_view(ctx, *panel, *index),
    }
}

/// Строка меню: значок + подпись, клик — действие и закрытие окна.
pub fn menu_item(glyph: &str, label: impl Into<String>, f: impl Fn() + Send + Sync + 'static) -> impl Widget {
    InputArea::new(
        DecoratedBox::new()
            .child(
                Row::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(icon(glyph).class("menu-icon"))
                    .child(Text::new(label.into()).class("menu-label")),
            )
            .class("menu-item"),
    )
    .pointer()
    .on_click(move |b, _, _| {
        if b == MouseButton::Left {
            f();
            ShellCtx::get().close_popup();
        }
    })
}

fn title(text: impl Into<String>) -> impl Widget {
    Text::new(text.into()).class("popup-title")
}

fn calendar(ctx: ShellCtx) -> impl Widget {
    let now = ctx.now.get_untracked();
    let (y, m, d) = crate::clock::ymd(now);
    Column::new()
        .gap(8.0)
        .child(move || {
            let now = ctx.now.get();
            Column::new()
                .gap(0.0)
                .child(Text::new(crate::clock::format(now, "%H:%M:%S")).class("cal-time"))
                .child(Text::new(crate::clock::format(now, "%A, %e %B %Y")).class("cal-date"))
        })
        .child(Calendar::new().selected(Date::new(y, m, d)).show_week_numbers(true).class("cal"))
}

fn volume(ctx: ShellCtx) -> impl Widget {
    crate::system::start(ctx);
    Column::new()
        .gap(10.0)
        .child(title("Звук"))
        .child(move || {
            let v = ctx.volume.get();
            let (pct, muted, mic) = v.as_ref().map(|v| (v.percent, v.muted, v.mic_muted)).unwrap_or((0, true, true));
            Column::new()
                .gap(10.0)
                .child(
                    Row::new()
                        .gap(10.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(
                            InputArea::new(boxed("round-btn", icon(if muted { mi::VOLUME_OFF } else { mi::VOLUME_UP })))
                                .pointer()
                                .on_click(|_, _, _| crate::system::toggle_mute(ShellCtx::get(), false)),
                        )
                        .child(
                            Slider::new()
                                .range(0.0, 150.0)
                                .step(1.0)
                                .value(pct as f32)
                                .on_change(|v| crate::system::change_volume(ShellCtx::get(), 0, Some(v.round() as u32)))
                                .class("grow"),
                        )
                        .child(Text::new(format!("{pct}%")).class("popup-value")),
                )
                .child(
                    Row::new()
                        .gap(10.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(
                            InputArea::new(boxed("round-btn", icon(if mic { mi::MIC_OFF } else { mi::MIC })))
                                .pointer()
                                .on_click(|_, _, _| crate::system::toggle_mute(ShellCtx::get(), true)),
                        )
                        .child(Text::new(if mic { "Микрофон выключен" } else { "Микрофон включён" }).class("popup-text")),
                )
        })
        .child(menu_item(mi::SETTINGS, "Параметры звука…", || {
            if crate::actions::which("pavucontrol") {
                crate::actions::spawn("pavucontrol");
            } else {
                crate::actions::spawn("syndesktop-settings audio");
            }
        }))
}

fn network(ctx: ShellCtx) -> impl Widget {
    Column::new()
        .gap(8.0)
        .child(title("Сеть"))
        .child(move || {
            let n = ctx.network.get();
            let status = if n.online {
                match n.signal {
                    Some(s) => format!("{} — {s}%", n.connection),
                    None => n.connection.clone(),
                }
            } else {
                "Нет подключения".into()
            };
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(icon(crate::applets::network_glyph(&n)).class("popup-big-icon"))
                .child(Text::new(status).class("popup-text"))
        })
        .child(menu_item(mi::SETTINGS, "Параметры сети…", || {
            for (bin, cmd) in [("nm-connection-editor", "nm-connection-editor"), ("kcmshell6", "kcmshell6 kcm_networkmanagement")] {
                if crate::actions::which(bin) {
                    crate::actions::spawn(cmd);
                    return;
                }
            }
            crate::actions::spawn("syndesktop-settings network");
        }))
}

fn battery(ctx: ShellCtx) -> impl Widget {
    Column::new().gap(8.0).child(title("Питание")).child(move || {
        let Some(b) = ctx.battery.get() else {
            return Column::new().child(Text::new("Батарея не найдена").class("popup-text"));
        };
        let state = if b.charging {
            "Заряжается"
        } else if b.full {
            "Заряжена"
        } else {
            "Разряжается"
        };
        let time = b.minutes.map(|m| format!(" — {}:{:02}", m / 60, m % 60)).unwrap_or_default();
        Column::new()
            .gap(8.0)
            .child(
                Row::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(icon(crate::applets::battery_glyph(&b)).class("popup-big-icon"))
                    .child(Text::new(format!("{}% · {state}{time}", b.percent)).class("popup-text")),
            )
            .child(crate::ui::meter(b.percent))
    })
}

fn power() -> impl Widget {
    let big = |glyph: &'static str, label: &'static str, action: Action| {
        InputArea::new(
            DecoratedBox::new()
                .child(
                    Column::new()
                        .gap(6.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(icon(glyph).class("power-icon"))
                        .child(Text::new(label).class("power-label")),
                )
                .class("power-btn"),
        )
        .pointer()
        .on_click(move |b, _, _| {
            if b == MouseButton::Left {
                ShellCtx::get().close_popup();
                crate::actions::run(action.clone());
            }
        })
    };
    Column::new()
        .gap(12.0)
        .child(title("Завершение работы"))
        .child(
            Row::new()
                .gap(8.0)
                .main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .child(big(mi::LOCK, "Блокировать", Action::Shell("lock".into())))
                .child(big(mi::LOGOUT, "Выйти", Action::Quit))
                .child(big(mi::SLEEP, "Сон", Action::Suspend))
                .child(big(mi::RESTART, "Перезагрузка", Action::Reboot))
                .child(big(mi::POWER, "Выключить", Action::PowerOff)),
        )
}

fn window_menu(ctx: ShellCtx, id: u64) -> impl Widget {
    let w = ctx.windows.get_untracked().into_iter().find(|w| w.id == id).unwrap_or_default();
    let op = move |o: WindowOp| move || crate::actions::window_op(id, o);
    let mut col = Column::new()
        .gap(2.0)
        .child(Text::new(if w.title.is_empty() { w.app_id.clone() } else { w.title.clone() }).max_lines(1).class("popup-title"))
        .child(menu_item(mi::MINIMIZE, if w.minimized { "Восстановить" } else { "Свернуть" }, op(WindowOp::ToggleMinimize)))
        .child(menu_item(mi::MAXIMIZE, if w.maximized { "Восстановить размер" } else { "Развернуть" }, op(WindowOp::ToggleMaximize)))
        .child(menu_item(mi::FULLSCREEN, if w.fullscreen { "Выйти из полноэкранного" } else { "На весь экран" }, op(WindowOp::ToggleFullscreen)))
        .child(menu_item(mi::FLOAT, if w.floating { "Встроить в мозаику" } else { "Плавающее" }, op(WindowOp::ToggleFloating)))
        .child(menu_item(mi::ARROW_UP, if w.always_on_top { "✓ Поверх всех" } else { "Поверх всех" }, op(WindowOp::ToggleAlwaysOnTop)))
        .child(menu_item(mi::PUSH_PIN, if w.sticky { "✓ На всех столах" } else { "На всех столах" }, op(WindowOp::ToggleSticky)));
    let wss = ctx.workspaces.get_untracked();
    for ws in wss.iter().filter(|ws| ws.index != w.workspace) {
        let idx = ws.index;
        col = col.child(menu_item(mi::LAYERS, format!("На стол «{}»", ws.name), move || {
            crate::actions::window_op(id, WindowOp::MoveToWorkspace(idx))
        }));
    }
    let _ = WorkspaceTarget::Next;
    col.child(DecoratedBox::new().class("menu-sep")).child(menu_item(mi::CLOSE, "Закрыть", op(WindowOp::Close)))
}
