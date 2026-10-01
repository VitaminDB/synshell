//! Телефония (демон synmodemd): «Мобильная сеть» — связь и интернет, SIM и eSIM, технологии, выбор оператора,
//! точки доступа, PIN, трафик; «Вызовы и SMS» — ожидание, скрытие номера, переадресация, SMS-центр, USSD;
//! «О модеме» — идентификаторы, прошивка, диапазоны, текущая сота.

use std::cell::Cell;

use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use synmodem::api::{self, Apn, CallServices, Info, Modes, Network, Request, Response, Status, Usage};
use synmodem::euicc::Profile;

use crate::state;
use crate::ui::*;

type R<T> = Option<std::result::Result<T, String>>;

#[derive(Clone, Copy)]
struct Sigs {
    status: RwSignal<R<Status>>,
    info: RwSignal<R<Info>>,
    cell: RwSignal<R<api::Cell>>,
    esim: RwSignal<R<Vec<Profile>>>,
    modes: RwSignal<R<Modes>>,
    apns: RwSignal<R<Vec<Apn>>>,
    nets: RwSignal<R<Vec<Network>>>,
    scanning: RwSignal<bool>,
    usage: RwSignal<R<Usage>>,
    services: RwSignal<R<CallServices>>,
    smsc: RwSignal<R<String>>,
    /// Переписка USSD: (моё, текст); сеть ждёт ответа.
    ussd: RwSignal<Vec<(bool, String)>>,
    ussd_reply: RwSignal<bool>,
    apn_edit: RwSignal<Option<Apn>>,
    esim_rename: RwSignal<Option<(String, String)>>,
    esim_delete: RwSignal<Option<String>>,
    /// Форма PIN: «enable», «disable», «change», «verify», «unblock».
    pin_form: RwSignal<Option<&'static str>>,
}

thread_local! {
    static SIGS: Cell<Option<Sigs>> = const { Cell::new(None) };
}

const PAGES: &[&str] = &["mobile-network", "calls-sms", "modem-info"];

fn sigs() -> Sigs {
    SIGS.with(|c| match c.get() {
        Some(s) => s,
        None => {
            let s = Sigs {
                status: use_signal(None),
                info: use_signal(None),
                cell: use_signal(None),
                esim: use_signal(None),
                modes: use_signal(None),
                apns: use_signal(None),
                nets: use_signal(None),
                scanning: use_signal(false),
                usage: use_signal(None),
                services: use_signal(None),
                smsc: use_signal(None),
                ussd: use_signal(Vec::new()),
                ussd_reply: use_signal(false),
                apn_edit: use_signal(None),
                esim_rename: use_signal(None),
                esim_delete: use_signal(None),
                pin_form: use_signal(None),
            };
            c.set(Some(s));
            // Состояние сети — раз в 3 с, пока открыта страница телефонии
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(3));
                run_on_main_thread(move || {
                    if PAGES.contains(&state::ctx().page.get_untracked().as_str()) {
                        load(s.status, api::status);
                    }
                });
            });
            watch_ussd(s);
            s
        }
    })
}

fn load<T: Clone + PartialEq + Send + Sync + 'static>(sig: RwSignal<R<T>>, f: impl FnOnce() -> anyhow::Result<T> + Send + 'static) {
    std::thread::spawn(move || {
        let r = f().map_err(|e| format!("{e:#}"));
        run_on_main_thread(move || sig.set(Some(r)));
    });
}

fn get<T>(req: Request, pick: impl FnOnce(Response) -> Option<T>) -> anyhow::Result<T> {
    let r = api::request(&req)?;
    pick(r).ok_or_else(|| anyhow::anyhow!("неожиданный ответ synmodemd"))
}

/// Запрос с подсказкой о ходе и результате; `after` — что перечитать.
fn act(label: &'static str, req: Request, after: impl FnOnce(Sigs) + Send + 'static) {
    state::toast(format!("{label}…"));
    std::thread::spawn(move || {
        let r = api::request(&req);
        run_on_main_thread(move || {
            state::toast(match &r {
                Ok(_) => format!("{label}: готово"),
                Err(e) => format!("{label}: {e:#}"),
            });
            after(sigs());
        });
    });
}

fn load_status(s: Sigs) {
    load(s.status, api::status);
}
fn load_esim(s: Sigs) {
    load(s.esim, || get(Request::EsimProfiles, |r| if let Response::EsimProfiles { profiles } = r { Some(profiles) } else { None }));
}
fn load_modes(s: Sigs) {
    load(s.modes, || get(Request::Modes, |r| if let Response::Modes { modes } = r { Some(modes) } else { None }));
}
fn load_apns(s: Sigs) {
    load(s.apns, || get(Request::ApnList, |r| if let Response::ApnList { apns } = r { Some(apns) } else { None }));
}
fn load_usage(s: Sigs) {
    load(s.usage, || get(Request::Usage, |r| if let Response::Usage { usage } = r { Some(usage) } else { None }));
}
fn load_info(s: Sigs) {
    load(s.info, || get(Request::Info, |r| if let Response::Info { info } = r { Some(info) } else { None }));
}
fn load_cell(s: Sigs) {
    load(s.cell, || get(Request::Cell, |r| if let Response::Cell { cell } = r { Some(cell) } else { None }));
}
fn load_services(s: Sigs) {
    load(s.services, || get(Request::CallServices, |r| if let Response::CallServices { services } = r { Some(services) } else { None }));
}
fn load_smsc(s: Sigs) {
    load(s.smsc, || get(Request::Smsc, |r| if let Response::Text { text } = r { Some(text) } else { None }));
}

/// Ответы USSD приходят событиями демона.
fn watch_ussd(s: Sigs) {
    std::thread::spawn(move || loop {
        let _ = api::subscribe(|ev| {
            if let api::Event::Ussd { text, reply, done } = ev {
                run_on_main_thread(move || {
                    if !text.is_empty() {
                        s.ussd.update(|l| l.push((false, text)));
                    }
                    s.ussd_reply.set(reply && !done);
                });
            }
            true
        });
        std::thread::sleep(std::time::Duration::from_secs(5));
    });
}

fn col(items: Vec<W>) -> W {
    let mut c = Column::new().gap(18.0);
    for w in items {
        c = c.child(w);
    }
    boxed(c)
}

fn kv(label: &str, value: impl Into<String>) -> W {
    let v: String = value.into();
    // Длинное значение (прошивка, список диапазонов) — под подписью с переносом
    if v.chars().count() > 22 {
        return row_wide(label, "", Text::new(v).max_lines(4).class("row-value"));
    }
    row_inline(label, "", Text::new(if v.is_empty() { "—".to_string() } else { v }).class("row-value"))
}

fn text_input(value: &str, placeholder: &str, sig: RwSignal<String>) -> impl Widget {
    sig.set(value.to_string());
    TextField::new().text(value).placeholder(placeholder).on_change(move |t| sig.set(t.to_string())).class("grow")
}

fn human_bytes(b: u64) -> String {
    let f = b as f64;
    if f >= 1e9 {
        format!("{:.2} ГБ", f / 1e9)
    } else if f >= 1e6 {
        format!("{:.1} МБ", f / 1e6)
    } else if f >= 1e3 {
        format!("{:.0} КБ", f / 1e3)
    } else {
        format!("{b} Б")
    }
}

fn no_daemon(e: &str) -> W {
    note(&format!("Модем недоступен: {e}. Демон synmodemd (synmodem.service) работает только на телефоне с модемом Qualcomm."))
}

// ═══ Мобильная сеть ═════════════════════════════════════════════════════════

pub fn mobile_network() -> W {
    let s = sigs();
    load_status(s);
    load_esim(s);
    load_modes(s);
    load_apns(s);
    load_usage(s);
    let status = Reactive::new(move || -> Vec<W> {
        let Some(r) = s.status.get() else { return vec![note("Чтение состояния модема…")] };
        let st = match r {
            Ok(st) => st,
            Err(e) => return vec![no_daemon(&e)],
        };
        vec![status_group(st)]
    });
    page(
        "Мобильная сеть",
        "Сотовая связь, мобильный интернет, SIM-карты и eSIM, выбор сети и точки доступа.",
        vec![
            boxed(status),
            boxed(Reactive::new(move || vec![sim_group(s)])),
            boxed(Reactive::new(move || vec![network_group(s)])),
            boxed(Reactive::new(move || vec![apn_group(s)])),
            boxed(Reactive::new(move || vec![pin_group(s)])),
            boxed(Reactive::new(move || vec![usage_group(s)])),
        ],
    )
}

fn status_group(st: Status) -> W {
    let op = if st.operator.is_empty() { st.plmn.clone() } else { st.operator.clone() };
    let reg = match st.registration {
        api::Registration::Home => "В домашней сети".to_string(),
        api::Registration::Roaming => "Роуминг".into(),
        api::Registration::Limited => "Только экстренные вызовы".into(),
        api::Registration::Denied => "Сеть отказала в регистрации".into(),
        api::Registration::Searching => "Поиск сети".into(),
        _ => "Нет сети".into(),
    };
    let signal = match (st.bars, st.dbm) {
        (Some(b), Some(d)) => format!("{b} из 4 · {d} дБм"),
        _ => "—".into(),
    };
    let data = &st.data;
    let data_hint = match data.state {
        api::DataState::Off => "Выключен".to_string(),
        api::DataState::Waiting if !data.error.is_empty() => data.error.clone(),
        api::DataState::Waiting => "Ждёт сеть".into(),
        api::DataState::Connecting => "Подключение…".into(),
        api::DataState::Connected => format!("Подключён · {}", data.address),
        api::DataState::Error => format!("Ошибка: {}", data.error),
    };
    let radio_on = st.radio;
    let data_on = data.enabled;
    group(
        "",
        vec![
            kv("Сеть", format!("{op} · {reg}")),
            kv("Технология", st.technology.clone()),
            kv("Сигнал", signal),
            row_inline(
                "Мобильная связь",
                "Выключено — режим полёта для сотовой сети (Wi-Fi и Bluetooth остаются)",
                Toggle::with_state(radio_on).on_change(move |on| act(if on { "Включение связи" } else { "Режим полёта" }, Request::SetRadio { on }, load_status)),
            ),
            row_inline(
                "Мобильный интернет",
                &data_hint,
                Toggle::with_state(data_on).on_change(move |on| act("Мобильный интернет", Request::SetData { on }, load_status)),
            ),
        ],
    )
}

fn sim_group(s: Sigs) -> W {
    let mut rows: Vec<W> = Vec::new();
    match s.esim.get() {
        None => rows.push(note("Чтение профилей eSIM…")),
        Some(Err(e)) => rows.push(note(&format!("eSIM: {e}"))),
        Some(Ok(list)) => {
            if list.is_empty() {
                rows.push(note("На eSIM нет профилей."));
            }
            for p in list {
                rows.push(esim_row(s, p));
            }
        }
    }
    rows.push(row_inline(
        "Обновить",
        "Перечитать профили с eSIM",
        button("Обновить", move || load_esim(s)),
    ));
    rows.push(note("Загрузка нового профиля по QR-коду оператора (SM-DP+) — в следующей версии; пока — через Android."));
    group("SIM-карты и eSIM", rows)
}

fn esim_row(s: Sigs, p: Profile) -> W {
    let title = if p.nickname.is_empty() { p.provider.clone() } else { p.nickname.clone() };
    let tail: String = p.iccid.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    let hint = format!(
        "{}{} · {} · ICCID …{tail}",
        if p.enabled { "Используется" } else { "Выключен" },
        if p.provider.is_empty() || p.provider == title { String::new() } else { format!(" · {}", p.provider) },
        p.class,
    );
    let iccid = p.iccid.clone();
    if s.esim_rename.get().as_ref().is_some_and(|(i, _)| *i == iccid) {
        let name = use_signal(String::new());
        let i2 = iccid.clone();
        return boxed(
            Row::new()
                .gap(8.0)
                .class("setting-row")
                .child(text_input(&title, "Имя профиля", name))
                .child(primary_button("Сохранить", move || {
                    let iccid = i2.clone();
                    s.esim_rename.set(None);
                    act("Имя профиля", Request::EsimNickname { iccid, name: name.get_untracked() }, load_esim);
                }))
                .child(button("Отмена", move || s.esim_rename.set(None))),
        );
    }
    if s.esim_delete.get().as_deref() == Some(iccid.as_str()) {
        let i2 = iccid.clone();
        return row_inline(
            &format!("Удалить «{title}»?"),
            "Профиль удаляется с eSIM навсегда; вернуть его можно только новым QR-кодом от оператора",
            Row::new()
                .gap(6.0)
                .child(button("Отмена", move || s.esim_delete.set(None)))
                .child(primary_button("Удалить", move || {
                    s.esim_delete.set(None);
                    act("Удаление профиля", Request::EsimDelete { iccid: i2.clone() }, load_esim);
                })),
        );
    }
    let mut actions = Row::new().gap(6.0);
    let (i1, i2, i3, t2) = (iccid.clone(), iccid.clone(), iccid.clone(), title.clone());
    if p.enabled {
        actions = actions.child(button("Выключить", move || {
            act("Выключение профиля", Request::EsimDisable { iccid: i1.clone() }, |s| {
                load_esim(s);
                load_status(s);
            })
        }));
    } else {
        actions = actions.child(primary_button("Включить", move || {
            act("Переключение профиля", Request::EsimEnable { iccid: i1.clone() }, |s| {
                load_esim(s);
                load_status(s);
            })
        }));
        actions = actions.child(icon_button(icons::DELETE, move || s.esim_delete.set(Some(i3.clone()))));
    }
    actions = actions.child(icon_button("\u{E3C9}", move || s.esim_rename.set(Some((i2.clone(), t2.clone())))));
    row_inline(&title, &hint, actions)
}

const TECHS: &[(&str, &str)] = &[("5g", "5G"), ("4g", "4G (LTE)"), ("3g", "3G"), ("2g", "2G")];

fn network_group(s: Sigs) -> W {
    let mut rows: Vec<W> = Vec::new();
    match s.modes.get() {
        None => rows.push(note("Чтение настроек сети…")),
        Some(Err(e)) => rows.push(note(&e)),
        Some(Ok(m)) => {
            for (key, label) in TECHS {
                let on = m.allowed.iter().any(|a| a == key);
                let allowed = m.allowed.clone();
                rows.push(row_inline(
                    label,
                    if *key == "2g" { "2G — запасной вариант для звонков, медленный интернет" } else { "" },
                    Toggle::with_state(on).on_change(move |v| {
                        let mut a = allowed.clone();
                        a.retain(|x| x != key);
                        if v {
                            a.push(key.to_string());
                        }
                        act("Технологии сети", Request::SetModes { allowed: a }, load_modes);
                    }),
                ));
            }
            let roaming = m.data_roaming;
            rows.push(row_inline(
                "Интернет в роуминге",
                "Передача данных вне домашней сети (может стоить дорого)",
                Toggle::with_state(roaming).on_change(move |on| {
                    act("Интернет в роуминге", Request::SetDataRoaming { on }, |s| {
                        load_modes(s);
                        load_status(s);
                    })
                }),
            ));
            let auto = m.manual.is_none();
            rows.push(row_inline(
                "Выбирать сеть автоматически",
                &match m.manual {
                    Some((mcc, mnc)) => format!("Вручную: {mcc:03}/{mnc:02}"),
                    None => "Телефон сам выбирает доступную сеть".into(),
                },
                Toggle::with_state(auto).on_change(move |on| {
                    if on {
                        act("Автовыбор сети", Request::NetworkSelect { plmn: None }, load_modes);
                    } else {
                        state::toast("Найдите сети и выберите оператора".to_string());
                    }
                }),
            ));
        }
    }
    let scanning = s.scanning.get();
    rows.push(row_inline(
        "Доступные сети",
        if scanning { "Поиск идёт до 3 минут; связь на это время может пропасть" } else { "Поиск операторов вокруг" },
        button(if scanning { "Поиск…" } else { "Найти сети" }, move || {
            if s.scanning.get_untracked() {
                return;
            }
            s.scanning.set(true);
            std::thread::spawn(move || {
                let r = get(Request::NetworkScan, |r| if let Response::Networks { networks } = r { Some(networks) } else { None }).map_err(|e| format!("{e:#}"));
                run_on_main_thread(move || {
                    s.scanning.set(false);
                    s.nets.set(Some(r));
                });
            });
        }),
    ));
    match s.nets.get() {
        Some(Err(e)) => rows.push(note(&format!("Поиск: {e}"))),
        Some(Ok(list)) => {
            for n in list {
                let name = if n.name.is_empty() { format!("{:03}/{:02}", n.mcc, n.mnc) } else { n.name.clone() };
                let mut hint = format!("{:03}/{:02}", n.mcc, n.mnc);
                if !n.technologies.is_empty() {
                    hint.push_str(&format!(" · {}", n.technologies.join(", ")));
                }
                if n.forbidden {
                    hint.push_str(" · запрещена");
                }
                let plmn = (n.mcc, n.mnc);
                let ctl: W = if n.current {
                    boxed(Text::new("Текущая").class("row-value"))
                } else {
                    boxed(button("Выбрать", move || act("Регистрация в сети", Request::NetworkSelect { plmn: Some(plmn) }, |s| {
                        load_modes(s);
                        load_status(s);
                    })))
                };
                rows.push(row_inline(&name, &hint, ctl));
            }
        }
        None => {}
    }
    group("Сеть", rows)
}

fn apn_group(s: Sigs) -> W {
    let mut rows: Vec<W> = Vec::new();
    if let Some(a) = s.apn_edit.get() {
        return group("Точка доступа", apn_form(s, a));
    }
    match s.apns.get() {
        None => rows.push(note("Чтение точек доступа…")),
        Some(Err(e)) => rows.push(note(&e)),
        Some(Ok(list)) => {
            for a in list {
                let title = if a.name.is_empty() { a.apn.clone() } else { format!("{} ({})", a.name, a.apn) };
                let hint = format!(
                    "{}{} · {}{}",
                    if a.default { "Для интернета · " } else { "" },
                    a.ip,
                    if a.auth == "none" { "без входа".to_string() } else { format!("вход {}", a.auth) },
                    if a.roaming_disallowed { " · не в роуминге" } else { "" }
                );
                let (a1, idx) = (a.clone(), a.index);
                let mut actions = Row::new().gap(6.0).child(icon_button("\u{E3C9}", move || s.apn_edit.set(Some(a1.clone()))));
                if !a.default {
                    actions = actions.child(icon_button(icons::DELETE, move || act("Удаление точки доступа", Request::ApnDelete { index: idx }, load_apns)));
                }
                rows.push(row_inline(&title, &hint, actions));
            }
        }
    }
    rows.push(row_inline(
        "Новая точка доступа",
        "APN, имя пользователя и пароль даёт оператор",
        button("Добавить", move || s.apn_edit.set(Some(Apn { auth: "none".into(), ip: "ipv4v6".into(), ..Apn::default() }))),
    ));
    group("Точки доступа (APN)", rows)
}

fn apn_form(s: Sigs, a: Apn) -> Vec<W> {
    let name = use_signal(String::new());
    let apn = use_signal(String::new());
    let user = use_signal(String::new());
    let pass = use_signal(String::new());
    let auth = use_signal(a.auth.clone());
    let ip = use_signal(a.ip.clone());
    let default = use_signal(a.default);
    let roam = use_signal(a.roaming_disallowed);
    let index = a.index;
    let dd = |sig: RwSignal<String>, items: &[(&str, &str)]| {
        let mut d = Dropdown::new().width(200.0);
        for (v, l) in items {
            d = d.item(DropdownItem::new(*v, *l));
        }
        d.selected(sig.get_untracked()).on_change(move |v: &str| sig.set(v.to_string()))
    };
    vec![
        row("Название", "", text_input(&a.name, "Например, «Интернет»", name)),
        row("APN", "", text_input(&a.apn, "internet", apn)),
        row("Пользователь", "", text_input(&a.user, "необязательно", user)),
        row("Пароль", "", text_input(&a.password, "необязательно", pass)),
        row("Проверка подлинности", "", dd(auth, &[("none", "Нет"), ("pap", "PAP"), ("chap", "CHAP"), ("pap-chap", "PAP или CHAP")])),
        row("Протокол", "", dd(ip, &[("ipv4", "IPv4"), ("ipv6", "IPv6"), ("ipv4v6", "IPv4 и IPv6")])),
        row_inline("Для мобильного интернета", "Эту точку доступа использует передача данных", Toggle::with_state(a.default).on_change(move |v| default.set(v))),
        row_inline("Запретить в роуминге", "", Toggle::with_state(a.roaming_disallowed).on_change(move |v| roam.set(v))),
        boxed(
            Row::new()
                .gap(8.0)
                .class("setting-row")
                .child(DecoratedBox::new().class("grow"))
                .child(button("Отмена", move || s.apn_edit.set(None)))
                .child(primary_button("Сохранить", move || {
                    let apn = Apn {
                        index,
                        name: name.get_untracked(),
                        apn: apn.get_untracked(),
                        user: user.get_untracked(),
                        password: pass.get_untracked(),
                        auth: auth.get_untracked(),
                        ip: ip.get_untracked(),
                        default: default.get_untracked(),
                        roaming_disallowed: roam.get_untracked(),
                    };
                    if apn.apn.trim().is_empty() {
                        state::toast("Укажите APN".to_string());
                        return;
                    }
                    s.apn_edit.set(None);
                    act("Точка доступа", Request::ApnSave { apn }, load_apns);
                })),
        ),
    ]
}

fn pin_group(s: Sigs) -> W {
    let st = s.status.get().and_then(|r| r.ok());
    let left = st.as_ref().and_then(|x| x.sim.pin_retries);
    let sim_state = st.as_ref().map(|x| x.sim.state).unwrap_or_default();
    let mut rows: Vec<W> = Vec::new();
    let hint = match (sim_state, left) {
        (api::SimState::PinRequired, Some(n)) => format!("SIM заблокирована: введите PIN (осталось попыток: {n})"),
        (api::SimState::PukRequired | api::SimState::Blocked, _) => "PIN заблокирован: нужен PUK-код от оператора".into(),
        (_, Some(n)) => format!(
            "{}Осталось попыток ввода PIN: {n}",
            match st.as_ref().and_then(|x| x.sim.pin_enabled) {
                Some(true) => "PIN запрашивается при включении. ",
                Some(false) => "PIN не запрашивается. ",
                None => "",
            }
        ),
        _ => String::new(),
    };
    if let Some(form) = s.pin_form.get() {
        let a = use_signal(String::new());
        let b = use_signal(String::new());
        let (la, lb) = match form {
            "change" => ("Текущий PIN", Some("Новый PIN")),
            "unblock" => ("PUK-код", Some("Новый PIN")),
            _ => ("PIN-код", None),
        };
        let mut r = Row::new().gap(8.0).class("setting-row").child(
            TextField::new().obscure(true).autofocus(true).placeholder(la).on_change(move |t| a.set(t.to_string())).class("grow"),
        );
        if let Some(lb) = lb {
            r = r.child(TextField::new().obscure(true).placeholder(lb).on_change(move |t| b.set(t.to_string())).class("grow"));
        }
        r = r.child(button("Отмена", move || s.pin_form.set(None))).child(primary_button("Готово", move || {
            let (x, y) = (a.get_untracked(), b.get_untracked());
            s.pin_form.set(None);
            let req = match form {
                "enable" => Request::PinEnable { on: true, pin: x },
                "disable" => Request::PinEnable { on: false, pin: x },
                "change" => Request::PinChange { old: x, new: y },
                "unblock" => Request::PinUnblock { puk: x, new: y },
                _ => Request::PinVerify { pin: x },
            };
            act("PIN-код", req, load_status);
        }));
        rows.push(boxed(r));
    } else {
        let mut actions = Row::new().gap(6.0);
        match sim_state {
            api::SimState::PinRequired => actions = actions.child(primary_button("Ввести PIN", move || s.pin_form.set(Some("verify")))),
            api::SimState::PukRequired | api::SimState::Blocked => {
                actions = actions.child(primary_button("Ввести PUK", move || s.pin_form.set(Some("unblock"))))
            }
            _ => {
                let on = st.as_ref().and_then(|x| x.sim.pin_enabled);
                if on != Some(true) {
                    actions = actions.child(button("Включить запрос", move || s.pin_form.set(Some("enable"))));
                }
                if on != Some(false) {
                    actions = actions
                        .child(button("Выключить запрос", move || s.pin_form.set(Some("disable"))))
                        .child(button("Сменить PIN", move || s.pin_form.set(Some("change"))));
                }
            }
        }
        rows.push(row("PIN-код SIM", &hint, actions));
    }
    group("Защита SIM", rows)
}

fn usage_group(s: Sigs) -> W {
    let rows = match s.usage.get() {
        None => vec![note("Чтение…")],
        Some(Err(e)) => vec![note(&e)],
        Some(Ok(u)) => {
            let since = if u.since > 0 { format!("С {}", synmodem::time::day_title(u.since).to_lowercase()) } else { "Всего".into() };
            vec![
                kv("В этом месяце", format!("↓ {} · ↑ {}", human_bytes(u.rx), human_bytes(u.tx))),
                kv(&since, format!("↓ {} · ↑ {}", human_bytes(u.total_rx), human_bytes(u.total_tx))),
                row_inline("Сбросить общий счётчик", "", button("Сбросить", move || act("Счётчик трафика", Request::UsageReset, load_usage))),
            ]
        }
    };
    group("Мобильный трафик", rows)
}

// ═══ Вызовы и SMS ═══════════════════════════════════════════════════════════

pub fn calls_sms() -> W {
    let s = sigs();
    load_services(s);
    load_smsc(s);
    page(
        "Вызовы и SMS",
        "Ожидание вызова, скрытие номера, переадресация, SMS-центр и USSD-запросы. Настройки вызовов хранит сеть оператора.",
        vec![
            boxed(Reactive::new(move || vec![services_group(s)])),
            boxed(Reactive::new(move || vec![ussd_group(s)])),
            boxed(Reactive::new(move || vec![smsc_group(s)])),
        ],
    )
}

fn services_group(s: Sigs) -> W {
    let sv = match s.services.get() {
        None => return group("Вызовы", vec![note("Запрос настроек у сети (до полуминуты)…")]),
        Some(Err(e)) => return group("Вызовы", vec![no_daemon(&e)]),
        Some(Ok(v)) => v,
    };
    let mut rows: Vec<W> = Vec::new();
    if !sv.error.is_empty() {
        rows.push(note(&format!(
            "Сеть не дала настройки вызовов ({}). В сетях с VoLTE эти услуги управляются через мобильный интернет оператора (XCAP) и в роуминге часто недоступны.",
            sv.error
        )));
    }
    match sv.waiting {
        Some(on) => rows.push(row_inline(
            "Ожидание вызова",
            "Сообщать о втором звонке во время разговора",
            Toggle::with_state(on).on_change(move |v| act("Ожидание вызова", Request::SetCallWaiting { on: v }, load_services)),
        )),
        None => rows.push(kv("Ожидание вызова", "недоступно")),
    }
    let mut dd = Dropdown::new().width(220.0);
    for (v, l) in [("network", "Как задано у оператора"), ("hide", "Скрывать"), ("show", "Показывать")] {
        dd = dd.item(DropdownItem::new(v, l));
    }
    rows.push(row(
        "Мой номер при звонке",
        "Скрытие номера (АОН) для исходящих",
        dd.selected(sv.clir.clone()).on_change(|v: &str| act("Скрытие номера", Request::SetClir { mode: v.to_string() }, load_services)),
    ));
    for f in sv.forwards {
        let label = match f.reason.as_str() {
            "always" => "Переадресация: всегда",
            "busy" => "Если занят",
            "no-reply" => "Если нет ответа",
            _ => "Если недоступен",
        };
        let num = use_signal(String::new());
        let reason = f.reason.clone();
        let reason2 = f.reason.clone();
        let hint = if f.active { format!("Включена на {}", f.number) } else { "Выключена".into() };
        rows.push(row(
            label,
            &hint,
            Row::new()
                .gap(6.0)
                .child(text_input(&f.number, "Номер", num))
                .child(button("Включить", move || {
                    let n = num.get_untracked();
                    if n.trim().is_empty() {
                        state::toast("Укажите номер".to_string());
                        return;
                    }
                    act("Переадресация", Request::SetForward { reason: reason.clone(), number: n, timer: Some(20) }, load_services);
                }))
                .child(button("Выключить", move || {
                    act("Переадресация", Request::SetForward { reason: reason2.clone(), number: String::new(), timer: None }, load_services)
                })),
        ));
    }
    group("Вызовы", rows)
}

fn ussd_group(s: Sigs) -> W {
    let code = use_signal(String::new());
    let mut rows: Vec<W> = Vec::new();
    for (mine, text) in s.ussd.get() {
        rows.push(boxed(
            Row::new()
                .class("setting-row")
                .main_axis_alignment(if mine { MainAxisAlignment::End } else { MainAxisAlignment::Start })
                .child(DecoratedBox::new().child(Text::new(text).class("row-label")).class(if mine { "chip chip-accent" } else { "chip" })),
        ));
    }
    let reply = s.ussd_reply.get();
    let send = move || {
        let c = code.get_untracked();
        if c.trim().is_empty() {
            return;
        }
        s.ussd.update(|l| l.push((true, c.clone())));
        let req = if s.ussd_reply.get_untracked() { Request::UssdReply { text: c } } else { Request::Ussd { code: c } };
        std::thread::spawn(move || {
            if let Err(e) = api::request(&req) {
                let m = format!("{e:#}");
                run_on_main_thread(move || s.ussd.update(|l| l.push((false, m))));
            }
        });
    };
    let send2 = send;
    let mut r = Row::new()
        .gap(8.0)
        .class("setting-row")
        .child(
            TextField::new()
                .placeholder(if reply { "Ответ (номер пункта)" } else { "Например, *100#" })
                .on_change(move |t| code.set(t.to_string()))
                .on_submit(move |_| send2())
                .class("grow"),
        )
        .child(primary_button(if reply { "Ответить" } else { "Отправить" }, send));
    if reply {
        r = r.child(button("Закончить", move || {
            s.ussd_reply.set(false);
            std::thread::spawn(|| {
                let _ = api::request(&Request::UssdCancel);
            });
        }));
    }
    rows.push(boxed(r));
    rows.push(note("USSD — служебные запросы оператора: баланс, остатки, подключение услуг."));
    group("USSD-запросы", rows)
}

fn smsc_group(s: Sigs) -> W {
    let num = use_signal(String::new());
    let rows = match s.smsc.get() {
        None => vec![note("Чтение…")],
        Some(Err(e)) => vec![note(&e)],
        Some(Ok(v)) => vec![row(
            "SMS-центр",
            "Номер центра SMS оператора; обычно его записывает SIM",
            Row::new()
                .gap(6.0)
                .child(text_input(&v, "+7…", num))
                .child(button("Сохранить", move || act("SMS-центр", Request::SetSmsc { number: num.get_untracked() }, load_smsc))),
        )],
    };
    group("SMS", rows)
}

// ═══ О модеме ═══════════════════════════════════════════════════════════════

pub fn modem_info() -> W {
    let s = sigs();
    load_info(s);
    load_cell(s);
    page(
        "О модеме",
        "Идентификаторы, прошивка, поддерживаемые диапазоны и текущая сота.",
        vec![
            boxed(Reactive::new(move || -> Vec<W> {
                match s.info.get() {
                    None => vec![note("Чтение…")],
                    Some(Err(e)) => vec![no_daemon(&e)],
                    Some(Ok(i)) => vec![info_groups(i)],
                }
            })),
            boxed(Reactive::new(move || -> Vec<W> {
                match s.cell.get() {
                    None => vec![],
                    Some(Err(e)) => vec![note(&e)],
                    Some(Ok(c)) => vec![cell_group(c)],
                }
            })),
        ],
    )
}

fn info_groups(i: Info) -> W {
    let slots: Vec<String> = i
        .slots
        .iter()
        .map(|s| {
            format!(
                "Слот {}: {}{}",
                s.physical,
                if !s.card { "пусто".to_string() } else if s.euicc { "eSIM".into() } else { "SIM".into() },
                if s.card && s.active { format!(" (логический {})", s.logical) } else { String::new() }
            )
        })
        .collect();
    let bands = |b: &[u16], p: &str| b.iter().map(|x| format!("{p}{x}")).collect::<Vec<_>>().join(" ");
    col(vec![
        group(
            "Устройство",
            vec![
                kv("IMEI", i.imei.clone()),
                kv("IMEI SV", i.imei_sv.clone()),
                kv("Прошивка модема", i.revision.clone()),
                kv("Сборка ПО модема", i.sw_version.clone()),
                kv("Аппаратная ревизия", i.hw_revision.clone()),
            ],
        ),
        group(
            "SIM",
            vec![
                kv("Оператор SIM", i.spn.clone()),
                kv("Номер телефона", if i.msisdn.is_empty() { "не записан на SIM".into() } else { i.msisdn.clone() }),
                kv("ICCID", i.iccid.clone()),
                kv("IMSI", i.imsi.clone()),
                kv("EID (eSIM)", i.eid.clone()),
                kv("Слоты", slots.join(" · ")),
            ],
        ),
        group("Диапазоны", vec![kv("LTE", bands(&i.lte_bands, "B")), kv("5G NR", bands(&i.nr_bands, "n"))]),
    ])
}

fn cell_group(c: api::Cell) -> W {
    let o = |v: Option<i32>, u: &str| v.map(|x| format!("{x} {u}")).unwrap_or_default();
    let s = sigs();
    group(
        "Текущая сота",
        vec![
            kv("Сеть", format!("{} {}", c.technology, c.plmn)),
            kv("Диапазон", format!("{}{}", c.band, c.bandwidth_mhz.map(|b| format!(" · {b} МГц")).unwrap_or_default())),
            kv("Канал (EARFCN/ARFCN)", c.channel.map(|x| x.to_string()).unwrap_or_default()),
            kv("Код зоны (TAC/LAC)", c.tac.or(c.lac).map(|x| x.to_string()).unwrap_or_default()),
            kv("Номер соты", c.cell_id.map(|x| format!("{x} (eNB {}, сектор {})", x >> 8, x & 0xFF)).unwrap_or_default()),
            kv("RSRP", o(c.rsrp, "дБм")),
            kv("RSRQ", o(c.rsrq, "дБ")),
            kv("RSSI", o(c.rssi, "дБм")),
            kv("SINR", c.snr.map(|x| format!("{x:.1} дБ")).unwrap_or_default()),
            row_inline("Обновить", "", button("Обновить", move || load_cell(s))),
        ],
    )
}
