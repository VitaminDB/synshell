//! Wi-Fi и Bluetooth. Состояние перечитывается раз в 3 с, пока страница
//! открыта; действия — в фоне, результат — подсказкой внизу.

use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use std::result::Result;
use synsystem::bluetooth::{BtState, Bluetooth};
use synsystem::wifi::{self, WifiState};

use crate::state;
use crate::store;
use crate::ui::*;

thread_local! {
    static WIFI: std::cell::Cell<Option<RwSignal<Option<Result<WifiState, String>>>>> = const { std::cell::Cell::new(None) };
    static BT: std::cell::Cell<Option<RwSignal<Option<Result<BtState, String>>>>> = const { std::cell::Cell::new(None) };
    /// Сеть, для которой раскрыто поле пароля.
    static PASS_FOR: std::cell::Cell<Option<RwSignal<Option<String>>>> = const { std::cell::Cell::new(None) };
    /// Службы iwd и NetworkManager.
    static SERVICES: std::cell::Cell<Option<RwSignal<Vec<wifi::Service>>>> = const { std::cell::Cell::new(None) };
}

fn services_sig() -> RwSignal<Vec<wifi::Service>> {
    SERVICES.with(|c| match c.get() {
        Some(s) => s,
        None => {
            let s = use_signal(Vec::new());
            c.set(Some(s));
            s
        }
    })
}

fn backend_choice() -> String {
    store::config().wifi.backend.clone()
}

fn refresh_wifi(sig: RwSignal<Option<Result<WifiState, String>>>) {
    let choice = backend_choice();
    let sv = services_sig();
    std::thread::spawn(move || {
        let r = match wifi::backend(&choice) {
            Some(b) => b.state(),
            None => Err(t!("Служба Wi-Fi не запущена — запустите iwd или NetworkManager ниже").into()),
        };
        let services = wifi::services();
        run_on_main_thread(move || {
            sig.set(Some(r));
            if sv.get_untracked() != services {
                sv.set(services);
            }
        });
    });
}

/// Запустить (`systemctl enable --now`) или остановить службу — в фоне,
/// затем перечитать состояние Wi-Fi.
fn service_do(unit: &'static str, name: &'static str, start: bool) {
    let sig = wifi_sig();
    let label = if start { t!("Запуск {name}", name = name) } else { t!("Остановка {name}", name = name) };
    state::toast(format!("{label}…"));
    std::thread::spawn(move || {
        let r = wifi::service_control(unit, start);
        std::thread::sleep(std::time::Duration::from_millis(if start { 1500 } else { 300 }));
        run_on_main_thread(move || {
            state::toast(match r {
                Ok(()) => t!("{label}: готово", label = label),
                Err(e) => format!("{label}: {e}"),
            });
            refresh_wifi(sig);
        });
    });
}

/// Включить DHCP в iwd — в фоне, затем перечитать состояние Wi-Fi.
fn iwd_dhcp_do() {
    let sig = wifi_sig();
    let label = t!("Включение DHCP в iwd");
    state::toast(format!("{label}…"));
    std::thread::spawn(move || {
        let r = wifi::enable_iwd_dhcp();
        std::thread::sleep(std::time::Duration::from_millis(1500));
        run_on_main_thread(move || {
            state::toast(match r {
                Ok(()) => t!("{label}: готово", label = label),
                Err(e) => format!("{label}: {e}"),
            });
            refresh_wifi(sig);
        });
    });
}

/// Строки служб: установленные — с кнопкой «Запустить» / «Остановить».
fn service_rows() -> W {
    let sv = services_sig();
    boxed(Reactive::new(move || -> Vec<W> {
        let services = sv.get();
        let installed: Vec<&wifi::Service> = services.iter().filter(|s| s.installed).collect();
        if installed.is_empty() {
            return vec![note(&t!("Службы не установлены: нужен пакет iwd или networkmanager (см. «Программы»)."))];
        }
        installed
            .into_iter()
            .map(|s| {
                let (unit, name, active) = (s.unit, s.name, s.active);
                if s.no_dhcp {
                    return row_inline(name, t!("Работает, но адрес не получает: DHCP выключен"), button(t!("Включить DHCP"), iwd_dhcp_do));
                }
                let hint = match (active, s.enabled) {
                    (true, true) => t!("Работает, включена при загрузке"),
                    (true, false) => t!("Работает"),
                    (false, true) => t!("Остановлена, включена при загрузке"),
                    (false, false) => t!("Остановлена (запуск — systemctl enable --now)"),
                };
                row_inline(name, hint, button(if active { t!("Остановить") } else { t!("Запустить") }, move || service_do(unit, name, !active)))
            })
            .collect()
    }))
}

fn wifi_sig() -> RwSignal<Option<Result<WifiState, String>>> {
    WIFI.with(|w| match w.get() {
        Some(s) => s,
        None => {
            let s = use_signal(None);
            w.set(Some(s));
            // Опрос, пока открыта страница.
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(3));
                run_on_main_thread(move || {
                    // Пока вводят пароль — не перестраивать страницу.
                    let typing = PASS_FOR.with(|p| p.get()).is_some_and(|p| p.get_untracked().is_some());
                    if state::ctx().page.get_untracked() == "wifi" && !typing {
                        refresh_wifi(s);
                    }
                });
            });
            s
        }
    })
}

/// Действие с Wi-Fi в фоне, затем перечитать.
fn wifi_do(label: impl AsRef<str>, f: impl FnOnce(&dyn wifi::WifiBackend) -> Result<(), String> + Send + 'static) {
    let label = tl(label);
    let sig = wifi_sig();
    let choice = backend_choice();
    state::toast(format!("{label}…"));
    std::thread::spawn(move || {
        let r = match wifi::backend(&choice) {
            Some(b) => f(b.as_ref()),
            None => Err(t!("нет службы Wi-Fi").into()),
        };
        run_on_main_thread(move || {
            state::toast(match r {
                Ok(()) => t!("{label}: готово", label = label),
                Err(e) => format!("{label}: {e}"),
            });
            refresh_wifi(sig);
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

pub fn wifi() -> W {
    let sig = wifi_sig();
    refresh_wifi(sig);
    let pass_for = PASS_FOR.with(|p| match p.get() {
        Some(s) => s,
        None => {
            let s = use_signal(None);
            p.set(Some(s));
            s
        }
    });
    let body = Reactive::new(move || -> Vec<W> {
        let Some(r) = sig.get() else { return vec![note(&t!("Чтение состояния Wi-Fi…"))] };
        let st = match r {
            Ok(st) => st,
            Err(e) => return vec![note(&e)],
        };
        if !st.present {
            return vec![note(&t!("Беспроводное устройство не найдено ({backend}). На телефоне Wi-Fi требует драйвера и прошивки.", backend = st.backend))];
        }
        let mut out: Vec<W> = Vec::new();
        let powered = st.powered;
        let status = match &st.connected {
            Some(n) => t!("Подключено к «{n}»{v}", n = n, v = st.ip.as_ref().map(|i| format!(" · {i}")).unwrap_or_default()),
            None => t!("Не подключено · {status}", status = st.status),
        };
        out.push(group(
            "",
            vec![
                row_inline(
                    "Wi-Fi",
                    &format!("{status} · {}", st.backend),
                    Toggle::with_state(powered).on_change(move |on| wifi_do(if on { t!("Включение") } else { t!("Выключение") }, move |b| b.set_powered(on))),
                ),
                row_inline(&t!("Найти сети"), "", button(&t!("Обновить"), || wifi_do(&t!("Поиск сетей"), |b| b.scan()))),
            ],
        ));
        if powered {
            let open_for = pass_for.get();
            let mut rows: Vec<W> = Vec::new();
            for n in st.networks {
                let ssid = n.ssid.clone();
                let lock = if n.secure() { " 🔒" } else { "" };
                let hint = if n.connected {
                    t!("Подключено").to_string()
                } else if n.known {
                    t!("Сохранена").to_string()
                } else {
                    String::new()
                };
                let mut actions = Row::new().gap(6.0);
                if n.connected {
                    actions = actions.child(button(&t!("Отключить"), || wifi_do(&t!("Отключение"), |b| b.disconnect())));
                } else {
                    let s2 = ssid.clone();
                    let needs_pass = n.secure() && !n.known;
                    actions = actions.child(button(&t!("Подключить"), move || {
                        if needs_pass {
                            pass_for.set(Some(s2.clone()));
                        } else {
                            let s3 = s2.clone();
                            wifi_do(&t!("Подключение"), move |b| b.connect(&s3, None));
                        }
                    }));
                }
                if n.known {
                    let s2 = ssid.clone();
                    actions = actions.child(icon_button(icons::DELETE, move || {
                        let s3 = s2.clone();
                        wifi_do(&t!("Забыть сеть"), move |b| b.forget(&s3));
                    }));
                }
                let label = format!("{}{lock}", n.ssid);
                let row_w = boxed(
                    Row::new()
                        .gap(12.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .class("setting-row")
                        .child(Icon::new(signal_icon(n.strength)).class("wifi-signal"))
                        .child(Column::new().gap(2.0).class("row-text").child(Text::new(label).class("row-label")).child(Text::new(format!("{hint} {}%", n.strength)).class("row-hint")))
                        .child(actions),
                );
                rows.push(row_w);
                if open_for.as_deref() == Some(ssid.as_str()) {
                    let pass = use_signal(String::new());
                    let s2 = ssid.clone();
                    let connect = move || {
                        let p = pass.get_untracked();
                        let s3 = s2.clone();
                        pass_for.set(None);
                        wifi_do(&t!("Подключение"), move |b| b.connect(&s3, Some(&p)));
                    };
                    let c2 = connect.clone();
                    rows.push(boxed(
                        Row::new()
                            .gap(8.0)
                            .class("setting-row")
                            .child(TextField::new().obscure(true).autofocus(true).placeholder(t!("Пароль сети")).on_change(move |t| pass.set(t.to_string())).on_submit(move |_| c2()).class("grow"))
                            .child(primary_button(&t!("Подключить"), connect)),
                    ));
                }
            }
            if rows.is_empty() {
                rows.push(note(&t!("Сетей не найдено — нажмите «Обновить».")));
            }
            out.push(group(&t!("Сети"), rows));
        }
        let mut col = Column::new().gap(18.0);
        for w in out {
            col = col.child(w);
        }
        vec![boxed(col)]
    });
    let c = store::config();
    page(
        "Wi-Fi",
        &t!("Беспроводные сети. Служба — iwd или NetworkManager."),
        vec![
            boxed(body),
            group(
                &t!("Служба"),
                vec![
                    choice_row(
                        t!("Бэкенд"),
                        t!("auto — NetworkManager, если запущен, иначе iwd"),
                        crate::op!["wifi", "backend"],
                        &c.wifi.backend,
                        &[("auto", n_!("Автоматически")), ("iwd", "iwd"), ("networkmanager", "NetworkManager")],
                    ),
                    service_rows(),
                ],
            ),
        ],
    )
}

// ─── Bluetooth ───────────────────────────────────────────────────────────────

fn refresh_bt(sig: RwSignal<Option<Result<BtState, String>>>) {
    std::thread::spawn(move || {
        let r = match Bluetooth::new() {
            Some(b) => b.state(),
            None => Err(t!("Служба Bluetooth (bluez) не запущена: systemctl enable --now bluetooth").into()),
        };
        run_on_main_thread(move || sig.set(Some(r)));
    });
}

fn bt_sig() -> RwSignal<Option<Result<BtState, String>>> {
    BT.with(|w| match w.get() {
        Some(s) => s,
        None => {
            let s = use_signal(None);
            w.set(Some(s));
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(3));
                run_on_main_thread(move || {
                    if state::ctx().page.get_untracked() == "bluetooth" {
                        refresh_bt(s);
                    }
                });
            });
            s
        }
    })
}

fn bt_do(label: impl AsRef<str>, f: impl FnOnce(&Bluetooth) -> Result<(), String> + Send + 'static) {
    let label = tl(label);
    let sig = bt_sig();
    state::toast(format!("{label}…"));
    std::thread::spawn(move || {
        let r = match Bluetooth::new() {
            Some(b) => f(&b),
            None => Err(t!("нет bluez").into()),
        };
        run_on_main_thread(move || {
            state::toast(match r {
                Ok(()) => t!("{label}: готово", label = label),
                Err(e) => format!("{label}: {e}"),
            });
            refresh_bt(sig);
        });
    });
}

fn bt_icon(icon: &str) -> &'static str {
    match icon {
        i if i.contains("audio") || i.contains("headset") || i.contains("headphone") => "\u{E310}",
        i if i.contains("keyboard") => "\u{E312}",
        i if i.contains("mouse") => "\u{E323}",
        i if i.contains("phone") => "\u{E325}",
        i if i.contains("computer") => "\u{E30A}",
        _ => "\u{E1A7}",
    }
}

pub fn bluetooth() -> W {
    let sig = bt_sig();
    refresh_bt(sig);
    let body = Reactive::new(move || -> Vec<W> {
        let Some(r) = sig.get() else { return vec![note(&t!("Чтение состояния Bluetooth…"))] };
        let st = match r {
            Ok(s) => s,
            Err(e) => return vec![note(&e)],
        };
        if !st.present {
            return vec![note(&t!("Адаптер Bluetooth не найден."))];
        }
        let mut col = Column::new().gap(18.0).child(group(
            "",
            vec![
                row_inline(
                    "Bluetooth",
                    &st.adapter.clone().unwrap_or_default(),
                    Toggle::with_state(st.powered).on_change(|on| bt_do(if on { n_!("Включение") } else { n_!("Выключение") }, move |b| b.set_powered(on))),
                ),
                row_inline(
                    &t!("Виден другим устройствам"),
                    "",
                    Toggle::with_state(st.discoverable).on_change(|on| bt_do(n_!("Видимость"), move |b| b.set_discoverable(on))),
                ),
                row_inline(
                    &t!("Поиск устройств"),
                    if st.discovering { n_!("Идёт поиск…") } else { "" },
                    Toggle::with_state(st.discovering).on_change(|on| bt_do(if on { n_!("Поиск") } else { n_!("Остановка поиска") }, move |b| b.discovery(on))),
                ),
            ],
        ));
        if st.powered {
            let (paired, other): (Vec<_>, Vec<_>) = st.devices.into_iter().partition(|d| d.paired);
            let dev_row = |d: synsystem::bluetooth::Device| -> W {
                let path = d.path.clone();
                let mut actions = Row::new().gap(6.0);
                if d.paired {
                    let p = path.clone();
                    actions = if d.connected {
                        actions.child(button(&t!("Отключить"), move || {
                            let p = p.clone();
                            bt_do(n_!("Отключение"), move |b| b.disconnect(&p));
                        }))
                    } else {
                        actions.child(button(&t!("Подключить"), move || {
                            let p = p.clone();
                            bt_do(n_!("Подключение"), move |b| b.connect(&p));
                        }))
                    };
                    let p = path.clone();
                    actions = actions.child(icon_button(icons::DELETE, move || {
                        let p = p.clone();
                        bt_do(n_!("Забыть устройство"), move |b| b.forget(&p));
                    }));
                } else {
                    let p = path.clone();
                    actions = actions.child(primary_button(&t!("Сопрячь"), move || {
                        let p = p.clone();
                        bt_do(n_!("Сопряжение"), move |b| b.pair(&p));
                    }));
                }
                let mut hint = if d.connected { t!("Подключено").to_string() } else { d.address.clone() };
                if let Some(bat) = d.battery {
                    hint.push_str(&t!(" · батарея {bat}%", bat = bat));
                }
                boxed(
                    Row::new()
                        .gap(12.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .class("setting-row")
                        .child(Icon::new(bt_icon(&d.icon)).class("wifi-signal"))
                        .child(Column::new().gap(2.0).class("row-text").child(Text::new(d.name.clone()).class("row-label")).child(Text::new(hint).class("row-hint")))
                        .child(actions),
                )
            };
            if !paired.is_empty() {
                col = col.child(group(&t!("Сопряжённые"), paired.into_iter().map(dev_row).collect()));
            }
            let others: Vec<W> = other.into_iter().filter(|d| !d.name.is_empty()).map(dev_row).collect();
            col = col.child(group(&t!("Доступные"), if others.is_empty() { vec![note(&t!("Включите «Поиск устройств» и переведите устройство в режим сопряжения."))] } else { others }));
        }
        vec![boxed(col)]
    });
    page("Bluetooth", &t!("Наушники, колонки, клавиатуры и другие устройства."), vec![boxed(body)])
}
