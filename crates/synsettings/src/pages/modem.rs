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
    /// Форма «Добавить eSIM»: код активации и код подтверждения.
    esim_add: RwSignal<Option<(String, String)>>,
    /// Идёт загрузка профиля: текущий шаг.
    esim_dl: RwSignal<Option<String>>,
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
                esim_add: use_signal(None),
                esim_dl: use_signal(None),
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
fn act(label: impl AsRef<str>, req: Request, after: impl FnOnce(Sigs) + Send + 'static) {
    let label = tl(label);
    state::toast(format!("{label}…"));
    std::thread::spawn(move || {
        let r = api::request(&req);
        run_on_main_thread(move || {
            state::toast(match &r {
                Ok(_) => t!("{label}: готово", label = label),
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
            match ev {
                api::Event::Ussd { text, reply, done } => run_on_main_thread(move || {
                    if !text.is_empty() {
                        s.ussd.update(|l| l.push((false, text)));
                    }
                    s.ussd_reply.set(reply && !done);
                }),
                // ход загрузки профиля eSIM (запрос ждёт в своём потоке)
                api::Event::EsimProgress { step, done: false, .. } => run_on_main_thread(move || {
                    if s.esim_dl.get_untracked().is_some() {
                        s.esim_dl.set(Some(step));
                    }
                }),
                _ => {}
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

fn kv(label: impl AsRef<str>, value: impl Into<String>) -> W {
    let v: String = value.into();
    // Длинное значение (прошивка, список диапазонов) — под подписью с переносом
    if v.chars().count() > 22 {
        return row_wide(label, "", Text::new(v).max_lines(4).class("row-value"));
    }
    row_inline(label, "", Text::new(if v.is_empty() { "—".to_string() } else { v }).class("row-value"))
}

fn text_input(value: &str, placeholder: impl AsRef<str>, sig: RwSignal<String>) -> impl Widget {
    sig.set(value.to_string());
    TextField::new().text(value).placeholder(tl(placeholder)).on_change(move |t| sig.set(t.to_string())).class("grow")
}

fn human_bytes(b: u64) -> String {
    let f = b as f64;
    if f >= 1e9 {
        t!("{v} ГБ", v = format!("{:.2}", f / 1e9))
    } else if f >= 1e6 {
        t!("{v} МБ", v = format!("{:.1}", f / 1e6))
    } else if f >= 1e3 {
        t!("{v} КБ", v = format!("{:.0}", f / 1e3))
    } else {
        t!("{b} Б", b = b)
    }
}

fn no_daemon(e: &str) -> W {
    note(&t!("Модем недоступен: {e}. Демон synmodemd (synmodem.service) работает только на телефоне с модемом Qualcomm.", e = e))
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
        let Some(r) = s.status.get() else { return vec![note(&t!("Чтение состояния модема…"))] };
        let st = match r {
            Ok(st) => st,
            Err(e) => return vec![no_daemon(&e)],
        };
        vec![status_group(st)]
    });
    page(
        t!("Мобильная сеть"),
        t!("Сотовая связь, мобильный интернет, SIM-карты и eSIM, выбор сети и точки доступа."),
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
        api::Registration::Home => t!("В домашней сети").to_string(),
        api::Registration::Roaming => t!("Роуминг").into(),
        api::Registration::Limited => t!("Только экстренные вызовы").into(),
        api::Registration::Denied => t!("Сеть отказала в регистрации").into(),
        api::Registration::Searching => t!("Поиск сети").into(),
        _ => t!("Нет сети").into(),
    };
    let signal = match (st.bars, st.dbm) {
        (Some(b), Some(d)) => t!("{b} из 4 · {d} дБм", b = b, d = d),
        _ => "—".into(),
    };
    let data = &st.data;
    let data_hint = match data.state {
        api::DataState::Off => t!("Выключен").to_string(),
        api::DataState::Waiting if !data.error.is_empty() => data.error.clone(),
        api::DataState::Waiting => t!("Ждёт сеть").into(),
        api::DataState::Connecting => t!("Подключение…").into(),
        api::DataState::Connected => t!("Подключён · {address}", address = data.address),
        api::DataState::Error => t!("Ошибка: {error}", error = data.error),
    };
    let radio_on = st.radio;
    let data_on = data.enabled;
    group(
        "",
        vec![
            kv(&t!("Сеть"), format!("{op} · {reg}")),
            kv(&t!("Технология"), st.technology.clone()),
            kv(&t!("Сигнал"), signal),
            row_inline(
                t!("Мобильная связь"),
                t!("Выключено — режим полёта для сотовой сети (Wi-Fi и Bluetooth остаются)"),
                Toggle::with_state(radio_on).on_change(move |on| act(if on { t!("Включение связи") } else { t!("Режим полёта") }, Request::SetRadio { on }, load_status)),
            ),
            row_inline(
                &t!("Мобильный интернет"),
                &data_hint,
                Toggle::with_state(data_on).on_change(move |on| act(&t!("Мобильный интернет"), Request::SetData { on }, load_status)),
            ),
        ],
    )
}

fn sim_group(s: Sigs) -> W {
    let mut rows: Vec<W> = Vec::new();
    match s.esim.get() {
        None => rows.push(note(&t!("Чтение профилей eSIM…"))),
        Some(Err(e)) => rows.push(note(&format!("eSIM: {e}"))),
        Some(Ok(list)) => {
            if list.is_empty() {
                rows.push(note(&t!("На eSIM нет профилей.")));
            }
            for p in list {
                rows.push(esim_row(s, p));
            }
        }
    }
    rows.push(row_inline(
        t!("Обновить"),
        t!("Перечитать профили с eSIM"),
        button(&t!("Обновить"), move || load_esim(s)),
    ));
    rows.push(esim_add(s));
    group(&t!("SIM-карты и eSIM"), rows)
}

/// Код активации из QR оператора: `LPA:1$сервер$код[$OID[$1]]`; `Some(true)` — нужен код подтверждения.
fn lpa_code(code: &str) -> Option<bool> {
    synmodem::lpa::parse_code(code).map(|(_, _, conf)| conf)
}

/// Распознать QR «Камерой» (`syncamera --scan-qr LPA:`): код — в форму.
fn scan_qr(s: Sigs) {
    state::toast(t!("Наведите камеру на QR-код оператора"));
    std::thread::spawn(move || {
        let out = std::process::Command::new("syncamera").args(["--scan-qr", "LPA:"]).output();
        run_on_main_thread(move || match out {
            Ok(o) if o.status.success() => {
                let code = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if !code.is_empty() {
                    let conf = s.esim_add.get_untracked().map(|f| f.1).unwrap_or_default();
                    s.esim_add.set(Some((code, conf)));
                }
            }
            Ok(_) => {}
            Err(e) => state::toast(t!("Камера не запустилась: {e}", e = e)),
        });
    });
}

/// Загрузка нового профиля: код активации (вручную или QR), код подтверждения, ход загрузки.
fn esim_add(s: Sigs) -> W {
    if let Some(step) = s.esim_dl.get() {
        let shown = if step.is_empty() { t!("Подготовка…") } else { step };
        return row_inline(
            t!("Загрузка профиля eSIM"),
            t!("{step} · не выключайте связь, это может занять несколько минут", step = shown),
            CircularProgress::new().indeterminate().size(22.0),
        );
    }
    let Some((code, conf)) = s.esim_add.get() else {
        return row_inline(
            t!("Добавить eSIM"),
            t!("Профиль оператора по QR-коду или коду активации (LPA:1$…); связь должна быть включена"),
            primary_button(&t!("Добавить"), move || s.esim_add.set(Some((String::new(), String::new())))),
        );
    };
    let code_sig = use_signal(code.clone());
    let conf_sig = use_signal(conf.clone());
    let need_conf = lpa_code(&code).unwrap_or(false);
    let valid = lpa_code(&code).is_some();
    let mut col = Column::new()
        .gap(10.0)
        .child(Text::new(t!("Добавить eSIM")).class("row-title"))
        .child(Text::new(t!("Код активации — в QR-коде оператора (начинается с LPA:1$). Его можно отсканировать камерой или вставить.")).max_lines(3).class("row-hint"))
        .child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(TextField::new().text(code.as_str()).placeholder("LPA:1$smdp.example.com$…").on_change(move |t| {
                    code_sig.set(t.to_string());
                    s.esim_add.set(Some((t.to_string(), conf_sig.get_untracked())));
                }).class("grow"))
                .child(button(&t!("Сканировать QR"), move || scan_qr(s))),
        );
    if need_conf || !conf.is_empty() {
        col = col.child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(t!("Код подтверждения")).class("row-hint")).child(
            TextField::new().text(conf.as_str()).placeholder(tl(n_!("от оператора"))).on_change(move |t| {
                conf_sig.set(t.to_string());
                s.esim_add.set(Some((code_sig.get_untracked(), t.to_string())));
            }).class("grow"),
        ));
    }
    if !code.trim().is_empty() && !valid {
        col = col.child(Text::new(t!("Это не код активации eSIM: ожидается LPA:1$сервер$код")).class("row-hint warn"));
    }
    col = col.child(
        Row::new()
            .gap(8.0)
            .main_axis_alignment(MainAxisAlignment::End)
            .child(button(&t!("Отмена"), move || s.esim_add.set(None)))
            .child(primary_button(&t!("Загрузить"), move || {
                let (code, conf) = (code_sig.get_untracked(), conf_sig.get_untracked());
                if lpa_code(&code).is_none() {
                    state::toast(t!("Неверный код активации"));
                    return;
                }
                s.esim_add.set(None);
                s.esim_dl.set(Some(String::new()));
                std::thread::spawn(move || {
                    let r = api::esim_download(&code, &conf);
                    run_on_main_thread(move || {
                        s.esim_dl.set(None);
                        state::toast(match &r {
                            Ok(()) => t!("Профиль eSIM загружен — включите его в списке"),
                            Err(e) => t!("Профиль не загружен: {e}", e = format!("{e:#}")),
                        });
                        load_esim(s);
                    });
                });
            })),
    );
    Box::new(DecoratedBox::new().child(col).class("row"))
}

fn esim_row(s: Sigs, p: Profile) -> W {
    let title = if p.nickname.is_empty() { p.provider.clone() } else { p.nickname.clone() };
    let tail: String = p.iccid.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    let hint = format!(
        "{}{} · {} · ICCID …{tail}",
        if p.enabled { t!("Используется") } else { t!("Выключен") },
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
                .child(text_input(&title, &t!("Имя профиля"), name))
                .child(primary_button(&t!("Сохранить"), move || {
                    let iccid = i2.clone();
                    s.esim_rename.set(None);
                    act(n_!("Имя профиля"), Request::EsimNickname { iccid, name: name.get_untracked() }, load_esim);
                }))
                .child(button(&t!("Отмена"), move || s.esim_rename.set(None))),
        );
    }
    if s.esim_delete.get().as_deref() == Some(iccid.as_str()) {
        let i2 = iccid.clone();
        return row_inline(
            &t!("Удалить «{title}»?", title = title),
            &t!("Профиль удаляется с eSIM навсегда; вернуть его можно только новым QR-кодом от оператора"),
            Row::new()
                .gap(6.0)
                .child(button(&t!("Отмена"), move || s.esim_delete.set(None)))
                .child(primary_button(&t!("Удалить"), move || {
                    s.esim_delete.set(None);
                    act(n_!("Удаление профиля"), Request::EsimDelete { iccid: i2.clone() }, load_esim);
                })),
        );
    }
    let mut actions = Row::new().gap(6.0);
    let (i1, i2, i3, t2) = (iccid.clone(), iccid.clone(), iccid.clone(), title.clone());
    if p.enabled {
        actions = actions.child(button(&t!("Выключить"), move || {
            act(n_!("Выключение профиля"), Request::EsimDisable { iccid: i1.clone() }, |s| {
                load_esim(s);
                load_status(s);
            })
        }));
    } else {
        actions = actions.child(primary_button(&t!("Включить"), move || {
            act(n_!("Переключение профиля"), Request::EsimEnable { iccid: i1.clone() }, |s| {
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
        None => rows.push(note(&t!("Чтение настроек сети…"))),
        Some(Err(e)) => rows.push(note(&e)),
        Some(Ok(m)) => {
            for (key, label) in TECHS {
                let on = m.allowed.iter().any(|a| a == key);
                let allowed = m.allowed.clone();
                rows.push(row_inline(
                    label,
                    if *key == "2g" { t!("2G — запасной вариант для звонков, медленный интернет") } else { String::new() },
                    Toggle::with_state(on).on_change(move |v| {
                        let mut a = allowed.clone();
                        a.retain(|x| x != key);
                        if v {
                            a.push(key.to_string());
                        }
                        act(&t!("Технологии сети"), Request::SetModes { allowed: a }, load_modes);
                    }),
                ));
            }
            let roaming = m.data_roaming;
            rows.push(row_inline(
                t!("Интернет в роуминге"),
                t!("Передача данных вне домашней сети (может стоить дорого)"),
                Toggle::with_state(roaming).on_change(move |on| {
                    act(&t!("Интернет в роуминге"), Request::SetDataRoaming { on }, |s| {
                        load_modes(s);
                        load_status(s);
                    })
                }),
            ));
            let auto = m.manual.is_none();
            rows.push(row_inline(
                &t!("Выбирать сеть автоматически"),
                &match m.manual {
                    Some((mcc, mnc)) => t!("Вручную: {mcc}/{mnc}", mcc = format!("{:03}", mcc), mnc = format!("{:02}", mnc)),
                    None => t!("Телефон сам выбирает доступную сеть").into(),
                },
                Toggle::with_state(auto).on_change(move |on| {
                    if on {
                        act(&t!("Автовыбор сети"), Request::NetworkSelect { plmn: None }, load_modes);
                    } else {
                        state::toast(t!("Найдите сети и выберите оператора").to_string());
                    }
                }),
            ));
        }
    }
    let scanning = s.scanning.get();
    rows.push(row_inline(
        &t!("Доступные сети"),
        if scanning { t!("Поиск идёт до 3 минут; связь на это время может пропасть") } else { t!("Поиск операторов вокруг") },
        button(if scanning { t!("Поиск…") } else { t!("Найти сети") }, move || {
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
        Some(Err(e)) => rows.push(note(&t!("Поиск: {e}", e = e))),
        Some(Ok(list)) => {
            for n in list {
                let name = if n.name.is_empty() { format!("{:03}/{:02}", n.mcc, n.mnc) } else { n.name.clone() };
                let mut hint = format!("{:03}/{:02}", n.mcc, n.mnc);
                if !n.technologies.is_empty() {
                    hint.push_str(&format!(" · {}", n.technologies.join(", ")));
                }
                if n.forbidden {
                    hint.push_str(&t!(" · запрещена"));
                }
                let plmn = (n.mcc, n.mnc);
                let ctl: W = if n.current {
                    boxed(Text::new(t!("Текущая")).class("row-value"))
                } else {
                    boxed(button(&t!("Выбрать"), move || act(&t!("Регистрация в сети"), Request::NetworkSelect { plmn: Some(plmn) }, |s| {
                        load_modes(s);
                        load_status(s);
                    })))
                };
                rows.push(row_inline(&name, &hint, ctl));
            }
        }
        None => {}
    }
    group(&t!("Сеть"), rows)
}

fn apn_group(s: Sigs) -> W {
    let mut rows: Vec<W> = Vec::new();
    if let Some(a) = s.apn_edit.get() {
        return group(&t!("Точка доступа"), apn_form(s, a));
    }
    match s.apns.get() {
        None => rows.push(note(&t!("Чтение точек доступа…"))),
        Some(Err(e)) => rows.push(note(&e)),
        Some(Ok(list)) => {
            for a in list {
                let title = if a.name.is_empty() { a.apn.clone() } else { format!("{} ({})", a.name, a.apn) };
                let hint = format!(
                    "{}{} · {}{}",
                    if a.default { t!("Для интернета · ") } else { "".to_string() },
                    a.ip,
                    if a.auth == "none" { t!("без входа").to_string() } else { t!("вход {auth}", auth = a.auth) },
                    if a.roaming_disallowed { t!(" · не в роуминге") } else { "".to_string() }
                );
                let (a1, idx) = (a.clone(), a.index);
                let mut actions = Row::new().gap(6.0).child(icon_button("\u{E3C9}", move || s.apn_edit.set(Some(a1.clone()))));
                if !a.default {
                    actions = actions.child(icon_button(icons::DELETE, move || act(&t!("Удаление точки доступа"), Request::ApnDelete { index: idx }, load_apns)));
                }
                rows.push(row_inline(&title, &hint, actions));
            }
        }
    }
    rows.push(row_inline(
        t!("Новая точка доступа"),
        t!("APN, имя пользователя и пароль даёт оператор"),
        button(&t!("Добавить"), move || s.apn_edit.set(Some(Apn { auth: "none".into(), ip: "ipv4v6".into(), ..Apn::default() }))),
    ));
    group(&t!("Точки доступа (APN)"), rows)
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
            d = d.item(DropdownItem::new(*v, tl(l)));
        }
        d.selected(sig.get_untracked()).on_change(move |v: &str| sig.set(v.to_string()))
    };
    vec![
        row(&t!("Название"), "", text_input(&a.name, &t!("Например, «Интернет»"), name)),
        row("APN", "", text_input(&a.apn, "internet", apn)),
        row(&t!("Пользователь"), "", text_input(&a.user, &t!("необязательно"), user)),
        row(&t!("Пароль"), "", text_input(&a.password, &t!("необязательно"), pass)),
        row(&t!("Проверка подлинности"), "", dd(auth, &[("none", n_!("Нет")), ("pap", "PAP"), ("chap", "CHAP"), ("pap-chap", n_!("PAP или CHAP"))])),
        row(&t!("Протокол"), "", dd(ip, &[("ipv4", "IPv4"), ("ipv6", "IPv6"), ("ipv4v6", n_!("IPv4 и IPv6"))])),
        row_inline(t!("Для мобильного интернета"), t!("Эту точку доступа использует передача данных"), Toggle::with_state(a.default).on_change(move |v| default.set(v))),
        row_inline(&t!("Запретить в роуминге"), "", Toggle::with_state(a.roaming_disallowed).on_change(move |v| roam.set(v))),
        boxed(
            Row::new()
                .gap(8.0)
                .class("setting-row")
                .child(DecoratedBox::new().class("grow"))
                .child(button(&t!("Отмена"), move || s.apn_edit.set(None)))
                .child(primary_button(&t!("Сохранить"), move || {
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
                        state::toast(t!("Укажите APN").to_string());
                        return;
                    }
                    s.apn_edit.set(None);
                    act(&t!("Точка доступа"), Request::ApnSave { apn }, load_apns);
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
        (api::SimState::PinRequired, Some(n)) => t!("SIM заблокирована: введите PIN (осталось попыток: {n})", n = n),
        (api::SimState::PukRequired | api::SimState::Blocked, _) => t!("PIN заблокирован: нужен PUK-код от оператора").into(),
        (_, Some(n)) => t!("{v}Осталось попыток ввода PIN: {n}", v = match st.as_ref().and_then(|x| x.sim.pin_enabled) {
                Some(true) => t!("PIN запрашивается при включении. "),
                Some(false) => t!("PIN не запрашивается. "),
                None => String::new(),
            }, n = n),
        _ => String::new(),
    };
    if let Some(form) = s.pin_form.get() {
        let a = use_signal(String::new());
        let b = use_signal(String::new());
        let (la, lb) = match form {
            "change" => (t!("Текущий PIN"), Some(t!("Новый PIN"))),
            "unblock" => (t!("PUK-код"), Some(t!("Новый PIN"))),
            _ => (t!("PIN-код"), None),
        };
        let mut r = Row::new().gap(8.0).class("setting-row").child(
            TextField::new().obscure(true).autofocus(true).placeholder(la).on_change(move |t| a.set(t.to_string())).class("grow"),
        );
        if let Some(lb) = lb {
            r = r.child(TextField::new().obscure(true).placeholder(lb).on_change(move |t| b.set(t.to_string())).class("grow"));
        }
        r = r.child(button(&t!("Отмена"), move || s.pin_form.set(None))).child(primary_button(&t!("Готово"), move || {
            let (x, y) = (a.get_untracked(), b.get_untracked());
            s.pin_form.set(None);
            let req = match form {
                "enable" => Request::PinEnable { on: true, pin: x },
                "disable" => Request::PinEnable { on: false, pin: x },
                "change" => Request::PinChange { old: x, new: y },
                "unblock" => Request::PinUnblock { puk: x, new: y },
                _ => Request::PinVerify { pin: x },
            };
            act(n_!("PIN-код"), req, load_status);
        }));
        rows.push(boxed(r));
    } else {
        let mut actions = Row::new().gap(6.0);
        match sim_state {
            api::SimState::PinRequired => actions = actions.child(primary_button(&t!("Ввести PIN"), move || s.pin_form.set(Some("verify")))),
            api::SimState::PukRequired | api::SimState::Blocked => {
                actions = actions.child(primary_button(&t!("Ввести PUK"), move || s.pin_form.set(Some("unblock"))))
            }
            _ => {
                let on = st.as_ref().and_then(|x| x.sim.pin_enabled);
                if on != Some(true) {
                    actions = actions.child(button(&t!("Включить запрос"), move || s.pin_form.set(Some("enable"))));
                }
                if on != Some(false) {
                    actions = actions
                        .child(button(&t!("Выключить запрос"), move || s.pin_form.set(Some("disable"))))
                        .child(button(&t!("Сменить PIN"), move || s.pin_form.set(Some("change"))));
                }
            }
        }
        rows.push(row(&t!("PIN-код SIM"), &hint, actions));
    }
    group(&t!("Защита SIM"), rows)
}

fn usage_group(s: Sigs) -> W {
    let rows = match s.usage.get() {
        None => vec![note(&t!("Чтение…"))],
        Some(Err(e)) => vec![note(&e)],
        Some(Ok(u)) => {
            let since = if u.since > 0 { t!("С {v}", v = synmodem::time::day_title(u.since).to_lowercase()) } else { t!("Всего").into() };
            vec![
                kv(&t!("В этом месяце"), format!("↓ {} · ↑ {}", human_bytes(u.rx), human_bytes(u.tx))),
                kv(&since, format!("↓ {} · ↑ {}", human_bytes(u.total_rx), human_bytes(u.total_tx))),
                row_inline(&t!("Сбросить общий счётчик"), "", button(&t!("Сбросить"), move || act(n_!("Счётчик трафика"), Request::UsageReset, load_usage))),
            ]
        }
    };
    group(&t!("Мобильный трафик"), rows)
}

// ═══ Вызовы и SMS ═══════════════════════════════════════════════════════════

pub fn calls_sms() -> W {
    let s = sigs();
    load_services(s);
    load_smsc(s);
    page(
        t!("Вызовы и SMS"),
        t!("Ожидание вызова, скрытие номера, переадресация, SMS-центр и USSD-запросы. Настройки вызовов хранит сеть оператора."),
        vec![
            boxed(Reactive::new(move || vec![services_group(s)])),
            boxed(Reactive::new(move || vec![ussd_group(s)])),
            boxed(Reactive::new(move || vec![smsc_group(s)])),
        ],
    )
}

fn services_group(s: Sigs) -> W {
    let sv = match s.services.get() {
        None => return group(&t!("Вызовы"), vec![note(&t!("Запрос настроек у сети (до полуминуты)…"))]),
        Some(Err(e)) => return group(&t!("Вызовы"), vec![no_daemon(&e)]),
        Some(Ok(v)) => v,
    };
    let mut rows: Vec<W> = Vec::new();
    if !sv.error.is_empty() {
        rows.push(note(&t!("Сеть не дала настройки вызовов ({error}). В сетях с VoLTE эти услуги управляются через мобильный интернет оператора (XCAP) и в роуминге часто недоступны.", error = sv.error)));
    }
    match sv.waiting {
        Some(on) => rows.push(row_inline(
            t!("Ожидание вызова"),
            t!("Сообщать о втором звонке во время разговора"),
            Toggle::with_state(on).on_change(move |v| act(&t!("Ожидание вызова"), Request::SetCallWaiting { on: v }, load_services)),
        )),
        None => rows.push(kv(&t!("Ожидание вызова"), t!("недоступно"))),
    }
    let mut dd = Dropdown::new().width(220.0);
    for (v, l) in [("network", t!("Как задано у оператора")), ("hide", t!("Скрывать")), ("show", t!("Показывать"))] {
        dd = dd.item(DropdownItem::new(v, l));
    }
    rows.push(row(
        t!("Мой номер при звонке"),
        t!("Скрытие номера (АОН) для исходящих"),
        dd.selected(sv.clir.clone()).on_change(|v: &str| act(&t!("Скрытие номера"), Request::SetClir { mode: v.to_string() }, load_services)),
    ));
    for f in sv.forwards {
        let label = match f.reason.as_str() {
            "always" => t!("Переадресация: всегда"),
            "busy" => t!("Если занят"),
            "no-reply" => t!("Если нет ответа"),
            _ => t!("Если недоступен"),
        };
        let num = use_signal(String::new());
        let reason = f.reason.clone();
        let reason2 = f.reason.clone();
        let hint = if f.active { t!("Включена на {number}", number = f.number) } else { t!("Выключена").into() };
        rows.push(row(
            label,
            &hint,
            Row::new()
                .gap(6.0)
                .child(text_input(&f.number, &t!("Номер"), num))
                .child(button(&t!("Включить"), move || {
                    let n = num.get_untracked();
                    if n.trim().is_empty() {
                        state::toast(t!("Укажите номер").to_string());
                        return;
                    }
                    act(&t!("Переадресация"), Request::SetForward { reason: reason.clone(), number: n, timer: Some(20) }, load_services);
                }))
                .child(button(&t!("Выключить"), move || {
                    act(&t!("Переадресация"), Request::SetForward { reason: reason2.clone(), number: String::new(), timer: None }, load_services)
                })),
        ));
    }
    group(&t!("Вызовы"), rows)
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
                .placeholder(if reply { t!("Ответ (номер пункта)") } else { t!("Например, *100#") })
                .on_change(move |t| code.set(t.to_string()))
                .on_submit(move |_| send2())
                .class("grow"),
        )
        .child(primary_button(if reply { n_!("Ответить") } else { n_!("Отправить") }, send));
    if reply {
        r = r.child(button(&t!("Закончить"), move || {
            s.ussd_reply.set(false);
            std::thread::spawn(|| {
                let _ = api::request(&Request::UssdCancel);
            });
        }));
    }
    rows.push(boxed(r));
    rows.push(note(&t!("USSD — служебные запросы оператора: баланс, остатки, подключение услуг.")));
    group(&t!("USSD-запросы"), rows)
}

fn smsc_group(s: Sigs) -> W {
    let num = use_signal(String::new());
    let rows = match s.smsc.get() {
        None => vec![note(&t!("Чтение…"))],
        Some(Err(e)) => vec![note(&e)],
        Some(Ok(v)) => vec![row(
            t!("SMS-центр"),
            t!("Номер центра SMS оператора; обычно его записывает SIM"),
            Row::new()
                .gap(6.0)
                .child(text_input(&v, "+7…", num))
                .child(button(&t!("Сохранить"), move || act(&t!("SMS-центр"), Request::SetSmsc { number: num.get_untracked() }, load_smsc))),
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
        t!("О модеме"),
        t!("Идентификаторы, прошивка, поддерживаемые диапазоны и текущая сота."),
        vec![
            boxed(Reactive::new(move || -> Vec<W> {
                match s.info.get() {
                    None => vec![note(&t!("Чтение…"))],
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
            t!("Слот {physical}: {v}{v2}", physical = s.physical, v = if !s.card { t!("пусто") } else if s.euicc { "eSIM".into() } else { "SIM".into() }, v2 = if s.card && s.active { t!(" (логический {n})", n = s.logical) } else { String::new() })
        })
        .collect();
    let bands = |b: &[u16], p: &str| b.iter().map(|x| format!("{p}{x}")).collect::<Vec<_>>().join(" ");
    col(vec![
        group(
            &t!("Устройство"),
            vec![
                kv("IMEI", i.imei.clone()),
                kv("IMEI SV", i.imei_sv.clone()),
                kv(&t!("Прошивка модема"), i.revision.clone()),
                kv(&t!("Сборка ПО модема"), i.sw_version.clone()),
                kv(&t!("Аппаратная ревизия"), i.hw_revision.clone()),
            ],
        ),
        group(
            "SIM",
            vec![
                kv(&t!("Оператор SIM"), i.spn.clone()),
                kv(&t!("Номер телефона"), if i.msisdn.is_empty() { t!("не записан на SIM").into() } else { i.msisdn.clone() }),
                kv("ICCID", i.iccid.clone()),
                kv("IMSI", i.imsi.clone()),
                kv("EID (eSIM)", i.eid.clone()),
                kv(&t!("Слоты"), slots.join(" · ")),
            ],
        ),
        group(&t!("Диапазоны"), vec![kv("LTE", bands(&i.lte_bands, "B")), kv("5G NR", bands(&i.nr_bands, "n"))]),
    ])
}

fn cell_group(c: api::Cell) -> W {
    let o = |v: Option<i32>, u: &str| v.map(|x| format!("{x} {}", tl(u))).unwrap_or_default();
    let s = sigs();
    group(
        &t!("Текущая сота"),
        vec![
            kv(&t!("Сеть"), format!("{} {}", c.technology, c.plmn)),
            kv(&t!("Диапазон"), format!("{}{}", c.band, c.bandwidth_mhz.map(|b| t!(" · {b} МГц", b = b)).unwrap_or_default())),
            kv(&t!("Канал (EARFCN/ARFCN)"), c.channel.map(|x| x.to_string()).unwrap_or_default()),
            kv(&t!("Код зоны (TAC/LAC)"), c.tac.or(c.lac).map(|x| x.to_string()).unwrap_or_default()),
            kv(&t!("Номер соты"), c.cell_id.map(|x| t!("{x} (eNB {v}, сектор {v2})", x = x, v = x >> 8, v2 = x & 0xFF)).unwrap_or_default()),
            kv("RSRP", o(c.rsrp, n_!("дБм"))),
            kv("RSRQ", o(c.rsrq, n_!("дБ"))),
            kv("RSSI", o(c.rssi, n_!("дБм"))),
            kv("SINR", c.snr.map(|x| t!("{x} дБ", x = format!("{:.1}", x))).unwrap_or_default()),
            row_inline(&t!("Обновить"), "", button(&t!("Обновить"), move || load_cell(s))),
        ],
    )
}
