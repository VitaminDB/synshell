//! Окно: на широком экране — экран машины во всё окно и плавающая панель сверху
//! ([`chrome`]: переключатель машин, настройки, снимки — поверх экрана); на узком (телефон) —
//! библиотека, по тапу — страница машины с «назад».
//! Поверх — мастер создания, подтверждения и уведомления.

pub(crate) mod capture;
mod chrome;
mod library;
mod overview;
pub mod parts;
mod screen;
mod settings;
mod snapshots;
mod wizard;

use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::widgets::{EventHook, KeyReply};
use syngui::GestureDetector;
use syngui::overlay::{ControlsSide, SystemWindowControls, WindowDragRegion, WindowResizeRegion};

use crate::model::Vm;
use crate::state::{self, icons, Confirm, Draft, Power, St, Tab};
use parts::{boxed, btn, icon_btn, spacer, W};

/// Уже этой ширины — телефонная раскладка.
pub const NARROW_BELOW: f32 = 760.0;

/// Открыть машину (или библиотеку — `None`).
pub fn open(st: St, id: Option<String>, tab: Option<Tab>) {
    let changed = st.sel.get_untracked() != id;
    if changed {
        st.snapshots.set(Vec::new());
        st.keys_menu.set(false);
        st.modes_menu.set(false);
        st.show_cmd.set(false);
        st.connected.set(false);
    }
    st.sel.set(id.clone());
    if changed {
        st.guest.set(None);
        state::refresh_guest(st);
    }
    if let Some(id) = id {
        let on = st.power(&id).on();
        // На широком экране машина открывается экраном (выключенная — кнопкой запуска на нём),
        // обзор и настройки — панели поверх; на телефоне — вкладки, у выключенной — обзор.
        let narrow = syngui::viewport::viewport_below(NARROW_BELOW).get_untracked();
        let tab = tab.unwrap_or(if changed { if on || !narrow { Tab::Screen } else { Tab::Overview } } else { st.tab.get_untracked() });
        st.tab.set(tab);
        if on {
            state::attach(st, &id);
        }
        if tab == Tab::Snapshots {
            state::load_snapshots(st);
        }
    }
    sync_grab(st);
}

pub fn set_tab(st: St, tab: Tab) {
    st.tab.set(tab);
    st.keys_menu.set(false);
    st.modes_menu.set(false);
    if tab == Tab::Snapshots {
        state::load_snapshots(st);
    }
    if tab == Tab::Screen {
        if let Some(id) = st.sel.get_untracked() {
            state::attach(st, &id);
        }
    }
    sync_grab(st);
}

/// Клавиатура уходит гостю целиком, пока виден его экран.
pub fn sync_grab(st: St) {
    let on = st.sel.get_untracked().is_some()
        && st.tab.get_untracked() == Tab::Screen
        && !st.switcher.get_untracked()
        && st.draft.get_untracked().is_none()
        && st.confirm.get_untracked().is_none();
    syngui::input::set_keyboard_grab(on);
}

pub fn new_vm(st: St) {
    st.draft.set(Some(Draft::default()));
    st.wiz.set(Some(0));
    sync_grab(st);
}

pub fn open_file(st: St, path: String) {
    wizard::open_with(st, path);
}

pub fn close_wizard(st: St) {
    st.wiz.set(None);
    st.draft.set(None);
    sync_grab(st);
}

pub fn root(st: St) -> EventHook {
    chrome::desk_test(st);
    let narrow = syngui::viewport::viewport_below(NARROW_BELOW);
    let body = move || Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let n = narrow.get();
        if !n {
            // Экран машины во всё окно, панель — поверх (свой заголовок окна — она же).
            return vec![stage(st)];
        }
        let full = st.fullscreen.get() && st.sel.get().is_some();
        if full {
            return vec![screen::page(st, true)];
        }
        let page = phone(st);
        // Заголовок с кнопками окна — и на телефоне (там без рамки и изменения размера).
        if !synshell_common::window_controls(false) {
            return vec![page];
        }
        vec![boxed(Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).child(titlebar(st, n)).child(DecoratedBox::new().class("grow").child(page)))]
    });
    let flat = use_signal(is_phone());
    create_effect(move || {
        let w = st.window.get();
        let v = is_phone() || w.maximized || w.fullscreen || (st.fullscreen.get() && st.sel.get().is_some());
        if flat.get_untracked() != v {
            flat.set(v);
        }
    });
    // Списки панели экрана закрываются при потере окном фокуса и при смене машины или вкладки.
    let seen: std::sync::Mutex<Option<(bool, Option<String>, Tab)>> = std::sync::Mutex::new(None);
    create_effect(move || {
        let now = (st.window.get().focused, st.sel.get(), st.tab.get());
        let mut last = seen.lock().unwrap();
        // меняется и размер окна — закрывать только при смене фокуса, машины или вкладки
        if last.as_ref().is_some_and(|l| *l != now) {
            screen::close_menus(st);
        }
        *last = Some(now);
    });
    let hook = EventHook::new()
        // Esc при открытом списке панели — раньше экрана машины (иначе он уходит гостю).
        .capture_keys(move |k| k == Key::Escape && (st.modes_menu.get_untracked() || st.keys_menu.get_untracked()))
        .on_key_down(move |k, m| {
            // Esc закрывает открытый список панели экрана (и тогда гостю не уходит).
            if k == Key::Escape && (st.modes_menu.get_untracked() || st.keys_menu.get_untracked()) {
                screen::close_menus(st);
                return KeyReply::Handled;
            }
            // Esc закрывает окна поверх; на экране машины клавиши уходят гостю.
            if k == Key::Escape && !syngui::input::keyboard_grab() {
                if st.confirm.get_untracked().is_none() && st.wiz.get_untracked().is_none() && (st.switcher.get_untracked() || (st.sel.get_untracked().is_some() && st.tab.get_untracked() != Tab::Screen && !narrow.get_untracked())) {
                    chrome::close_overlays(st);
                    return KeyReply::Handled;
                }
                if st.confirm.get_untracked().is_some() {
                    st.confirm.set(None);
                    sync_grab(st);
                    return KeyReply::Handled;
                }
                if st.wiz.get_untracked().is_some() {
                    close_wizard(st);
                    return KeyReply::Handled;
                }
            }
            if m.ctrl && !m.alt && k == Key::N && !syngui::input::keyboard_grab() {
                new_vm(st);
                return KeyReply::Handled;
            }
            KeyReply::Ignore
        })
        .child(Reactive::new(move || -> Vec<W> {
            // Перестраивается только при смене «рамки», а не фокуса окна.
            let flat = flat.get();
            let stack = Stack::new()
                .fit(StackFit::Expand)
                .child(DecoratedBox::new().child(body()).class("root"))
                .child(wide_only(narrow, move || chrome::overlay_layer(st)))
                .child(wide_only(narrow, move || chrome::topbar(st)))
                .child(toast_layer(st))
                .child(wizard::layer(st))
                .child(confirm_layer(st));
            let frame = DecoratedBox::new().class(if flat { "window-frame flat" } else { "window-frame" }).clip(true).child(stack);
            vec![boxed(WindowResizeRegion::new().over_content(true).inset(5.0).enabled(!flat).child(frame))]
        }));
    hook
}

/// Телефон: окно на весь экран, без рамки и изменения размера (заголовок с кнопками окна есть).
fn is_phone() -> bool {
    std::env::var("SYNSHELL_FORM_FACTOR").is_ok_and(|v| v == "phone")
}

/// Свой заголовок окна: знак, название открытой машины (за него окно перетаскивается),
/// кнопки окна из настроек рабочего стола.
fn titlebar(st: St, narrow: bool) -> W {
    let title = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let name = st.sel.get().and_then(|id| st.vms.get().into_iter().find(|v| v.id == id)).map(|v| v.name).unwrap_or_else(|| t!("Виртуальные машины"));
        vec![boxed(Text::new(name).max_lines(1).elide(Elide::End).class("tb-title"))]
    });
    let controls = Reactive::new(move || -> Vec<W> {
        let w = st.window.get();
        vec![boxed(SystemWindowControls::new(ControlsSide::Right).maximized(w.maximized).active(w.focused))]
    });
    let mut row = Row::new().cross_axis_alignment(CrossAxisAlignment::Stretch);
    row = row
        .child(DecoratedBox::new().class("grow tb-drag").child(WindowDragRegion::new().child(DecoratedBox::new().class("tb-drag-space").child(title))))
        .child(DecoratedBox::new().class("window-controls").child(controls));
    boxed(DecoratedBox::new().class(if narrow { "titlebar narrow" } else { "titlebar" }).child(row))
}

// ─── Компьютер ──────────────────────────────────────────────────────────

/// Во всё окно: экран открытой машины (выключенная — кнопка запуска на её последнем кадре)
/// или, если машина не выбрана, библиотека.
fn stage(st: St) -> W {
    let content = Reactive::new(move || -> Vec<W> {
        match st.sel.get() {
            Some(_) => vec![screen::page(st, st.fullscreen.get())],
            None => vec![library::page(st, false)],
        }
    });
    // Сверху — тонкий заголовок окна (во весь экран его нет).
    let title = Reactive::new(move || -> Vec<W> { if st.fullscreen.get() { vec![] } else { vec![chrome::titlebar(st)] } });
    boxed(Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(title).child(DecoratedBox::new().child(content).class("grow")))
}

/// Слой только для широкого окна (на телефоне — свои заголовок и вкладки).
fn wide_only(narrow: RwSignal<bool>, f: impl Fn() -> W + Send + Sync + 'static) -> W {
    boxed(Reactive::new(move || -> Vec<W> { if narrow.get() { vec![] } else { vec![f()] } }))
}

// ─── Телефон ────────────────────────────────────────────────────────────

fn phone(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        match st.sel.get() {
            Some(id) => vec![vm_page(st, id, true)],
            None => vec![library::page(st, true)],
        }
    }))
}

// ─── Страница машины ────────────────────────────────────────────────────

fn vm_page(st: St, id: String, narrow: bool) -> W {
    let id2 = id.clone();
    let head = Reactive::new(move || -> Vec<W> {
        let vms = st.vms.get();
        let Some(vm) = vms.iter().find(|v| v.id == id2) else { return vec![] };
        let l = st.live.get().get(&vm.id).cloned().unwrap_or_default();
        let busy = st.busy.get().get(&vm.id).cloned();
        vec![header(st, vm, &l, busy, narrow)]
    });
    let tabs = Reactive::new(move || -> Vec<W> {
        let cur = st.tab.get();
        let items: Vec<(String, &'static str)> = Tab::ALL.iter().map(|t| (if narrow { String::new() } else { syngui::i18n::t(t.title()) }, t.icon())).collect();
        let idx = Tab::ALL.iter().position(|t| *t == cur).unwrap_or(0);
        vec![parts::segments(items, idx, move |i| set_tab(st, Tab::ALL[i]))]
    });
    let content = Reactive::new(move || -> Vec<W> {
        let tab = st.tab.get();
        vec![match tab {
            Tab::Screen => screen::page(st, false),
            Tab::Overview => overview::page(st, narrow),
            Tab::Settings => settings::page(st, narrow),
            Tab::Snapshots => snapshots::page(st, narrow),
        }]
    });
    let bar = Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(tabs).child(spacer()).class(if narrow { "tabbar narrow" } else { "tabbar" });
    // Настройки ждут перезапуска, в госте нет драйвера syn-gpu — над любой вкладкой.
    let notes = screen::notices(st, id.clone());
    boxed(Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(head).child(bar).child(notes).child(DecoratedBox::new().child(content).class("content grow")))
}

fn header(st: St, vm: &Vm, l: &state::Live, busy: Option<String>, narrow: bool) -> W {
    let id = vm.id.clone();
    let mut row = Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center);
    if narrow {
        row = row.child(icon_btn(icons::BACK, String::new(), move || open(st, None, None)));
    }
    let sub = format!("{} · {} · {}", syngui::i18n::t(vm.os.title()), parts::cores(vm.cpus), parts::mem_mb(vm.memory_mb));
    row = row.child(parts::badge(parts::os_icon(vm.os), Some(vm.accent()), if narrow { 36.0 } else { 44.0 })).child(
        Column::new()
            .gap(3.0)
            .child(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(vm.name.clone()).max_lines(1).class("h1")).child(parts::status_chip(l.power, busy.clone())))
            .child(Text::new(sub).max_lines(1).class("muted"))
            .class("grow"),
    );
    row = row.child(power_actions(st, &id, l.power, busy.is_some(), narrow));
    boxed(row.class(if narrow { "head narrow" } else { "head" }))
}

/// Кнопки питания по состоянию.
pub fn power_actions(st: St, id: &str, power: Power, busy: bool, narrow: bool) -> W {
    let mut r = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center);
    let label = |s: String| if narrow { String::new() } else { s };
    if busy {
        // Пока идёт выключение или перезапуск — «Выключить сразу» остаётся: гость может не ответить.
        if power.on() {
            let f = id.to_string();
            r = r.child(icon_btn(icons::STOP, t!("Выключить сразу"), move || {
                st.confirm.set(Some(Confirm::ForceOff(f.clone())));
                sync_grab(st);
            }));
        }
        return boxed(r.child(parts::btn_disabled(icons::HOURGLASS, label(t!("Подождите")), "soft")));
    }
    let (a, b, c, d) = (id.to_string(), id.to_string(), id.to_string(), id.to_string());
    match power {
        Power::Off => {
            r = r.child(btn(icons::PLAY, label(t!("Запустить")), "primary", move || state::start(st, &a)));
        }
        Power::Running | Power::Busy => {
            r = r
                .child(icon_btn(icons::PAUSE, t!("Пауза"), move || state::toggle_pause(st, &a)))
                .child(icon_btn(icons::RESTART, t!("Перезагрузить"), move || state::reset(st, &b)))
                .child(icon_btn(icons::STOP, t!("Выключить сразу"), move || {
                    st.confirm.set(Some(Confirm::ForceOff(c.clone())));
                    sync_grab(st);
                }))
                .child(btn(icons::POWER, label(t!("Выключить")), "soft", move || state::shutdown(st, &d)));
        }
        Power::Paused => {
            r = r
                .child(icon_btn(icons::STOP, t!("Выключить сразу"), move || {
                    st.confirm.set(Some(Confirm::ForceOff(c.clone())));
                    sync_grab(st);
                }))
                .child(btn(icons::PLAY, label(t!("Продолжить")), "primary", move || state::toggle_pause(st, &a)));
        }
    }
    boxed(r)
}

// ─── Поверх ─────────────────────────────────────────────────────────────

fn toast_layer(st: St) -> W {
    boxed(
        Column::new().main_axis_alignment(MainAxisAlignment::End).cross_axis_alignment(CrossAxisAlignment::Center).child(Reactive::new(move || -> Vec<W> {
            let Some(t) = st.toast.get() else { return vec![] };
            let row = Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Icon::new(if t.error { icons::ERROR } else { icons::CHECK_CIRCLE }).class("toast-icon"))
                // Колонка с grow — текст получает ширину и переносится (иначе длинное сообщение режется краем окна).
                .child(Column::new().child(Text::new(t.text).max_lines(6).selectable(true).class("toast-text")).class("grow"))
                .child(icon_btn(icons::CLOSE, String::new(), move || st.toast.set(None)));
            let vw = viewport_size().get_untracked().width;
            vec![boxed(
                Animated::new(DecoratedBox::new().child(row).class(if t.error { "toast error" } else { "toast" }).style("max-width", StyleValue::px((vw - 32.0).min(620.0))))
                    .opacity(Animation::tween(Easing::EaseOutQuad).from(0.0).to(1.0).duration_ms(180).build())
                    .translate_y(Animation::tween(Easing::EaseOutCubic).from(16.0).to(0.0).duration_ms(240).build()),
            )]
        })),
    )
}

/// Затемнение и карточка по центру.
pub fn modal(card: W, on_close: impl Fn() + Send + Sync + 'static) -> W {
    let vp = viewport_size().get_untracked();
    boxed(
        Stack::new()
            .fit(StackFit::Expand)
            .child(GestureDetector::new().on_click(on_close).child(DecoratedBox::new().class("scrim").style("width", vp.width).style("height", vp.height)))
            .child(
                Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(GestureDetector::new().child(
                    Animated::new(DecoratedBox::new().child(card))
                        .opacity(Animation::tween(Easing::EaseOutQuad).from(0.0).to(1.0).duration_ms(160).build())
                        .scale(Animation::spring().from(0.96).to(1.0).stiffness(420.0).damping(30.0).build()),
                )),
            ),
    )
}

fn confirm_layer(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let Some(c) = st.confirm.get() else { return vec![] };
        let close = move || {
            st.confirm.set(None);
            sync_grab(st);
        };
        let name = |id: &str| st.vm(id).map(|v| v.name).unwrap_or_default();
        let (icon, title, text, action, danger): (&str, String, String, String, bool) = match &c {
            Confirm::Delete(id) => (
                icons::DELETE,
                t!("Удалить «{n}»?", n = name(id)),
                t!("Машина, её диски в папке машины и снимки будут удалены навсегда."),
                t!("Удалить"),
                true,
            ),
            Confirm::ForceOff(id) => (
                icons::STOP,
                t!("Выключить «{n}» сразу?", n = name(id)),
                t!("Как выдернуть шнур: несохранённые данные в госте пропадут. Обычное «Выключить» просит систему завершить работу самой."),
                t!("Выключить"),
                true,
            ),
            Confirm::Restore(_, snap) => (
                icons::RESTORE,
                t!("Вернуться к снимку «{n}»?", n = snap),
                t!("Текущее состояние машины будет заменено состоянием из снимка. Сделайте новый снимок, если оно ещё понадобится."),
                t!("Вернуться"),
                false,
            ),
            Confirm::DeleteSnapshot(_, snap) => (icons::DELETE, t!("Удалить снимок «{n}»?", n = snap), t!("Снимок будет удалён навсегда."), t!("Удалить"), true),
            Confirm::Repack(id, frag) => (
                icons::PLAY,
                t!("Перепаковать диск «{n}»?", n = name(id)),
                t!(
                    "Диск на btrfs раздроблен на {k} кусков (копирование при записи) — гость может подвисать. Перепаковка сделает полную копию без CoW (нужно ещё {g} ГБ на время копирования), снимки сохранятся. Затем машина запустится.",
                    k = frag.iter().map(|f| f.extents).sum::<u64>(),
                    g = (frag.iter().map(|f| f.size).sum::<u64>() >> 30).max(1)
                ),
                t!("Перепаковать"),
                false,
            ),
        };
        let cancel = match &c {
            Confirm::Repack(..) => t!("Запустить так"),
            _ => t!("Отмена"),
        };
        let c3 = c.clone();
        let cancel_go = move || {
            close();
            if let Confirm::Repack(id, _) = &c3 {
                state::start_declined_repack(st, id);
            }
        };
        let c2 = c.clone();
        let go = move || {
            st.confirm.set(None);
            sync_grab(st);
            match c2.clone() {
                Confirm::Delete(id) => {
                    let Some(vm) = st.vm(&id) else { return };
                    if st.power(&id).on() {
                        state::toast_err(st, t!("Сначала выключите машину"));
                        return;
                    }
                    match crate::model::delete(&vm) {
                        Ok(()) => {
                            if st.sel.get_untracked().as_deref() == Some(id.as_str()) {
                                open(st, None, None);
                            }
                            state::reload(st);
                            state::toast(st, t!("Машина «{n}» удалена", n = vm.name));
                        }
                        Err(e) => state::toast_err(st, e.to_string()),
                    }
                }
                Confirm::ForceOff(id) => state::force_off(st, &id),
                Confirm::Restore(_, snap) => state::snapshot_op(st, "restore", snap),
                Confirm::DeleteSnapshot(_, snap) => state::snapshot_op(st, "delete", snap),
                Confirm::Repack(id, _) => state::repack_and_start(st, &id),
            }
        };
        let card = DecoratedBox::new()
            .child(
                Column::new()
                    .gap(14.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(parts::badge(icon, None, 40.0)).child(Text::new(title).max_lines(3).class("dialog-title grow")))
                    .child(Text::new(text).max_lines(6).class("dialog-text"))
                    .child(Row::new().gap(8.0).child(spacer()).child(btn("", cancel, "ghost", cancel_go)).child(btn("", action, if danger { "danger" } else { "primary" }, go))),
            )
            .class("dialog");
        vec![modal(boxed(card), close)]
    }))
}
