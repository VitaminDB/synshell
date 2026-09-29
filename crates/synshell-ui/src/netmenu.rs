//! Окно «Сеть» (апплет `network` на панели и доке, плитка шторки): текущее
//! подключение и управление Wi-Fi через `synsystem::wifi` (iwd или
//! NetworkManager, `[wifi] backend`): включить/выключить, список сетей с силой
//! сигнала, подключение (пароль — для защищённой новой сети), отключение,
//! «забыть», «Параметры сети…».
//!
//! Все вызовы бэкенда блокирующие — в фоновом потоке; состояние
//! перечитывается раз в 3 с, пока окно открыто (и не вводят пароль),
//! результат действия — строкой под списком.

use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use syngui::async_runtime::run_on_main_thread;
use syngui::input::MouseButton;
use syngui::mss::StyleValue;
use syngui::prelude::*;
use std::result::Result;
use synsystem::wifi::{self, WifiState};

use crate::ctx::{PopupKind, ShellCtx};
use crate::ui::{icon, mi, rx, InputArea};

type WifiSig = RwSignal<Option<Result<WifiState, String>>>;

thread_local! {
    static WIFI: Cell<Option<WifiSig>> = const { Cell::new(None) };
    /// Сеть с раскрытым полем пароля.
    static PASS_FOR: Cell<Option<RwSignal<Option<String>>>> = const { Cell::new(None) };
    /// Подключённая сеть с раскрытыми действиями (отключить, забыть).
    static OPEN_FOR: Cell<Option<RwSignal<Option<String>>>> = const { Cell::new(None) };
    /// Итог последнего действия.
    static TOAST: Cell<Option<RwSignal<Option<String>>>> = const { Cell::new(None) };
}

/// Поколение опроса: новый показ окна останавливает поток прежнего.
static POLL_GEN: AtomicU64 = AtomicU64::new(0);

fn sig<T: Clone + 'static>(cell: &'static std::thread::LocalKey<Cell<Option<RwSignal<T>>>>, init: impl FnOnce() -> T) -> RwSignal<T> {
    cell.with(|c| match c.get() {
        Some(s) => s,
        None => {
            let s = use_signal(init());
            c.set(Some(s));
            s
        }
    })
}

fn wifi_sig() -> WifiSig {
    sig(&WIFI, || None)
}
fn pass_for() -> RwSignal<Option<String>> {
    sig(&PASS_FOR, || None)
}
fn open_for() -> RwSignal<Option<String>> {
    sig(&OPEN_FOR, || None)
}
fn toast_sig() -> RwSignal<Option<String>> {
    sig(&TOAST, || None)
}

fn backend_choice() -> String {
    ShellCtx::get().cfg().wifi.backend.clone()
}

/// Окно сети открыто.
fn is_open() -> bool {
    ShellCtx::get().popup.get_untracked().is_some_and(|p| matches!(p.kind, PopupKind::Network))
}

fn refresh() {
    let s = wifi_sig();
    let choice = backend_choice();
    std::thread::spawn(move || {
        let r = match wifi::backend(&choice) {
            Some(b) => b.state(),
            None => Err("Нет службы Wi-Fi: запустите iwd или NetworkManager".into()),
        };
        run_on_main_thread(move || s.set(Some(r)));
    });
}

/// Опрос, пока окно открыто.
fn start_polling() {
    let my = POLL_GEN.fetch_add(1, Ordering::Relaxed) + 1;
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(3));
        if POLL_GEN.load(Ordering::Relaxed) != my {
            return;
        }
        let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let a2 = alive.clone();
        run_on_main_thread(move || {
            if !is_open() {
                a2.store(false, Ordering::Relaxed);
                return;
            }
            // Пока вводят пароль — не перестраивать список.
            if pass_for().get_untracked().is_none() {
                refresh();
            }
        });
        // Дать главному потоку ответить.
        std::thread::sleep(Duration::from_millis(200));
        if !alive.load(Ordering::Relaxed) {
            return;
        }
    });
}

/// Действие с Wi-Fi в фоне, затем перечитать состояние.
fn wifi_do(label: &'static str, f: impl FnOnce(&dyn wifi::WifiBackend) -> Result<(), String> + Send + 'static) {
    let toast = toast_sig();
    toast.set(Some(format!("{label}…")));
    let choice = backend_choice();
    std::thread::spawn(move || {
        let r = match wifi::backend(&choice) {
            Some(b) => f(b.as_ref()),
            None => Err("нет службы Wi-Fi".into()),
        };
        run_on_main_thread(move || {
            toast.set(Some(match r {
                Ok(()) => format!("{label}: готово"),
                Err(e) => format!("{label}: {e}"),
            }));
            refresh();
        });
    });
}

fn signal_icon(s: u8) -> &'static str {
    match s {
        75.. => "\u{E1D8}",
        50..=74 => "\u{EBE1}",
        25..=49 => "\u{EBD6}",
        _ => "\u{EBE4}",
    }
}

/// Содержимое окна «Сеть».
pub fn view(ctx: ShellCtx) -> impl Widget {
    crate::system::start(ctx);
    pass_for().set(None);
    open_for().set(None);
    toast_sig().set(None);
    refresh();
    start_polling();
    let phone = ctx.is_phone();
    Column::new()
        .gap(8.0)
        .child(Text::new("Сеть").class("popup-title"))
        .child(move || status_row(ctx))
        .child(rx(move || wifi_section(phone)))
        .child(rx(|| match toast_sig().get() {
            Some(t) => Box::new(Text::new(t).max_lines(2).class("net-toast")) as Box<dyn Widget>,
            None => Box::new(DecoratedBox::new().class("net-toast-none")),
        }))
        .child(crate::popup::menu_item(mi::SETTINGS, "Параметры сети…", move || {
            if !phone {
                for (bin, cmd) in [("nm-connection-editor", "nm-connection-editor"), ("kcmshell6", "kcmshell6 kcm_networkmanagement")] {
                    if crate::actions::which(bin) {
                        crate::actions::spawn(cmd);
                        return;
                    }
                }
            }
            crate::actions::spawn("synsettings wifi");
        }))
}

/// Текущее подключение (проводное или Wi-Fi) — из общего состояния сети.
fn status_row(ctx: ShellCtx) -> impl Widget {
    let n = ctx.network.get();
    let status = if n.online {
        let what = match n.kind.as_str() {
            "ethernet" => "Проводная сеть",
            "wifi" => "Wi-Fi",
            _ => "Сеть",
        };
        match n.signal {
            Some(s) => format!("{what}: {} — {s}%", n.connection),
            None => format!("{what}: {}", n.connection),
        }
    } else {
        "Нет подключения".into()
    };
    Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(icon(crate::applets::network_glyph(&n)).class("popup-big-icon"))
        .child(Text::new(status).max_lines(2).class("popup-text"))
}

fn wifi_section(phone: bool) -> Box<dyn Widget> {
    let Some(r) = wifi_sig().get() else {
        return Box::new(Text::new("Чтение состояния Wi-Fi…").class("net-note"));
    };
    let st = match r {
        Ok(st) => st,
        Err(e) => return Box::new(Text::new(e).max_lines(3).class("net-note")),
    };
    if !st.present {
        return Box::new(Text::new(format!("Беспроводное устройство не найдено ({})", st.backend)).max_lines(2).class("net-note"));
    }
    let powered = st.powered;
    let mut col = Column::new().gap(6.0);
    // Wi-Fi: включить/выключить.
    let sub = match &st.connected {
        Some(n) => format!("Подключено к «{n}»{}", st.ip.as_ref().map(|i| format!(" · {i}")).unwrap_or_default()),
        None if powered => "Не подключено".to_string(),
        None => "Выключен".to_string(),
    };
    col = col.child(
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(icon(mi::WIFI).class("net-icon"))
            .child(Column::new().gap(1.0).child(Text::new("Wi-Fi").class("net-ssid")).child(Text::new(sub).max_lines(1).class("net-hint")).class("grow"))
            .child(Toggle::with_state(powered).on_change(move |on| wifi_do(if on { "Включение Wi-Fi" } else { "Выключение Wi-Fi" }, move |b| b.set_powered(on))))
            .class("net-row net-row-static"),
    );
    if !powered {
        return Box::new(col);
    }
    // Заголовок списка и поиск сетей.
    col = col.child(
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Text::new("Сети").class("net-section"))
            .child(DecoratedBox::new().class("grow"))
            .child(
                InputArea::new(DecoratedBox::new().child(icon(mi::REFRESH).class("net-refresh-icon")).class("net-refresh"))
                    .pointer()
                    .on_click(|b, _, _| {
                        if b == MouseButton::Left {
                            wifi_do("Поиск сетей", |b| b.scan());
                        }
                    }),
            ),
    );
    let pass = pass_for();
    let open = open_for();
    let pass_now = pass.get();
    let open_now = open.get();
    let mut list = Column::new().gap(2.0);
    if st.networks.is_empty() {
        list = list.child(Text::new("Сетей не найдено — нажмите обновить").class("net-note"));
    }
    for n in &st.networks {
        let ssid = n.ssid.clone();
        let hint = if n.connected {
            format!("Подключено · {}%", n.strength)
        } else if n.known {
            format!("Сохранена · {}%", n.strength)
        } else {
            format!("{}%", n.strength)
        };
        let mut name_row = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(n.ssid.clone()).max_lines(1).class("net-ssid"));
        if n.secure() {
            name_row = name_row.child(icon(mi::LOCK).class("net-lock"));
        }
        let row = Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(icon(signal_icon(n.strength)).class(if n.connected { "net-icon net-icon-on" } else { "net-icon" }))
            .child(Column::new().gap(1.0).child(name_row).child(Text::new(hint).class("net-hint")).class("grow"))
            .class(if n.connected { "net-row net-row-connected" } else { "net-row" });
        let (connected, known, secure) = (n.connected, n.known, n.secure());
        let s_click = ssid.clone();
        list = list.child(InputArea::new(row).pointer().on_click(move |b, _, _| {
            if b != MouseButton::Left {
                return;
            }
            let s = s_click.clone();
            if connected || known {
                // Подключённая (сохранённая) сеть: раскрыть действия.
                open.set(if open.get_untracked().as_deref() == Some(s.as_str()) { None } else { Some(s) });
                pass.set(None);
            } else if secure {
                pass.set(Some(s));
                open.set(None);
                if phone {
                    crate::popup::request_keyboard();
                }
            } else {
                wifi_do("Подключение", move |b| b.connect(&s, None));
            }
        }));
        if open_now.as_deref() == Some(ssid.as_str()) {
            let mut actions = Row::new().gap(6.0).class("net-actions");
            if connected {
                actions = actions.child(action_button("Отключить", || wifi_do("Отключение", |b| b.disconnect())));
            } else {
                let s = ssid.clone();
                actions = actions.child(action_button("Подключить", move || {
                    let s = s.clone();
                    open_for().set(None);
                    wifi_do("Подключение", move |b| b.connect(&s, None));
                }));
            }
            let s = ssid.clone();
            actions = actions.child(action_button("Забыть", move || {
                let s = s.clone();
                open_for().set(None);
                wifi_do("Забыть сеть", move |b| b.forget(&s));
            }));
            list = list.child(actions);
        }
        if pass_now.as_deref() == Some(ssid.as_str()) {
            let text = use_signal(String::new());
            let s = ssid.clone();
            let connect = move || {
                let p = text.get_untracked();
                let s = s.clone();
                pass_for().set(None);
                wifi_do("Подключение", move |b| b.connect(&s, Some(&p)));
            };
            let c2 = connect.clone();
            list = list.child(
                Row::new()
                    .gap(6.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(
                        TextField::new()
                            .obscure(true)
                            .autofocus(true)
                            .placeholder("Пароль сети")
                            .on_change(move |t| text.set(t.to_string()))
                            .on_submit(move |_| c2())
                            .class("search-field grow net-pass"),
                    )
                    .child(action_button("Подключить", connect))
                    .class("net-actions"),
            );
        }
    }
    // Длинный список — в прокрутке, чтобы окно не выросло выше экрана.
    let (_, oh) = crate::manager::output_size(None);
    let max_h = if phone { (oh * 0.4).max(160.0) } else { 300.0 };
    let approx = st.networks.len() as f32 * 46.0 + 60.0;
    if approx > max_h {
        col = col.child(ScrollView::new().vertical().child(list).style("height", StyleValue::px(max_h)).class("net-list"));
    } else {
        col = col.child(list);
    }
    Box::new(col)
}

fn action_button(label: &str, f: impl Fn() + Send + Sync + 'static) -> impl Widget {
    InputArea::new(DecoratedBox::new().child(Text::new(label.to_string()).class("net-btn-label")).class("net-btn")).pointer().on_click(move |b, _, _| {
        if b == MouseButton::Left {
            f();
        }
    })
}
