//! Окно: на широком экране — боковая панель машин с живыми миниатюрами и страница
//! справа; на узком (телефон) — библиотека, по тапу — страница машины с «назад».
//! Поверх — мастер создания, подтверждения и уведомления.

mod library;
mod overview;
pub mod parts;
mod screen;
mod settings;
mod snapshots;
mod wizard;

use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::widgets::{EventHook, KeyReply, LiveView};
use syngui::GestureDetector;
use syngui::CursorIcon;

use crate::model::Vm;
use crate::qemu;
use crate::state::{self, icons, Confirm, Draft, Power, St, Tab};
use parts::{boxed, btn, icon_btn, spacer, W};

/// Уже этой ширины — телефонная раскладка.
pub const NARROW_BELOW: f32 = 760.0;
const SIDEBAR_W: f32 = 290.0;

/// Открыть машину (или библиотеку — `None`).
pub fn open(st: St, id: Option<String>, tab: Option<Tab>) {
    let changed = st.sel.get_untracked() != id;
    if changed {
        st.snapshots.set(Vec::new());
        st.keys_menu.set(false);
        st.show_cmd.set(false);
        st.connected.set(false);
    }
    st.sel.set(id.clone());
    if let Some(id) = id {
        let on = st.power(&id).on();
        let tab = tab.unwrap_or(if changed { if on { Tab::Screen } else { Tab::Overview } } else { st.tab.get_untracked() });
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
    let on = st.sel.get_untracked().is_some() && st.tab.get_untracked() == Tab::Screen && st.draft.get_untracked().is_none() && st.confirm.get_untracked().is_none();
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
    let narrow = syngui::viewport::viewport_below(NARROW_BELOW);
    let body = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let full = st.fullscreen.get() && st.sel.get().is_some();
        if full {
            return vec![screen::page(st, true)];
        }
        vec![if narrow.get() { phone(st) } else { desktop(st) }]
    });
    let hook = EventHook::new()
        .on_key_down(move |k, m| {
            // Esc закрывает окна поверх; на экране машины клавиши уходят гостю.
            if k == Key::Escape && !syngui::input::keyboard_grab() {
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
        .child(
            Stack::new()
                .fit(StackFit::Expand)
                .child(DecoratedBox::new().child(body).class("root"))
                .child(toast_layer(st))
                .child(wizard::layer(st))
                .child(confirm_layer(st)),
        );
    hook
}

// ─── Компьютер ──────────────────────────────────────────────────────────

fn desktop(st: St) -> W {
    let main = Reactive::new(move || -> Vec<W> {
        match st.sel.get() {
            Some(id) => vec![vm_page(st, id, false)],
            None => vec![library::page(st, false)],
        }
    });
    boxed(
        Row::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(sidebar(st))
            .child(DecoratedBox::new().child(main).class("main grow")),
    )
}

fn sidebar(st: St) -> W {
    let list = Reactive::new(move || -> Vec<W> {
        let vms = st.vms.get();
        let live = st.live.get();
        let busy = st.busy.get();
        let sel = st.sel.get();
        let q = st.query.get().to_lowercase();
        st.thumb_tick.get();
        let mut col = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        // Сначала работающие.
        let mut items: Vec<&Vm> = vms.iter().filter(|v| q.is_empty() || v.name.to_lowercase().contains(&q)).collect();
        items.sort_by_key(|v| !live.get(&v.id).is_some_and(|l| l.power.on()));
        for vm in items {
            let l = live.get(&vm.id).cloned().unwrap_or_default();
            col = col.child(nav_item(st, vm, &l, busy.get(&vm.id).cloned(), sel.as_deref() == Some(vm.id.as_str())));
        }
        if vms.is_empty() {
            col = col.child(Text::new(t!("Машин пока нет")).class("muted small pad"));
        }
        vec![boxed(col)]
    });
    let home = Reactive::new(move || -> Vec<W> {
        let active = st.sel.get().is_none();
        vec![boxed(
            GestureDetector::new().cursor(CursorIcon::Pointer).on_click(move || open(st, None, None)).child(
                DecoratedBox::new()
                    .child(
                        Row::new()
                            .gap(10.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(Icon::new(icons::APPS).class("nav-home-icon"))
                            .child(Text::new(t!("Все машины")).class("nav-title grow"))
                            .child(Reactive::new(move || -> Vec<W> {
                                let n = st.live.get().values().filter(|l| l.power.on()).count();
                                if n == 0 {
                                    vec![]
                                } else {
                                    vec![boxed(DecoratedBox::new().child(Text::new(n.to_string()).class("count-label")).class("count"))]
                                }
                            })),
                    )
                    .class(if active { "nav-home active" } else { "nav-home" }),
            ),
        )]
    });
    let search = TextField::new()
        .placeholder(t!("Поиск"))
        .prefix_icon(icons::SEARCH)
        .on_change(move |s: &str| st.query.set(s.to_string()))
        .class("search");
    boxed(
        Column::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(brand())
            .child(btn(icons::ADD, t!("Новая машина"), "primary wide", move || new_vm(st)))
            .child(search)
            .child(home)
            .child(ScrollView::new().vertical().child(list).class("grow"))
            .child(host_card())
            .class("sidebar")
            .style("width", StyleValue::px(SIDEBAR_W)),
    )
}

fn brand() -> W {
    boxed(
        Row::new()
            .gap(11.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(DecoratedBox::new().child(Icon::new(icons::LAYERS).class("brand-icon")).class("brand-logo"))
            .child(Column::new().gap(0.0).child(Text::new(t!("Машины")).class("brand-title")).child(Text::new(t!("QEMU · KVM")).class("muted small")))
            .class("brand"),
    )
}

/// Пункт машины: миниатюра, имя, состояние; у работающей — нагрузка.
fn nav_item(st: St, vm: &Vm, l: &state::Live, busy: Option<String>, active: bool) -> W {
    let id = vm.id.clone();
    let sub = match (&busy, l.power) {
        (Some(b), _) => b.clone(),
        (None, Power::Running) => t!("Работает · {c}%", c = format!("{:.0}", l.cpu)),
        (None, Power::Off) => format!("{} · {}", syngui::i18n::t(vm.os.title()), parts::ago(vm.last_run)),
        (None, p) => syngui::i18n::t(p.title()),
    };
    let thumb: W = match state::thumb(vm) {
        Some(f) => boxed(DecoratedBox::new().child(LiveView::new(f).fit(ImageFit::Cover).class("nav-thumb-img")).clip(true).class("nav-thumb")),
        None => parts::badge(parts::os_icon(vm.os), Some(vm.accent()), 40.0),
    };
    let dot = DecoratedBox::new().class(format!("dot {}", l.power.class()));
    let row = Row::new()
        .gap(11.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(thumb)
        .child(
            Column::new()
                .gap(2.0)
                .child(Row::new().gap(7.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(vm.name.clone()).max_lines(1).class("nav-title grow")).child(dot))
                .child(Text::new(sub).max_lines(1).class("nav-sub"))
                .class("grow"),
        );
    let mut item = DecoratedBox::new().child(row).class(if active { "nav-item active" } else { "nav-item" });
    if active {
        item = item.style("border-color", vm.accent().with_alpha(0.6));
    }
    boxed(GestureDetector::new().cursor(CursorIcon::Pointer).on_click(move || open(st, Some(id.clone()), None)).child(item))
}

/// Внизу панели: есть ли KVM и QEMU, свободное место.
fn host_card() -> W {
    let kvm = qemu::kvm_available();
    let qemu_ok = qemu::qemu_installed();
    let (icon, cls, title) = if !qemu_ok {
        (icons::ERROR, "host bad", t!("QEMU не установлен"))
    } else if !kvm {
        (icons::WARNING, "host warn", t!("Без KVM — медленно"))
    } else {
        (icons::BOLT, "host ok", t!("KVM включён"))
    };
    let free = crate::model::free_space().map(|b| t!("свободно {v}", v = parts::bytes(b))).unwrap_or_default();
    let sub = format!("{} · {} · {}", parts::cores(crate::model::host_cpus()), parts::mem_mb(crate::model::host_memory_mb()), free);
    boxed(
        DecoratedBox::new()
            .child(
                Row::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new(icon).class("host-icon"))
                    .child(Column::new().gap(1.0).child(Text::new(title).max_lines(1).class("host-title")).child(Text::new(sub).max_lines(2).class("muted small")).class("grow")),
            )
            .class(cls),
    )
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
    boxed(Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(head).child(bar).child(DecoratedBox::new().child(content).class("content grow")))
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
                .child(Text::new(t.text).max_lines(6).selectable(true).class("toast-text"))
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
            }
        };
        let card = DecoratedBox::new()
            .child(
                Column::new()
                    .gap(14.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(parts::badge(icon, None, 40.0)).child(Text::new(title).max_lines(3).class("dialog-title grow")))
                    .child(Text::new(text).max_lines(6).class("dialog-text"))
                    .child(Row::new().gap(8.0).child(spacer()).child(btn("", t!("Отмена"), "ghost", close)).child(btn("", action, if danger { "danger" } else { "primary" }, go))),
            )
            .class("dialog");
        vec![modal(boxed(card), close)]
    }))
}
