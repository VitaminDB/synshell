//! Быстрое меню gameshell (Guide, Super, жест «домой»): своя поверхность на
//! слое Overlay с монопольной клавиатурой поверх игры, геймпады на это
//! время захвачены (нажатия не уходят в игру). Панель слева: продолжить,
//! на главную, закрыть игру, запущенные, громкость и яркость, экранная
//! клавиатура, параметры, выход из игрового режима, питание.

use std::cell::Cell;

use synshell_common::action::FormFactorTarget;
use synshell_common::ipc::WindowOp;
use synshell_common::Action;
use synshell_ui::ui::{icon, meter, mi, rx, InputArea};
use synshell_ui::ShellCtx;
use syngui::input::MouseButton;
use syngui::prelude::*;
use syngui::ScrollbarPolicy;
use syngui_layer::{Anchor, KeyboardInteractivity, Layer, SurfaceHooks, SurfaceId, SurfaceSpec};

use crate::nav::{self, Nav};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Main,
    Running,
    Power,
}

#[derive(Debug, Clone, PartialEq)]
enum Item {
    Resume,
    Home,
    CloseGame(u64, String),
    Switch(u64, String),
    RunningMore,
    Volume,
    Brightness,
    Keyboard,
    Settings,
    ExitMode,
    PowerMenu,
    Sleep,
    Lock,
    Reboot,
    PowerOff,
    Logout,
    Back,
}

#[derive(Clone, Copy)]
struct St {
    section: RwSignal<Option<Section>>,
    sel: RwSignal<usize>,
    /// «Закрыть игру» нажато один раз — ждём подтверждения.
    armed: RwSignal<bool>,
}

thread_local! {
    static SURFACE: Cell<Option<SurfaceId>> = const { Cell::new(None) };
    static ST: Cell<Option<St>> = const { Cell::new(None) };
}

fn st() -> St {
    ST.with(|s| match s.get() {
        Some(v) => v,
        None => {
            let v = St { section: use_signal(None), sel: use_signal(0), armed: use_signal(false) };
            s.set(Some(v));
            v
        }
    })
}

pub fn is_open() -> bool {
    SURFACE.with(|s| s.get()).is_some()
}

pub fn open(section: Section) {
    if !is_open() {
        synshell_ui::sounds::play(synshell_ui::sounds::Ev::MenuOpen);
    }
    let s = st();
    s.section.set(Some(section));
    s.sel.set(0);
    s.armed.set(false);
    crate::gamepad::set_grab(true);
    if is_open() {
        return;
    }
    let ctx = ShellCtx::get();
    let out = synshell_ui::manager::focused_output(&ctx).or_else(|| synshell_ui::manager::primary_output(&ctx));
    let spec = SurfaceSpec {
        namespace: "syngame-overlay".into(),
        layer: Layer::Overlay,
        anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
        size: (0, 0),
        margin: [0; 4],
        exclusive_zone: -1,
        keyboard: KeyboardInteractivity::Exclusive,
        output: out,
        auto_size: false,
        clear_color: [0.0; 4],
    };
    let hooks = SurfaceHooks {
        on_key: Some(Box::new(nav::key)),
        on_closed: Some(Box::new(|| {
            SURFACE.with(|s| s.set(None));
            crate::gamepad::set_grab(false);
        })),
        ..Default::default()
    };
    let id = syngui_layer::create_surface_with(spec, hooks, || Box::new(view()));
    SURFACE.with(|s| s.set(Some(id)));
}

pub fn close() {
    crate::gamepad::set_grab(false);
    if let Some(id) = SURFACE.with(|s| s.take()) {
        synshell_ui::sounds::play(synshell_ui::sounds::Ev::MenuClose);
        syngui_layer::close_surface(id);
    }
    st().section.set(None);
}

/// Окно игры поверх главного экрана (в фокусе).
fn front_window(ctx: &ShellCtx) -> Option<(u64, String)> {
    let wins = ctx.windows.get_untracked();
    let w = wins.iter().find(|w| w.focused && !w.minimized).or_else(|| wins.iter().find(|w| !w.minimized && !w.skip_taskbar))?;
    let lib = nav::gs().lib.get_untracked();
    let name = nav::running(ctx, &lib).into_iter().find(|(_, id)| *id == w.id).map(|(g, _)| g.name).unwrap_or_else(|| w.title.clone());
    Some((w.id, name))
}

fn items(section: Section) -> Vec<Item> {
    let ctx = ShellCtx::get();
    let front = if nav::home_visible(&ctx) { None } else { front_window(&ctx) };
    match section {
        Section::Main => {
            let mut v = Vec::new();
            if let Some((id, name)) = &front {
                v.push(Item::Resume);
                v.push(Item::Home);
                v.push(Item::CloseGame(*id, name.clone()));
            }
            let lib = nav::gs().lib.get_untracked();
            let others: Vec<(u64, String)> = nav::running(&ctx, &lib).into_iter().filter(|(_, id)| Some(*id) != front.as_ref().map(|f| f.0)).map(|(g, id)| (id, g.name)).collect();
            if !others.is_empty() {
                v.push(Item::RunningMore);
            }
            v.push(Item::Volume);
            if ctx.brightness.get_untracked().is_some() {
                v.push(Item::Brightness);
            }
            v.push(Item::Keyboard);
            v.push(Item::Settings);
            v.push(Item::ExitMode);
            v.push(Item::PowerMenu);
            v
        }
        Section::Running => {
            let lib = nav::gs().lib.get_untracked();
            let mut v: Vec<Item> = nav::running(&ctx, &lib).into_iter().map(|(g, id)| Item::Switch(id, g.name)).collect();
            v.push(Item::Back);
            v
        }
        Section::Power => vec![Item::Sleep, Item::Lock, Item::Reboot, Item::PowerOff, Item::Logout, Item::Back],
    }
}

fn label(it: &Item) -> (&'static str, String) {
    let ctx = ShellCtx::get();
    match it {
        Item::Resume => ("\u{E037}", t!("Продолжить").to_string()),
        Item::Home => (mi::HOME, t!("На главную").to_string()),
        Item::CloseGame(_, n) if st().armed.get() => (mi::CLOSE, t!("Ещё раз — закрыть «{n}»", n = n).to_string()),
        Item::CloseGame(_, n) => (mi::CLOSE, t!("Закрыть «{n}»", n = n).to_string()),
        Item::Switch(_, n) => (mi::WINDOW, n.clone()),
        Item::RunningMore => (mi::LAYERS, t!("Запущенные").to_string()),
        Item::Volume => {
            let v = ctx.volume.get();
            let muted = v.as_ref().is_some_and(|v| v.muted);
            (if muted { mi::VOLUME_MUTE } else { mi::VOLUME_UP }, t!("Громкость").to_string())
        }
        Item::Brightness => (mi::BRIGHTNESS, t!("Яркость").to_string()),
        Item::Keyboard => (mi::KEYBOARD, t!("Экранная клавиатура").to_string()),
        Item::Settings => (mi::SETTINGS, t!("Параметры").to_string()),
        Item::ExitMode => (mi::DESKTOP, t!("Выйти из игрового режима").to_string()),
        Item::PowerMenu => (mi::POWER, t!("Питание").to_string()),
        Item::Sleep => (mi::SLEEP, t!("Сон").to_string()),
        Item::Lock => (mi::LOCK, t!("Заблокировать").to_string()),
        Item::Reboot => (mi::RESTART, t!("Перезагрузить").to_string()),
        Item::PowerOff => (mi::POWER, t!("Выключить").to_string()),
        Item::Logout => (mi::LOGOUT, t!("Выйти из сеанса").to_string()),
        Item::Back => (mi::BACK, t!("Назад").to_string()),
    }
}

fn activate(it: &Item) {
    let ctx = ShellCtx::get();
    match it {
        Item::Resume => close(),
        Item::Home => nav::go_home(),
        Item::CloseGame(id, _) => {
            if ctx.cfg().gameshell.confirm_close && !st().armed.get_untracked() {
                st().armed.set(true);
                return;
            }
            close();
            synshell_ui::actions::window_op(*id, WindowOp::Close);
        }
        Item::Switch(id, _) => {
            close();
            synshell_ui::actions::window_op(*id, WindowOp::Activate);
        }
        Item::RunningMore => {
            st().section.set(Some(Section::Running));
            st().sel.set(0);
        }
        Item::Volume => synshell_ui::system::toggle_mute(ctx, false),
        Item::Brightness => {}
        Item::Keyboard => {
            close();
            synshell_ui::actions::spawn("synkeyboard toggle");
        }
        Item::Settings => {
            close();
            synshell_ui::actions::spawn("synsettings");
        }
        Item::ExitMode => {
            close();
            synshell_ui::actions::run(Action::FormFactor(FormFactorTarget::ToggleGame));
        }
        Item::PowerMenu => {
            st().section.set(Some(Section::Power));
            st().sel.set(0);
        }
        Item::Sleep => {
            close();
            synshell_ui::actions::run(Action::Suspend);
        }
        Item::Lock => {
            close();
            synshell_ui::actions::run(Action::Lock);
        }
        Item::Reboot => synshell_ui::actions::run(Action::Reboot),
        Item::PowerOff => synshell_ui::actions::run(Action::PowerOff),
        Item::Logout => synshell_ui::actions::run(Action::Quit),
        Item::Back => {
            st().section.set(Some(Section::Main));
            st().sel.set(0);
        }
    }
}

fn adjust(it: &Item, d: i32) -> bool {
    let ctx = ShellCtx::get();
    match it {
        Item::Volume => synshell_ui::system::change_volume(ctx, d * 5, None),
        Item::Brightness => synshell_ui::system::change_brightness(ctx, d * 5),
        _ => return false,
    }
    true
}

/// Ввод, пока меню открыто.
pub fn handle(n: Nav) {
    let s = st();
    let Some(section) = s.section.get_untracked() else {
        close();
        return;
    };
    let list = items(section);
    let sel = s.sel.get_untracked().min(list.len().saturating_sub(1));
    if !matches!(n, Nav::Accept) {
        s.armed.set(false);
    }
    use synshell_ui::sounds::{play, Ev};
    match n {
        Nav::Up => {
            play(if sel > 0 { Ev::Navigate } else { Ev::Bump });
            s.sel.set(sel.saturating_sub(1))
        }
        Nav::Down => {
            play(if sel + 1 < list.len() { Ev::Navigate } else { Ev::Bump });
            s.sel.set((sel + 1).min(list.len().saturating_sub(1)))
        }
        Nav::Left | Nav::Right => {
            if let Some(it) = list.get(sel) {
                adjust(it, if n == Nav::Left { -1 } else { 1 });
            }
        }
        Nav::Accept => {
            if let Some(it) = list.get(sel).cloned() {
                play(Ev::Select);
                activate(&it);
            }
        }
        Nav::Back => {
            if section == Section::Main {
                close();
            } else {
                play(Ev::Back);
                s.section.set(Some(Section::Main));
                s.sel.set(0);
            }
        }
        Nav::Guide | Nav::Menu => close(),
        _ => {}
    }
}

fn view() -> impl Widget {
    let s = st();
    let panel = rx(move || {
        let ctx = ShellCtx::get();
        let section = s.section.get().unwrap_or(Section::Main);
        let sel = s.sel.get();
        let _ = ctx.volume.get();
        let _ = ctx.brightness.get();
        let _ = ctx.windows.get();
        let list = items(section);
        let title = match section {
            Section::Main => match front_window(&ctx).filter(|_| !nav::home_visible(&ctx)) {
                Some((_, n)) => n,
                None => t!("Меню").to_string(),
            },
            Section::Running => t!("Запущенные").to_string(),
            Section::Power => t!("Питание").to_string(),
        };
        let mut col = Column::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(
                Row::new()
                    .gap(12.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new(title).max_lines(1).class("gs-ov-title grow"))
                    .child(Text::new(synshell_ui::clock::format(ctx.now.get(), "%H:%M")).class("gs-ov-clock")),
            )
            .child(DecoratedBox::new().class("gs-ov-sep"));
        for (i, it) in list.iter().enumerate() {
            let (glyph, text) = label(it);
            let mut row = Row::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(glyph).class("gs-item-icon")).child(Text::new(text).max_lines(1).class("gs-item-text grow"));
            match it {
                Item::Volume => {
                    let v = ctx.volume.get().map(|v| v.percent).unwrap_or(0);
                    row = row.child(DecoratedBox::new().child(meter(v)).class("gs-ov-meter")).child(Text::new(format!("{v}%")).class("gs-ov-value"));
                }
                Item::Brightness => {
                    let b = (ctx.brightness.get().unwrap_or(0.0) * 100.0).round() as u32;
                    row = row.child(DecoratedBox::new().child(meter(b)).class("gs-ov-meter")).child(Text::new(format!("{b}%")).class("gs-ov-value"));
                }
                Item::RunningMore | Item::PowerMenu => row = row.child(icon(mi::CHEVRON_RIGHT).class("gs-item-icon")),
                _ => {}
            }
            let it2 = it.clone();
            let it3 = it.clone();
            col = col.child(
                InputArea::new(DecoratedBox::new().child(row).class(if i == sel { "gs-item gs-item-sel" } else { "gs-item" }))
                    .pointer()
                    .on_hover(move |h| {
                        if h && st().sel.get_untracked() != i {
                            st().sel.set(i);
                            st().armed.set(false);
                        }
                    })
                    .on_wheel(move |d| {
                        adjust(&it3, if d > 0.0 { 1 } else { -1 });
                    })
                    .on_click(move |b, _, _| {
                        if b == MouseButton::Left {
                            activate(&it2);
                        }
                    }),
            );
        }
        Box::new(col)
    });
    Stack::new()
        .fit(StackFit::Expand)
        .child(InputArea::new(DecoratedBox::new().class("gs-ov-scrim")).on_press(|_, _, _| close()))
        .child(
            Row::new()
                .child(InputArea::new(DecoratedBox::new().child(ScrollView::new().vertical().scrollbar_policy(ScrollbarPolicy::Never).child(panel)).class("gs-ov-panel")).absorb())
                .child(DecoratedBox::new().class("grow")),
        )
}
