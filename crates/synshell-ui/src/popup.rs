//! Всплывающие окна апплетов. Каждое — прозрачная overlay-поверхность на
//! весь вывод: карточка прижата к кнопке апплета, клик мимо карточки или
//! Esc закрывают окно (layer-shell не сообщает клиенту о кликах мимо его
//! поверхностей, поэтому «мимо» — это та же поверхность).

use std::cell::{Cell, RefCell};
use std::sync::{Arc, Mutex};
use synshell_common::action::WorkspaceTarget;
use synshell_common::config::Edge;
use synshell_common::ipc::WindowOp;
use synshell_common::Action;
use syngui::input::{Key, MouseButton};
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui_layer::{Anchor, KeyInfo, KeyboardInteractivity, Layer, SurfaceHooks, SurfaceId, SurfaceSpec};

use crate::ctx::{Popup, PopupKind, ShellCtx};
use crate::ui::{boxed, icon, mi, InputArea};

const GAP: f32 = 8.0;

thread_local! {
    static SURFACE: Cell<Option<SurfaceId>> = const { Cell::new(None) };
    /// Видимость карточки текущего окна: `false` запускает анимацию ухода,
    /// по её концу поверхность закрывается.
    static OPEN: Cell<Option<RwSignal<bool>>> = const { Cell::new(None) };
    static CURRENT: RefCell<Option<Popup>> = const { RefCell::new(None) };
    /// Перехватчик клавиш содержимого (меню запуска: стрелки, Enter).
    static KEY_HANDLER: RefCell<Option<Box<dyn FnMut(&KeyInfo) -> bool>>> = RefCell::new(None);
}

/// Задать перехватчик клавиш текущего окна.
pub fn set_key_handler(f: impl FnMut(&KeyInfo) -> bool + 'static) {
    KEY_HANDLER.with(|k| *k.borrow_mut() = Some(Box::new(f)));
}

thread_local! {
    /// Содержимое окна вызвало экранную клавиатуру (поле пароля Wi-Fi) —
    /// с закрытием окна она прячется.
    static KEYBOARD: Cell<bool> = const { Cell::new(false) };
}

/// Телефон: показать экранную клавиатуру для поля ввода в текущем окне
/// (уйдёт вместе с окном).
pub fn request_keyboard() {
    if ShellCtx::get().is_phone() {
        KEYBOARD.with(|k| k.set(true));
        crate::actions::spawn("synkeyboard show");
    }
}

/// Ширина карточки окна.
fn width_of(kind: &PopupKind, ctx: &ShellCtx) -> f32 {
    match kind {
        PopupKind::Launcher => ctx.cfg().launcher.width as f32,
        PopupKind::Run => 560.0,
        PopupKind::Calendar => 340.0,
        PopupKind::Volume => 320.0,
        PopupKind::Network | PopupKind::Battery => 300.0,
        PopupKind::WifiConnect { .. } => 380.0,
        PopupKind::Power => 360.0,
        PopupKind::Notifications => ctx.cfg().notifications.width as f32 + 20.0,
        PopupKind::WindowMenu(_) => 240.0,
        PopupKind::GlobalMenu { .. } => 300.0,
        PopupKind::TrayMenu(_) => 280.0,
        PopupKind::TrayOverflow => 260.0,
        PopupKind::Stack { panel, index, .. } => crate::launchers::stack_width(ctx, *panel, *index),
        PopupKind::ItemMenu { .. } | PopupKind::AppMenu { .. } | PopupKind::PanelMenu(_) | PopupKind::DesktopMenu | PopupKind::HomeAppMenu(_) => 280.0,
        PopupKind::AddItem(_) => 480.0,
        PopupKind::EditItem { .. } => 480.0,
        PopupKind::Link => 380.0,
        PopupKind::LinkPair(_) => 360.0,
        PopupKind::LocationAsk(_) | PopupKind::AccessAsk(_) => 400.0,
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
    // Телефон: поле ввода в окне оболочки получило фокус (касание поиска
    // «Пуска») — экранная клавиатура; уйдёт вместе с окном.
    syngui_layer::set_virtual_keyboard_handler(|_, show| {
        if show && ShellCtx::get().popup.get_untracked().is_some() {
            request_keyboard();
        }
    });
    create_effect(move || {
        let p = ctx.popup.get();
        let same = CURRENT.with(|c| *c.borrow() == p);
        if same {
            return;
        }
        // Телефон: клавиатура, вызванная полем окна (поиск «Пуска», пароль
        // Wi-Fi), уходит вместе с окном. Сама при открытии не показывается.
        let was_keyboard = KEYBOARD.with(|k| k.replace(false));
        if ctx.is_phone() && was_keyboard {
            crate::actions::spawn("synkeyboard hide");
        }
        CURRENT.with(|c| *c.borrow_mut() = p.clone());
        KEY_HANDLER.with(|k| k.borrow_mut().take());
        let prev_open = OPEN.with(|o| o.take());
        if let Some(id) = SURFACE.with(|s| s.take()) {
            // Закрытие «в никуда» — с анимацией ухода, поверхность закроется
            // по её концу; смена окна на другое — сразу: новому нужен
            // монопольный ввод.
            match prev_open {
                Some(open) if p.is_none() && crate::anim::on(&ctx) => open.set(false),
                _ => syngui_layer::close_surface(id),
            }
        }
        let Some(p) = p else { return };
        let output = p.anchor.output.clone().or_else(|| crate::manager::focused_output(&ctx));
        let fullscreen_launcher = matches!(p.kind, PopupKind::Launcher) && ctx.cfg().launcher.style == "fullscreen";
        let my_id: Arc<Mutex<Option<SurfaceId>>> = Arc::new(Mutex::new(None));
        let hook_id = my_id.clone();
        let open = use_signal(true);
        let hooks = SurfaceHooks {
            on_key: Some(Box::new(move |k: &KeyInfo| {
                if k.pressed && k.key == Key::Escape {
                    ShellCtx::get().close_popup();
                    return true;
                }
                KEY_HANDLER.with(|h| h.borrow_mut().as_mut().map(|f| f(k)).unwrap_or(false))
            })),
            on_closed: Some(Box::new(move || {
                // Поверхность, доигравшую уход, композитор закрывает уже
                // после того, как открылось следующее окно — его не трогаем.
                if SURFACE.with(|s| s.get()) == *hook_id.lock().unwrap() {
                    SURFACE.with(|s| s.set(None));
                    OPEN.with(|o| o.take());
                    ShellCtx::get().close_popup();
                }
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
            // Телефон: учитывать зоны панелей и экранной клавиатуры — «Пуск» с
            // поиском встаёт над клавиатурой, а не под неё.
            exclusive_zone: if ctx.is_phone() { 0 } else { -1 },
            keyboard: KeyboardInteractivity::Exclusive,
            output: output.clone(),
            auto_size: false,
            clear_color: [0.0; 4],
        };
        let sid = my_id.clone();
        let id = syngui_layer::create_surface_with(spec, hooks, move || {
            let ctx = ShellCtx::get();
            if fullscreen_launcher {
                return Box::new(crate::launcher::fullscreen(ctx, open, sid));
            }
            let out_size = crate::manager::output_size(output.as_deref());
            Box::new(frame(&p, width_of(&p.kind, &ctx), out_size, is_fan(&p.kind, &ctx), open, sid))
        });
        *my_id.lock().unwrap() = Some(id);
        SURFACE.with(|s| s.set(Some(id)));
        OPEN.with(|o| o.set(Some(open)));
    });
}

/// Подложка (клик — закрыть) + карточка у якоря. Карточка вырастает из
/// края панели и уходит обратно (`Presence` по сигналу `open`); когда уход
/// доиграл, поверхность `sid` закрывается.
fn frame(p: &Popup, width: f32, out: (f32, f32), fan: bool, open: RwSignal<bool>, sid: Arc<Mutex<Option<SurfaceId>>>) -> impl Widget {
    let output_name = p.anchor.output.clone();
    // Клик мимо окна закрывает его; по другому пункту строки глобального
    // меню — открывает его меню (как в строке меню программы).
    let backdrop = InputArea::new(DecoratedBox::new().class("popup-backdrop")).on_press(|_, p, _| {
        if !crate::applets::window::switch_menu_at(p) {
            ShellCtx::get().close_popup();
        }
    });
    let hover_close = matches!(p.kind, PopupKind::Stack { hover: true, .. });
    let leave_timer: std::sync::Arc<std::sync::Mutex<Option<u64>>> = Default::default();
    if hover_close {
        // Открыто наведением: закрыть, если указатель так и не дошёл до окна.
        arm_leave(&leave_timer, 1400);
    }
    let attached = p.anchor.attached && p.anchor.rect.is_some() && !fan;
    let edge_class = match p.anchor.edge {
        Edge::Top => "popup-flow-top",
        Edge::Bottom => "popup-flow-bottom",
        Edge::Left => "popup-flow-left",
        Edge::Right => "popup-flow-right",
    };
    let card_class = if fan {
        "popup-card popup-card-fan".to_string()
    } else if attached {
        format!("popup-card {edge_class}")
    } else {
        "popup-card".to_string()
    };
    let ctx = ShellCtx::get();
    let kind = p.kind.clone();
    let (ow0, _) = out;
    let is_sheet = ctx.is_phone() && !p.kind.is_context_menu() && !centered_on_phone(&p.kind);
    // Телефон: окно по центру экрана (часы) — во всю ширину, как лист.
    let full = is_sheet || (ctx.is_phone() && centered_on_phone(&p.kind));
    // На узком экране карточка не шире вывода.
    let width = if full { ow0 - 2.0 * GAP } else { width.min((ow0 - 2.0 * GAP).max(120.0)) };
    let card_class = if is_sheet { "popup-card popup-sheet".to_string() } else { card_class };
    let presence_edge = if is_sheet {
        Some(Edge::Bottom)
    } else if full {
        None
    } else {
        p.anchor.rect.map(|_| p.anchor.edge)
    };
    // Меню запуска на рабочем столе само задаёт ширину (уголок меняет её живьём).
    let launcher_desk = matches!(p.kind, PopupKind::Launcher) && !ctx.is_phone();
    let card = crate::anim::popup_presence(&ctx, open, presence_edge, attached, move || {
        let leave_timer = leave_timer.clone();
        let mut card = DecoratedBox::new().child(content(&kind, ShellCtx::get())).class(&card_class);
        if !launcher_desk {
            card = card.style("width", StyleValue::px(width));
        }
        Box::new(
            InputArea::new(card)
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
            }),
        )
    })
    .on_exit_complete(move || {
        if let Some(id) = *sid.lock().unwrap() {
            syngui_layer::close_surface(id);
        }
    });
    // Примыкающая карточка ложится вплотную к панели, остальные — с зазором.
    let gap = if attached { 0.0 } else { GAP };
    let (ow, oh) = out;
    // Телефон: окна (громкость, сеть, «Добавить»…) — нижним листом во всю
    // ширину; контекстные меню — у пальца.
    let sheet = is_sheet;
    let rect = if ctx.is_phone() && centered_on_phone(&p.kind) { None } else { p.anchor.rect };
    let placed: Box<dyn Widget> = if sheet {
        Box::new(
            Column::new()
                .main_axis_alignment(MainAxisAlignment::End)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(card)
                .class("popup-sheet-place"),
        )
    } else { match rect {
        None => Box::new(
            Column::new()
                .main_axis_alignment(MainAxisAlignment::Center)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(card)
                .class("popup-center"),
        ),
        Some([x, y, w, h]) => {
            let clamp_x = |cx: f32| cx.clamp(GAP, (ow - width - GAP).max(GAP));
            // Точка (контекстное меню) в нижней части экрана — меню
            // раскрывается вверх от неё, иначе ушло бы за край.
            let edge = if w <= 0.0 && h <= 0.0 && p.anchor.edge == Edge::Top && y > oh * 0.55 { Edge::Bottom } else { p.anchor.edge };
            match edge {
                Edge::Bottom | Edge::Top => {
                    // Точка (меню окна) — от неё вправо, кнопка — по центру над ней;
                    // меню запуска — как задано в `[launcher] align`.
                    let left = if w <= 0.0 {
                        clamp_x(x)
                    } else if let Some((a, len)) = launcher_desk.then(|| launcher_span(&ctx, output_name.as_deref(), [x, y, w, h], false)).flatten() {
                        clamp_x(align_in(&ctx, a, len, width))
                    } else {
                        clamp_x(x + w / 2.0 - width / 2.0)
                    };
                    let row = Row::new().child(card).style("padding-left", StyleValue::px(left));
                    let col = if edge == Edge::Bottom {
                        Column::new()
                            .main_axis_alignment(MainAxisAlignment::End)
                            .child(row)
                            .style("padding-bottom", StyleValue::px((oh - y + gap).max(gap)))
                    } else {
                        Column::new()
                            .main_axis_alignment(MainAxisAlignment::Start)
                            .child(row)
                            .style("padding-top", StyleValue::px(y + h + gap))
                    };
                    Box::new(col.class("popup-place"))
                }
                Edge::Left | Edge::Right => {
                    let menu_h = ctx.cfg().launcher.height as f32;
                    let y = match launcher_desk.then(|| launcher_span(&ctx, output_name.as_deref(), [x, y, w, h], true)).flatten() {
                        Some((a, len)) => align_in(&ctx, a, len, menu_h),
                        None => y,
                    };
                    let top = y.clamp(GAP, (oh - 200.0).max(GAP));
                    let col = Column::new().child(card).style("padding-top", StyleValue::px(top));
                    let row = if p.anchor.edge == Edge::Left {
                        Row::new().main_axis_alignment(MainAxisAlignment::Start).child(col).style("padding-left", StyleValue::px(x + w + gap))
                    } else {
                        Row::new()
                            .main_axis_alignment(MainAxisAlignment::End)
                            .child(col)
                            .style("padding-right", StyleValue::px((ow - x + gap).max(gap)))
                    };
                    Box::new(row.class("popup-place"))
                }
            }
        }
    } };
    Stack::new().fit(StackFit::Expand).child(backdrop).child(placed)
}

/// Отрезок, вдоль которого ставится меню запуска: кнопка (`icon`) или полоса
/// панели (`center`, `start`, `end`) — начало и длина по оси панели.
fn launcher_span(ctx: &ShellCtx, output: Option<&str>, rect: [f32; 4], vertical: bool) -> Option<(f32, f32)> {
    let align = ctx.cfg().launcher.align.clone();
    let r = if align == "icon" || align.is_empty() { rect } else { crate::panel::span_at(output, rect).unwrap_or(rect) };
    Some(if vertical { (r[1], r[3]) } else { (r[0], r[2]) })
}

/// Начало меню размера `size` на отрезке (`a`, `len`) по `[launcher] align`.
fn align_in(ctx: &ShellCtx, a: f32, len: f32, size: f32) -> f32 {
    match ctx.cfg().launcher.align.as_str() {
        "start" => a,
        "end" => a + len - size,
        _ => a + len / 2.0 - size / 2.0,
    }
}

/// Телефон: окно по центру экрана, а не нижним листом (календарь часов).
fn centered_on_phone(kind: &PopupKind) -> bool {
    matches!(kind, PopupKind::Calendar)
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
    let w = content_inner(kind, ctx);
    if !(ctx.is_phone() && matches!(kind, PopupKind::Launcher)) {
        return w;
    }
    // Телефон: меню открывают смахиванием вверх со стола — закрывают вниз.
    // Список, прокрученный не к началу, тянется сам: жест наружу отдаёт
    // только упёршаяся в край прокрутка.
    Box::new(
        syngui::GestureDetector::new()
            .pan_axis(syngui::widgets::PanAxis::Vertical)
            .on_swipe(|dir, _| {
                if dir == syngui::widgets::SwipeDirection::Down {
                    ShellCtx::get().close_popup();
                }
            })
            .child(w),
    )
}

fn content_inner(kind: &PopupKind, ctx: ShellCtx) -> Box<dyn Widget> {
    match kind {
        PopupKind::Launcher if ctx.cfg().launcher.style == "win11" => crate::start_menu::view(ctx),
        PopupKind::Launcher => Box::new(crate::launcher::menu(ctx)),
        PopupKind::Run => Box::new(crate::launcher::run_prompt(ctx)),
        PopupKind::Calendar => Box::new(calendar(ctx)),
        PopupKind::Volume => Box::new(volume(ctx)),
        PopupKind::Network => Box::new(crate::netmenu::view(ctx)),
        PopupKind::Link => Box::new(crate::link::view(ctx)),
        PopupKind::LinkPair(id) => Box::new(crate::link::pair_view(ctx, id.clone())),
        PopupKind::LocationAsk(id) => Box::new(crate::location::ask_view(ctx, *id)),
        PopupKind::AccessAsk(id) => Box::new(crate::access::ask_view(ctx, *id)),
        PopupKind::WifiConnect { ssid } => Box::new(crate::netmenu::connect_view(ctx, ssid.clone())),
        PopupKind::Battery => Box::new(battery(ctx)),
        PopupKind::Power => Box::new(power()),
        PopupKind::Notifications => Box::new(crate::notifications::center(ctx)),
        PopupKind::WindowMenu(id) => Box::new(window_menu(ctx, *id)),
        PopupKind::TrayMenu(key) => Box::new(crate::tray::menu_view(ctx, key.clone())),
        PopupKind::GlobalMenu { id, .. } => Box::new(crate::applets::window::menu_view(*id)),
        PopupKind::TrayOverflow => Box::new(crate::tray::overflow_view(ctx)),
        PopupKind::Stack { panel, index, .. } => crate::launchers::stack_view(ctx, *panel, *index),
        PopupKind::ItemMenu { panel, index } => Box::new(crate::edit::item_menu(ctx, *panel, *index)),
        PopupKind::AppMenu { panel, app } => Box::new(crate::edit::app_menu(ctx, *panel, app)),
        PopupKind::PanelMenu(panel) => Box::new(crate::edit::panel_menu(ctx, *panel)),
        PopupKind::AddItem(panel) => Box::new(crate::edit::add_view(ctx, *panel)),
        PopupKind::EditItem { panel, index } => crate::edit::edit_view(ctx, *panel, *index),
        PopupKind::DesktopMenu => Box::new(crate::edit::desktop_menu(ctx)),
        PopupKind::HomeAppMenu(app) => Box::new(crate::edit::home_app_menu(ctx, app)),
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
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(move || {
            let now = ctx.now.get();
            Column::new()
                .gap(0.0)
                .child(Text::new(crate::clock::format(now, "%H:%M:%S")).class("cal-time"))
                .child(Text::new(crate::clock::format(now, "%A, %e %B %Y")).class("cal-date"))
        })
        .child(Calendar::new().selected(Date::new(y, m, d)).show_week_numbers(true).fill_width(ctx.is_phone()).class("cal"))
        .child(crate::datetime::quick(ctx))
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
                                .range(0.0, ShellCtx::get().config.get().sound.max_volume() as f32)
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
                crate::actions::spawn("synsettings audio");
            }
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
