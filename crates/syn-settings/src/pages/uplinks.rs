//! Несколько подключений к интернету (служба synnetd): каналы и их скорость,
//! режим — основной канал или все сразу, основной канал, USB-модемы, выбор
//! для каждой программы. Состояние перечитывается раз в 2 с, пока страница
//! открыта.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::time::Instant;

use syn_netd::{AppPolicy, Config, Mode, Status};
use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use synshell_common::xdg::{self, DesktopEntry};

use crate::state;
use crate::ui::*;

const PAGE: &str = "uplinks";

type R = Option<std::result::Result<Status, String>>;

#[derive(Clone, Copy)]
struct Sigs {
    st: RwSignal<R>,
    /// Скорость каналов: (приём, отдача) байт/с.
    rates: RwSignal<HashMap<String, (f64, f64)>>,
    show_all: RwSignal<bool>,
    filter: RwSignal<String>,
}

thread_local! {
    static SIGS: Cell<Option<Sigs>> = const { Cell::new(None) };
    /// Прежние счётчики каналов — для скорости.
    static PREV: RefCell<HashMap<String, (u64, u64, Instant)>> = RefCell::new(HashMap::new());
}

fn sigs() -> Sigs {
    SIGS.with(|c| match c.get() {
        Some(s) => s,
        None => {
            let s = Sigs { st: use_signal(None), rates: use_signal(HashMap::new()), show_all: use_signal(false), filter: use_signal(String::new()) };
            c.set(Some(s));
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(2));
                run_on_main_thread(move || {
                    if state::ctx().page.get_untracked() == PAGE {
                        reload(s);
                    }
                });
            });
            s
        }
    })
}

fn err_text(e: std::io::Error) -> String {
    if !syn_netd::installed() {
        t!("Служба synnetd не установлена (пакет synshell-core).")
    } else if matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused) {
        t!("Служба synnetd не запущена.")
    } else {
        e.to_string()
    }
}

/// Без счётчиков байтов — чтобы сравнить, изменилось ли что-то ещё.
fn strip(r: &R) -> R {
    r.clone().map(|r| {
        r.map(|mut st| {
            for u in &mut st.uplinks {
                u.rx_bytes = 0;
                u.tx_bytes = 0;
            }
            st
        })
    })
}

fn set_status(s: Sigs, r: std::result::Result<Status, String>) {
    if let Ok(st) = &r {
        let now = Instant::now();
        let mut rates = HashMap::new();
        PREV.with(|p| {
            let mut p = p.borrow_mut();
            for u in &st.uplinks {
                if let Some((rx, tx, t)) = p.get(&u.iface) {
                    let dt = now.duration_since(*t).as_secs_f64().max(0.1);
                    rates.insert(u.iface.clone(), (u.rx_bytes.saturating_sub(*rx) as f64 / dt, u.tx_bytes.saturating_sub(*tx) as f64 / dt));
                }
                p.insert(u.iface.clone(), (u.rx_bytes, u.tx_bytes, now));
            }
        });
        s.rates.set(rates);
    }
    // Страницу перестраивать, только если изменилось что-то кроме счётчиков
    // (иначе закрывались бы раскрытые списки).
    let new = Some(r);
    if strip(&s.st.get_untracked()) != strip(&new) {
        s.st.set(new);
    }
}

fn reload(s: Sigs) {
    std::thread::spawn(move || {
        let r = syn_netd::status().map_err(err_text);
        run_on_main_thread(move || set_status(s, r));
    });
}

/// Поменять настройки службы.
fn change(f: impl FnOnce(&mut Config) + Send + 'static) {
    let s = sigs();
    let Some(Ok(st)) = s.st.get_untracked() else { return };
    let mut c = st.config.clone();
    f(&mut c);
    std::thread::spawn(move || {
        let r = syn_netd::set_config(c).map_err(|e| e.to_string());
        run_on_main_thread(move || match r {
            Ok(st) => set_status(s, Ok(st)),
            Err(e) => {
                state::toast(t!("Настройки сети: {e}", e = e));
                reload(s);
            }
        });
    });
}

fn start_service() {
    let s = sigs();
    state::toast(t!("Запуск synnetd…"));
    std::thread::spawn(move || {
        let r = syn_system::wifi::service_control("synnetd.service", true);
        std::thread::sleep(std::time::Duration::from_millis(800));
        run_on_main_thread(move || {
            if let Err(e) = r {
                state::toast(t!("Запуск synnetd: {e}", e = e));
            }
            reload(s);
        });
    });
}

fn speed(b: f64) -> String {
    if b >= 1e6 {
        format!("{:.1} {}", b / 1e6, t!("МБ/с"))
    } else if b >= 1e3 {
        format!("{:.0} {}", b / 1e3, t!("КБ/с"))
    } else {
        format!("{:.0} {}", b, t!("Б/с"))
    }
}

fn kind_name(kind: &str) -> String {
    match kind {
        "wifi" => "Wi-Fi".into(),
        "ethernet" => t!("Проводная сеть"),
        "usb" => t!("USB-модем"),
        "mobile" => t!("Мобильная сеть"),
        _ => t!("Сеть"),
    }
}

fn kind_icon(kind: &str) -> &'static str {
    match kind {
        "wifi" => icons::WIFI,
        "ethernet" => "\u{E8BE}",
        "usb" => icons::USB,
        "mobile" => "\u{E1C8}",
        _ => "\u{E894}",
    }
}

fn note_text(note: &str) -> Option<String> {
    Some(match note {
        "single" => t!("Подключён один канал — работает как обычно. Подключите второй (Wi-Fi, кабель, телефон по USB), и его можно будет использовать вместе с первым."),
        "none" => t!("Нет подключения к интернету."),
        "bpf" => t!("Ядро не дало загрузить BPF-программу — выбор канала для отдельных программ недоступен."),
        n if n.starts_with("vpn:") => t!("Включён VPN ({v}) — пока он работает, весь трафик идёт через него, служба не вмешивается.", v = &n[4..]),
        _ => return None,
    })
}

// ─── Программы ───────────────────────────────────────────────────────────────

/// Программы, которым полезны все каналы сразу: торренты и загрузчики.
const MULTI_FRIENDLY: &[&str] = &[
    "qbittorrent", "transmission-gtk", "transmission-qt", "deluge", "deluge-gtk", "ktorrent", "fragments", "tixati", "biglybt", "motrix", "aria2c", "jdownloader", "uget-gtk", "persepolis", "kget", "steam",
];

/// Имя процесса программы по её команде: `/usr/bin/qbittorrent %U` → `qbittorrent`.
fn exe_of(e: &DesktopEntry) -> String {
    let w = e.exec.split_whitespace().find(|w| !w.contains('=') && *w != "env").unwrap_or("");
    w.rsplit('/').next().unwrap_or(w).to_string()
}

fn menu_apps() -> Vec<DesktopEntry> {
    let mut v: Vec<DesktopEntry> = xdg::apps().iter().filter(|e| !e.no_display && e.android.is_none() && !exe_of(e).is_empty())
        // Ярлыки игр Steam и ссылок (`steam steam://rungameid/…`) — не сама программа.
        .filter(|e| !e.exec.contains("://"))
        .cloned()
        .collect();
    v.sort_by_key(|e| e.name.to_lowercase());
    let mut seen = std::collections::HashSet::new();
    v.retain(|e| seen.insert(exe_of(e)));
    v
}

fn app_for_exe(exe: &str) -> Option<DesktopEntry> {
    // Не ярлык игры Steam (`steam steam://…`) вместо самого Steam.
    xdg::apps().iter().find(|e| exe_of(e) == exe && !e.exec.contains("://")).cloned()
}

fn app_icon(e: Option<&DesktopEntry>) -> W {
    match e.and_then(|e| xdg::lookup_icon(&e.icon)) {
        Some(p) => boxed(Image::new(p.to_string_lossy()).placeholder(false).class("da-app-icon")),
        None => boxed(Icon::new(icons::APPS).class("da-app-icon da-icon-fallback")),
    }
}

fn app_row(e: Option<&DesktopEntry>, exe: &str, hint: String, control: W) -> W {
    let name = e.map(|e| e.name.clone()).unwrap_or_else(|| exe.to_string());
    boxed(
        DecoratedBox::new().class("da-item").child(
            Row::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(app_icon(e))
                .child(Column::new().gap(2.0).class("grow").child(Text::new(name).max_lines(1).class("da-title")).child(Text::new(hint).max_lines(1).class("row-hint")))
                .child(control),
        ),
    )
}

fn card(items: Vec<W>) -> W {
    let mut card = Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("group-card");
    for (n, w) in items.into_iter().enumerate() {
        if n > 0 {
            card = card.child(DecoratedBox::new().class("row-sep"));
        }
        card = card.child(w);
    }
    boxed(card)
}

fn titled(title: String, body: W) -> W {
    boxed(Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(Text::new(title).class("group-title")).child(body))
}

/// Выбор для программы: как в системе, основной, все каналы или конкретный
/// канал (каждый подключённый и выбранный раньше, если сейчас не подключён).
fn policy_dropdown(exe: String, cur: Option<AppPolicy>, uplinks: &[syn_netd::Uplink]) -> W {
    let mut dd = Dropdown::new()
        .width(260.0)
        .item(DropdownItem::new("system", t!("Как в системе")))
        .item(DropdownItem::new("primary", t!("Только основной канал")))
        .item(DropdownItem::new("both", t!("Все каналы сразу")));
    for u in uplinks {
        dd = dd.item(DropdownItem::new(format!("iface:{}", u.iface), t!("Только {k} · {i}", k = kind_name(&u.kind), i = u.iface)));
    }
    if let Some(AppPolicy::Iface(i)) = &cur {
        if !uplinks.iter().any(|u| u.iface == *i) {
            dd = dd.item(DropdownItem::new(format!("iface:{i}"), t!("Только {i} (не подключён)", i = i)));
        }
    }
    boxed(
        dd.selected(cur.map(|p| p.id()).unwrap_or_else(|| "system".into()))
            .on_change(move |v: &str| {
                let (exe, p) = (exe.clone(), AppPolicy::from_id(v));
                change(move |c| match p {
                    Some(p) => {
                        c.apps.insert(exe, p);
                    }
                    None => {
                        c.apps.remove(&exe);
                    }
                });
            }),
    )
}

fn add_row(e: &DesktopEntry) -> W {
    let exe = exe_of(e);
    let hint = [&e.generic_name, &e.comment].into_iter().find(|h| !h.is_empty() && **h != e.name).cloned().unwrap_or_else(|| t!("Процесс {p}", p = exe));
    let x = exe.clone();
    app_row(
        Some(e),
        &exe,
        hint,
        boxed(button(t!("Все каналы"), move || {
            let x = x.clone();
            change(move |c| {
                c.apps.insert(x, AppPolicy::Both);
            });
        })),
    )
}

// ─── Страница ────────────────────────────────────────────────────────────────

pub fn uplinks() -> W {
    let s = sigs();
    reload(s);
    let body = Reactive::new(move || -> Vec<W> {
        let Some(r) = s.st.get() else { return vec![note(t!("Чтение состояния…"))] };
        let st = match r {
            Ok(st) => st,
            Err(e) => {
                let mut rows = vec![note(e)];
                if syn_netd::installed() {
                    rows.push(boxed(Row::new().child(primary_button(t!("Запустить службу"), start_service))));
                }
                return vec![group(t!("Служба"), rows)];
            }
        };
        let c = st.config.clone();
        let mut out: Vec<W> = Vec::new();

        // Каналы.
        let mut rows: Vec<W> = st
            .uplinks
            .iter()
            .map(|u| {
                let mut hint = Vec::new();
                if u.primary {
                    hint.push(t!("основной"));
                }
                if let Some(a) = &u.ipv4 {
                    hint.push(a.clone());
                }
                if u.ipv6 {
                    hint.push("IPv6".into());
                }
                if u.dhcp_by_synnetd {
                    hint.push(t!("адрес получен службой"));
                }
                let iface = u.iface.clone();
                let rate = Reactive::new(move || -> Vec<W> {
                    let (rx, tx) = s.rates.get().get(&iface).copied().unwrap_or((0.0, 0.0));
                    vec![boxed(Text::new(format!("↓ {}  ↑ {}", speed(rx), speed(tx))).class("row-value"))]
                });
                boxed(
                    Row::new()
                        .gap(12.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .class("setting-row")
                        .child(Icon::new(kind_icon(&u.kind)).class("wifi-signal"))
                        .child(
                            Column::new()
                                .gap(2.0)
                                .class("row-text")
                                .child(Text::new(format!("{} · {}", kind_name(&u.kind), u.iface)).class("row-label"))
                                .child(Text::new(hint.join(" · ")).class("row-hint")),
                        )
                        .child(rate),
                )
            })
            .collect();
        let card_note = |t: String| -> W { boxed(DecoratedBox::new().class("setting-row").child(Text::new(t).class("row-hint"))) };
        if let Some(n) = st.note.as_deref().and_then(note_text) {
            rows.push(card_note(n));
        } else if st.active {
            rows.push(card_note(if c.mode == Mode::Both {
                t!("Все каналы работают одновременно: разные соединения расходятся по разным каналам.")
            } else {
                t!("Трафик идёт через основной канал, остальные — запасные. Программы с выбором «Все каналы сразу» используют все.")
            }));
        }
        out.push(group(t!("Каналы"), rows));

        // Как использовать.
        let mode = Dropdown::new()
            .width(260.0)
            .item(DropdownItem::new("primary", t!("Только основной канал")))
            .item(DropdownItem::new("both", t!("Все каналы сразу")))
            .selected(if c.mode == Mode::Both { "both" } else { "primary" }.to_string())
            .on_change(|v: &str| {
                let m = if v == "both" { Mode::Both } else { Mode::Primary };
                change(move |c| c.mode = m);
            });
        let mut pri = Dropdown::new().width(260.0).item(DropdownItem::new("", t!("Автоматически")));
        let mut names: Vec<(String, String)> = st.uplinks.iter().map(|u| (u.iface.clone(), format!("{} · {}", kind_name(&u.kind), u.iface))).collect();
        if !c.primary.is_empty() && !names.iter().any(|(i, _)| *i == c.primary) {
            names.push((c.primary.clone(), t!("{i} (не подключён)", i = c.primary)));
        }
        for (i, l) in names {
            pri = pri.item(DropdownItem::new(i, l));
        }
        let pri = pri.selected(c.primary.clone()).on_change(|v: &str| {
            let v = v.to_string();
            change(move |c| c.primary = v);
        });
        out.push(group(
            t!("Как использовать"),
            vec![
                row(
                    t!("Режим"),
                    t!("Все каналы сразу — соединения распределяются между Wi-Fi, кабелем и модемом: быстрее загрузки из многих источников (торренты, обновления). Одно соединение всегда идёт через один канал."),
                    mode,
                ),
                row(t!("Основной канал"), t!("Автоматически — по метрике маршрута: кабель, затем Wi-Fi, затем USB-модем."), pri),
                row_inline(
                    t!("Поднимать USB-модемы"),
                    t!("Телефон в режиме USB-модема получает адрес сам, если его не настраивает NetworkManager или другая служба"),
                    Toggle::with_state(c.usb_dhcp).on_change(|on| change(move |c| c.usb_dhcp = on)),
                ),
            ],
        ));

        // Телефоны по USB: подключённые сейчас и запомненные.
        let mut phones: Vec<W> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let usb_row = |key: String, name: String, hint: String, cur: Option<syn_netd::UsbChoice>| -> W {
            let label = name.clone();
            let dd = Dropdown::new()
                .width(260.0)
                .item(DropdownItem::new("ask", t!("Спрашивать при подключении")))
                .item(DropdownItem::new("modem", t!("Модем — интернет через телефон")))
                .item(DropdownItem::new("no", t!("Только зарядка и файлы")))
                .selected(match cur {
                    Some(syn_netd::UsbChoice::Modem) => "modem",
                    Some(syn_netd::UsbChoice::No) => "no",
                    None => "ask",
                }.to_string())
                .on_change(move |v: &str| {
                    let choice = match v {
                        "modem" => Some(syn_netd::UsbChoice::Modem),
                        "no" => Some(syn_netd::UsbChoice::No),
                        _ => None,
                    };
                    let (k, n) = (key.clone(), name.clone());
                    change(move |c| match choice {
                        Some(choice) => {
                            c.usb_devices.insert(k, syn_netd::UsbDecision { choice, name: n });
                        }
                        None => {
                            c.usb_devices.remove(&k);
                        }
                    });
                });
            row(label, hint, dd)
        };
        for m in &st.usb_modems {
            seen.insert(m.key.clone());
            let name = if m.name.is_empty() { m.iface.clone() } else { m.name.clone() };
            let hint = if m.managed_elsewhere {
                t!("Подключён · {i} · управляет NetworkManager", i = m.iface)
            } else if m.choice.is_none() {
                t!("Подключён · {i} · ждёт вашего выбора — интернет через него не идёт", i = m.iface)
            } else {
                t!("Подключён · {i}", i = m.iface)
            };
            phones.push(usb_row(m.key.clone(), name, hint, m.choice));
        }
        for (k, d) in &c.usb_devices {
            if seen.contains(k) {
                continue;
            }
            let name = if d.name.is_empty() { k.clone() } else { d.name.clone() };
            phones.push(usb_row(k.clone(), name, t!("Не подключён"), Some(d.choice)));
        }
        if !phones.is_empty() {
            out.push(group(t!("Телефоны по USB"), phones));
        }

        // Программы со своим выбором.
        let running: HashMap<String, u32> = st.running.iter().map(|r| (r.app.clone(), r.processes)).collect();
        let rows: Vec<W> = c
            .apps
            .iter()
            .map(|(exe, p)| {
                let e = app_for_exe(exe);
                let mut hint = match running.get(exe) {
                    Some(n) => t!("Запущена · процессов {n}", n = n),
                    None => t!("Процесс {p}", p = exe),
                };
                if let AppPolicy::Iface(i) = p {
                    if !st.uplinks.iter().any(|u| u.iface == *i) {
                        hint = t!("{h} · канал {i} не подключён — через основной", h = hint, i = i);
                    }
                }
                app_row(e.as_ref(), exe, hint, policy_dropdown(exe.clone(), Some(p.clone()), &st.uplinks))
            })
            .collect();
        out.push(titled(
            t!("Программы"),
            if rows.is_empty() { note(t!("Все программы идут как в системе. Добавьте программу ниже, чтобы выбрать для неё канал.")) } else { card(rows) },
        ));
        let friendly: Vec<W> = menu_apps().iter().filter(|e| MULTI_FRIENDLY.contains(&exe_of(e).as_str()) && !c.apps.contains_key(&exe_of(e))).map(add_row).collect();
        if !friendly.is_empty() {
            out.push(titled(t!("Полезно все каналы сразу"), card(friendly)));
        }
        vec![boxed(Column::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Stretch).children(out))]
    });
    // Остальные программы — по кнопке, с поиском (поле вне `Reactive`
    // списка, чтобы не терять фокус при наборе).
    let more = Reactive::new(move || -> Vec<W> {
        if !matches!(s.st.get(), Some(Ok(_))) {
            return vec![];
        }
        if !s.show_all.get() {
            return vec![boxed(Row::new().child(button(t!("Добавить другую программу"), move || s.show_all.set(true))))];
        }
        let list = Reactive::new(move || -> Vec<W> {
            let Some(Ok(st)) = s.st.get() else { return vec![] };
            let q = s.filter.get().to_lowercase();
            let rows: Vec<W> = menu_apps()
                .iter()
                .filter(|e| !st.config.apps.contains_key(&exe_of(e)))
                .filter(|e| q.is_empty() || e.name.to_lowercase().contains(&q) || exe_of(e).to_lowercase().contains(&q))
                .take(60)
                .map(add_row)
                .collect();
            vec![if rows.is_empty() { note(t!("Ничего не найдено")) } else { card(rows) }]
        });
        vec![titled(
            t!("Все программы"),
            boxed(
                Column::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(TextField::new().text(s.filter.get_untracked()).placeholder(t!("Найти программу")).on_change(move |v| s.filter.set(v.to_string())))
                    .child(list),
            ),
        )]
    });
    page(
        t!("Подключения к интернету"),
        t!("Несколько подключений сразу: Wi-Fi, кабель, телефон по USB. Один канал основной или все вместе — для всей системы и для каждой программы."),
        vec![boxed(Column::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(body).child(more))],
    )
}
