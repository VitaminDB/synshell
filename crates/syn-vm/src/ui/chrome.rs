//! Окно без боковой панели: экран машины во всё окно, сверху — одна плавающая панель
//! (она же заголовок окна): переключатель машин, питание, работа с гостем, обзор, настройки,
//! снимки, «на свой рабочий стол», полный экран и кнопки окна. Переключатель машин (карусель)
//! и панели обзора, настроек и снимков открываются с неё поверх экрана — над размытым и
//! затемнённым фоном; панель остаётся над ними, повторное нажатие закрывает.

use std::sync::atomic::{AtomicBool, Ordering};

use syngui::mss::StyleValue;
use syngui::overlay::{ControlsSide, SystemWindowControls, WindowDragRegion};
use syngui::prelude::*;
use syngui::widgets::LiveView;
use syngui::CursorIcon;
use syngui::GestureDetector;

use super::parts::{self, boxed, btn, icon_btn, spacer, W};
use super::{open, set_tab, sync_grab};
use crate::model::Vm;
use crate::state::{self, icons, Power, St, Tab};

/// Свёрнутая панель: ручка, машина и «развернуть» — почти не закрывает экран гостя.
static MINI: AtomicBool = AtomicBool::new(false);

const DESK: &str = "\u{E89E}";
const DESK_BACK: &str = "\u{E5C4}";
const DROP: &str = "\u{E5CF}";

/// Закрыть переключатель машин и панели поверх экрана.
pub fn close_overlays(st: St) {
    if st.switcher.get_untracked() {
        st.switcher.set(false);
    }
    if st.tab.get_untracked() != Tab::Screen {
        set_tab(st, Tab::Screen);
    }
    sync_grab(st);
}

/// Открыть (или закрыть, если уже открыта) панель поверх экрана.
fn toggle_panel(st: St, tab: Tab) {
    st.switcher.set(false);
    super::screen::close_menus(st);
    set_tab(st, if st.tab.get_untracked() == tab { Tab::Screen } else { tab });
}

fn toggle_switcher(st: St) {
    let on = !st.switcher.get_untracked();
    super::screen::close_menus(st);
    if st.tab.get_untracked() != Tab::Screen {
        set_tab(st, Tab::Screen);
    }
    st.switcher.set(on);
    sync_grab(st);
}

// ─── Верхняя панель ─────────────────────────────────────────────────────

pub fn topbar(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        st.bar_tick.get();
        let sel = st.sel.get();
        let live = st.live.get();
        let busy = st.busy.get();
        let switcher = st.switcher.get();
        let tab = st.tab.get();
        let full = st.fullscreen.get();
        let on_desk = st.desk_from.get().is_some();
        // Мышь захвачена игрой — панели нет (Ctrl+Alt — освободить).
        if sel.as_deref().is_some_and(super::capture::active) {
            return vec![];
        }
        let vm = sel.as_deref().and_then(|id| st.vm(id));
        let power = vm.as_ref().and_then(|v| live.get(&v.id)).map(|l| l.power).unwrap_or_default();
        let _ = bar_offset();
        let mini = MINI.load(Ordering::Relaxed);

        // Ручка: тянуть панель в любое место над экраном; двойной щелчок — сверху по центру.
        let grip = GestureDetector::new()
            .cursor(CursorIcon::Move)
            .on_pan_update(move |u| {
                let mut o = bar_offset().lock().unwrap();
                *o = clamp_offset(st, (o.0 + u.delta.x, o.1 + u.delta.y));
                drop(o);
                state::poke(st);
            })
            .on_pan_end(|_| save_bar())
            .on_double_click(move || {
                *bar_offset().lock().unwrap() = (0.0, 0.0);
                save_bar();
                state::poke(st);
            })
            .child(DecoratedBox::new().child(Icon::new(icons::DRAG).class("grip-icon")).class("grip"));
        let grip = Tooltip::new(grip, t!("Передвинуть панель (двойной щелчок — на место)")).delay_ms(600);
        let mut bar = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).child(grip).child(vm_chip(st, vm.as_ref(), power, switcher));
        let fold = icon_btn(if mini { icons::CHEVRON_RIGHT } else { icons::CHEVRON_LEFT }, if mini { t!("Развернуть панель") } else { t!("Свернуть панель") }, move || {
            MINI.store(!MINI.load(Ordering::Relaxed), Ordering::Relaxed);
            save_bar();
            super::screen::close_menus(st);
            state::poke(st);
        });
        if mini {
            if let Some(v) = &vm {
                bar = bar.children(super::screen::recording(st, &v.id));
            }
            bar = bar.child(fold);
        } else {
            if let Some(v) = &vm {
                let b = busy.get(&v.id).cloned();
                bar = bar.child(vsep()).child(super::power_actions(st, &v.id, power, b.is_some(), true));
                if power.on() && st.connected.get() {
                    bar = bar.child(vsep()).children(super::screen::guest_tools(st, &v.id));
                }
                bar = bar
                    .child(vsep())
                    .child(panel_btn(st, Tab::Overview, icons::DASHBOARD, t!("Обзор"), tab))
                    .child(panel_btn(st, Tab::Settings, icons::TUNE, t!("Настройки машины"), tab))
                    .child(panel_btn(st, Tab::Snapshots, icons::HISTORY, t!("Снимки"), tab));
            }
            bar = bar.child(vsep());
            if synwm_here() || on_desk {
                bar = bar.child(icon_btn(
                    if on_desk { DESK_BACK } else { DESK },
                    if on_desk { t!("Вернуть окно на прежний рабочий стол") } else { t!("На свой рабочий стол: машина займёт свободный рабочий стол целиком") },
                    move || toggle_desk(st),
                ));
            }
            bar = bar
                .child(icon_btn(if full { icons::FULLSCREEN_EXIT } else { icons::FULLSCREEN }, t!("Во весь экран (Ctrl+Alt+F)"), move || super::screen::toggle_full(st)))
                .child(fold);
        }
        let (dx, dy) = clamp_offset(st, *bar_offset().lock().unwrap());
        let mut panel = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().child(bar).class(if mini { "top-bar mini" } else { "top-bar" }));
        if let Some(v) = vm.as_ref().filter(|_| power.on()) {
            panel = panel.child(super::screen::guest_menus(st, &v.id));
            panel = panel.child(super::screen::notices(st, v.id.clone()));
        }
        vec![boxed(
            Column::new()
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(syngui::widgets::ClickAway::new(panel).on_away(move || super::screen::close_menus(st)))
                .class("top-wrap")
                .style("padding-top", StyleValue::px(10.0 + dy + if full { 0.0 } else { TITLE_H }))
                .style("padding-left", StyleValue::px((2.0 * dx).max(0.0)))
                .style("padding-right", StyleValue::px((-2.0 * dx).max(0.0))),
        )]
    }))
}

fn vsep() -> W {
    boxed(DecoratedBox::new().class("vsep"))
}

/// Кнопка панели поверх экрана: подсвечена, пока панель открыта.
fn panel_btn(st: St, tab: Tab, icon: &'static str, tip: String, cur: Tab) -> W {
    let b = icon_btn(icon, tip, move || toggle_panel(st, tab));
    boxed(DecoratedBox::new().child(b).class(if cur == tab { "bar-toggle on" } else { "bar-toggle" }))
}

/// Машина на панели: состояние, имя и стрелка — открывает переключатель машин.
fn vm_chip(st: St, vm: Option<&Vm>, power: Power, open_now: bool) -> W {
    let (name, dot) = match vm {
        Some(v) => (v.name.clone(), format!("dot {}", power.class())),
        None => (t!("Все машины"), "dot off".to_string()),
    };
    let row = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().class(dot))
        .child(Text::new(name).max_lines(1).elide(Elide::End).class("chip-name"))
        .child(Icon::new(if open_now { icons::COLLAPSE } else { DROP }).class("chip-drop"));
    let b = GestureDetector::new()
        .cursor(CursorIcon::Pointer)
        .on_click(move || toggle_switcher(st))
        .child(DecoratedBox::new().child(row).class(if open_now { "vm-chip on" } else { "vm-chip" }));
    boxed(Tooltip::new(b, t!("Машины: переключить, создать")).delay_ms(500))
}

// ─── Поверх экрана ──────────────────────────────────────────────────────

/// Переключатель машин или панель обзора, настроек, снимков — над размытым фоном.
pub fn overlay_layer(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let switcher = st.switcher.get();
        let tab = st.tab.get();
        let sel = st.sel.get();
        if !switcher && (tab == Tab::Screen || sel.is_none()) {
            return vec![];
        }
        let vp = viewport_size().get();
        let w = (vp.width - 32.0).min(1180.0).max(300.0);
        let sheet: W = if switcher {
            switcher_sheet(st, w)
        } else {
            let h = (vp.height - 96.0).max(240.0);
            panel_sheet(st, tab, w, h)
        };
        let scrim = GestureDetector::new()
            .on_click(move || close_overlays(st))
            .child(DecoratedBox::new().class("ov-scrim").style("width", vp.width).style("height", vp.height));
        vec![boxed(
            Stack::new().fit(StackFit::Expand).child(scrim).child(
                Column::new().cross_axis_alignment(CrossAxisAlignment::Center).child(GestureDetector::new().child(
                    Animated::new(DecoratedBox::new().child(sheet))
                        .opacity(Animation::tween(Easing::EaseOutQuad).from(0.0).to(1.0).duration_ms(160).build())
                        .translate_y(Animation::spring().from(-14.0).to(0.0).stiffness(420.0).damping(32.0).build()),
                )).class("ov-place").style("padding-top", StyleValue::px(if st.fullscreen.get_untracked() { 64.0 } else { 64.0 + TITLE_H })),
            ),
        )]
    }))
}

/// Обзор, настройки или снимки открытой машины.
fn panel_sheet(st: St, tab: Tab, w: f32, h: f32) -> W {
    let name = st.sel.get_untracked().and_then(|id| st.vm(&id)).map(|v| v.name).unwrap_or_default();
    let head = Row::new()
        .gap(12.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().child(Icon::new(tab.icon()).class("sheet-icon")).class("sheet-badge"))
        .child(Column::new().gap(1.0).child(Text::new(syngui::i18n::t(tab.title())).class("sheet-title")).child(Text::new(name).max_lines(1).class("muted small")).class("grow"))
        .child(icon_btn(icons::CLOSE, t!("Закрыть (Esc)"), move || close_overlays(st)));
    let body: W = match tab {
        Tab::Overview => super::overview::page(st, false),
        Tab::Settings => super::settings::page(st, false),
        Tab::Snapshots => super::snapshots::page(st, false),
        Tab::Screen => spacer(),
    };
    boxed(
        DecoratedBox::new()
            .child(Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(DecoratedBox::new().child(head).class("sheet-head")).child(DecoratedBox::new().child(body).class("sheet-body grow")))
            .class("ov-sheet")
            .style("width", StyleValue::px(w))
            .style("height", StyleValue::px(h)),
    )
}

// ─── Переключатель машин ────────────────────────────────────────────────

const CARD_MIN: f32 = 210.0;
const GAP: f32 = 14.0;

fn switcher_sheet(st: St, w: f32) -> W {
    let page = use_signal(usize::MAX);
    let search = TextField::new()
        .text(st.query.get_untracked())
        .placeholder(t!("Поиск машины"))
        .prefix_icon(icons::SEARCH)
        .on_change(move |s: &str| {
            st.query.set(s.to_string());
            page.set(0);
        })
        .class("search sw-search");
    let running = Reactive::new(move || -> Vec<W> {
        let n = st.live.get().values().filter(|l| l.power.on()).count();
        let all = st.vms.get().len();
        vec![boxed(Text::new(if n > 0 { t!("{all} · работают {n}", all = all, n = n) } else { all.to_string() }).class("muted"))]
    });
    let head = Row::new()
        .gap(12.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().child(Icon::new(icons::LAYERS).class("sheet-icon")).class("sheet-badge"))
        .child(Text::new(t!("Машины")).class("sheet-title"))
        .child(running)
        .child(spacer())
        .child(search)
        .child(btn(icons::ADD, t!("Новая машина"), "primary", move || {
            st.switcher.set(false);
            super::new_vm(st);
        }))
        .child(icon_btn(icons::CLOSE, t!("Закрыть (Esc)"), move || close_overlays(st)));
    // Поля панели (22 с каждой стороны), стрелки (по 52), поля страницы.
    let inner = w - 2.0 * 22.0 - 2.0 * 52.0 - 8.0;
    let per = (((inner + GAP) / (CARD_MIN + GAP)).floor() as usize).clamp(1, 6);
    let cw = (inner - GAP * (per as f32 - 1.0)) / per as f32;
    // Миниатюра, поля карточки, имя и подпись.
    let ch = cw * 0.6 + 96.0;
    let strip = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        st.thumb_tick.get();
        let vms = st.vms.get();
        let live = st.live.get();
        let busy = st.busy.get();
        let sel = st.sel.get();
        let q = st.query.get().to_lowercase();
        let mut items: Vec<&Vm> = vms.iter().filter(|v| q.is_empty() || v.name.to_lowercase().contains(&q)).collect();
        // Работающие — первыми.
        items.sort_by_key(|v| !live.get(&v.id).is_some_and(|l| l.power.on()));
        let mut cards: Vec<W> = items
            .iter()
            .map(|vm| {
                let l = live.get(&vm.id).cloned().unwrap_or_default();
                vm_card(st, vm, &l, busy.get(&vm.id).cloned(), sel.as_deref() == Some(vm.id.as_str()), cw)
            })
            .collect();
        if q.is_empty() {
            cards.push(new_card(st, cw));
        }
        let pages_n = cards.len().div_ceil(per).max(1);
        // Сначала — страница с открытой машиной.
        if page.get_untracked() == usize::MAX {
            let at = sel.as_ref().and_then(|s| items.iter().position(|v| v.id == *s)).unwrap_or(0);
            page.set(at / per);
        }
        if page.get_untracked() >= pages_n {
            page.set(pages_n - 1);
        }
        let mut car = Carousel::new().page_signal(page).show_arrows(false).show_indicators(pages_n > 1).slide_duration_ms(320);
        let mut cards = cards.into_iter().peekable();
        while cards.peek().is_some() {
            let mut row = Row::new().gap(GAP).cross_axis_alignment(CrossAxisAlignment::Start);
            for c in cards.by_ref().take(per) {
                row = row.child(c);
            }
            car = car.child(DecoratedBox::new().child(row).class("sw-page"));
        }
        let cur = page.get();
        let arrow = |icon: &'static str, tip: String, to: Option<usize>| -> W {
            match to {
                Some(p) => boxed(DecoratedBox::new().child(icon_btn(icon, tip, move || page.set(p))).class("sw-arrow")),
                None => boxed(DecoratedBox::new().class("sw-arrow-gap")),
            }
        };
        vec![boxed(
            Row::new()
                .gap(0.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(arrow(icons::CHEVRON_LEFT, t!("Назад"), cur.checked_sub(1)))
                .child(DecoratedBox::new().child(car.style("height", StyleValue::px(ch + if pages_n > 1 { 26.0 } else { 6.0 }))).class("grow"))
                .child(arrow(icons::CHEVRON_RIGHT, t!("Дальше"), (cur + 1 < pages_n).then_some(cur + 1))),
        )]
    });
    boxed(
        DecoratedBox::new()
            .child(Column::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(head).child(strip).child(host_line()))
            .class("ov-sheet switcher")
            .style("width", StyleValue::px(w)),
    )
}

/// Карточка машины в карусели: живой экран (или последний кадр), имя, состояние.
fn vm_card(st: St, vm: &Vm, l: &state::Live, busy: Option<String>, active: bool, w: f32) -> W {
    let id = vm.id.clone();
    let th = w * 0.6;
    let thumb: W = match state::thumb(vm) {
        Some(f) => boxed(DecoratedBox::new().child(LiveView::new(f).fit(ImageFit::Cover).class("sw-thumb-img")).clip(true).class("sw-thumb").style("height", StyleValue::px(th))),
        None => boxed(
            DecoratedBox::new()
                .child(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(parts::badge(parts::os_icon(vm.os), Some(vm.accent()), 46.0)))
                .class("sw-thumb empty")
                .style("height", StyleValue::px(th)),
        ),
    };
    let sub = match (&busy, l.power) {
        (Some(b), _) => b.clone(),
        (None, Power::Running) => t!("Работает · {c}%", c = format!("{:.0}", l.cpu)),
        (None, Power::Off) => format!("{} · {}", syngui::i18n::t(vm.os.title()), parts::ago(vm.last_run)),
        (None, p) => syngui::i18n::t(p.title()),
    };
    let info = Column::new()
        .gap(2.0)
        .child(
            Row::new()
                .gap(7.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class(format!("dot {}", l.power.class())))
                .child(Text::new(vm.name.clone()).max_lines(1).elide(Elide::End).class("sw-name grow")),
        )
        .child(Text::new(sub).max_lines(1).elide(Elide::End).class("sw-sub"));
    let mut card = DecoratedBox::new()
        .child(Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(thumb).child(info))
        .class(if active { "sw-card active" } else { "sw-card" })
        .style("width", StyleValue::px(w));
    if active {
        card = card.style("border-color", vm.accent().with_alpha(0.85));
    }
    boxed(GestureDetector::new().cursor(CursorIcon::Pointer).on_click(move || {
        st.switcher.set(false);
        open(st, Some(id.clone()), Some(Tab::Screen));
    }).child(card))
}

fn new_card(st: St, w: f32) -> W {
    let th = w * 0.6;
    let card = DecoratedBox::new()
        .child(
            Column::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(
                    DecoratedBox::new()
                        .child(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(icons::ADD).class("sw-new-icon")))
                        .class("sw-thumb new")
                        .style("height", StyleValue::px(th)),
                )
                .child(Column::new().gap(2.0).child(Text::new(t!("Новая машина")).class("sw-name")).child(Text::new(t!("Из образа системы или диска")).max_lines(1).class("sw-sub"))),
        )
        .class("sw-card new")
        .style("width", StyleValue::px(w));
    boxed(GestureDetector::new().cursor(CursorIcon::Pointer).on_click(move || {
        st.switcher.set(false);
        super::new_vm(st);
    }).child(card))
}

/// Строка о компьютере: KVM, ядра, память, свободное место.
fn host_line() -> W {
    let kvm = crate::qemu::kvm_available();
    let qemu_ok = crate::qemu::qemu_installed();
    let (icon, cls, title) = if !qemu_ok {
        (icons::ERROR, "host-line bad", t!("QEMU не установлен"))
    } else if !kvm {
        (icons::WARNING, "host-line warn", t!("Без KVM — медленно"))
    } else {
        (icons::BOLT, "host-line ok", t!("KVM включён"))
    };
    let free = crate::model::free_space().map(|b| t!("свободно {v}", v = parts::bytes(b))).unwrap_or_default();
    let sub = format!("{} · {} · {}", parts::cores(crate::model::host_cpus()), parts::mem_mb(crate::model::host_memory_mb()), free);
    boxed(
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Icon::new(icon).class("host-icon"))
            .child(Text::new(title).class("host-title"))
            .child(Text::new(sub).max_lines(1).class("muted small"))
            .class(cls),
    )
}

// ─── Свой рабочий стол ──────────────────────────────────────────────────
// Окно уходит на первый пустой рабочий стол synshell и разворачивается во весь экран; повторное
// нажатие — назад на прежний стол и из полного экрана. Через IPC synwm: своё окно — по pid.

fn own_window(c: &mut synshell_common::ipc::Client) -> Option<synshell_common::ipc::WindowInfo> {
    use synshell_common::ipc::{Request, Response};
    let me = std::process::id() as i32;
    match c.request(&Request::Windows).ok()? {
        Response::Windows { windows } => {
            let mine: Vec<_> = windows.into_iter().filter(|w| w.pid == Some(me)).collect();
            mine.iter().find(|w| w.focused).cloned().or_else(|| mine.into_iter().next())
        }
        _ => None,
    }
}

/// Проверка во вложенном synwm: `SYN_VM_TEST_DESK=1` — через 4 с после запуска нажать «на свой
/// рабочий стол», `=2` — и ещё через 4 с вернуть.
pub fn desk_test(st: St) {
    let Some(n) = std::env::var("SYN_VM_TEST_DESK").ok().and_then(|v| v.parse::<u32>().ok()) else { return };
    std::thread::spawn(move || {
        for _ in 0..n {
            std::thread::sleep(std::time::Duration::from_secs(4));
            syngui::async_runtime::run_on_main_thread(move || toggle_desk(st));
        }
    });
}

pub fn toggle_desk(st: St) {
    use synshell_common::ipc::Client;
    let back = st.desk_from.get_untracked();
    std::thread::spawn(move || {
        let run = || -> std::result::Result<Option<u32>, String> {
            let mut c = Client::connect().map_err(|e| t!("нет связи с synwm: {e}", e = e))?;
            desk_step(&mut c, back)
        };
        let r = run();
        syngui::async_runtime::run_on_main_thread(move || match r {
            Ok(Some(from)) => {
                st.desk_from.set(Some(from));
                if !st.fullscreen.get_untracked() {
                    super::screen::toggle_full(st);
                }
                state::poke(st);
            }
            Ok(None) if back.is_some() => {
                st.desk_from.set(None);
                if st.fullscreen.get_untracked() {
                    super::screen::toggle_full(st);
                }
                state::poke(st);
            }
            Ok(None) => {
                // Свободного стола нет — во весь экран здесь.
                if !st.fullscreen.get_untracked() {
                    super::screen::toggle_full(st);
                }
                state::toast(st, t!("Свободного рабочего стола нет — машина во весь экран на этом. Число столов — в Параметрах → Рабочие столы."));
            }
            Err(e) => state::toast_err(st, e),
        });
    });
}

/// Окно в сеансе synshell (synwm отвечает) — проверяется один раз.
fn synwm_here() -> bool {
    static HERE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *HERE.get_or_init(|| synshell_common::ipc::Client::connect().is_ok())
}

/// Шаг «на свой стол» / «назад» через подключение к synwm: `Some(откуда)` — окно перенесено,
/// `None` — возвращено (или свободного стола нет).
fn desk_step(c: &mut synshell_common::ipc::Client, back: Option<u32>) -> std::result::Result<Option<u32>, String> {
    use synshell_common::action::{Action, WorkspaceTarget};
    use synshell_common::ipc::{Request, Response, WindowOp};
    let win = own_window(c).ok_or_else(|| t!("synwm не нашёл окно машин"))?;
    let go = |c: &mut synshell_common::ipc::Client, ws: u32| {
        let _ = c.request(&Request::WindowAction { id: win.id, op: WindowOp::MoveToWorkspace(ws) });
        let _ = c.request(&Request::Action { action: Action::Workspace(WorkspaceTarget::Index(ws + 1)) });
        let _ = c.request(&Request::WindowAction { id: win.id, op: WindowOp::Activate });
    };
    match back {
        Some(from) => {
            go(c, from);
            Ok(None)
        }
        None => {
            let wss = match c.request(&Request::Workspaces).map_err(|e| e.to_string())? {
                Response::Workspaces { workspaces } => workspaces,
                _ => Vec::new(),
            };
            let from = win.workspace;
            let free = wss.iter().find(|w| w.windows == 0 && w.index != from).map(|w| w.index);
            if let Some(to) = free {
                go(c, to);
            }
            Ok(Some(from).filter(|_| free.is_some()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    /// Поддельный synwm: отвечает на запросы и запоминает их.
    fn fake(windows: String, workspaces: String) -> (std::path::PathBuf, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let dir = std::env::temp_dir().join(format!("syn-vm-desk-{}-{}", std::process::id(), windows.len() + workspaces.len()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("ipc.sock");
        let _ = std::fs::remove_file(&path);
        let l = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log2 = log.clone();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let mut w = s.try_clone().unwrap();
                for line in BufReader::new(s).lines().map_while(std::result::Result::ok) {
                    log2.lock().unwrap().push(line.clone());
                    let resp = if line.contains(r#""request":"windows""#) {
                        windows.clone()
                    } else if line.contains(r#""request":"workspaces""#) {
                        workspaces.clone()
                    } else {
                        r#"{"response":"ok"}"#.to_string()
                    };
                    let _ = writeln!(w, "{resp}");
                }
            }
        });
        (path, log)
    }

    fn win_json(ws: u32) -> String {
        format!(
            r#"{{"response":"windows","windows":[{{"id":7,"title":"x","app_id":"other","pid":1,"workspace":0,"output":"eDP-1","focused":false,"minimized":false,"maximized":false,"fullscreen":false,"floating":true,"sticky":false,"always_on_top":false,"urgent":false,"skip_taskbar":false,"geometry":[0,0,10,10]}},{{"id":42,"title":"Машины","app_id":"syn-vm","pid":{},"workspace":{ws},"output":"eDP-1","focused":true,"minimized":false,"maximized":false,"fullscreen":false,"floating":true,"sticky":false,"always_on_top":false,"urgent":false,"skip_taskbar":false,"geometry":[0,0,10,10]}}]}}"#,
            std::process::id()
        )
    }

    #[test]
    fn moves_to_free_desk_and_back() {
        let wss = r#"{"response":"workspaces","workspaces":[{"index":0,"name":"1","active":true,"windows":7,"urgent":false,"layout":"floating"},{"index":1,"name":"2","active":false,"windows":3,"urgent":false,"layout":"floating"},{"index":2,"name":"3","active":false,"windows":0,"urgent":false,"layout":"floating"}]}"#;
        let (path, log) = fake(win_json(0), wss.to_string());
        let mut c = synshell_common::ipc::Client::connect_to(&path).unwrap();
        // Первый пустой — третий стол (индекс 2); окно — наше по pid, не чужое.
        assert_eq!(desk_step(&mut c, None), Ok(Some(0)));
        let l = log.lock().unwrap().clone();
        assert!(l.iter().any(|x| x.contains(r#""id":42"#) && x.contains(r#""move-to-workspace":2"#)), "{l:#?}");
        assert!(l.iter().any(|x| x.contains(r#""action":"workspace 3""#)), "{l:#?}");
        assert!(l.iter().any(|x| x.contains(r#""id":42"#) && x.contains("activate")), "{l:#?}");
        assert!(!l.iter().any(|x| x.contains(r#""id":7"#)));
        // Назад — на прежний стол.
        log.lock().unwrap().clear();
        assert_eq!(desk_step(&mut c, Some(0)), Ok(None));
        let l = log.lock().unwrap().clone();
        assert!(l.iter().any(|x| x.contains(r#""move-to-workspace":0"#)), "{l:#?}");
        assert!(l.iter().any(|x| x.contains(r#""action":"workspace 1""#)), "{l:#?}");
    }

    #[test]
    fn no_free_desk() {
        let wss = r#"{"response":"workspaces","workspaces":[{"index":0,"name":"1","active":true,"windows":2,"urgent":false,"layout":"floating"},{"index":1,"name":"2","active":false,"windows":1,"urgent":false,"layout":"floating"}]}"#;
        let (path, log) = fake(win_json(0), wss.to_string());
        let mut c = synshell_common::ipc::Client::connect_to(&path).unwrap();
        assert_eq!(desk_step(&mut c, None), Ok(None));
        assert!(!log.lock().unwrap().iter().any(|x| x.contains("move-to-workspace")));
    }
}

// ─── Положение плавающей панели ─────────────────────────────────────────
// Сдвиг (логические пиксели) от места по умолчанию — сверху по центру под заголовком; панель
// тянут за ручку слева, двойной щелчок по ручке — обратно. Запоминается вместе со свёрнутостью
// в `<данные>/syn-vm/screen-bar` («dx dy свёрнута»).

/// Высота заголовка окна.
pub const TITLE_H: f32 = 34.0;

fn bar_file() -> std::path::PathBuf {
    crate::model::root().join("screen-bar")
}

fn bar_offset() -> &'static std::sync::Mutex<(f32, f32)> {
    static O: std::sync::OnceLock<std::sync::Mutex<(f32, f32)>> = std::sync::OnceLock::new();
    O.get_or_init(|| {
        let v = std::fs::read_to_string(bar_file()).ok().and_then(|s| {
            let mut it = s.split_whitespace().filter_map(|x| x.parse::<f32>().ok());
            let o = (it.next()?, it.next()?);
            MINI.store(it.next().is_some_and(|m| m != 0.0), Ordering::Relaxed);
            Some(o)
        });
        std::sync::Mutex::new(v.unwrap_or((0.0, 0.0)))
    })
}

/// Сдвиг в пределах окна.
fn clamp_offset(st: St, (dx, dy): (f32, f32)) -> (f32, f32) {
    let _ = st;
    let vp = viewport_size().get_untracked();
    let mx = (vp.width / 2.0 - 140.0).max(0.0);
    (dx.clamp(-mx, mx), dy.clamp(0.0, (vp.height - TITLE_H - 80.0).max(0.0)))
}

fn save_bar() {
    let (dx, dy) = *bar_offset().lock().unwrap();
    let mini = MINI.load(Ordering::Relaxed) as u8;
    let _ = std::fs::write(bar_file(), format!("{dx:.0} {dy:.0} {mini}\n"));
}

// ─── Заголовок окна ─────────────────────────────────────────────────────

/// Тонкий заголовок: знак, «Машины · имя» с состоянием, кнопки окна. За него окно
/// перетаскивается, двойной щелчок разворачивает. Во весь экран не показывается.
pub fn titlebar(st: St) -> W {
    let title = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let sel = st.sel.get();
        let live = st.live.get();
        let vm = sel.as_deref().and_then(|id| st.vm(id));
        let mut row = Row::new()
            .gap(9.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(DecoratedBox::new().child(Icon::new(icons::LAYERS).class("tl-icon")).class("tl-logo"))
            .child(Text::new(t!("Машины")).class("tl-app"));
        if let Some(v) = vm {
            let power = live.get(&v.id).map(|l| l.power).unwrap_or_default();
            row = row
                .child(Text::new("·".to_string()).class("tl-sep"))
                .child(DecoratedBox::new().class(format!("dot {}", power.class())))
                .child(Text::new(v.name.clone()).max_lines(1).elide(Elide::End).class("tl-name"))
                .child(Text::new(syngui::i18n::t(power.title())).max_lines(1).class("tl-state"));
        }
        vec![boxed(row)]
    });
    let controls = Reactive::new(move || -> Vec<W> {
        let w = st.window.get();
        vec![boxed(SystemWindowControls::new(ControlsSide::Right).maximized(w.maximized).active(w.focused))]
    });
    let drag = WindowDragRegion::new().child(
        DecoratedBox::new()
            .child(Row::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(title))
            .class("tl-drag"),
    );
    let mut row = Row::new().cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().child(drag).class("grow"));
    if synshell_common::window_controls(false) {
        row = row.child(DecoratedBox::new().child(controls).class("tl-controls"));
    }
    boxed(DecoratedBox::new().child(row).class("titlebar-slim").style("height", StyleValue::px(TITLE_H)))
}
