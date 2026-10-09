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
    /// Подключённая сеть с раскрытыми действиями (отключить, забыть).
    static OPEN_FOR: Cell<Option<RwSignal<Option<String>>>> = const { Cell::new(None) };
    /// Итог последнего действия.
    static TOAST: Cell<Option<RwSignal<Option<String>>>> = const { Cell::new(None) };
    /// Службы iwd и NetworkManager: установлены ли, запущены ли.
    static SERVICES: Cell<Option<RwSignal<Vec<wifi::Service>>>> = const { Cell::new(None) };
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
fn open_for() -> RwSignal<Option<String>> {
    sig(&OPEN_FOR, || None)
}
fn toast_sig() -> RwSignal<Option<String>> {
    sig(&TOAST, || None)
}
fn services_sig() -> RwSignal<Vec<wifi::Service>> {
    sig(&SERVICES, Vec::new)
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
    let sv = services_sig();
    let choice = backend_choice();
    std::thread::spawn(move || {
        let r = match wifi::backend(&choice) {
            Some(b) => b.state(),
            None => Err(t!("Служба Wi-Fi не запущена").into()),
        };
        let services = wifi::services();
        run_on_main_thread(move || {
            s.set(Some(r));
            if sv.get_untracked() != services {
                sv.set(services);
            }
        });
    });
}

/// Запустить (`systemctl enable --now`) или остановить службу — в фоне.
fn service_do(unit: &'static str, name: &'static str, start: bool) {
    let toast = toast_sig();
    let label = if start { t!("Запуск {name}", name = name) } else { t!("Остановка {name}", name = name) };
    toast.set(Some(format!("{label}…")));
    std::thread::spawn(move || {
        let r = wifi::service_control(unit, start);
        // Службе нужно время подняться, прежде чем читать состояние.
        std::thread::sleep(Duration::from_millis(if start { 1500 } else { 300 }));
        run_on_main_thread(move || {
            toast.set(Some(match r {
                Ok(()) => t!("{label}: готово", label = label),
                Err(e) => format!("{label}: {e}"),
            }));
            refresh();
        });
    });
}

/// Включить DHCP в iwd — в фоне.
fn iwd_dhcp_do() {
    let toast = toast_sig();
    let label = t!("Включение DHCP в iwd");
    toast.set(Some(format!("{label}…")));
    std::thread::spawn(move || {
        let r = wifi::enable_iwd_dhcp();
        std::thread::sleep(Duration::from_millis(1500));
        run_on_main_thread(move || {
            toast.set(Some(match r {
                Ok(()) => t!("{label}: готово", label = label),
                Err(e) => format!("{label}: {e}"),
            }));
            refresh();
        });
    });
}

/// Службы Wi-Fi: `all` — все установленные (с кнопкой «Запустить» /
/// «Остановить»), иначе только работающие (с «Остановить»).
fn services_view(all: bool) -> Box<dyn Widget> {
    let services = services_sig().get();
    let shown: Vec<&wifi::Service> = services.iter().filter(|s| s.installed && (all || s.active)).collect();
    if shown.is_empty() {
        if all {
            return Box::new(Text::new(t!("Службы Wi-Fi не установлены: нужен пакет iwd или networkmanager")).max_lines(2).class("net-note"));
        }
        return Box::new(DecoratedBox::new().class("net-toast-none"));
    }
    let mut col = Column::new().gap(2.0);
    for s in shown {
        let (unit, name, active, no_dhcp) = (s.unit, s.name, s.active, s.no_dhcp);
        let hint = match (active, s.enabled) {
            _ if no_dhcp => t!("Работает, но адрес не получает: DHCP выключен"),
            (true, true) => t!("Работает, включена при загрузке"),
            (true, false) => t!("Работает"),
            (false, true) => t!("Остановлена, включена при загрузке"),
            (false, false) => t!("Остановлена"),
        };
        col = col.child(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(icon(if active { mi::WIFI } else { mi::WIFI_OFF }).class(if active { "net-icon net-icon-on" } else { "net-icon" }))
                .child(Column::new().gap(1.0).child(Text::new(t!("Служба {name}", name = name)).class("net-ssid")).child(Text::new(hint).class("net-hint")).class("grow"))
                .child(if no_dhcp {
                    Box::new(action_button(n_!("Включить DHCP"), iwd_dhcp_do)) as Box<dyn Widget>
                } else {
                    Box::new(action_button(if active { n_!("Остановить") } else { n_!("Запустить") }, move || service_do(unit, name, !active)))
                })
                .class("net-row net-row-static"),
        );
    }
    Box::new(col)
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
            refresh();
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
    toast.set(Some(format!("{}…", syngui::i18n::t(label))));
    let choice = backend_choice();
    std::thread::spawn(move || {
        let r = match wifi::backend(&choice) {
            Some(b) => f(b.as_ref()),
            None => Err(t!("нет службы Wi-Fi").into()),
        };
        run_on_main_thread(move || {
            toast.set(Some(match r {
                Ok(()) => t!("{label}: готово", label = label),
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
    open_for().set(None);
    toast_sig().set(None);
    refresh();
    start_polling();
    let phone = ctx.is_phone();
    Column::new()
        .gap(8.0)
        .child(Text::new(t!("Сеть")).class("popup-title"))
        .child(move || status_row(ctx))
        .child(rx(move || wifi_section(phone)))
        .child(rx(|| match toast_sig().get() {
            Some(t) => Box::new(Text::new(t).max_lines(2).class("net-toast")) as Box<dyn Widget>,
            None => Box::new(DecoratedBox::new().class("net-toast-none")),
        }))
        .child(crate::popup::menu_item(mi::SETTINGS, t!("Параметры сети…"), move || {
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
            "ethernet" => t!("Проводная сеть"),
            "wifi" => "Wi-Fi".into(),
            _ => t!("Сеть"),
        };
        match n.signal {
            Some(s) => format!("{what}: {} — {s}%", n.connection),
            None => format!("{what}: {}", n.connection),
        }
    } else {
        t!("Нет подключения").into()
    };
    Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(icon(crate::applets::network_glyph(&n)).class("popup-big-icon"))
        .child(Text::new(status).max_lines(2).class("popup-text"))
}

fn wifi_section(phone: bool) -> Box<dyn Widget> {
    let Some(r) = wifi_sig().get() else {
        return Box::new(Text::new(t!("Чтение состояния Wi-Fi…")).class("net-note"));
    };
    let st = match r {
        Ok(st) => st,
        // Ни iwd, ни NetworkManager не работают: предложить запустить.
        Err(e) => return Box::new(Column::new().gap(6.0).child(Text::new(e).max_lines(3).class("net-note")).child(services_view(true))),
    };
    if !st.present {
        return Box::new(
            Column::new()
                .gap(6.0)
                .child(Text::new(t!("Беспроводное устройство не найдено ({backend})", backend = st.backend)).max_lines(2).class("net-note"))
                .child(services_view(false)),
        );
    }
    let powered = st.powered;
    let mut col = Column::new().gap(6.0).child(services_view(false));
    // Wi-Fi: включить/выключить.
    let sub = match &st.connected {
        Some(n) => t!("Подключено к «{n}»{v}", n = n, v = st.ip.as_ref().map(|i| format!(" · {i}")).unwrap_or_default()),
        None if powered && st.status.contains("connecting") => t!("Подключение…").to_string(),
        None if powered && st.status == "unavailable" => t!("Интерфейс недоступен").to_string(),
        None if powered => t!("Не подключено").to_string(),
        None => t!("Выключен").to_string(),
    };
    col = col.child(
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(icon(mi::WIFI).class("net-icon"))
            .child(Column::new().gap(1.0).child(Text::new("Wi-Fi").class("net-ssid")).child(Text::new(sub).max_lines(1).class("net-hint")).class("grow"))
            .child(Toggle::with_state(powered).on_change(move |on| wifi_do(if on { n_!("Включение Wi-Fi") } else { n_!("Выключение Wi-Fi") }, move |b| b.set_powered(on))))
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
            .child(Text::new(t!("Сети")).class("net-section"))
            .child(DecoratedBox::new().class("grow"))
            .child(
                InputArea::new(DecoratedBox::new().child(icon(mi::REFRESH).class("net-refresh-icon")).class("net-refresh"))
                    .pointer()
                    .on_click(|b, _, _| {
                        if b == MouseButton::Left {
                            wifi_do(n_!("Поиск сетей"), |b| b.scan());
                        }
                    }),
            ),
    );
    let open = open_for();
    let open_now = open.get();
    let mut list = Column::new().gap(2.0);
    if st.networks.is_empty() {
        list = list.child(Text::new(t!("Сетей не найдено — нажмите обновить")).class("net-note"));
    }
    for n in &st.networks {
        let ssid = n.ssid.clone();
        let hint = if n.connected {
            t!("Подключено · {strength}%", strength = n.strength)
        } else if n.known {
            t!("Сохранена · {strength}%", strength = n.strength)
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
            } else if secure {
                ShellCtx::get().open_popup(PopupKind::WifiConnect { ssid: s }, crate::commands::centered());
            } else {
                wifi_do(n_!("Подключение"), move |b| b.connect(&s, None));
            }
        }));
        if open_now.as_deref() == Some(ssid.as_str()) {
            let mut actions = Row::new().gap(6.0).class("net-actions");
            if connected {
                actions = actions.child(action_button(&t!("Отключить"), || wifi_do(n_!("Отключение"), |b| b.disconnect())));
            } else {
                let s = ssid.clone();
                actions = actions.child(action_button(&t!("Подключить"), move || {
                    let s = s.clone();
                    open_for().set(None);
                    wifi_do(n_!("Подключение"), move |b| b.connect(&s, None));
                }));
            }
            if secure {
                // Пароль изменился — ввести заново в окне подключения.
                let s = ssid.clone();
                actions = actions.child(action_button(&t!("Пароль…"), move || {
                    ShellCtx::get().open_popup(PopupKind::WifiConnect { ssid: s.clone() }, crate::commands::centered());
                }));
            }
            let s = ssid.clone();
            actions = actions.child(action_button(&t!("Забыть"), move || {
                let s = s.clone();
                open_for().set(None);
                wifi_do(n_!("Забыть сеть"), move |b| b.forget(&s));
            }));
            list = list.child(actions);
        }
    }
    // Длинный список — в прокрутке, чтобы окно не выросло выше экрана
    // (высота — у обёртки: ScrollView сам её не задаёт).
    let (_, oh) = crate::manager::output_size(None);
    let max_h = if phone { (oh * 0.4).max(160.0) } else { 300.0 };
    let approx = st.networks.len() as f32 * 46.0 + 60.0;
    if approx > max_h {
        col = col.child(DecoratedBox::new().child(ScrollView::new().vertical().child(list).class("net-list")).style("height", StyleValue::px(max_h)));
    } else {
        col = col.child(list);
    }
    Box::new(col)
}

// ─── Окно подключения ────────────────────────────────────────────────────────

#[derive(Clone, PartialEq)]
enum ConnState {
    Idle,
    Connecting,
    Done(String),
    Failed(String),
}

/// Понятное объяснение ошибки nmcli/iwctl.
fn humanize(e: &str) -> String {
    let l = e.to_lowercase();
    if l.contains("secrets were required") || l.contains("wireless-security") || l.contains("invalid") || l.contains("psk") || l.contains("wrong") || l.contains("not authorized") {
        t!("Неверный пароль (или сеть отклонила подключение)").into()
    } else if l.contains("no network with ssid") || l.contains("not found") {
        t!("Сеть не найдена — обновите список").into()
    } else if l.contains("timeout") || l.contains("timed out") {
        t!("Истекло время ожидания ответа сети").into()
    } else {
        e.trim().trim_start_matches("Error: ").to_string()
    }
}

/// Окно подключения к сети: пароль (с показом), «Подключить», статус
/// подключения и ошибка; по успеху — обратно к списку сетей.
pub fn connect_view(ctx: ShellCtx, ssid: String) -> impl Widget {
    let phone = ctx.is_phone();
    if phone {
        crate::popup::request_keyboard();
    }
    let pass = use_signal(String::new());
    let show = use_signal(false);
    let state = use_signal(ConnState::Idle);
    let known = wifi_sig().get_untracked().and_then(|r| r.ok()).and_then(|st| st.networks.into_iter().find(|n| n.ssid == ssid));
    let hint = match &known {
        Some(n) if n.security == "8021x" => t!("Корпоративная сеть (802.1X) — имя пользователя запросит служба"),
        Some(n) if n.secure() => t!("Защищённая сеть — введите пароль"),
        Some(_) => t!("Открытая сеть"),
        None => t!("Введите пароль сети"),
    };
    let ssid_c = ssid.clone();
    // Неудачная попытка к новой сети не должна оставлять «сохранённую» сеть с
    // неверным паролем (NetworkManager создаёт профиль до проверки).
    let was_known = known.as_ref().is_some_and(|n| n.known);
    let connect = {
        let ssid = ssid.clone();
        move || {
            if state.get_untracked() == ConnState::Connecting {
                return;
            }
            let p = pass.get_untracked();
            state.set(ConnState::Connecting);
            let choice = backend_choice();
            let ssid = ssid.clone();
            std::thread::spawn(move || {
                let r = match wifi::backend(&choice) {
                    Some(b) => {
                        let r = b.connect(&ssid, Some(&p));
                        if r.is_err() && !was_known {
                            let _ = b.forget(&ssid);
                        }
                        r
                    }
                    None => Err(t!("нет службы Wi-Fi").into()),
                };
                run_on_main_thread(move || {
                    refresh();
                    match r {
                        Ok(()) => {
                            state.set(ConnState::Done(t!("Подключено к «{ssid}»", ssid = ssid)));
                            toast_sig().set(Some(t!("Подключено к «{ssid}»", ssid = ssid)));
                            // Показать итог и вернуться к списку сетей.
                            let ssid2 = ssid.clone();
                            syngui_layer::add_timer(Duration::from_millis(1400), move || {
                                let ctx = ShellCtx::get();
                                let still = ctx.popup.get_untracked().is_some_and(|p| matches!(&p.kind, PopupKind::WifiConnect { ssid: s } if *s == ssid2));
                                if still {
                                    ctx.open_popup(PopupKind::Network, crate::commands::centered());
                                }
                                None
                            });
                        }
                        Err(e) => state.set(ConnState::Failed(humanize(&e))),
                    }
                });
            });
        }
    };
    let connect_submit = connect.clone();
    let connect_btn = connect.clone();
    Column::new()
        .gap(10.0)
        .child(Text::new(format!("Wi-Fi: {ssid_c}")).max_lines(1).class("popup-title"))
        .child(Text::new(hint).max_lines(2).class("net-note"))
        .child(rx(move || {
            let visible = show.get();
            let busy = state.get() == ConnState::Connecting;
            Box::new(
                Row::new()
                    .gap(6.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(
                        TextField::new()
                            .text(pass.get_untracked())
                            .obscure(!visible)
                            .disabled(busy)
                            .autofocus(true)
                            .placeholder(t!("Пароль"))
                            .on_change(move |t| pass.set(t.to_string()))
                            .on_submit({
                                let c = connect_submit.clone();
                                move |_| c()
                            })
                            .class("search-field grow net-pass"),
                    )
                    .child(
                        InputArea::new(DecoratedBox::new().child(icon(if visible { "\u{E8F5}" } else { "\u{E8F4}" }).class("net-refresh-icon")).class("net-refresh"))
                            .pointer()
                            .on_click(move |b, _, _| {
                                if b == MouseButton::Left {
                                    show.set(!show.get_untracked());
                                }
                            }),
                    ),
            ) as Box<dyn Widget>
        }))
        .child(rx(move || match state.get() {
            ConnState::Idle => Box::new(DecoratedBox::new().class("net-toast-none")) as Box<dyn Widget>,
            ConnState::Connecting => Box::new(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(CircularProgress::new().indeterminate().size(18.0).stroke_width(2.0).class("net-spinner"))
                    .child(Text::new(t!("Подключение…")).class("net-status")),
            ),
            // Текст — в растягиваемой колонке, иначе строка не переносится и уходит за край.
            ConnState::Done(t) => Box::new(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon("\u{E86C}").class("net-ok-icon")).child(Column::new().child(Text::new(t).max_lines(2).class("net-ok")).class("grow"))),
            ConnState::Failed(t) => Box::new(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon("\u{E000}").class("net-error-icon")).child(Column::new().child(Text::new(t).max_lines(4).class("net-error")).class("grow"))),
        }))
        .child(
            Row::new()
                .gap(8.0)
                .main_axis_alignment(MainAxisAlignment::End)
                .child(action_button(&t!("Отмена"), || {
                    ShellCtx::get().open_popup(PopupKind::Network, crate::commands::centered());
                }))
                .child(InputArea::new(DecoratedBox::new().child(Text::new(t!("Подключить")).class("net-btn-primary-label")).class("net-btn net-btn-primary")).pointer().on_click(move |b, _, _| {
                    if b == MouseButton::Left {
                        connect_btn();
                    }
                })),
        )
}

fn action_button(label: &str, f: impl Fn() + Send + Sync + 'static) -> impl Widget {
    InputArea::new(DecoratedBox::new().child(Text::new(syngui::i18n::t(label)).class("net-btn-label")).class("net-btn")).pointer().on_click(move |b, _, _| {
        if b == MouseButton::Left {
            f();
        }
    })
}
