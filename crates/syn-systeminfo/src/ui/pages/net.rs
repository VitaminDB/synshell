//! Сеть: скорость, программы (расход сейчас, за день, неделю, месяц — от
//! synnetd), каналы, архив по дням и журнал соединений (окно по двойному
//! щелчку на программе).

use std::collections::HashMap;

use syngui::core::Color;
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::GestureDetector;

use super::{header, hero, Cx};
use crate::netapps;
use crate::sampler::{Iface, NetD, Snap};
use crate::state::{icons, NetPeriod, Page, St};
use crate::ui::conns::{self, Range};
use crate::ui::parts::{self, boxed, Line, W};

const RX: &str = "#22c3e6";
const TX: &str = "#ffca28";
/// Строк программ без «показать все».
const TOP: usize = 12;

fn iface_icon(kind: &str) -> &'static str {
    match kind {
        "wifi" => icons::WIFI,
        "mobile" => icons::CELL,
        "usb" => icons::USB,
        "vpn" => icons::VPN,
        "ethernet" => icons::ETHERNET,
        _ => icons::HUB,
    }
}

fn iface_kind(kind: &str) -> String {
    match kind {
        "wifi" => t!("Wi-Fi"),
        "mobile" => t!("Мобильная сеть"),
        "usb" => t!("USB"),
        "vpn" => t!("VPN"),
        "ethernet" => t!("Ethernet"),
        "bridge" => t!("Мост"),
        _ => t!("Виртуальный"),
    }
}

/// Строка таблицы программ за период.
#[derive(Clone, Default)]
struct AppRow {
    app: String,
    uid: u32,
    comm: String,
    rx: u64,
    tx: u64,
    rx_bps: f64,
    tx_bps: f64,
    active: u32,
    conns: u32,
    ifaces: HashMap<String, u64>,
}

impl AppRow {
    fn total(&self) -> u64 {
        self.rx + self.tx
    }
}

/// Программы за период: сегодня — из `traffic`, неделя и месяц — плюс архив.
fn rows(period: NetPeriod, t: &syn_netd::Traffic, usage: &[syn_netd::DayUsage]) -> Vec<AppRow> {
    let mut m: HashMap<(String, u32), AppRow> = HashMap::new();
    for a in &t.apps {
        let r = m.entry((a.app.clone(), a.uid)).or_insert_with(|| AppRow { app: a.app.clone(), uid: a.uid, ..Default::default() });
        r.comm = a.comm.clone();
        r.rx_bps = a.rx_bps;
        r.tx_bps = a.tx_bps;
        r.active = a.active;
        r.rx += a.rx_today;
        r.tx += a.tx_today;
        r.conns += a.conns_today + a.active;
        for (i, rx, tx) in &a.ifaces {
            *r.ifaces.entry(i.clone()).or_default() += rx + tx;
        }
    }
    let days = match period {
        NetPeriod::Week => 6,
        NetPeriod::Month => 29,
        _ => 0,
    };
    if days > 0 {
        let from = crate::sampler::day_str(netapps::now_ms() / 1000 - days * 86_400);
        let today = crate::sampler::day_str(netapps::now_ms() / 1000);
        for d in usage.iter().filter(|d| d.day >= from && d.day != today) {
            for u in &d.rows {
                let r = m.entry((u.app.clone(), u.uid)).or_insert_with(|| AppRow { app: u.app.clone(), uid: u.uid, ..Default::default() });
                r.rx += u.rx;
                r.tx += u.tx;
                r.conns += u.conns;
                if !u.iface.is_empty() {
                    *r.ifaces.entry(u.iface.clone()).or_default() += u.rx + u.tx;
                }
            }
        }
    }
    let mut v: Vec<AppRow> = m.into_values().collect();
    if period == NetPeriod::Now {
        v.retain(|r| r.rx_bps + r.tx_bps > 0.5 || r.active > 0);
        v.sort_by(|a, b| (b.rx_bps + b.tx_bps).total_cmp(&(a.rx_bps + a.tx_bps)).then(b.active.cmp(&a.active)));
    } else {
        v.retain(|r| r.total() > 0 || r.active > 0);
        v.sort_by(|a, b| b.total().cmp(&a.total()));
    }
    v
}

/// Строки одного дня архива.
fn day_rows(day: &str, t: &syn_netd::Traffic, usage: &[syn_netd::DayUsage]) -> Vec<AppRow> {
    if day == crate::sampler::day_str(netapps::now_ms() / 1000) {
        return rows(NetPeriod::Today, t, &[]);
    }
    let mut m: HashMap<(String, u32), AppRow> = HashMap::new();
    for u in usage.iter().filter(|d| d.day == day).flat_map(|d| d.rows.iter()) {
        let r = m.entry((u.app.clone(), u.uid)).or_insert_with(|| AppRow { app: u.app.clone(), uid: u.uid, ..Default::default() });
        r.rx += u.rx;
        r.tx += u.tx;
        r.conns += u.conns;
        if !u.iface.is_empty() {
            *r.ifaces.entry(u.iface.clone()).or_default() += u.rx + u.tx;
        }
    }
    let mut v: Vec<AppRow> = m.into_values().filter(|r| r.total() > 0).collect();
    v.sort_by(|a, b| b.total().cmp(&a.total()));
    v
}

fn avatar(key: &str, icon: Option<&std::path::Path>, size: f32) -> W {
    if let Some(p) = icon {
        return boxed(Image::new(p.to_string_lossy()).placeholder(false).class("net-app-icon").style("width", StyleValue::px(size)).style("height", StyleValue::px(size)));
    }
    if key.is_empty() {
        return boxed(parts::badge_icon(icons::HUB, Page::Network.color(), size));
    }
    let letter = key.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    const C: [&str; 8] = ["#4f9dff", "#2ec27e", "#b77cff", "#ff9f43", "#22c3e6", "#f06292", "#a3d65c", "#ff6b5b"];
    let h = key.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    boxed(
        DecoratedBox::new()
            .child(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(letter).class("avatar-letter")))
            .class("avatar")
            .style("background-color", Color::from_hex(C[h as usize % C.len()]))
            .style("width", StyleValue::px(size))
            .style("height", StyleValue::px(size)),
    )
}

/// Подпись под именем программы: процесс, пользователь, доли каналов.
fn row_sub(r: &AppRow, uplinks: &[syn_netd::Uplink], all_users: bool) -> String {
    let mut parts_v: Vec<String> = Vec::new();
    if r.app.is_empty() {
        parts_v.push(t!("ядро, пересылка, туннели"));
    } else if !r.comm.is_empty() && r.comm != r.app {
        parts_v.push(r.comm.clone());
    }
    // Чужой пользователь (у служб — их системная учётная запись, она и так ясна из имени).
    if all_users && !r.app.is_empty() && r.uid >= 1000 && r.uid != netapps::my_uid() {
        parts_v.push(netapps::user(r.uid));
    }
    let total: u64 = r.ifaces.values().sum();
    if total > 0 && r.ifaces.len() > 1 {
        let mut v: Vec<(&String, &u64)> = r.ifaces.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1));
        parts_v.push(v.iter().take(3).map(|(i, b)| format!("{} {}%", netapps::iface_label(i, uplinks), (**b * 100 / total))).collect::<Vec<_>>().join(" · "));
    } else if let Some(i) = r.ifaces.keys().next() {
        parts_v.push(netapps::iface_label(i, uplinks));
    }
    if r.active > 0 {
        parts_v.push(t!("открыто {n}", n = r.active));
    }
    parts_v.join(" · ")
}

/// Строка программы. Двойной щелчок (на телефоне — касание) — окно соединений.
fn app_row(cx: &Cx, r: &AppRow, max: f64, live: bool, range: Range, uplinks: &[syn_netd::Uplink], all_users: bool) -> W {
    let st = cx.st;
    let info = netapps::info(&r.app);
    let selected = st.net_sel.get_untracked().as_deref() == Some(r.app.as_str());
    let (down, up, value) = if live { (r.rx_bps, r.tx_bps, r.rx_bps + r.tx_bps) } else { (r.rx as f64, r.tx as f64, r.total() as f64) };
    let fmt = |v: f64| if live { parts::rate(v) } else { parts::bytes(v as u64) };
    let name = Column::new()
        .gap(1.0)
        .child(Text::new(info.name.clone()).max_lines(1).class("row-title"))
        .child(Text::new(row_sub(r, uplinks, all_users)).max_lines(1).class("muted small"))
        .class("grow");
    let mut row = Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(avatar(&r.app, info.icon.as_deref(), 32.0)).child(name);
    if cx.narrow {
        row = row.child(Column::new().gap(1.0).cross_axis_alignment(CrossAxisAlignment::End).child(Text::new(format!("↓ {}", fmt(down))).class("num net-rx")).child(Text::new(format!("↑ {}", fmt(up))).class("num net-tx")));
    } else {
        let share = if max > 0.0 { (value / max * 100.0) as f32 } else { 0.0 };
        row = row
            .child(DecoratedBox::new().child(parts::stacked(vec![((down / max.max(1.0)) as f32, Color::from_hex(RX)), ((up / max.max(1.0)) as f32, Color::from_hex(TX))], 6.0)).class("net-share").style("opacity", if share > 0.0 { 1.0 } else { 0.3 }))
            .child(Text::new(fmt(down)).class("num net-col net-rx"))
            .child(Text::new(fmt(up)).class("num net-col net-tx"))
            .child(Text::new(if r.conns > 0 { r.conns.to_string() } else { "—".into() }).class("num net-col-s muted"));
    }
    let app = r.app.clone();
    let title = info.name.clone();
    let (app2, title2) = (app.clone(), title.clone());
    let (app3, title3) = (app.clone(), title.clone());
    let touch = synshell_common::csd::is_phone();
    let range2 = range.clone();
    let range3 = range.clone();
    boxed(
        GestureDetector::new()
            .on_click(move || {
                if touch {
                    conns::open(Some(app.clone()), title.clone(), range.clone());
                } else {
                    st.net_sel.set(Some(app.clone()));
                }
            })
            .on_double_click(move || conns::open(Some(app2.clone()), title2.clone(), range2.clone()))
            .on_long_press(move |_| conns::open(Some(app3.clone()), title3.clone(), range3.clone()))
            .child(DecoratedBox::new().child(row).class(if selected { "row-item selected" } else { "row-item" })),
    )
}

/// «Сейчас | Сегодня | 7 дней | 30 дней».
fn period_switch(st: St) -> W {
    let cur = st.net_period.get_untracked();
    let mut row = Row::new().gap(2.0).class("seg");
    for (p, label) in [(NetPeriod::Now, t!("Сейчас")), (NetPeriod::Today, t!("Сегодня")), (NetPeriod::Week, t!("7 дней")), (NetPeriod::Month, t!("30 дней"))] {
        row = row.child(
            GestureDetector::new()
                .on_click(move || {
                    st.net_period.set(p);
                    st.net_all.set(false);
                })
                .child(DecoratedBox::new().child(Text::new(label).class("seg-label")).class(if cur == p { "seg-item active" } else { "seg-item" })),
        );
    }
    boxed(row)
}

fn button(label: String, icon: &'static str, f: impl Fn() + Send + Sync + 'static) -> W {
    boxed(Button::new(label).icon(icon).class("btn").on_click(f))
}

fn open_settings() {
    if let Err(e) = std::process::Command::new("synsettings").arg("uplinks").spawn() {
        tracing::warn!("synsettings: {e}");
    }
}

/// Карточка программ.
fn apps_card(cx: &Cx, d: &NetD, t: &syn_netd::Traffic) -> W {
    let st = cx.st;
    let color = Page::Network.color();
    let period = st.net_period.get();
    let uplinks: Vec<syn_netd::Uplink> = d.status.as_ref().and_then(|s| s.as_ref().ok()).map(|s| s.uplinks.clone()).unwrap_or_default();
    let list = rows(period, t, &d.usage);
    let live = period == NetPeriod::Now;
    let max = list.iter().map(|r| if live { r.rx_bps + r.tx_bps } else { r.total() as f64 }).fold(0.0, f64::max);
    let (sum_rx, sum_tx) = list.iter().fold((0f64, 0f64), |a, r| if live { (a.0 + r.rx_bps, a.1 + r.tx_bps) } else { (a.0 + r.rx as f64, a.1 + r.tx as f64) });
    let range = match period {
        NetPeriod::Now => Range::Active,
        NetPeriod::Today => Range::Today,
        NetPeriod::Week => Range::Week,
        NetPeriod::Month => Range::Month,
    };
    let mut head = Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(parts::badge_icon(icons::APPS, color, 30.0)).child(
        Column::new()
            .gap(0.0)
            .child(Text::new(t!("Программы")).max_lines(1).class("card-title"))
            .child(Text::new(if live { format!("↓ {}  ↑ {}", parts::rate(sum_rx), parts::rate(sum_tx)) } else { format!("↓ {}  ↑ {}", parts::bytes(sum_rx as u64), parts::bytes(sum_tx as u64)) }).max_lines(1).class("muted small"))
            .class("grow"),
    );
    if !cx.narrow {
        head = head.child(period_switch(st));
    }
    let mut col = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(head);
    if cx.narrow {
        col = col.child(ScrollView::new().horizontal().child(period_switch(st)));
    }
    if !cx.narrow && !list.is_empty() {
        col = col.child(
            Row::new()
                .gap(10.0)
                .child(Text::new(if synshell_common::csd::is_phone() { t!("Касание — соединения программы") } else { t!("Двойной щелчок — соединения программы") }).max_lines(1).class("th grow"))
                .child(Text::new(if live { t!("↓ в секунду") } else { t!("↓ принято") }).class("th net-col"))
                .child(Text::new(if live { t!("↑ в секунду") } else { t!("↑ передано") }).class("th net-col"))
                .child(Text::new(t!("Соед.")).class("th net-col-s"))
                .class("thead net-thead"),
        );
    }
    if list.is_empty() {
        col = col.child(parts::empty(if live { t!("Сейчас программы ничего не передают.") } else { t!("За этот период трафика не было.") }));
    }
    let all = st.net_all.get();
    let n = list.len();
    for r in list.iter().take(if all { usize::MAX } else { TOP }) {
        col = col.child(app_row(cx, r, max, live, range.clone(), &uplinks, t.all_users));
    }
    let mut foot = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center);
    if n > TOP {
        foot = foot.child(button(if all { t!("Свернуть") } else { t!("Ещё {n}", n = n - TOP) }, icons::LIST, move || st.net_all.set(!all)));
    }
    foot = foot.child(DecoratedBox::new().class("grow")).child(button(t!("Все соединения"), icons::LIST_ALT, move || conns::open(None, t!("Все программы"), range.clone())));
    col = col.child(foot.class("net-foot"));
    boxed(parts::card(col))
}

/// Каналы: скорость, мини-график, адреса, роль в synnetd.
fn ifaces(cx: &Cx, s: &Snap) -> Vec<W> {
    let color = Page::Network.color();
    let st = s.netd.status.as_ref().and_then(|r| r.as_ref().ok());
    let mut up: Vec<&Iface> = s.net.iter().filter(|i| i.info.up && i.info.kind != "virtual").collect();
    up.sort_by(|a, b| (b.info.rx_bytes + b.info.tx_bytes).cmp(&(a.info.rx_bytes + a.info.tx_bytes)));
    let mut items: Vec<W> = Vec::new();
    for i in up {
        let mut rows = vec![(t!("Скорость"), format!("↓ {}  ↑ {}", parts::rate(i.rx_bps), parts::rate(i.tx_bps))), (t!("Всего"), format!("↓ {}  ↑ {}", parts::bytes(i.info.rx_bytes), parts::bytes(i.info.tx_bytes)))];
        if !i.addrs.is_empty() {
            rows.push((t!("Адреса"), i.addrs.join("\n")));
        }
        if let Some(m) = &i.info.mac {
            rows.push((t!("MAC"), m.clone()));
        }
        if let Some(sp) = i.info.speed_mbps {
            rows.push((t!("Линия"), t!("{n} Мбит/с", n = sp)));
        }
        // Роль в synnetd.
        let role = st.and_then(|st| {
            let u = st.uplinks.iter().find(|u| u.iface == i.info.name)?;
            Some(if st.active && st.config.mode == syn_netd::Mode::Both {
                t!("все каналы сразу")
            } else if u.primary {
                t!("основной канал")
            } else {
                t!("запасной канал")
            })
        });
        let kind = iface_kind(&i.info.kind);
        let mut head = Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(parts::badge_icon(iface_icon(&i.info.kind), color, 30.0))
            .child(Column::new().gap(0.0).child(Text::new(i.info.name.clone()).max_lines(1).class("card-title")).child(Text::new(match &role { Some(r) => format!("{kind} · {r}"), None => kind }).max_lines(1).class("muted small")).class("grow"));
        if let Some((rx, tx)) = s.if_hist.get(&i.info.name) {
            let (rx, tx) = (rx.tail(60), tx.tail(60));
            let max = parts::nice_max(rx.iter().chain(tx.iter()).copied().fold(0.0, f32::max), 16.0 * 1024.0);
            head = head.child(
                Stack::new()
                    .child(parts::spark(rx, 60, max, Color::from_hex(RX), 96.0, 30.0))
                    .child(parts::spark(tx, 60, max, Color::from_hex(TX), 96.0, 30.0)),
            );
        }
        items.push(boxed(parts::card(Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(head).child(parts::kv_list(rows)))));
    }
    if items.is_empty() {
        return vec![parts::empty(t!("Нет подключений."))];
    }
    let cols = if cx.w >= 760.0 && items.len() > 1 { 2 } else { 1 };
    vec![boxed(Grid::new(cols).gap(14.0).children(items))]
}

/// Архив: столбики по дням, выбранный день — программы, журнал соединений.
fn archive_card(cx: &Cx, d: &NetD, t: &syn_netd::Traffic) -> W {
    let st = cx.st;
    let color = Page::Network.color();
    let today = crate::sampler::day_str(netapps::now_ms() / 1000);
    // Дни: 29 из архива и сегодня.
    let mut days: Vec<(String, u64, u64)> = Vec::new();
    let by_day: HashMap<&str, &syn_netd::DayUsage> = d.usage.iter().map(|u| (u.day.as_str(), u)).collect();
    let now_s = netapps::now_ms() / 1000;
    for k in (1..crate::sampler::ARCHIVE_DAYS).rev() {
        let day = crate::sampler::day_str(now_s - k * 86_400);
        let (rx, tx) = by_day.get(day.as_str()).map(|u| u.rows.iter().fold((0, 0), |a, r| (a.0 + r.rx, a.1 + r.tx))).unwrap_or((0, 0));
        days.push((day, rx, tx));
    }
    let (trx, ttx) = t.apps.iter().fold((0, 0), |a, r| (a.0 + r.rx_today, a.1 + r.tx_today));
    days.push((today.clone(), trx, ttx));
    // Первые дни без данных (учёт ещё не шёл) — не показывать.
    while days.len() > 7 && days.first().is_some_and(|d| d.1 + d.2 == 0) {
        days.remove(0);
    }
    let total: u64 = days.iter().map(|d| d.1 + d.2).sum();
    let with_data = days.iter().filter(|d| d.1 + d.2 > 0).count().max(1) as u64;
    let max = days.iter().map(|d| d.1 + d.2).max().unwrap_or(0).max(1) as f32;
    let sel = st.net_day.get().unwrap_or_else(|| today.clone());

    let mut col = Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(
        Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(parts::badge_icon(icons::ARCHIVE, color, 30.0)).child(
            Column::new()
                .gap(0.0)
                .child(Text::new(t!("Архив")).max_lines(1).class("card-title"))
                .child(Text::new(t!("{n} дн. · всего {t} · в среднем {a} в день", n = days.len(), t = parts::bytes(total), a = parts::bytes(total / with_data))).max_lines(2).class("muted small"))
                .class("grow"),
        ),
    );
    // Столбики.
    let h = if cx.narrow { 90.0 } else { 120.0 };
    let mut bars = Row::new().gap(if cx.narrow { 2.0 } else { 4.0 }).cross_axis_alignment(CrossAxisAlignment::End);
    for (day, rx, tx) in &days {
        let (rx_h, tx_h) = ((*rx as f32 / max * h).max(if *rx > 0 { 1.5 } else { 0.0 }), (*tx as f32 / max * h).max(if *tx > 0 { 1.5 } else { 0.0 }));
        let active = *day == sel;
        let (dd, wd) = netapps::day_label(day);
        let weekend = wd == syngui::i18n::t(n_!("сб")) || wd == syngui::i18n::t(n_!("вс"));
        let label = if active || days.len() <= 14 || day.ends_with("-01") || weekend && cx.w > 900.0 { dd.split('.').next().unwrap_or("").to_string() } else { String::new() };
        let day2 = day.clone();
        bars = bars.child(
            GestureDetector::new()
                .on_click(move || st.net_day.set(Some(day2.clone())))
                .child(
                    Column::new()
                        .gap(0.0)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .main_axis_alignment(MainAxisAlignment::End)
                        .child(DecoratedBox::new().class("net-bar-tx").style("height", StyleValue::px(tx_h)))
                        .child(DecoratedBox::new().class("net-bar-rx").style("height", StyleValue::px(rx_h)))
                        .child(Text::new(label).max_lines(1).class(if active { "net-bar-label active" } else { "net-bar-label" }))
                        .class(if active { "net-bar active" } else { "net-bar" })
                        .style("height", StyleValue::px(h + 18.0)),
                )
                .class("grow"),
        );
    }
    col = col.child(DecoratedBox::new().child(bars).style("height", StyleValue::px(h + 18.0)));
    // Выбранный день.
    let (dd, wd) = netapps::day_label(&sel);
    let (srx, stx) = days.iter().find(|d| d.0 == sel).map(|d| (d.1, d.2)).unwrap_or((0, 0));
    let day_title = if sel == today { t!("Сегодня") } else { format!("{dd}, {wd}") };
    let legend = Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(parts::legend(Color::from_hex(RX), t!("Принято"), parts::bytes(srx))).child(parts::legend(Color::from_hex(TX), t!("Передано"), parts::bytes(stx)));
    if cx.narrow {
        col = col.child(Text::new(day_title.clone()).max_lines(1).class("card-title")).child(legend);
    } else {
        col = col.child(Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(day_title.clone()).max_lines(1).class("card-title grow")).child(legend));
    }
    // По часам.
    let mut hours = [0u64; 24];
    for r in d.usage.iter().filter(|u| u.day == sel).flat_map(|u| u.rows.iter()) {
        for (i, v) in r.hours.iter().take(24).enumerate() {
            hours[i] += v;
        }
    }
    if hours.iter().any(|h| *h > 0) {
        let cur = (sel == today).then(|| crate::netapps::hour_now());
        col = col.child(hour_chart(&hours, cur, if cx.narrow { 48.0 } else { 64.0 }));
    }
    let list = day_rows(&sel, t, &d.usage);
    let rmax = list.first().map(|r| r.total()).unwrap_or(1) as f64;
    let uplinks: Vec<syn_netd::Uplink> = d.status.as_ref().and_then(|s| s.as_ref().ok()).map(|s| s.uplinks.clone()).unwrap_or_default();
    if list.is_empty() {
        col = col.child(parts::empty(t!("В этот день учёт не вёлся или трафика не было.")));
    }
    let range = if sel == today { Range::Today } else { Range::Day(sel.clone()) };
    for r in list.iter().take(if sel == today { 5 } else { 8 }) {
        col = col.child(app_row(cx, r, rmax, false, range.clone(), &uplinks, t.all_users));
    }
    // Журнал.
    let since = t.log_since.map(|ms| t!("с {d}", d = netapps::when_full(ms))).unwrap_or_else(|| t!("пока пуст"));
    let log = t!("Журнал соединений: {u} из {l} · {s}", u = parts::bytes(t.log_bytes), l = parts::bytes(t.log_limit), s = since);
    let dns = if t.dns { String::new() } else { t!(" · имена узлов недоступны (нет systemd-resolved)") };
    let sel2 = sel.clone();
    let title_day = day_title_for(&sel, &today);
    col = col.child(Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(parts::meter((t.log_bytes as f32 / t.log_limit.max(1) as f32 * 100.0).min(100.0), color, 4.0)).child(Text::new(format!("{log}{dns}")).max_lines(2).class("muted small")));
    let mut foot = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(button(t!("Соединения за день"), icons::HISTORY, move || {
        let r = if sel2 == crate::sampler::day_str(netapps::now_ms() / 1000) { Range::Today } else { Range::Day(sel2.clone()) };
        conns::open(None, title_day.clone(), r)
    }));
    foot = foot.child(DecoratedBox::new().class("grow"));
    if t.all_users {
        foot = foot.child(button(t!("Настройки учёта"), icons::SETTINGS, open_settings));
    }
    col = col.child(foot.class("net-foot"));
    boxed(parts::card(col))
}

/// Расход дня по часам: 24 столбика, подписи 0 · 6 · 12 · 18; текущий час выделен.
fn hour_chart(hours: &[u64; 24], cur: Option<usize>, h: f32) -> W {
    let max = hours.iter().copied().max().unwrap_or(1).max(1) as f32;
    let mut bars = Row::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::End);
    for (i, v) in hours.iter().enumerate() {
        let bh = (*v as f32 / max * h).max(if *v > 0 { 1.5 } else { 0.0 });
        let c = if Some(i) == cur { "net-hour cur" } else { "net-hour" };
        bars = bars.child(DecoratedBox::new().class(c).style("height", StyleValue::px(bh)).class("grow"));
    }
    let mut labels = Row::new();
    for l in ["0", "6", "12", "18", "24"] {
        labels = labels.child(Text::new(l).class("axis grow"));
    }
    boxed(
        Column::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(DecoratedBox::new().child(bars).style("height", StyleValue::px(h)))
            .child(Row::new().child(Text::new(t!("по часам · в самый загруженный час {m}", m = parts::bytes(max as u64))).max_lines(1).class("axis grow")))
            .child(labels),
    )
}

fn day_title_for(day: &str, today: &str) -> String {
    if day == today {
        return t!("Все программы — сегодня");
    }
    let (dd, wd) = netapps::day_label(day);
    t!("Все программы — {d}", d = format!("{dd}, {wd}"))
}

/// Почему нет учёта по программам.
fn netd_missing(d: &NetD) -> Option<W> {
    let text = match &d.traffic {
        Some(Ok(t)) if t.enabled => return None,
        Some(Ok(t)) => match &t.error {
            Some(e) => t!("Учёт трафика по программам не работает: {e}", e = e),
            None => t!("Учёт трафика по программам выключен."),
        },
        Some(Err(_)) if !d.installed => t!("Трафик по программам, журнал соединений и архив ведёт служба synnetd — она не установлена."),
        Some(Err(e)) if e.contains("обновите") => t!("Служба synnetd старой версии — обновите пакет, чтобы видеть трафик по программам."),
        Some(Err(_)) => t!("Служба synnetd не запущена — трафик по программам, журнал соединений и архив недоступны."),
        None => return None,
    };
    let mut col = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(parts::card_head(icons::APPS, t!("Программы"), None, Page::Network.color())).child(Text::new(text).max_lines(4).class("muted"));
    if d.installed {
        col = col.child(Row::new().gap(8.0).child(button(t!("Открыть настройки"), icons::SETTINGS, open_settings)));
    }
    Some(boxed(parts::card(col)))
}

pub fn network(cx: &Cx, s: &Snap) -> Vec<W> {
    let color = Page::Network.color();
    let (rx_c, tx_c) = (Color::from_hex(RX), Color::from_hex(TX));
    let status = s.netd.status.as_ref().and_then(|r| r.as_ref().ok());
    let sub = status.filter(|st| st.uplinks.len() > 1).map(|st| {
        let mode = if st.config.mode == syn_netd::Mode::Both { t!("все каналы сразу") } else { t!("основной канал {i}", i = st.uplinks.iter().find(|u| u.primary).map(|u| u.iface.clone()).unwrap_or_default()) };
        t!("Каналов в интернет: {n} · {m}", n = st.uplinks.len(), m = mode)
    });
    let mut out = vec![header(cx, Page::Network, sub, true)];
    let rx = s.hist.rx.tail(cx.span);
    let tx = s.hist.tx.tail(cx.span);
    let max = parts::nice_max(rx.iter().chain(tx.iter()).copied().fold(0.0, f32::max), 64.0 * 1024.0);
    let (total_rx, total_tx) = s.net.iter().fold((0u64, 0u64), |a, i| (a.0 + i.info.rx_bytes, a.1 + i.info.tx_bytes));
    let mut legend = vec![parts::legend(rx_c, t!("Принято с загрузки"), parts::bytes(total_rx)), parts::legend(tx_c, t!("Передано с загрузки"), parts::bytes(total_tx))];
    let traffic = s.netd.traffic.as_ref().and_then(|r| r.as_ref().ok()).filter(|t| t.enabled);
    if let Some(t) = traffic {
        let (r, x) = t.apps.iter().fold((0, 0), |a, p| (a.0 + p.rx_today, a.1 + p.tx_today));
        legend.push(parts::legend(color, t!("Сегодня (с полуночи)"), format!("↓ {}  ↑ {}", parts::bytes(r), parts::bytes(x))));
    }
    out.push(hero(cx, color, format!("↓ {}", parts::rate(s.rx())), format!("↑ {} · {}", parts::rate(s.tx()), t!("шкала {m}", m = parts::rate(max as f64))), vec![Line::new(rx, rx_c), Line::new(tx, tx_c)], max, legend));
    match traffic {
        Some(t) => out.push(apps_card(cx, &s.netd, t)),
        None => out.extend(netd_missing(&s.netd)),
    }
    out.extend(ifaces(cx, s));
    if let Some(t) = traffic {
        out.push(archive_card(cx, &s.netd, t));
    }
    let down: Vec<String> = s.net.iter().filter(|i| !i.info.up && i.info.kind != "virtual").map(|i| i.info.name.clone()).collect();
    if !down.is_empty() {
        out.push(parts::empty(t!("Отключены: {l}", l = down.join(", "))));
    }
    out
}
