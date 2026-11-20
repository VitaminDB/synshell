//! Окно соединений: кто, куда и когда соединялся (журнал synnetd и открытые
//! сейчас), с поиском, выбором промежутка и подробностями записи. Одно окно
//! на программу: повторный двойной щелчок поднимает уже открытое.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use syngui::app::{open_window, WindowConfig, WindowHandle};
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::window::WindowState;
use syngui::GestureDetector;
use syngui::widgets::containers::{VirtualList, VirtualRow};
use syn_netd::{Conn, ConnQuery};

use crate::netapps;
use crate::state::icons;
use crate::ui::parts::{self, boxed, W};

/// Промежуток журнала.
#[derive(Clone, PartialEq, Debug)]
pub enum Range {
    /// Только открытые сейчас.
    Active,
    Hour,
    Today,
    Week,
    Month,
    All,
    /// День архива `ГГГГ-ММ-ДД`.
    Day(String),
}

impl Range {
    fn label(&self) -> String {
        match self {
            Range::Active => t!("Открытые"),
            Range::Hour => t!("Час"),
            Range::Today => t!("Сегодня"),
            Range::Week => t!("7 дней"),
            Range::Month => t!("30 дней"),
            Range::All => t!("Весь журнал"),
            Range::Day(d) => {
                let (dd, wd) = netapps::day_label(d);
                format!("{dd}, {wd}")
            }
        }
    }

    /// (с, по, только открытые).
    fn bounds(&self) -> (Option<u64>, Option<u64>, bool) {
        let now = netapps::now_ms();
        match self {
            Range::Active => (None, None, true),
            Range::Hour => (Some(now - 3_600_000), None, false),
            Range::Today => (Some(netapps::today_start()), None, false),
            Range::Week => (Some(now - 7 * 86_400_000), None, false),
            Range::Month => (Some(now - 30 * 86_400_000), None, false),
            Range::All => (None, None, false),
            Range::Day(d) => match netapps::day_range(d) {
                Some((a, b)) => (Some(a), Some(b), false),
                None => (None, None, false),
            },
        }
    }

    /// Обновлять раз в 2 с (прошлый день не меняется).
    fn live(&self) -> bool {
        !matches!(self, Range::Day(_))
    }
}

/// Сколько записей запрашивать.
const LIMIT: u32 = 3000;

#[derive(Clone, Default)]
struct Data {
    list: Arc<Vec<Conn>>,
    error: Option<String>,
    loaded: bool,
}

thread_local! {
    static OPEN: RefCell<HashMap<Option<String>, WindowHandle>> = RefCell::new(HashMap::new());
}

fn key(c: &Conn) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (c.start, c.pid, &c.remote, c.port, &c.proto, &c.comm, &c.app).hash(&mut h);
    h.finish()
}

fn duration_ms(c: &Conn) -> String {
    let end = c.end.unwrap_or_else(netapps::now_ms);
    let ms = end.saturating_sub(c.start);
    if ms < 1000 {
        t!("{n} мс", n = ms)
    } else {
        parts::duration(ms / 1000)
    }
}

fn addr(c: &Conn) -> String {
    if c.remote.contains(':') {
        format!("[{}]:{}", c.remote, c.port)
    } else {
        format!("{}:{}", c.remote, c.port)
    }
}

/// Известные порты — подпись вместо числа.
fn service(port: u16, proto: &str) -> Option<&'static str> {
    Some(match (port, proto) {
        (443, "udp") => "QUIC",
        (443, _) => "HTTPS",
        (80, _) => "HTTP",
        (53, _) => "DNS",
        (853, _) => "DoT",
        (22, _) => "SSH",
        (123, _) => "NTP",
        (993, _) | (143, _) => "IMAP",
        (465, _) | (587, _) | (25, _) => "SMTP",
        (5228, _) => "Google Push",
        (3478, _) | (19302, _) => "STUN",
        (6881..=6889, _) => "BitTorrent",
        (51820, "udp") => "WireGuard",
        (1194, _) => "OpenVPN",
        _ => return None,
    })
}

/// Открыть окно соединений программы (`None` — всех) за промежуток.
pub fn open(app: Option<String>, title: String, range: Range) {
    if let Some(h) = OPEN.with(|o| o.borrow().get(&app).copied()).filter(|h| h.is_open()) {
        h.focus();
        return;
    }
    let info = app.as_deref().map(netapps::info);
    let win: RwSignal<WindowState> = use_signal(WindowState::default());
    let data: RwSignal<Data> = use_signal(Data::default());
    let range_sig: RwSignal<Range> = use_signal(range.clone());
    let text: RwSignal<String> = use_signal(String::new());
    let sel: RwSignal<Option<u64>> = use_signal(None);
    let alive = Arc::new(AtomicBool::new(true));
    let (tx, rx) = mpsc::channel::<(Range, String)>();
    // Загрузка в своём потоке: новый запрос — сразу, иначе раз в 2 с.
    {
        let alive = alive.clone();
        let app = app.clone();
        std::thread::Builder::new()
            .name("conns".into())
            .spawn(move || {
                let mut cur = (range.clone(), String::new());
                loop {
                    let (from, to, active_only) = cur.0.bounds();
                    let q = ConnQuery { app: app.clone(), from, to, text: cur.1.clone(), active_only, limit: LIMIT };
                    let r = syn_netd::connections(q);
                    if !alive.load(Ordering::Relaxed) {
                        return;
                    }
                    run_on_main_thread(move || match r {
                        Ok(list) => data.set_always(Data { list: Arc::new(list), error: None, loaded: true }),
                        Err(e) => data.update(|d| {
                            d.error = Some(e.to_string());
                            d.loaded = true;
                        }),
                    });
                    let wait = if cur.0.live() { Duration::from_secs(2) } else { Duration::from_secs(3600) };
                    match rx.recv_timeout(wait) {
                        Ok(mut q) => {
                            // Пачка изменений (набор текста) — последнее.
                            while let Ok(n) = rx.try_recv() {
                                q = n;
                            }
                            cur = q;
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                    if !alive.load(Ordering::Relaxed) {
                        return;
                    }
                }
            })
            .ok();
    }
    let tx = Arc::new(std::sync::Mutex::new(tx));
    let send = {
        let tx = tx.clone();
        move || {
            let _ = tx.lock().unwrap().send((range_sig.get_untracked(), text.get_untracked()));
        }
    };
    let window_title = t!("Соединения — {t}", t = title);
    let app_key = app.clone();
    let alive2 = alive.clone();
    let cfg = WindowConfig::new("connections")
        .title(window_title.clone())
        .size(1120, 700)
        .min_size(380, 360)
        .frameless()
        .transparent(true)
        .with_window_state(win)
        .on_close(move || {
            alive2.store(false, Ordering::Relaxed);
            OPEN.with(|o| o.borrow_mut().remove(&app_key));
        });
    let icon_path = info.and_then(|i| i.icon);
    let app_for_body = app.clone();
    let h = open_window(cfg, move |_| body(win, window_title.clone(), range_sig, text, data, sel, send.clone(), icon_path.clone(), app_for_body.clone()));
    OPEN.with(|o| o.borrow_mut().insert(app, h));
}

#[allow(clippy::too_many_arguments)]
fn body(win: RwSignal<WindowState>, title: String, range: RwSignal<Range>, text: RwSignal<String>, data: RwSignal<Data>, sel: RwSignal<Option<u64>>, send: impl Fn() + Clone + Send + Sync + 'static, icon: Option<std::path::PathBuf>, app: Option<String>) -> W {
    let all_apps = app.is_none();
    let body = Column::new()
        .gap(0.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(synshell_common::csd::titlebar(win, move || title.clone(), Some(icons::LIST_ALT)))
        .child(toolbar(range, text, data, send, icon, app))
        .child(list(data, sel, all_apps).class("grow"))
        .child(detail(data, sel));
    synshell_common::csd::frame(DecoratedBox::new().child(body).class("root conns-root"))
}

/// Окно соединений целиком, с данными, загруженными сразу (снимок экрана
/// `--screenshot … --connections ПРОГРАММА`); `select` — раскрыть первую запись.
pub fn snapshot(app: Option<String>, range: Range, select: bool) -> W {
    let (from, to, active_only) = range.bounds();
    let list = syn_netd::connections(ConnQuery { app: app.clone(), from, to, active_only, limit: LIMIT, ..Default::default() });
    let title = app.as_deref().map(|a| netapps::info(a).name).unwrap_or_else(|| t!("Все программы"));
    let data = match list {
        Ok(l) => Data { list: Arc::new(l), error: None, loaded: true },
        Err(e) => Data { list: Arc::new(Vec::new()), error: Some(e.to_string()), loaded: true },
    };
    let first = data.list.first().map(key);
    let sel = use_signal(if select { first } else { None });
    let icon = app.as_deref().and_then(|a| netapps::info(a).icon);
    body(use_signal(WindowState::default()), t!("Соединения — {t}", t = title), use_signal(range), use_signal(String::new()), use_signal(data), sel, || {}, icon, app)
}

fn toolbar(range: RwSignal<Range>, text: RwSignal<String>, data: RwSignal<Data>, send: impl Fn() + Clone + Send + Sync + 'static, icon: Option<std::path::PathBuf>, app: Option<String>) -> W {
    let saved: RwSignal<Option<std::result::Result<std::path::PathBuf, String>>> = use_signal(None);
    let s1 = send.clone();
    let search = TextField::new()
        .placeholder(t!("Поиск: адрес, узел, порт, процесс, канал"))
        .prefix_icon(icons::SEARCH)
        .on_change(move |s| {
            text.set(s.to_string());
            s1();
        })
        .class("search grow");
    let ranges = Reactive::new(move || -> Vec<W> {
        let cur = range.get();
        let mut row = Row::new().gap(2.0).class("seg");
        let mut opts = vec![Range::Active, Range::Hour, Range::Today, Range::Week, Range::Month, Range::All];
        if let Range::Day(_) = &cur {
            opts.push(cur.clone());
        }
        for r in opts {
            let active = r == cur;
            let s = send.clone();
            let label = r.label();
            row = row.child(
                GestureDetector::new()
                    .on_click(move || {
                        range.set(r.clone());
                        s();
                    })
                    .child(DecoratedBox::new().child(Text::new(label).class("seg-label")).class(if active { "seg-item active" } else { "seg-item" })),
            );
        }
        vec![boxed(row)]
    });
    let summary = Reactive::new(move || -> Vec<W> {
        let d = data.get();
        if let Some(e) = &d.error {
            return vec![boxed(Text::new(t!("Ошибка: {e}", e = e)).max_lines(2).class("muted small conns-err"))];
        }
        if !d.loaded {
            return vec![boxed(Text::new(t!("Загрузка…")).class("muted small"))];
        }
        let (rx, tx) = d.list.iter().fold((0u64, 0u64), |a, c| (a.0 + c.rx, a.1 + c.tx));
        let open = d.list.iter().filter(|c| c.end.is_none()).count();
        let mut s = t!("Записей: {n} · открыто {o} · ↓ {rx}  ↑ {tx}", n = d.list.len(), o = open, rx = parts::bytes(rx), tx = parts::bytes(tx));
        if d.list.len() as u32 >= LIMIT {
            s.push_str(&t!(" · показаны последние {n}", n = LIMIT));
        }
        let app = app.clone();
        let list = d.list.clone();
        let mut row = Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(s).max_lines(1).class("muted small grow"));
        match saved.get() {
            Some(Ok(p)) => {
                let dir = p.parent().map(|d| d.to_path_buf()).unwrap_or_default();
                row = row.child(Text::new(t!("Сохранено: {p}", p = p.display())).max_lines(1).class("muted small")).child(Button::new(t!("Открыть папку")).icon(icons::FOLDER).class("btn").on_click(move || {
                    let _ = std::process::Command::new("xdg-open").arg(&dir).spawn();
                }));
            }
            Some(Err(e)) => row = row.child(Text::new(t!("Не сохранить: {e}", e = e)).max_lines(1).class("muted small conns-err")),
            None => {}
        }
        if !list.is_empty() {
            row = row.child(Button::new(t!("Экспорт CSV")).icon(icons::DOWNLOAD).class("btn").on_click(move || saved.set(Some(export_csv(&list, app.as_deref()).map_err(|e| e.to_string())))));
        }
        vec![boxed(row)]
    });
    let mut top = Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center);
    if let Some(p) = icon {
        top = top.child(Image::new(p.to_string_lossy()).placeholder(false).class("net-app-icon").style("width", StyleValue::px(28.0)).style("height", StyleValue::px(28.0)));
    }
    top = top.child(search);
    boxed(Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(top).child(ScrollView::new().horizontal().child(ranges)).child(summary).class("toolbar conns-toolbar"))
}

fn list(data: RwSignal<Data>, sel: RwSignal<Option<u64>>, all_apps: bool) -> Column {
    let narrow = syngui::viewport::viewport_below(760.0);
    let head = Reactive::new(move || -> Vec<W> {
        if narrow.get() {
            return vec![];
        }
        let mut r = Row::new()
            .gap(10.0)
            .child(DecoratedBox::new().style("width", StyleValue::px(8.0)).style("height", StyleValue::px(1.0)))
            .child(Text::new(t!("Время")).class("th c-time"))
            .child(Text::new(if all_apps { t!("Программа") } else { t!("Процесс") }).class("th c-proc"))
            .child(Text::new(t!("Протокол")).class("th c-proto"))
            .child(Text::new(t!("Куда")).class("th grow"))
            .child(Text::new(t!("Канал")).class("th c-if"))
            .child(Text::new(t!("↓ принято")).class("th c-num"))
            .child(Text::new(t!("↑ передано")).class("th c-num"))
            .child(Text::new(t!("Длилось")).class("th c-dur"));
        r = r.cross_axis_alignment(CrossAxisAlignment::Center).class("thead conns-thead");
        vec![boxed(r)]
    });
    let rows = Reactive::new(move || -> Vec<W> {
        let d = data.get();
        let narrow = narrow.get();
        let selected = sel.get();
        if d.loaded && d.list.is_empty() && d.error.is_none() {
            return vec![boxed(DecoratedBox::new().child(parts::empty(t!("Соединений нет — смените промежуток или поиск."))).class("conns-empty"))];
        }
        let list = d.list.clone();
        let rows: Vec<VirtualRow> = (0..list.len())
            .map(|i| {
                let c = &list[i];
                let k = key(c);
                let version = c.rx.wrapping_add(c.tx).wrapping_mul(31).wrapping_add(c.end.unwrap_or(1)) ^ (selected == Some(k)) as u64 ^ (narrow as u64) << 1;
                let list = list.clone();
                VirtualRow::new(k, version, move || row(&list[i], k, sel, selected == Some(k), narrow, all_apps))
            })
            .collect();
        vec![boxed(VirtualList::new(rows).estimated_row_height(if narrow { 64.0 } else { 46.0 }).gap(2.0).class("grow conns-list"))]
    });
    Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(head).child(DecoratedBox::new().child(rows).class("grow conns-body"))
}

fn row(c: &Conn, k: u64, sel: RwSignal<Option<u64>>, selected: bool, narrow: bool, all_apps: bool) -> W {
    let open = c.end.is_none();
    let dot = DecoratedBox::new().class(if open { "conn-dot open" } else { "conn-dot" });
    let who = if all_apps {
        netapps::info(&c.app).name
    } else if c.pid > 0 {
        format!("{} · {}", if c.comm.is_empty() { c.app.clone() } else { c.comm.clone() }, c.pid)
    } else {
        t!("ядро")
    };
    let host = c.host.clone().unwrap_or_else(|| c.remote.clone());
    let mut second = if c.host.is_some() { addr(c) } else { format!(":{}", c.port) };
    if let Some(s) = service(c.port, &c.proto) {
        second = format!("{second} · {s}");
    }
    if !c.also.is_empty() || c.more > 0 {
        second = format!("{second} · {}", t!("и ещё {n} адресов", n = c.also.len() as u32 + c.more));
    }
    if c.incoming {
        second = format!("{} · {second}", t!("входящее"));
    }
    let where_ = Column::new().gap(0.0).child(Text::new(host).max_lines(1).class("conn-host")).child(Text::new(second).max_lines(1).class("muted small"));
    let content: W = if narrow {
        boxed(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(dot)
                .child(
                    Column::new()
                        .gap(1.0)
                        .child(where_)
                        .child(Text::new(format!("{} · {} · {} · ↓ {} ↑ {}", netapps::when(c.start), who, c.proto.to_uppercase(), parts::bytes(c.rx), parts::bytes(c.tx))).max_lines(1).class("muted small"))
                        .class("grow"),
                ),
        )
    } else {
        boxed(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(dot)
                .child(Text::new(netapps::when(c.start)).max_lines(1).class("c-time conn-time"))
                .child(Text::new(who).max_lines(1).class("c-proc"))
                .child(DecoratedBox::new().child(Text::new(c.proto.to_uppercase()).class("conn-proto-label")).class("c-proto conn-proto"))
                .child(where_.class("grow"))
                .child(Text::new(c.iface.clone()).max_lines(1).class("c-if muted"))
                .child(Text::new(parts::bytes(c.rx)).class("c-num num net-rx"))
                .child(Text::new(parts::bytes(c.tx)).class("c-num num net-tx"))
                .child(Text::new(if open { t!("открыто") } else { duration_ms(c) }).max_lines(1).class(if open { "c-dur conn-open" } else { "c-dur muted" })),
        )
    };
    boxed(GestureDetector::new().on_click(move || sel.set(if selected { None } else { Some(k) })).child(DecoratedBox::new().child(content).class(if selected { "row-item selected" } else { "row-item" })))
}

/// Папка загрузок (`XDG_DOWNLOAD_DIR` из user-dirs.dirs), иначе домашняя.
fn downloads_dir() -> std::path::PathBuf {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_else(|| "/tmp".into());
    let cfg = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| home.join(".config"));
    if let Ok(text) = std::fs::read_to_string(cfg.join("user-dirs.dirs")) {
        for l in text.lines() {
            if let Some(v) = l.trim().strip_prefix("XDG_DOWNLOAD_DIR=") {
                let p = v.trim_matches('"').replace("$HOME", &home.to_string_lossy());
                let p = std::path::PathBuf::from(p);
                if p.is_dir() {
                    return p;
                }
            }
        }
    }
    home
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Записи — в CSV (UTF-8, разделитель «,», время по местному часовому поясу) в папку загрузок.
pub fn export_csv(list: &[Conn], app: Option<&str>) -> std::io::Result<std::path::PathBuf> {
    let stamp = netapps::when_full(netapps::now_ms()).replace([' ', ':'], "-").replace('.', "-");
    let name = format!("connections-{}-{stamp}.csv", app.map(|a| a.replace(['/', ' '], "_")).unwrap_or_else(|| "all".into()));
    let path = downloads_dir().join(name);
    let mut out = String::from("start,end,duration_s,app,process,pid,user,protocol,direction,remote,port,host,interface,rx_bytes,tx_bytes,other_addresses\n");
    for c in list {
        let dur = c.end.unwrap_or_else(netapps::now_ms).saturating_sub(c.start) as f64 / 1000.0;
        let fields = [
            netapps::when_full(c.start),
            c.end.map(netapps::when_full).unwrap_or_default(),
            format!("{dur:.1}"),
            c.app.clone(),
            c.comm.clone(),
            c.pid.to_string(),
            if c.pid > 0 { netapps::user(c.uid) } else { String::new() },
            c.proto.clone(),
            if c.incoming { "in".into() } else { "out".into() },
            c.remote.clone(),
            c.port.to_string(),
            c.host.clone().unwrap_or_default(),
            c.iface.clone(),
            c.rx.to_string(),
            c.tx.to_string(),
            {
                let mut a = c.also.join(" ");
                if c.more > 0 {
                    a.push_str(&format!(" +{}", c.more));
                }
                a.trim().to_string()
            },
        ];
        out.push_str(&fields.iter().map(|f| csv_field(f)).collect::<Vec<_>>().join(","));
        out.push('\n');
    }
    std::fs::write(&path, out)?;
    Ok(path)
}

/// Подробности выбранной записи.
fn detail(data: RwSignal<Data>, sel: RwSignal<Option<u64>>) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let Some(k) = sel.get() else { return vec![] };
        let d = data.get();
        let Some(c) = d.list.iter().find(|c| key(c) == k) else { return vec![] };
        let mut rows = vec![(t!("Начало"), netapps::when_full(c.start))];
        match c.end {
            Some(e) => rows.push((t!("Конец"), format!("{} · {}", netapps::when_full(e), duration_ms(c)))),
            None => rows.push((t!("Состояние"), t!("открыто, {d}", d = duration_ms(c)))),
        }
        let info = netapps::info(&c.app);
        rows.push((t!("Программа"), if c.app.is_empty() || info.name == c.app { info.name } else { format!("{} ({})", info.name, c.app) }));
        if c.pid > 0 {
            rows.push((t!("Процесс"), format!("{} · PID {} · {}", c.comm, c.pid, netapps::user(c.uid))));
        }
        rows.push((t!("Куда"), format!("{} · {}{}", addr(c), c.proto.to_uppercase(), service(c.port, &c.proto).map(|s| format!(" ({s})")).unwrap_or_default())));
        if let Some(h) = &c.host {
            rows.push((t!("Узел"), h.clone()));
        }
        rows.push((t!("Направление"), if c.incoming { t!("входящее — к службе на этом компьютере") } else { t!("исходящее") }));
        if !c.iface.is_empty() {
            rows.push((t!("Канал"), c.iface.clone()));
        }
        rows.push((t!("Принято / передано"), format!("{} / {}", parts::bytes(c.rx), parts::bytes(c.tx))));
        if !c.also.is_empty() {
            let mut s = c.also.join("\n");
            if c.more > 0 {
                s.push_str(&format!("\n{}", t!("…и ещё {n}", n = c.more)));
            }
            rows.push((t!("Другие адреса"), s));
        }
        let text: String = rows.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join("\n");
        let head = Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Text::new(c.host.clone().unwrap_or_else(|| addr(c))).max_lines(1).selectable(true).class("card-title grow"))
            .child(Button::new(t!("Копировать")).icon(icons::COPY).class("btn").on_click(move || syngui::clipboard::copy(&text)))
            .child(Button::new("").icon(icons::CLOSE).class("btn round").on_click(move || sel.set(None)));
        // Широкое окно — в две колонки, чтобы список оставался виден.
        let kv: W = if viewport_size().get().width >= 900.0 {
            let half = rows.len().div_ceil(2);
            let right = rows.split_off(half);
            boxed(Grid::new(2).gap(16.0).child(parts::kv_list(rows)).child(parts::kv_list(right)))
        } else {
            parts::kv_list(rows)
        };
        vec![boxed(DecoratedBox::new().child(Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(head).child(kv)).class("card conns-detail"))]
    }))
}
